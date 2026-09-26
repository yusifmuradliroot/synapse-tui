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
    AskUser {
        id: u32,
        label: String,
        question: String,
        options: Vec<String>,
        buttons: bool,
    },
    PermSet {
        tool: String,
        value: String,
    },
    Sess(Box<Session>),
    Done,
}

#[derive(Debug)]
pub enum Cmd {
    Ask {
        text: String,
        sess: Session,
        prov: api::Prov,
        model: String,
        cfg: crate::config::Config,
    },
    Compact {
        sess: Session,
        prov: api::Prov,
        model: String,
    },
    AskReply {
        id: u32,
        text: String,
    },
    Shutdown,
}

pub fn system_prompt(ws: &Path) -> String {
    let mut mem = String::new();
    for cand in [
        ws.join(".synapse").join("MEMORY.md"),
        crate::config::dir().join("MEMORY.md"),
    ] {
        if let Ok(t) = std::fs::read_to_string(&cand) {
            let cut = t.floor_char_boundary(4000.min(t.len()));
            mem.push_str(&format!(
                "\n--- memory ({}) ---\n{}\n",
                cand.display(),
                &t[..cut]
            ));
        }
    }
    format!(
        "You are Synapse Distilled, an expert software engineering agent running inside a \
terminal on the user's own machine.\n\n\
Environment:\n- OS: {os}/{arch}\n- Workspace: {ws}\n- Today (unix seconds): {now}\n\
{mem}\n\
Operating rules:\n\
1. You have tools. Use them instead of guessing: glob to find files, read a file before editing it.\n\
2. For multi-step work, plan first with todowrite, then execute, marking progress.\n\
3. Prefer edit_file for existing files; write_file only for new files or full rewrites. \
apply_patch is good for multi-file diffs.\n\
4. Keep responses short and factual. No filler, no restating the request.\n\
5. Never invent file contents, paths or command output. If you did not run it, you do not know.\n\
6. Group related tool calls, then report the result in one or two sentences.\n\
7. When the task is done, stop. Do not offer further work unless asked.\n\
8. Plain text output. Short markdown lists are fine; no tables.\n\
9. If a request is ambiguous in a way that changes the result, use ask_user to clarify \
instead of guessing.\n\
10. Destructive or irreversible actions require an explicit user request. Long-running \
commands belong in run_background so you can keep working.\n\
11. Project conventions live in skills (skill tool) and MEMORY.md; follow them.",
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        ws = ws.display(),
        now = crate::session::now()
    )
}

