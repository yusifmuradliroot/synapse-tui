// Ajan orkestratörü: worker thread'te API turu + tool calistirma dongusu.
// Main thread ile tek yonlu olay akisi (cmd alir, wevent yayar).
use crate::api::{self, Message, ToolCall};
use crate::config::MAX_ITER;
use crate::session::Session;
use crate::tools;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AState {
    Waiting,
    Thinking,
    Working,
}

#[derive(Debug)]
pub enum WEvent {
    State(AState),
    Reasoning(String),
    Text(String),
    Say(String),
    ToolStart {
        name: String,
        args: String,
    },
    ToolEnd {
        name: String,
        ok: bool,
        line: String,
    },
    Usage {
        prompt: u32,
        completion: u32,
    },
    Failed(String),
    Compacted(usize),
    Sess(Box<Session>),
    Done,
}

#[derive(Debug)]
pub enum Cmd {
    Ask { text: String, sess: Session },
    Compact { sess: Session },
    Shutdown,
}

pub fn system_prompt(ws: &Path) -> String {
    format!(
        "You are Synapse Distilled, an expert software engineering agent running inside a \
terminal on the user's own machine.\n\n\
Environment:\n- OS: {os}/{arch}\n- Workspace: {ws}\n- Today (unix seconds): {now}\n\n\
Operating rules:\n\
1. You have tools. Use them instead of guessing: always read a file before editing it.\n\
2. Prefer edit_file for existing files; write_file only for new files or full rewrites.\n\
3. Keep responses short and factual. No filler, no restating the request, no summary of \
what you are about to do before doing it.\n\
4. Never invent file contents, paths or command output. If you did not run it, you do not know.\n\
5. Group related tool calls, then report the result in one or two sentences.\n\
6. When the task is done, stop. Do not offer further work unless asked.\n\
7. Plain text output. Short markdown lists are fine; no tables.\n\
8. If a request is ambiguous in a way that changes the result, ask one short question, \
otherwise make the reasonable choice and say which one you made.\n\
9. Destructive or irreversible actions (deleting files, force push, wiping data) require an \
explicit user request in the conversation.",
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        ws = ws.display(),
        now = crate::session::now()
    )
}

fn tool_label(name: &str, args_json: &str) -> String {
    let v: Value = serde_json::from_str(args_json).unwrap_or(Value::Null);
    let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("");
    match name {
        "read_file" => format!("read {}", g("path")),
        "write_file" => format!("write {}", g("path")),
        "edit_file" => format!("edit {}", g("path")),
        "list_dir" => format!(
            "list {}",
            if g("path").is_empty() { "." } else { g("path") }
        ),
        "run_command" => {
            let c = g("command");
            if c.chars().count() > 60 {
                format!("run {}...", c.chars().take(60).collect::<String>())
            } else {
                format!("run {c}")
            }
        }
        "search" => format!("search \"{}\"", g("pattern")),
        "git" => format!("git {}", g("args")),
        "system_info" => "system info".into(),
        other => other.to_string(),
    }
}

pub fn spawn(
    cmd_rx: mpsc::Receiver<Cmd>,
    key: String,
    model: String,
    ws: PathBuf,
    tools_on: bool,
) -> mpsc::Receiver<WEvent> {
    let (tx, rx) = mpsc::channel::<WEvent>();
    std::thread::spawn(move || loop {
        let Ok(cmd) = cmd_rx.recv() else {
            return;
        };
        match cmd {
            Cmd::Shutdown => return,
            Cmd::Compact { mut sess } => {
                let _ = tx.send(WEvent::State(AState::Thinking));
                let msgs = sess.messages.len();
                match crate::session::compact(&key, &model, &mut sess) {
                    Ok(_) => {
                        let _ = tx.send(WEvent::Compacted(msgs));
                    }
                    Err(e) => {
                        let _ = tx.send(WEvent::Failed(e));
                    }
                }
                let _ = tx.send(WEvent::Sess(Box::new(sess)));
                let _ = tx.send(WEvent::State(AState::Waiting));
                let _ = tx.send(WEvent::Done);
            }
            Cmd::Ask { text, mut sess } => {
                if text.trim().is_empty() {
                    continue;
                }
                sess.messages.push(Message::user(text.trim()));
                run_turn(&tx, &key, &model, &ws, tools_on, &mut sess);
                let _ = tx.send(WEvent::Sess(Box::new(sess)));
                let _ = tx.send(WEvent::State(AState::Waiting));
                let _ = tx.send(WEvent::Done);
            }
        }
    });
    rx
}

