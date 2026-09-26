// Ayar ve API anahtarı kalıcılığı.
// Windows: %APPDATA%\synapse-distilled\   Linux/macOS: ~/.config/synapse-distilled/
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP: &str = "synapse-distilled";
pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.5";
pub const DEFAULT_NV_MODEL: &str = "meta/llama-3.1-70b-instruct";
pub const MAX_ITER: usize = 40;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub nvidia_key: String,
    #[serde(default)]
    pub nv_model: String,
    #[serde(default)]
    pub or_model: String,
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
    #[serde(default)]
    pub permissions: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub custom_models: Vec<String>,
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
            provider: "openrouter".into(),
            nvidia_key: String::new(),
            nv_model: DEFAULT_NV_MODEL.into(),
            or_model: DEFAULT_MODEL.into(),
            context_limit: d_ctx(),
            tools_enabled: true,
            confirm_writes: true,
            workspace: d_cwd(),
            session: String::new(),
            custom_models: Vec::new(),
            permissions: std::collections::HashMap::new(),
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
    let mut c: Config = std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if c.provider.is_empty() {
        c.provider = "openrouter".into();
    }
    if c.or_model.is_empty() {
        c.or_model = if c.model.is_empty() {
            DEFAULT_MODEL.into()
        } else {
            c.model.clone()
        };
    }
    if c.nv_model.is_empty() {
        c.nv_model = DEFAULT_NV_MODEL.into();
    }
    if c.model.is_empty() {
        c.model = if c.provider == "nvidia" {
            c.nv_model.clone()
        } else {
            c.or_model.clone()
        };
    }
    c
}

pub fn save(c: &Config) -> std::io::Result<()> {
    let d = dir();
    std::fs::create_dir_all(&d)?;
    let tmp = d.join("config.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(c).unwrap_or_default())?;
    std::fs::rename(&tmp, d.join("config.json"))
}

// Aktif saglayici anahtari
pub fn active_key(c: &Config) -> String {
    if c.provider == "nvidia" {
        c.nvidia_key.clone()
    } else {
        c.api_key.clone()
    }
}

// Anahtar bos degilse gecerli sayilir (aktif saglayicinin anahtari).
pub fn has_key(c: &Config) -> bool {
    active_key(c).trim().len() > 10
}

pub fn key_hint(provider: &str) -> &'static str {
    if provider == "nvidia" {
        "nvapi-…"
    } else {
        "sk-or-…"
    }
}

// Izin: allow | ask | deny (yoksa varsayilan)
pub fn perm(c: &Config, tool: &str) -> String {
    if let Some(p) = c.permissions.get(tool) {
        return p.clone();
    }
    match tool {
        "read_file" | "list_dir" | "search" | "glob" | "system_info" | "git" | "skill"
        | "webfetch" | "todoread" | "todowrite" => "allow".into(),
        _ => {
            if c.confirm_writes {
                "ask".into()
            } else {
                "allow".into()
            }
        }
    }
}
