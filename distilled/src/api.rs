// OpenRouter istemcisi: chat/completions streaming (SSE), reasoning (thinking),
// tool_calls akisi, model listesi.
// SSE kurallari (OpenRouter docs): ':' ile baslayan yorumlar keep-alive,
// 'data: [DONE]' son, son chunk usage tasir ve finish_reason'i tekrarlar,
// stream ici hata HTTP 200 ile 'error' alaninda gelir.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::BufRead;
use std::time::Duration;

pub const BASE: &str = "https://openrouter.ai/api/v1";
pub const REFERER: &str = "https://github.com/aurion";

// Test/yerel sunucu icin taban adresi degistirilebilir.
fn base() -> String {
    std::env::var("SYNAPSE_API_BASE").unwrap_or_else(|_| BASE.to_string())
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Fn {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "fn_kind")]
    pub kind: String,
    pub function: Fn,
}

fn fn_kind() -> String {
    "function".into()
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn sys(c: &str) -> Message {
        Message {
            role: "system".into(),
            content: c.into(),
            ..Default::default()
        }
    }
    pub fn user(c: &str) -> Message {
        Message {
            role: "user".into(),
            content: c.into(),
            ..Default::default()
        }
    }
    pub fn tool_result(id: &str, name: &str, out: &str) -> Message {
        Message {
            role: "tool".into(),
            content: out.into(),
            tool_call_id: Some(id.into()),
            name: Some(name.into()),
            ..Default::default()
        }
    }
    pub fn approx_tokens(&self) -> u32 {
        let mut n = self.content.len() as u32 / 4;
        if let Some(r) = &self.reasoning {
            n += r.len() as u32 / 4;
        }
        for t in &self.tool_calls {
            n += t.function.arguments.len() as u32 / 4 + 8;
        }
        n + 4
    }
}

#[derive(Clone, Debug)]
pub enum Ev {
    Reasoning(String),
    Text(String),
    ToolCall(ToolCall),
    Usage { prompt: u32, completion: u32 },
    Failed(String),
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(900)))
        .timeout_connect(Some(Duration::from_secs(30)))
        .build()
        .new_agent()
}

fn auth(key: &str) -> String {
    format!("Bearer {}", key.trim())
}

pub fn models(key: &str) -> Result<Vec<(String, String)>, String> {
    let r = agent()
        .get(&format!("{}/models", base()))
        .header("Authorization", &auth(key))
        .call()
        .map_err(|e| e.to_string())?;
    let v: Value = r.into_body().read_json().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(arr) = v.get("data").and_then(|d| d.as_array()) {
        for m in arr {
            let id = m
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let name = m
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or(&id)
                .to_string();
            if !id.is_empty() {
                out.push((id, name));
            }
        }
    }
    out.sort();
    Ok(out)
}

struct Acc {
    id: String,
    name: String,
    args: String,
}

pub fn chat_stream(
    key: &str,
    model: &str,
    messages: &[Message],
    tools: Option<&Value>,
    mut on_ev: impl FnMut(Ev),
) {
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "usage": { "include": true },
    });
    if let Some(t) = tools {
        body["tools"] = t.clone();
    }
    let resp = agent()
        .post(&format!("{}/chat/completions", base()))
        .header("Authorization", &auth(key))
        .header("Content-Type", "application/json")
        .header("HTTP-Referer", REFERER)
        .header("X-Title", "synapse-distilled")
        .send_json(&body);
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            on_ev(Ev::Failed(e.to_string()));
            return;
        }
    };
    let mut reader = std::io::BufReader::new(resp.into_body().into_reader());
    let mut acc: Vec<Acc> = Vec::new();
    let (mut ptok, mut ctok) = (0u32, 0u32);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let t = line.trim_end();
        if t.is_empty() || t.starts_with(':') {
            continue;
        }
        let Some(payload) = t.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        if let Some(e) = v.get("error") {
            let msg = e
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string();
            on_ev(Ev::Failed(msg));
            return;
        }
        if let Some(u) = v.get("usage") {
            if let Some(p) = u.get("prompt_tokens").and_then(|x| x.as_u64()) {
                ptok = p as u32;
            }
            if let Some(c) = u.get("completion_tokens").and_then(|x| x.as_u64()) {
                ctok = c as u32;
            }
        }
        let Some(delta) = v.pointer("/choices/0/delta") else {
            continue;
        };
        if let Some(r) = delta.get("reasoning").and_then(|x| x.as_str()) {
            if !r.is_empty() {
                on_ev(Ev::Reasoning(r.to_string()));
            }
        }
        if let Some(c) = delta.get("content").and_then(|x| x.as_str()) {
            if !c.is_empty() {
                on_ev(Ev::Text(c.to_string()));
            }
        }
        if let Some(list) = delta.get("tool_calls").and_then(|x| x.as_array()) {
            for tc in list {
                let idx = tc.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                while acc.len() <= idx {
                    acc.push(Acc {
                        id: String::new(),
                        name: String::new(),
                        args: String::new(),
                    });
                }
                let slot = &mut acc[idx];
                if let Some(id) = tc.get("id").and_then(|x| x.as_str()) {
                    slot.id = id.to_string();
                }
                if let Some(f) = tc.get("function") {
                    if let Some(n) = f.get("name").and_then(|x| x.as_str()) {
                        slot.name = n.to_string();
                    }
                    if let Some(a) = f.get("arguments").and_then(|x| x.as_str()) {
                        slot.args.push_str(a);
                    }
                }
            }
        }
    }
    for a in acc {
        if a.name.is_empty() {
            continue;
        }
        on_ev(Ev::ToolCall(ToolCall {
            id: if a.id.is_empty() {
                format!("call_{}", a.name)
            } else {
                a.id
            },
            kind: "function".into(),
            function: Fn {
                name: a.name,
                arguments: a.args,
            },
        }));
    }
    on_ev(Ev::Usage {
        prompt: ptok,
        completion: ctok,
    });
}
