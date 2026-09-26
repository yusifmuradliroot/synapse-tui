// synapse-distilled: TUI AI ajan/orchestrator.
// Animasyon durum makinesi uygulama durumuna baglidir:
//   Waiting  -> static (Hold)      Thinking -> idle spin (Idle)
//   Working  -> wheel (Production)  Boot     -> intro reveal
#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/render.rs"]
mod render;
#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/screen.rs"]
mod screen;
#[allow(dead_code, unused_imports)]
mod term;

mod agent;
mod api;
mod config;
mod session;
mod tools;

use agent::{AState, Cmd, WEvent};
use config::Config;
use render::{Canvas, Solid};
use screen::{Screen, BG_BLACK};
use session::Session;
use std::f64::consts::TAU;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const NAME: &str = env!("CARGO_BIN_NAME");
const VER: &str = env!("CARGO_PKG_VERSION");
const STAR_N: i32 = 120;

// Intro (acilis) ve faz zamanlari
const T_BLANK: f64 = 0.9;
const T_SWEEP: f64 = 1.3;
const T_HOLD: f64 = 0.35;
const IDLE_R: f64 = 0.08;
const PARK_Y: f64 = -0.15;
const PROD_TILT: f64 = 0.18;
const WHEEL_CRUISE: f64 = 2.2;
const IDLE_W: f64 = 1.25663706144;
const FRAME: Duration = Duration::from_millis(33);

// ── faz (animasyon) ─────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Blank,
    Scan,
    ScanFill,
    Hold,
    Idle,
    Production,
}

fn angle_rate(a: f64) -> f64 {
    1.5 * (0.5 + (1.0 - a.cos().abs()))
}

fn clip_y(cv: &mut Canvas, lo: f64, hi: f64) {
    let n = cv.n;
    for y in 0..n {
        let py = y as f64 + 0.5;
        if py < lo || py >= hi {
            for x in 0..n {
                cv.px[(y * n + x) as usize] = 0;
            }
        }
    }
}

fn coin_face(cv: &mut Canvas, t: f64, ox: f64, oy: f64) {
    render::draw_solid(
        cv,
        &Solid {
            t,
            angle: Some(0.0),
            rim_gain: 1.0,
            wob_gain: 0.0,
            ox,
            oy,
            dark_bg: true,
            ..Solid::base(t)
        },
    );
}

fn mirror_x(cv: &mut Canvas) {
    let n = cv.n;
    for y in 0..n {
        for x in 0..(n + 1) / 2 {
            let mx = n - 1 - x;
            let m = cv.px[(y * n + x) as usize].max(cv.px[(y * n + mx) as usize]);
            cv.px[(y * n + x) as usize] = m;
            cv.px[(y * n + mx) as usize] = m;
        }
    }
}

// ── gorunum / mesaj gorunumu ────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Split,
    Star,
    Chat,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VK {
    User,
    Bot,
    Think,
    Tool,
    Ok,
    Err,
    Meta,
    Hint,
}