fn run_turn(
    tx: &mpsc::Sender<WEvent>,
    key: &str,
    model: &str,
    ws: &Path,
    tools_on: bool,
    sess: &mut Session,
) {
    let sys = system_prompt(ws);
    let tool_schema = if tools_on { Some(tools::all()) } else { None };
    for iter in 0..MAX_ITER {
        // Sistem + varsa ozet + gecmis
        let mut msgs: Vec<Message> = Vec::new();
        msgs.push(Message::sys(&sys));
        if let Some(sm) = &sess.summary {
            msgs.push(Message::sys(&format!(
                "Summary of earlier conversation (compacted):\n{sm}"
            )));
        }
        msgs.extend(sess.messages.iter().cloned());

        let _ = tx.send(WEvent::State(AState::Thinking));
        let mut reasoning = String::new();
        let mut content = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut err: Option<String> = None;
        api::chat_stream(key, model, &msgs, tool_schema.as_ref(), |ev| match ev {
            api::Ev::Reasoning(r) => {
                reasoning.push_str(&r);
                let _ = tx.send(WEvent::Reasoning(r));
            }
            api::Ev::Text(t) => {
                content.push_str(&t);
                let _ = tx.send(WEvent::Text(t));
            }
            api::Ev::ToolCall(c) => calls.push(c),
            api::Ev::Usage { prompt, completion } => {
                sess.tok_prompt = sess.tok_prompt.max(prompt);
                sess.tok_completion += completion;
                let _ = tx.send(WEvent::Usage { prompt, completion });
            }
            api::Ev::Failed(e) => err = Some(e),
        });
        if let Some(e) = err {
            let _ = tx.send(WEvent::Failed(e));
            return;
        }
        if !content.trim().is_empty() {
            let _ = tx.send(WEvent::Say(content.trim().to_string()));
        }
        let mut asst = Message {
            role: "assistant".into(),
            content: content.clone(),
            ..Default::default()
        };
        if !reasoning.trim().is_empty() {
            asst.reasoning = Some(reasoning.trim().to_string());
        }
        if !calls.is_empty() {
            asst.tool_calls = calls.clone();
        }
        sess.messages.push(asst);

        if calls.is_empty() || !tools_on {
            if calls.is_empty() {
                return;
            }
            // Tool'lar kapali: model yine de cagri yaptiysa birak.
            for c in &calls {
                let out = "error: tools are disabled in this session (/tools to enable)";
                sess.messages
                    .push(Message::tool_result(&c.id, &c.function.name, out));
            }
            return;
        }

        // Tool calistir (bloklayici; bu thread worker).
        for c in &calls {
            let label = tool_label(&c.function.name, &c.function.arguments);
            let _ = tx.send(WEvent::State(AState::Working));
            let _ = tx.send(WEvent::ToolStart {
                name: c.function.name.clone(),
                args: label,
            });
            let out = tools::run(&c.function.name, &c.function.arguments, ws);
            let ok = !out.starts_with("error:");
            let line = out
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .chars()
                .take(110)
                .collect::<String>();
            let _ = tx.send(WEvent::ToolEnd {
                name: c.function.name.clone(),
                ok,
                line,
            });
            sess.messages
                .push(Message::tool_result(&c.id, &c.function.name, &out));
        }
        let _ = iter;
    }
}
