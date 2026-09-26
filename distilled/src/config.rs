// Ayar ve API anahtarı kalıcılığı.
// Windows: %APPDATA%\synapse-distilled\   Linux/macOS: ~/.config/synapse-distilled/
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP: &str = "synapse-distilled";
pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.5";
pub const MAX_ITER: usize = 40;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub api_key: String,
    pub model: String,
    #[serde(default = "d_ctx")]
    pub context_limit: u32,
    #[serde(default = "d_true")]
    pub tools_enabled: bool,
    #[serde(default = "d_true")]
    pub confirm_writes: bool,
    #[serde(default = "d_cwd")]
    pub workspace: String,
    #[serde(default)]
    pub session: String,
}

fn d_ctx() -> u32 {
    120_000
}
fn d_true() -> bool {
    true
}
fn d_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".into())
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_key: String::new(),
            model: DEFAULT_MODEL.into(),
            context_limit: d_ctx(),
            tools_enabled: true,
            confirm_writes: true,
            workspace: d_cwd(),
            session: String::new(),
        }
    }
}

pub fn dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(a) = std::env::var("APPDATA") {
            if !a.is_empty() {
                return PathBuf::from(a).join(APP);
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(h) = std::env::var("HOME") {
            if !h.is_empty() {
                return PathBuf::from(h).join(".config").join(APP);
            }
        }
    }
    PathBuf::from(".").join(".synapse-distilled")
}

pub fn path() -> PathBuf {
    dir().join("config.json")
}

pub fn load() -> Config {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(c: &Config) -> std::io::Result<()> {
    let d = dir();
    std::fs::create_dir_all(&d)?;
    let tmp = d.join("config.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(c).unwrap_or_default())?;
    std::fs::rename(&tmp, d.join("config.json"))
}

// Anahtar bos degilse gecerli sayilir.
pub fn has_key(c: &Config) -> bool {
    c.api_key.trim().len() > 10
}