fn tool_label(name: &str, args_json: &str) -> String {
    let v: Value = serde_json::from_str(args_json).unwrap_or(Value::Null);
    let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("");
    let gi = |k: &str| {
        v.get(k)
            .and_then(|x| {
                x.as_u64()
                    .map(|n| n.to_string())
                    .or_else(|| x.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default()
    };
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
        "glob" => format!("glob {}", g("pattern")),
        "apply_patch" => "apply patch".into(),
        "skill" => format!("skill {}", g("name")),
        "webfetch" => format!("fetch {}", g("url")),
        "todowrite" => "update todos".into(),
        "todoread" => "read todos".into(),
        "run_background" => format!("background: {}", g("command")),
        "job_output" => format!("job {} output", gi("id")),
        "job_stop" => format!("stop job {}", gi("id")),
        "ask_user" => format!("ask: {}", g("question")),
        "git" => format!("git {}", g("args")),
        "system_info" => "system info".into(),
        other => other.to_string(),
    }
}

// Arka plan isleri (oturumlar arasi yasar: worker thread'e ait).
struct Job {
    child: std::process::Child,
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    cursor: usize,
    done: bool,
}

struct Rt {
    jobs: std::collections::HashMap<u32, Job>,
    next_job: u32,
    pending: std::collections::VecDeque<Cmd>,
    ask_id: u32,
}

impl Rt {
    fn new() -> Rt {
        Rt {
            jobs: std::collections::HashMap::new(),
            next_job: 1,
            pending: std::collections::VecDeque::new(),
            ask_id: 1,
        }
    }
}

// Tek tool calistirma: izin + yonlendirme (background/ask_user ozel).
fn exec_tool(
    tx: &mpsc::Sender<WEvent>,
    cmd_rx: &mpsc::Receiver<Cmd>,
    rt: &mut Rt,
    cfg: &crate::config::Config,
    ws: &Path,
    name: &str,
    args_json: &str,
) -> String {
    match name {
        "run_background" | "job_output" | "job_stop" => return bg_tool(rt, ws, name, args_json),
        "ask_user" => {
            let v: Value = serde_json::from_str(args_json).unwrap_or(Value::Null);
            let q = v
                .get("question")
                .and_then(|x| x.as_str())
                .unwrap_or("?")
                .to_string();
            let opts: Vec<String> = v
                .get("options")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .take(6)
                        .collect()
                })
                .unwrap_or_default();
            rt.ask_id += 1;
            let id = rt.ask_id;
            return match ask_round(
                tx,
                cmd_rx,
                &mut rt.pending,
                id,
                "agent question",
                &q,
                opts,
                false,
            ) {
                Some(t) if !t.trim().is_empty() => format!("user answer: {}", t.trim()),
                _ => "user gave no answer".into(),
            };
        }
        _ => {}
    }
    // izin denetimi
    let perm = crate::config::perm(cfg, name);
    if std::env::var("SYNAPSE_DBG").is_ok() {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/perm_dbg.log")
        {
            let _ = writeln!(
                f,
                "TOOL {name} perm={perm} has_map={}",
                cfg.permissions.contains_key(name)
            );
        }
    }
    if perm == "deny" {
        return format!("denied by permissions ({name} is disabled)");
    }
    if perm == "ask" {
        rt.ask_id += 1;
        let id = rt.ask_id;
        let v: Value = serde_json::from_str(args_json).unwrap_or(Value::Null);
        let detail = match name {
            "write_file" | "edit_file" | "apply_patch" => v
                .get("path")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            "run_command" | "run_background" => v
                .get("command")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .chars()
                .take(120)
                .collect(),
            _ => String::new(),
        };
        let q = if detail.is_empty() {
            format!("Allow the agent to run {name}?")
        } else {
            format!("Allow the agent to run {name} ({detail})?")
        };
        let ans = ask_round(
            tx,
            cmd_rx,
            &mut rt.pending,
            id,
            "permission",
            &q,
            vec!["allow once".into(), "always allow".into(), "deny".into()],
            true,
        );
        let a = ans.unwrap_or_default().trim().to_lowercase();
        if a.starts_with("always") {
            let _ = tx.send(WEvent::PermSet {
                tool: name.to_string(),
                value: "allow".into(),
            });
        } else if !(a == "allow" || a == "allow once" || a == "1" || a == "yes" || a == "y") {
            return format!("denied by user ({name} not run)");
        }
    }
    tools::run(name, args_json, ws)
}

fn bg_tool(rt: &mut Rt, ws: &Path, name: &str, args_json: &str) -> String {
    use std::io::Read;
    let v: Value = serde_json::from_str(args_json).unwrap_or(Value::Null);
    let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("");
    let gi = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    match name {
        "run_background" => {
            let cmd = g("command");
            if cmd.trim().is_empty() {
                return "error: empty command".into();
            }
            let mut c = if cfg!(windows) {
                let mut c = std::process::Command::new("cmd");
                c.arg("/C").arg(cmd);
                c
            } else {
                let mut c = std::process::Command::new("sh");
                c.arg("-c").arg(cmd);
                c
            };
            c.current_dir(ws)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = match c.spawn() {
                Ok(c) => c,
                Err(e) => return format!("error: cannot start: {e}"),
            };
            let (btx, brx) = mpsc::channel::<Vec<u8>>();
            if let Some(mut o) = child.stdout.take() {
                let b2 = btx.clone();
                std::thread::spawn(move || {
                    let mut b = Vec::new();
                    let _ = o.read_to_end(&mut b);
                    let _ = b2.send(b);
                });
            }
            if let Some(mut e) = child.stderr.take() {
                std::thread::spawn(move || {
                    let mut b = Vec::new();
                    let _ = e.read_to_end(&mut b);
                    let _ = btx.send(b);
                });
            }
            let id = rt.next_job;
            rt.next_job += 1;
            rt.jobs.insert(
                id,
                Job {
                    child,
                    rx: brx,
                    buf: Vec::new(),
                    cursor: 0,
                    done: false,
                },
            );
            format!("started background job {id}")
        }
        "job_output" => {
            let id = gi("id") as u32;
            let lim = if gi("limit") == 0 {
                8000
            } else {
                gi("limit").clamp(200, 60000)
            };
            let Some(j) = rt.jobs.get_mut(&id) else {
                return format!("error: no such job {id}");
            };
            while let Ok(b) = j.rx.try_recv() {
                if j.buf.len() < 200_000 {
                    j.buf.extend_from_slice(&b);
                }
            }
            if !j.done {
                match j.child.try_wait() {
                    Ok(Some(st)) => {
                        j.done = true;
                        while let Ok(b) = j.rx.try_recv() {
                            if j.buf.len() < 200_000 {
                                j.buf.extend_from_slice(&b);
                            }
                        }
                        j.buf.extend_from_slice(
                            format!("\n[exit {}]", st.code().unwrap_or(-1)).as_bytes(),
                        );
                    }
                    Ok(None) => {}
                    Err(_) => j.done = true,
                }
            }
            let from = j.cursor.min(j.buf.len());
            let to = (from + lim).min(j.buf.len());
            j.cursor = to;
            let mut s = String::from_utf8_lossy(&j.buf[from..to]).into_owned();
            if s.trim().is_empty() {
                s = if j.done {
                    "(job finished, no new output)".into()
                } else {
                    "(no new output yet)".into()
                };
            } else if to < j.buf.len() {
                s.push_str("\n... [more buffered]");
            }
            s
        }
        "job_stop" => {
            let id = gi("id") as u32;
            let Some(mut j) = rt.jobs.remove(&id) else {
                return format!("error: no such job {id}");
            };
            let _ = j.child.kill();
            let _ = j.child.wait();
            format!("stopped job {id}")
        }
        _ => "error: unknown background tool".into(),
    }
}

