// Oturum kaliciligi + context yonetimi (/compact, token hesabi).
use crate::api::{self, Message};
use crate::config;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    pub model: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tok_prompt: u32,
    #[serde(default)]
    pub tok_completion: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Meta {
    pub id: String,
    pub title: String,
    pub updated: u64,
    pub messages: usize,
    pub tokens: u32,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sessions_dir() -> PathBuf {
    config::dir().join("sessions")
}

pub fn new_id() -> String {
    let t = now();
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{}-{:04}", t, s % 10000)
}

pub fn new_session(model: &str) -> Session {
    Session {
        id: new_id(),
        title: "new session".into(),
        created: now(),
        updated: now(),
        model: model.into(),
        summary: None,
        messages: Vec::new(),
        tok_prompt: 0,
        tok_completion: 0,
    }
}

fn file_of(id: &str) -> PathBuf {
    sessions_dir().join(format!("{id}.json"))
}

pub fn save(s: &Session) -> std::io::Result<()> {
    let d = sessions_dir();
    std::fs::create_dir_all(&d)?;
    let json = serde_json::to_string_pretty(s).unwrap_or_default();
    let tmp = d.join(format!("{}.tmp", s.id));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, file_of(&s.id))
}

pub fn load(id: &str) -> Option<Session> {
    std::fs::read_to_string(file_of(id))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

pub fn list() -> Vec<Meta> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        for e in rd.filter_map(|e| e.ok()) {
            if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Ok(txt) = std::fs::read_to_string(e.path()) {
                if let Ok(s) = serde_json::from_str::<Session>(&txt) {
                    out.push(Meta {
                        tokens: used_tokens(&s, 120_000),
                        messages: s.messages.len(),
                        id: s.id,
                        title: s.title,
                        updated: s.updated,
                    });
                }
            }
        }
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.updated));
    out
}

pub fn remove(id: &str) -> bool {
    std::fs::remove_file(file_of(id)).is_ok()
}

// Baglam kullanimi: ozet varsa ozet + son mesajlar, yoksa tum mesajlar.
pub fn used_tokens(s: &Session, _limit: u32) -> u32 {
    let sum: u32 = s.messages.iter().map(|m| m.approx_tokens()).sum();
    let summ = s
        .summary
        .as_ref()
        .map(|x| x.len() as u32 / 4 + 8)
        .unwrap_or(0);
    summ + sum
}

pub const KEEP_AFTER_COMPACT: usize = 6;

const COMPACT_PROMPT: &str = "\
Summarize the following coding session so it can replace the raw history.
Keep, in this order:
1. User goal and explicit requirements (verbatim where important).
2. Decisions made and their reasons.
3. Files created/modified/read, with paths.
4. Commands run and their outcomes (especially failures and fixes).
5. Code patterns, APIs, signatures, schemas that must stay consistent.
6. Open questions, next steps, pending work.
Be specific and factual. No pleasantries. Use short sections with bullets. \
This summary is the only memory that survives compaction.";

/// /compact: gecmisi ozetleyip korunacak son mesajlari birakiyor.
/// Ozet metni dondurur (cagiran yazar).
pub fn compact(p: &api::Prov, model: &str, s: &mut Session) -> Result<String, String> {
    if s.messages.len() <= KEEP_AFTER_COMPACT {
        return Err("history is too short to compact".into());
    }
    let mut convo = String::new();
    for m in &s.messages {
        let who = match m.role.as_str() {
            "user" => "USER",
            "assistant" => "ASSISTANT",
            "tool" => "TOOL RESULT",
            _ => "SYSTEM",
        };
        let body = if m.content.len() > 4000 {
            format!("{}...[cut]", &m.content[..4000])
        } else {
            m.content.clone()
        };
        convo.push_str(&format!("[{who}]\n{body}\n\n"));
    }
    let msgs = vec![Message::sys(COMPACT_PROMPT), Message::user(&convo)];
    let mut out = String::new();
    api::chat_stream(p, model, &msgs, None, |ev| match ev {
        api::Ev::Text(t) => out.push_str(&t),
        api::Ev::Failed(e) => out = format!("__ERR__{e}"),
        _ => {}
    });
    if out.starts_with("__ERR__") {
        return Err(out.trim_start_matches("__ERR__").to_string());
    }
    let keep_from = s.messages.len() - KEEP_AFTER_COMPACT;
    let kept: Vec<Message> = s.messages[keep_from..].to_vec();
    let sum = out.trim().to_string();
    s.summary = Some(sum.clone());
    s.messages = kept;
    s.updated = now();
    Ok(sum)
}