impl VK {
    fn v(self) -> u8 {
        match self {
            VK::User => 255,
            VK::Bot => 225,
            VK::Think => 118,
            VK::Tool => 158,
            VK::Ok => 176,
            VK::Err => 214,
            VK::Meta => 108,
            VK::Hint => 140,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ev {
    Key(u8),
    Mouse(u32, i32, i32, bool),
}

fn parse_sgr_mouse(first: u8) -> Option<(u32, i32, i32, bool)> {
    if first != b'<' {
        return None;
    }
    let mut nums = [0u32; 3];
    let mut ni = 0usize;
    let mut cur = 0u32;
    let mut digits = 0u32;
    for _ in 0..16 {
        let b = term::poll_byte()?;
        if b.is_ascii_digit() {
            cur = cur.saturating_mul(10).saturating_add((b - b'0') as u32);
            digits += 1;
        } else if b == b';' {
            if ni >= 3 || digits == 0 {
                return None;
            }
            nums[ni] = cur;
            ni += 1;
            cur = 0;
            digits = 0;
        } else if b == b'M' || b == b'm' {
            if ni != 2 || digits == 0 {
                return None;
            }
            nums[ni] = cur;
            return Some((nums[0], nums[1] as i32 - 1, nums[2] as i32 - 1, b == b'm'));
        } else {
            return None;
        }
    }
    None
}

// Metni sarmala (UTF-8 guvenli, kelime tabanli).
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw.split(' ') {
            let mut w = word.chars();
            let first = w.next();
            if line.is_empty() {
                if let Some(c) = first {
                    line.push(c);
                }
                for c in w {
                    if line.chars().count() >= width {
                        out.push(std::mem::take(&mut line));
                    }
                    line.push(c);
                }
            } else {
                let add = 1 + word.chars().count();
                if line.chars().count() + add <= width {
                    line.push(' ');
                    line.push_str(word);
                } else {
                    out.push(std::mem::take(&mut line));
                    for c in word.chars() {
                        if line.chars().count() >= width {
                            out.push(std::mem::take(&mut line));
                        }
                        line.push(c);
                    }
                }
            }
        }
        out.push(line);
    }
    out
}

struct VLine {
    text: String,
    kind: VK,
}

struct App {
    cfg: Config,
    sess: Session,
    view: Vec<VLine>,
    live_think: String,
    live_text: String,
    show_think: bool,
    state: AState,
    buf: Vec<u8>,
    focus: bool,
    scroll: usize,
    capture: bool,
    cap_err: Option<String>,
    star_bx: f64,
    star_by: f64,
    panel_w: f64,
    view_mode: View,
    note: Option<(String, Instant)>,
    status: String,
    last_panel: Option<(usize, usize)>,
    last_w: usize,
    last_h: usize,
}
impl App {
    fn say(&mut self, s: &str, k: VK) {
        for l in wrap(s, 60) {
            self.view.push(VLine { text: l, kind: k });
        }
        self.scroll = 0;
    }
    fn meta(&mut self, s: &str) {
        self.say(&format!("· {s}"), VK::Meta);
    }
}

fn model_short(m: &str) -> String {
    let s = m.rsplit('/').next().unwrap_or(m);
    if s.chars().count() > 20 {
        s.chars().take(19).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

// Canli thinking/metin tamponunu kalici mesajlara cevir (tur sonu veya
// assistant mesaji bittiginde cagrilir).
// Canli thinking tamponunu kalici mesaja cevir.
fn flush_think(app: &mut App) {
    if !app.live_think.trim().is_empty() && app.show_think {
        for l in wrap(app.live_think.trim(), 60) {
            app.view.push(VLine {
                text: l,
                kind: VK::Think,
            });
        }
    }
    app.live_think.clear();
}

// Canli metin tamponunu kalici mesaja cevir.
fn flush_text(app: &mut App) {
    if !app.live_text.trim().is_empty() {
        for l in wrap(app.live_text.trim(), 60) {
            app.view.push(VLine {
                text: l,
                kind: VK::Bot,
            });
        }
    }
    app.live_text.clear();
}

fn help_text() -> &'static str {
    "/help /model [id] /models /new /sessions /resume <id> /compact /context /clear \
/key /tools /thinking /m /c /quit\n\
keys: 0 stop 1 restart 2 idle 3 production · TAB focus chat · wheel scroll · q quit"
}

fn slash(
    app: &mut App,
    line: &str,
    tx: &mpsc::Sender<Cmd>,
    mtx: &mpsc::Sender<Vec<String>>,
) -> bool {
    let first = line.split_whitespace().next().unwrap_or("").to_string();
    let arg = line[first.len()..].trim().to_string();
    let cmd = first.trim_start_matches('/').to_lowercase();
    match cmd.as_str() {
        "help" | "?" => app.say(help_text(), VK::Hint),
        "quit" | "exit" => return true,
        "model" => {
            if arg.is_empty() {
                app.meta(&format!("model: {}", app.cfg.model));
            } else {
                app.cfg.model = arg.clone();
                app.sess.model = arg.clone();
                let _ = config::save(&app.cfg);
                app.meta(&format!("model set: {arg}"));
            }
        }
        "new" => {
            let s = session::new_session(&app.cfg.model);
            app.meta(&format!("new session {}", s.id));
            app.sess = s;
            app.view.clear();
            app.say(
                "Session started. Ask anything, or /help for commands.",
                VK::Meta,
            );
        }
        "sessions" => {
            let list = session::list();
            if list.is_empty() {
                app.meta("no saved sessions");
            } else {
                app.meta(&format!("{} saved session(s):", list.len()));
                for m in list.iter().take(12) {
                    app.say(
                        &format!(
                            "  {}  {}  {} msg  {} tok",
                            m.id, m.title, m.messages, m.tokens
                        ),
                        VK::Meta,
                    );
                }
                app.say("  /resume <id>", VK::Hint);
            }
        }
        "resume" => {
            if arg.is_empty() {
                app.meta("usage: /resume <id>");
            } else if let Some(s) = session::load(&arg) {
                app.meta(&format!("resumed {} ({} messages)", s.id, s.messages.len()));
                app.sess = s;
                app.view.clear();
            } else {
                app.meta(&format!("no such session: {arg}"));
            }
        }
        "context" => {
            let used = session::used_tokens(&app.sess, app.cfg.context_limit);
            let pct = (used * 100 / app.cfg.context_limit.max(1)).min(999);
            app.meta(&format!(
                "context: ~{used} / {} tokens ({}%)",
                app.cfg.context_limit, pct
            ));
            app.meta(&format!(
                "messages: {} · session tok in/out: {}/{} · summary: {}",
                app.sess.messages.len(),
                app.sess.tok_prompt,
                app.sess.tok_completion,
                if app.sess.summary.is_some() {
                    "yes"
                } else {
                    "no"
                }
            ));
            if used * 100 > app.cfg.context_limit * 80 {
                app.say("  ⚠ context is >80% full — /compact recommended", VK::Err);
            }
        }
        "delete" => {
            if arg.is_empty() {
                app.meta("usage: /delete <id>");
            } else if session::remove(&arg) {
                app.meta(&format!("deleted session {arg}"));
            } else {
                app.meta(&format!("no such session: {arg}"));
            }
        }
        "clear" => {
            app.sess.messages.clear();
            app.sess.summary = None;
            app.view.clear();
            app.meta("conversation cleared");
        }
        "key" => {
            app.capture = true;
            app.buf.clear();
            app.cap_err = None;
            app.status = "enter new OpenRouter API key".into();
        }
        "tools" => {
            app.cfg.tools_enabled = !app.cfg.tools_enabled;
            let _ = config::save(&app.cfg);
            app.meta(&format!(
                "tools {}",
                if app.cfg.tools_enabled { "on" } else { "off" }
            ));
        }
        "thinking" => {
            app.show_think = !app.show_think;
            app.meta(&format!(
                "thinking stream {}",
                if app.show_think { "shown" } else { "hidden" }
            ));
        }
        "m" => {
            app.view_mode = if app.view_mode == View::Star {
                View::Split
            } else {
                View::Star
            };
        }
        "c" => {
            app.view_mode = if app.view_mode == View::Chat {
                View::Split
            } else {
                View::Chat
            };
        }
        "compact" => {
            let _ = tx.send(Cmd::Compact {
                sess: app.sess.clone(),
            });
            app.meta("compacting context…");
        }
        "models" => {
            let key = app.cfg.api_key.clone();
            let tx = mtx.clone();
            let _ = tx.send(Vec::new());
            std::thread::spawn(move || {
                let out = match api::models(&key) {
                    Ok(ms) => ms
                        .into_iter()
                        .map(|(id, _)| id)
                        .filter(|id| {
                            id.contains("claude")
                                || id.contains("gpt")
                                || id.contains("gemini")
                                || id.contains("deepseek")
                                || id.contains("qwen")
                                || id.contains("llama")
                        })
                        .take(60)
                        .collect(),
                    Err(e) => vec![format!("error: {e}")],
                };
                let _ = tx.send(out);
            });
            app.meta("fetching model list from OpenRouter…");
        }
        _ => {
            if cmd.is_empty() {
                return false;
            }
            app.say(&format!("unknown command /{cmd} — /help"), VK::Err);
        }
    }
    false
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut key_override: Option<String> = None;
    let mut model_override: Option<String> = None;
    let mut session_override: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => {
                println!("{NAME} v{VER}");
                return;
            }
            "--key" => {
                i += 1;
                key_override = args.get(i).cloned();
            }
            "--model" => {
                i += 1;
                model_override = args.get(i).cloned();
            }
            "--session" => {
                i += 1;
                session_override = args.get(i).cloned();
            }
            "--help" | "-h" => {
                println!("{NAME} v{VER}\n  --key <openrouter-key>  --model <id>  --session <id>  --version");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let mut cfg = config::load();
    if let Some(k) = key_override {
        cfg.api_key = k;
        let _ = config::save(&cfg);
    }
    if let Some(m) = model_override {
        cfg.model = m;
    }
    let ws = PathBuf::from(&cfg.workspace);

    let mut sess = session_override
        .as_ref()
        .and_then(|id| session::load(id))
        .unwrap_or_else(|| session::new_session(&cfg.model));
    sess.model = cfg.model.clone();
    if !sess.id.is_empty() {
        cfg.session = sess.id.clone();
    }

    let saved = term::raw_start();
    term::setup_window();
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[?1000h\x1b[?1006h");
    let _ = out.flush();

    let (etx, erx) = mpsc::channel::<Ev>();
    std::thread::spawn(move || {
        let mut tmp = Vec::new();
        loop {
            tmp.clear();
            term::read_key_bytes(&mut tmp);
            if tmp.len() == 1 && tmp[0] == 0 {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            if tmp.len() != 1 || tmp[0] != 0x1b {
                let mut dead = false;
                for b in tmp.iter() {
                    if etx.send(Ev::Key(*b)).is_err() {
                        dead = true;
                        break;
                    }
                }
                if dead {
                    return;
                }
                continue;
            }
            let nb = match term::poll_byte() {
                Some(v) => v,
                None => {
                    let _ = etx.send(Ev::Key(0x1b));
                    continue;
                }
            };
            if nb != b'[' {
                let _ = etx.send(Ev::Key(0x1b));
                let _ = etx.send(Ev::Key(nb));
                continue;
            }
            if let Some(f) = term::poll_byte() {
                if let Some((cb, col, row, rel)) = parse_sgr_mouse(f) {
                    let _ = etx.send(Ev::Mouse(cb, col, row, rel));
                }
            } else {
                let _ = etx.send(Ev::Key(0x1b));
            }
        }
    });

    let (ctx, ctx_rx) = mpsc::channel::<Cmd>();
    let (mtx, mrx) = mpsc::channel::<Vec<String>>();
    let no_key = !config::has_key(&cfg);
    let wtx = if no_key {
        mpsc::channel::<WEvent>().1
    } else {
        agent::spawn(
            ctx_rx,
            cfg.api_key.clone(),
            cfg.model.clone(),
            ws.clone(),
            cfg.tools_enabled,
        )
    };

    let mut app = App {
        cfg,
        sess,
        view: Vec::new(),
        live_think: String::new(),
        live_text: String::new(),
        show_think: true,
        state: AState::Waiting,
        buf: Vec::new(),
        focus: false,
        scroll: 0,
        capture: no_key,
        cap_err: None,
        star_bx: 0.0,
        star_by: 0.0,
        panel_w: 0.0,
        view_mode: View::Split,
        note: None,
        status: if no_key {
            "OpenRouter API key required".into()
        } else {
            String::new()
        },
        last_panel: None,
        last_w: 0,
        last_h: 0,
    };
    if no_key {
        app.say(
            "Welcome to synapse-distilled.\nPaste your OpenRouter API key below (it is stored in the user config). Get one at https://openrouter.ai/keys",
            VK::Hint,
        );
    } else {
        app.say(
            &format!(
                "synapse-distilled v{VER} · model {} · session {}\nType a task and press Enter. /help for commands.",
                model_short(&app.cfg.model),
                app.sess.id
            ),
            VK::Hint,
        );
    }

    let mut screen = Screen::new(80, 24);
    screen.set_bg(BG_BLACK);
    let mut last_size = (0usize, 0usize);
    let mut cv = Canvas::new_exact(STAR_N);
    cv.aa = 2;
    let mut phase = Phase::Blank;
    let mut intro_done = false;
    let mut pt = 0.0f64;
    let mut scan_prog = 0.0f64;
    let mut coin_angle = 0.0f64;
    let mut spin_vel = 0.0f64;
    let mut park_env = 0.0f64;
    let mut spin_env = 0.0f64;
    let mut prod_wheel = 0.0f64;
    let mut prod_wvel = 0.0f64;
    let mut last = Instant::now();
    let t0 = last;
    let mut quit = false;

    while !quit {
        // ── olaylar: klavye/fare ───────────────────────────────
        let mut dirty = false;
        while let Ok(ev) = erx.try_recv() {
            match ev {
                Ev::Mouse(cb, col, row, rel) => {
                    if cb & 64 != 0 {
                        if cb & 1 == 0 {
                            app.scroll = app.scroll.saturating_add(3);
                        } else {
                            app.scroll = app.scroll.saturating_sub(3);
                        }
                        dirty = true;
                    } else if !rel {
                        let h = last_size.1;
                        let on_input = h > 3
                            && row >= h as i32 - 3
                            && app.last_panel.is_some_and(|(x0, _)| col >= x0 as i32);
                        if on_input != app.focus {
                            app.focus = on_input;
                            dirty = true;
                        }
                    }
                }
                Ev::Key(b) if app.capture => match b {
                    13 | 10 => {
                        let k = String::from_utf8_lossy(&app.buf).trim().to_string();
                        if k.starts_with("sk-or-") || k.len() > 20 {
                            app.cfg.api_key = k;
                            let _ = config::save(&app.cfg);
                            app.capture = false;
                            app.buf.clear();
                            app.cap_err = None;
                            app.meta("API key saved. Starting session.");
                            app.say(
                                    "Key stored. Ask a task to begin — the agent can read, write and run commands in the workspace.",
                                    VK::Meta,
                                );
                        } else {
                            app.cap_err = Some(
                                "that does not look like an OpenRouter key (expected sk-or-…)"
                                    .into(),
                            );
                            app.buf.clear();
                        }
                        dirty = true;
                    }
                    127 | 8 => {
                        let first = app.buf.pop();
                        if first.is_some_and(|b| b & 0xC0 == 0x80) {
                            while app.buf.last().is_some_and(|b| b & 0xC0 == 0x80) {
                                app.buf.pop();
                            }
                            if app.buf.last().is_some_and(|b| *b >= 0xC0) {
                                app.buf.pop();
                            }
                        }
                        dirty = true;
                    }
                    3 => quit = true,
                    _ if b >= 32 && app.buf.len() < 256 => {
                        app.buf.push(b);
                        dirty = true;
                    }
                    _ => {}
                },
                Ev::Key(b) if app.focus => match b {
                    13 | 10 => {
                        let line = String::from_utf8_lossy(&app.buf).trim().to_string();
                        app.buf.clear();
                        if !line.is_empty() {
                            if line.starts_with('/') {
                                if slash(&mut app, &line, &ctx, &mtx) {
                                    quit = true;
                                }
                            } else {
                                app.say(&format!("❯ {line}"), VK::User);
                                app.state = AState::Thinking;
                                let _ = ctx.send(Cmd::Ask {
                                    text: line,
                                    sess: app.sess.clone(),
                                });
                            }
                        }
                        dirty = true;
                    }
                    127 | 8 => {
                        let first = app.buf.pop();
                        if first.is_some_and(|b| b & 0xC0 == 0x80) {
                            while app.buf.last().is_some_and(|b| b & 0xC0 == 0x80) {
                                app.buf.pop();
                            }
                            if app.buf.last().is_some_and(|b| *b >= 0xC0) {
                                app.buf.pop();
                            }
                        }
                        dirty = true;
                    }
                    0x1b => {
                        app.focus = false;
                        dirty = true;
                    }
                    b if b >= 32 && app.buf.len() < 4000 => {
                        app.buf.push(b);
                        dirty = true;
                    }
                    _ => {}
                },
                Ev::Key(b) => {
                    if b == b'q' || b == 0x1b || b == 3 {
                        quit = true;
                    } else if b == b'\t' {
                        app.focus = true;
                        dirty = true;
                    } else if b == b'0' {
                        app.state = AState::Waiting;
                        dirty = true;
                    } else if b == b'1' {
                        phase = Phase::Blank;
                        intro_done = false;
                        pt = 0.0;
                        scan_prog = 0.0;
                        dirty = true;
                    } else if b == b'2' {
                        app.state = AState::Thinking;
                        intro_done = true;
                        dirty = true;
                    } else if b == b'3' {
                        app.state = AState::Working;
                        intro_done = true;
                        dirty = true;
                    } else if b == b'm' || b == b'M' {
                        app.view_mode = if app.view_mode == View::Star {
                            View::Split
                        } else {
                            View::Star
                        };
                        dirty = true;
                    } else if b == b'c' || b == b'C' {
                        app.view_mode = if app.view_mode == View::Chat {
                            View::Split
                        } else {
                            View::Chat
                        };
                        dirty = true;
                    } else if b >= 32 {
                        // odak yokken yazmak odagi açar
                        app.focus = true;
                        app.buf.push(b);
                        dirty = true;
                    }
                }
            }
        }

        // ── olaylar: worker ────────────────────────────────────
        while let Ok(we) = wtx.try_recv() {
            match we {
                WEvent::State(s) => {
                    if app.state != s {
                        app.state = s;
                        dirty = true;
                    }
                }
                WEvent::Reasoning(r) => {
                    app.live_think.push_str(&r);
                    dirty = true;
                }
                WEvent::Text(t) => {
                    app.live_text.push_str(&t);
                    dirty = true;
                }
                WEvent::Say(t) => {
                    flush_think(&mut app);
                    app.live_text.clear();
                    for l in wrap(t.trim(), 60) {
                        app.view.push(VLine {
                            text: l,
                            kind: VK::Bot,
                        });
                    }
                    dirty = true;
                }
                WEvent::ToolStart { name, args } => {
                    app.say(&format!("⚙ {name} · {args}"), VK::Tool);
                }
                WEvent::ToolEnd { name, ok, line } => {
                    let mark = if ok { "✓" } else { "✗" };
                    let mut l = format!("{mark} {name}");
                    if !line.is_empty() {
                        l.push_str(&format!(" — {line}"));
                    }
                    app.say(&l, if ok { VK::Ok } else { VK::Err });
                }
                WEvent::Usage { prompt, completion } => {
                    app.sess.tok_prompt = app.sess.tok_prompt.max(prompt);
                    app.sess.tok_completion += completion;
                    dirty = true;
                }
                WEvent::Failed(e) => {
                    app.say(&format!("error: {e}"), VK::Err);
                    app.state = AState::Waiting;
                }
                WEvent::Compacted(n) => {
                    app.meta(&format!("context compacted: {n} messages summarized"));
                }
                WEvent::Sess(s) => {
                    app.sess = *s;
                    app.sess.updated = session::now();
                    let _ = session::save(&app.sess);
                    app.cfg.session = app.sess.id.clone();
                    let _ = config::save(&app.cfg);
                }
                WEvent::Done => {
                    flush_think(&mut app);
                    flush_text(&mut app);
                    app.scroll = 0;
                    dirty = true;
                }
            }
        }
        if !app
            .note
            .as_ref()
            .is_none_or(|(_, i)| i.elapsed().as_secs() < 4)
        {
            dirty = true;
        }
        if let Some((_, i)) = &app.note {
            if i.elapsed().as_secs() >= 4 {
                app.note = None;
            }
        }
        // model listesi sonucu
        while let Ok(list) = mrx.try_recv() {
            if list.is_empty() {
                continue;
            }
            if list[0].starts_with("error:") {
                app.say(&list[0], VK::Err);
            } else {
                app.meta(&format!("{} candidate models:", list.len()));
                for id in list {
                    app.say(&format!("  {id}"), VK::Meta);
                }
                app.say("  /model <id> to switch", VK::Hint);
            }
            dirty = true;
        }

        // ── zaman + animasyon fizigi ───────────────────────────
        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.05);
        last = now;
        let t = now.duration_since(t0).as_secs_f64();
        pt += dt;

        let (w0, h0) = term::size();
        let resized = (w0, h0) != last_size;
        if resized {
            last_size = (w0, h0);
        }
        let w = w0.max(20);
        let h = h0.max(6);

        // faz gecisi
        if !intro_done {
            match phase {
                Phase::Blank if pt >= T_BLANK => {
                    phase = Phase::Scan;
                    pt = 0.0;
                    scan_prog = 0.0;
                }
                Phase::Scan if scan_prog >= 1.0 => {
                    phase = Phase::ScanFill;
                    pt = 0.0;
                    scan_prog = 0.0;
                }
                Phase::ScanFill if scan_prog >= 1.0 && pt >= T_SWEEP + T_HOLD => {
                    phase = Phase::Hold;
                    intro_done = true;
                    pt = 0.0;
                }
                Phase::Scan | Phase::ScanFill => {
                    scan_prog = (scan_prog + dt / T_SWEEP).min(1.0);
                }
                _ => {}
            }
            if intro_done {
                app.meta("ready");
            }
        } else {
            let want = match app.state {
                AState::Waiting => Phase::Hold,
                AState::Thinking => Phase::Idle,
                AState::Working => Phase::Production,
            };
            if want != phase {
                phase = want;
                pt = 0.0;
            }
        }

        let animating = !intro_done
            || phase == Phase::Idle
            || phase == Phase::Production
            || phase == Phase::Scan
            || phase == Phase::ScanFill;
        if !animating && !resized && !dirty {
            std::thread::sleep(Duration::from_millis(40));
            continue;
        }

        // yildiz fiziği
        let spin_target = if phase == Phase::Idle { 1.0 } else { 0.0 };
        spin_env += (spin_target - spin_env) * (1.0 - (-dt / 0.3).exp());
        let park_target = if intro_done { 1.0 } else { 0.0 };
        park_env += (park_target - park_env) * (1.0 - (-dt / 0.25).exp());
        if phase == Phase::Idle {
            let cruise = angle_rate(coin_angle);
            spin_vel += (cruise - spin_vel) * (1.0 - (-dt / 0.25).exp());
            coin_angle += spin_vel * dt;
        } else if phase == Phase::Production {
            let tilt = PROD_TILT + ((coin_angle - PROD_TILT) / TAU).round() * TAU;
            let servo = ((tilt - coin_angle) * 4.0).clamp(-8.0, 8.0);
            spin_vel += (servo - spin_vel) * (1.0 - (-dt / 0.15).exp());
            coin_angle += spin_vel * dt;
            prod_wvel += (WHEEL_CRUISE - prod_wvel) * (1.0 - (-dt / 0.3).exp());
            prod_wheel += prod_wvel * dt;
        }

        // ── duzen (animasyonlu) ────────────────────────────────
        let n = STAR_N.min(w as i32).min(h as i32 * 2).clamp(41, 256);
        if n != cv.n {
            cv = Canvas::new_exact(n);
            cv.aa = 2;
        }
        let rows_i = (cv.n + 1) / 2;
        let (tx_, ty_, tw_) = match app.view_mode {
            View::Split => (0.0, 0.0, (w as i32 - cv.n - 1).max(0) as f64),
            View::Star => (
                (w as i32 - cv.n) as f64 / 2.0,
                (h as i32 - rows_i) as f64 / 2.0,
                0.0,
            ),
            View::Chat => (-(cv.n as f64), 0.0, w as f64),
        };
        let k = 1.0 - (-dt / 0.25).exp();
        app.star_bx += (tx_ - app.star_bx) * k;
        app.star_by += (ty_ - app.star_by) * k;
        app.panel_w += (tw_ - app.panel_w) * k;
        if (app.star_bx - tx_).abs() < 0.5
            && (app.star_by - ty_).abs() < 0.5
            && (app.panel_w - tw_).abs() < 0.5
        {
            app.star_bx = tx_;
            app.star_by = ty_;
            app.panel_w = tw_;
        }

        // ── cizim ─────────────────────────────────────────────
        let nf = n as f64;
        let sweep = -14.0 + (nf + 28.0) * scan_prog;
        cv.clear();
        match phase {
            Phase::Blank => {}
            Phase::Scan => {
                let r = cv.r;
                coin_face(&mut cv, t, 0.0, park_env * PARK_Y);
                clip_y(&mut cv, sweep - 0.4 * r, sweep);
                mirror_x(&mut cv);
            }
            Phase::ScanFill => {
                coin_face(&mut cv, t, 0.0, park_env * PARK_Y);
                clip_y(&mut cv, f64::MIN, sweep + 7.0);
                mirror_x(&mut cv);
            }
            Phase::Hold => {
                coin_face(&mut cv, t, 0.0, park_env * PARK_Y);
                mirror_x(&mut cv);
            }
            Phase::Idle => {
                let wt = t * IDLE_W;
                render::draw_solid(
                    &mut cv,
                    &Solid {
                        t,
                        angle: Some(coin_angle),
                        rim_gain: 1.0,
                        wob_gain: spin_env,
                        ox: spin_env * IDLE_R * wt.cos(),
                        oy: spin_env * IDLE_R * wt.sin() + park_env * PARK_Y,
                        dark_bg: true,
                        ..Solid::base(t)
                    },
                );
            }
            Phase::Production => {
                render::draw_solid(
                    &mut cv,
                    &Solid {
                        t,
                        angle: Some(coin_angle),
                        rot_z: prod_wheel,
                        rim_gain: 1.0,
                        wob_gain: 0.0,
                        ox: spin_env * IDLE_R * (t * IDLE_W).cos(),
                        oy: spin_env * IDLE_R * (t * IDLE_W).sin() + park_env * PARK_Y,
                        dark_bg: true,
                        ..Solid::base(t)
                    },
                );
            }
        }

        screen.reset(w, h);
        let rows = (cv.n + 1) / 2;
        let bx = app.star_bx.round() as i32;
        let by = app.star_by.round() as i32;
        for j in 0..rows {
            let y0 = 2 * j;
            let y1 = y0 + 1;
            for i in 0..cv.n {
                let tv = if y0 < cv.n {
                    cv.px[(y0 * cv.n + i) as usize]
                } else {
                    0
                };
                let bv = if y1 < cv.n {
                    cv.px[(y1 * cv.n + i) as usize]
                } else {
                    0
                };
                screen.half(bx + i, by + j, tv, bv);
            }
        }

        // panel
        let pw = app.panel_w.round().clamp(0.0, w as f64) as usize;
        let px0 = w.saturating_sub(pw);
        let panel = if h >= 6 && pw >= 16 {
            Some((px0, pw))
        } else {
            None
        };
        app.last_panel = panel;
        if let Some((x0, cw)) = panel {
            let x0i = x0 as i32;
            if x0i > 0 {
                for j in 1..h as i32 - 3 {
                    screen.text(x0i - 1, j, "│", 70);
                }
            }
            // durum cubugu
            let used = session::used_tokens(&app.sess, app.cfg.context_limit);
            let pct = (used * 100 / app.cfg.context_limit.max(1)).min(999);
            let st = match app.state {
                AState::Waiting => "ready",
                AState::Thinking => "thinking",
                AState::Working => "working",
            };
            let head: String = format!(
                "{} v{} │ {} │ ctx {}% │ {} │ {}",
                NAME,
                VER,
                model_short(&app.cfg.model),
                pct,
                st,
                app.sess.id
            )
            .chars()
            .take(cw.saturating_sub(1))
            .collect();
            screen.text(x0i, 0, &head, 150);
            // mesajlar
            let hist_rows = h - 4;
            let mut lines: Vec<(String, u8)> = Vec::new();
            for v in &app.view {
                lines.push((v.text.clone(), v.kind.v()));
            }
            if !app.live_think.is_empty() && app.show_think {
                for l in wrap(app.live_think.trim_end(), cw.saturating_sub(4)) {
                    lines.push((format!("· {l}"), VK::Think.v()));
                }
            }
            if !app.live_text.is_empty() {
                for l in wrap(app.live_text.trim_end(), cw.saturating_sub(4)) {
                    lines.push((l, VK::Bot.v()));
                }
            }
            if let Some((t, _)) = &app.note {
                for l in wrap(t, cw.saturating_sub(2)) {
                    lines.push((l, VK::Hint.v()));
                }
            }
            let maxs = lines.len().saturating_sub(hist_rows);
            if app.scroll > maxs {
                app.scroll = maxs;
            }
            let end = lines.len().saturating_sub(app.scroll);
            let start = end.saturating_sub(hist_rows);
            let shown = &lines[start..end];
            let base = hist_rows - shown.len();
            for (r, (txt, v)) in shown.iter().enumerate() {
                let s: String = txt.chars().take(cw).collect();
                screen.text(x0i, 1 + base as i32 + r as i32, &s, *v);
            }
            // girdi kutusu
            let inner = cw.saturating_sub(2);
            let h3 = h as i32;
            screen.text(x0i, h3 - 3, &format!("┌{}┐", "─".repeat(inner)), 255);
            screen.text(x0i, h3 - 1, &format!("└{}┘", "─".repeat(inner)), 255);
            screen.text(x0i, h3 - 2, &format!("│{}│", " ".repeat(inner)), 255);
            let (content, cval) = if app.capture {
                let shown = "*".repeat(app.buf.len().min(inner.saturating_sub(2)));
                (
                    if app.cap_err.is_some() {
                        shown
                    } else {
                        format!("key: {shown}")
                    },
                    if app.cap_err.is_some() {
                        VK::Err.v()
                    } else {
                        255
                    },
                )
            } else if app.buf.is_empty() && !app.focus {
                ("TAB or click to write · /help".to_string(), VK::Hint.v())
            } else {
                (
                    format!("> {}", String::from_utf8_lossy(&app.buf)),
                    if app.focus { 255 } else { 200 },
                )
            };
            let crow: String = content.chars().take(inner).collect();
            screen.text(x0i + 1, h3 - 2, &crow, cval);
        }
        screen.text(0, 0, &format!("v{VER}"), 120);

        let mut buf = screen.render();
        // imlec + durum satiri
        if let Some((x0, cw)) = panel {
            let (line, extra) = if app.capture {
                (
                    app.buf.len().min(cw.saturating_sub(8)),
                    if app.cap_err.is_some() {
                        format!("  {}", app.cap_err.clone().unwrap_or_default())
                    } else {
                        String::new()
                    },
                )
            } else {
                (
                    String::from_utf8_lossy(&app.buf)
                        .chars()
                        .count()
                        .min(cw.saturating_sub(4))
                        + 2,
                    String::new(),
                )
            };
            let cc = (x0 as i32 + 2 + line as i32).min(w as i32 - 2).max(0);
            if app.capture || app.focus {
                buf.push_str(&format!("\x1b[{};{}H\x1b[?25h", h - 1, cc + 1));
            } else {
                buf.push_str("\x1b[?25l");
            }
            if !extra.is_empty() {
                let e: String = extra.chars().take(cw).collect();
                buf.push_str(&format!("\x1b[{};{}H", h - 1, x0 + 1));
                buf.push_str(&e);
                buf.push_str(&format!("\x1b[{};{}H", h - 1, cc + 1));
            }
        } else {
            buf.push_str("\x1b[?25l");
        }
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
        app.last_w = w;
        app.last_h = h;
        if animating {
            let spent = now.elapsed();
            if spent < FRAME {
                std::thread::sleep(FRAME - spent);
            }
        }
    }

    let _ = ctx.send(Cmd::Shutdown);
    term::raw_stop(&saved);
    let _ = out.write_all(b"\x1b[?25h\x1b[0m\x1b[?1000l\x1b[?1006l\x1b[?1049l");
    let _ = out.flush();
}