// Kullaniciya soru sor, cevabi bekle. None = vazgecildi/kapatildi.
fn ask_round(
    tx: &mpsc::Sender<WEvent>,
    cmd_rx: &mpsc::Receiver<Cmd>,
    pending: &mut std::collections::VecDeque<Cmd>,
    id: u32,
    label: &str,
    question: &str,
    options: Vec<String>,
    buttons: bool,
) -> Option<String> {
    let _ = tx.send(WEvent::State(AState::Waiting));
    let _ = tx.send(WEvent::AskUser {
        id,
        label: label.to_string(),
        question: question.to_string(),
        options,
        buttons,
    });
    loop {
        match cmd_rx.recv() {
            Ok(Cmd::AskReply { id: i, text }) if i == id => return Some(text),
            Ok(Cmd::Shutdown) => return None,
            Ok(other) => pending.push_back(other),
            Err(_) => return None,
        }
    }
}

pub fn spawn(cmd_rx: mpsc::Receiver<Cmd>, ws: PathBuf) -> mpsc::Receiver<WEvent> {
    let (tx, rx) = mpsc::channel::<WEvent>();
    std::thread::spawn(move || {
        let mut rt = Rt::new();
        loop {
            let cmd = rt.pending.pop_front().or_else(|| cmd_rx.recv().ok());
            let Some(cmd) = cmd else {
                return;
            };
            match cmd {
                Cmd::Shutdown => return,
                Cmd::Compact {
                    mut sess,
                    prov,
                    model,
                } => {
                    let _ = tx.send(WEvent::State(AState::Thinking));
                    let msgs = sess.messages.len();
                    match crate::session::compact(&prov, &model, &mut sess) {
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
                Cmd::Ask {
                    text,
                    mut sess,
                    prov,
                    model,
                    cfg,
                } => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    sess.messages.push(Message::user(text.trim()));
                    run_turn(&tx, &cmd_rx, &mut rt, &prov, &model, &ws, &cfg, &mut sess);
                    let _ = tx.send(WEvent::Sess(Box::new(sess)));
                    let _ = tx.send(WEvent::State(AState::Waiting));
                    let _ = tx.send(WEvent::Done);
                }
                Cmd::AskReply { .. } => {
                    // bekleyen soru yoksa yoksay
                }
            }
        }
    });
    rx
}

fn run_turn(
    tx: &mpsc::Sender<WEvent>,
    cmd_rx: &mpsc::Receiver<Cmd>,
    rt: &mut Rt,
    prov: &api::Prov,
    model: &str,
    ws: &Path,
    cfg: &crate::config::Config,
    sess: &mut Session,
) {
    let tools_on = cfg.tools_enabled;
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
        api::chat_stream(prov, model, &msgs, tool_schema.as_ref(), |ev| match ev {
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
            let out = exec_tool(
                tx,
                cmd_rx,
                rt,
                cfg,
                ws,
                &c.function.name,
                &c.function.arguments,
            );
            let ok = !out.starts_with("error:") && !out.starts_with("denied");
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
