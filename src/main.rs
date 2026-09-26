// synapse-distilled: TUI AI ajan/orchestrator.
// Animasyon durum makinesi uygulama durumuna baglidir:
//   Waiting  -> static (Hold)      Thinking -> idle spin (Idle)
//   Working  -> wheel (Production)  Boot     -> intro reveal
#[allow(dead_code, unused_imports)]
mod render;
#[allow(dead_code, unused_imports)]
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
    Up,
    Down,
    Left,
    Right,
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

// Ekrana basilacak metni guvenli hale getir: ANSI kacis dizileri, \r,
// TAB ve kontrol karakterleri imleci ziplatip satirlari ust uste bindirir.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut ch = s.chars().peekable();
    while let Some(c) = ch.next() {
        if c == '\x1b' {
            match ch.peek() {
                Some('[') => {
                    ch.next();
                    for c2 in ch.by_ref() {
                        if c2.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    ch.next();
                    loop {
                        match ch.next() {
                            None => break,
                            Some('\x07') => break,
                            Some('\x1b') if ch.peek() == Some(&'\\') => {
                                ch.next();
                                break;
                            }
                            Some('\x1b') => {}
                            _ => {}
                        }
                    }
                }
                Some('(') | Some(')') | Some('#') => {
                    ch.next();
                    ch.next();
                }
                Some(_) => {
                    ch.next();
                }
                None => {}
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        if c == '\n' {
            out.push('\n');
            continue;
        }
        if c == '\t' {
            out.push_str("  ");
            continue;
        }
        if c == '\u{7f}' || (c.is_control() && c != '\n') {
            continue;
        }
        out.push(c);
    }
    out
}

// Metni sarmala (UTF-8 guvenli, kelime tabanli).
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for raw in clean(text).split('\n') {
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
    picker: Option<Picker>,
    ask_modal: Option<AskModal>,
    ask_pos: Option<(i32, i32)>,
    shell_mode: bool,
    shell_rx: Option<mpsc::Receiver<String>>,
    settings: Option<Settings>,
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

// UTF-8 guvenli tek karakter sil (capture/chat/custom girislerde ortak).
fn pop_utf8(buf: &mut Vec<u8>) {
    let first = buf.pop();
    if matches!(first, Some(b) if b & 0xC0 == 0x80) {
        while matches!(buf.last(), Some(b) if b & 0xC0 == 0x80) {
            buf.pop();
        }
        if matches!(buf.last(), Some(b) if *b >= 0xC0) {
            buf.pop();
        }
    }
}

// Panoya/yapistirmayi hedef tampona ekle (limitli). Tek satirsa ilk satir.
fn paste_into(buf: &mut Vec<u8>, cap: usize, single_line: bool) {
    let Some(t) = term::clipboard_text() else {
        return;
    };
    let flat = if single_line {
        t.lines().next().unwrap_or("").trim().to_string()
    } else {
        t.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    for b in flat.bytes() {
        if buf.len() >= cap {
            break;
        }
        buf.push(b);
    }
}

// ── model secici ────────────────────────────────────────────────
const CUSTOM_ENTRY: &str = "[type a custom model id…]";

struct Picker {
    customs: Vec<String>,
    fetched: Vec<String>,
    error: Option<String>,
    sel: usize,
    scroll: usize,
    custom_mode: bool,
    custom_buf: Vec<u8>,
    prompt_row: Option<i32>,
}

impl Picker {
    fn len(&self) -> usize {
        self.customs.len() + self.fetched.len() + 1
    }
    fn id_at(&self, i: usize) -> Option<String> {
        if i < self.customs.len() {
            Some(self.customs[i].clone())
        } else if i < self.customs.len() + self.fetched.len() {
            Some(self.fetched[i - self.customs.len()].clone())
        } else {
            None
        }
    }
}
fn open_picker(app: &mut App, mtx: &mpsc::Sender<Vec<String>>) {
    app.focus = false;
    app.picker = Some(Picker {
        customs: app.cfg.custom_models.clone(),
        fetched: Vec::new(),
        error: None,
        sel: 0,
        scroll: 0,
        custom_mode: false,
        custom_buf: Vec::new(),
        prompt_row: None,
    });
    let prov = prov_of(&app.cfg);
    let tx = mtx.clone();
    std::thread::spawn(move || {
        let out = match api::models(&prov) {
            Ok(ms) => ms.into_iter().map(|(id, _)| id).take(120).collect(),
            Err(e) => vec![format!("error: {e}")],
        };
        let _ = tx.send(out);
    });
    app.meta("fetching model list… (Up/Down + Enter, c custom, Esc cancel)");
}

fn prov_of(c: &Config) -> api::Prov {
    let mut p = if c.provider == "nvidia" {
        api::Prov::nvidia(&c.nvidia_key)
    } else {
        api::Prov::openrouter(&c.api_key)
    };
    p.temp = c.temperature;
    p.top_p = c.top_p;
    p.max_tokens = c.max_tokens;
    p
}

fn set_provider(app: &mut App, to: &str) {
    app.cfg.provider = to.to_string();
    let m = if to == "nvidia" {
        app.cfg.nv_model.clone()
    } else {
        app.cfg.or_model.clone()
    };
    app.cfg.model = m.clone();
    app.sess.model = m;
    let _ = config::save(&app.cfg);
    app.meta(&format!("provider: {to} · model: {}", app.cfg.model));
    if !config::has_key(&app.cfg) {
        app.capture = true;
        app.buf.clear();
        app.cap_err = None;
        app.status = format!(
            "enter {} API key ({})",
            app.cfg.provider,
            config::key_hint(&app.cfg.provider)
        );
        app.meta("no key for this provider — paste it below");
    }
}

fn set_model(c: &mut Config, id: &str) {
    c.model = id.to_string();
    if c.provider == "nvidia" {
        c.nv_model = id.to_string();
    } else {
        c.or_model = id.to_string();
    }
}

fn prov_tag(c: &Config) -> &'static str {
    if c.provider == "nvidia" {
        "nv"
    } else {
        "or"
    }
}

fn apply_model(app: &mut App, id: &str) {
    set_model(&mut app.cfg, id);
    app.sess.model = id.to_string();
    let _ = config::save(&app.cfg);
    app.meta(&format!("model set: {id}"));
    app.picker = None;
}

fn picker_confirm(app: &mut App) {
    let total = match app.picker.as_ref() {
        Some(p) => p.customs.len() + p.fetched.len(),
        None => return,
    };
    let sel = app.picker.as_ref().map(|p| p.sel).unwrap_or(0);
    if sel >= total {
        if let Some(p) = app.picker.as_mut() {
            p.custom_mode = true;
            p.custom_buf.clear();
        }
        return;
    }
    if let Some(id) = app.picker.as_ref().and_then(|p| p.id_at(sel)) {
        apply_model(app, &id);
    }
}

fn picker_custom_ok(app: &mut App) {
    let id = match app.picker.as_ref() {
        Some(p) => String::from_utf8_lossy(&p.custom_buf).trim().to_string(),
        None => return,
    };
    if id.is_empty() {
        return;
    }
    if !app.cfg.custom_models.contains(&id) {
        app.cfg.custom_models.push(id.clone());
        app.cfg.custom_models.truncate(20);
    }
    apply_model(app, &id);
}

fn picker_event(app: &mut App, ev: Ev) {
    let Some(p) = app.picker.as_mut() else {
        return;
    };
    match ev {
        Ev::Up => {
            if !p.custom_mode {
                p.sel = p.sel.saturating_sub(1);
            }
        }
        Ev::Down => {
            if !p.custom_mode && p.sel + 1 < p.len() {
                p.sel += 1;
            }
        }
        Ev::Left | Ev::Right => {}
        Ev::Mouse(cb, col, row, rel) => {
            if cb & 64 != 0 {
                if p.custom_mode {
                    return;
                }
                if cb & 1 == 0 {
                    p.sel = p.sel.saturating_sub(3);
                } else if p.sel + 3 < p.len() {
                    p.sel += 3;
                } else if p.len() > 0 {
                    p.sel = p.len() - 1;
                }
                return;
            }
            if rel {
                return;
            }
            // tiklama: liste satiri -> sec+onayla, custom satiri -> custom mod
            if let Some(pr) = p.prompt_row {
                let (x0, _) = app.last_panel.unwrap_or((0, 0));
                if col >= x0 as i32 && row == pr {
                    p.custom_mode = true;
                    p.custom_buf.clear();
                    return;
                }
            }
            let (x0, _) = app.last_panel.unwrap_or((0, 0));
            if col < x0 as i32 || row < 2 {
                return;
            }
            let idx = p.scroll + (row - 2) as usize;
            if idx < p.len() {
                p.sel = idx;
                picker_confirm(app);
            }
        }
        Ev::Key(b) => {
            if p.custom_mode {
                match b {
                    13 | 10 => picker_custom_ok(app),
                    0x1b => {
                        p.custom_mode = false;
                        p.custom_buf.clear();
                    }
                    127 | 8 => pop_utf8(&mut p.custom_buf),
                    22 => paste_into(&mut p.custom_buf, 120, true),
                    _ if b >= 32 && p.custom_buf.len() < 120 => p.custom_buf.push(b),
                    _ => {}
                }
            } else {
                match b {
                    13 | 10 => picker_confirm(app),
                    0x1b => {
                        app.picker = None;
                    }
                    b'c' | b'C' => {
                        p.custom_mode = true;
                        p.custom_buf.clear();
                    }
                    _ => {}
                }
            }
        }
    }
}

type BtnHit = Vec<(i32, Vec<(i32, i32, usize)>)>;

struct AskModal {
    id: u32,
    label: String,
    question: String,
    options: Vec<String>,
    buf: Vec<u8>,
    buttons: bool,
    sel: usize,
    btn_rows: BtnHit,
}

fn modal_submit(app: &mut App, ctx: &mpsc::Sender<Cmd>, text: String) {
    if let Some(m) = app.ask_modal.take() {
        let _ = ctx.send(Cmd::AskReply { id: m.id, text });
    }
}

// Modal duzeni (kirpma + cizim ayni hesabi kullanir).
struct ModalLayout {
    start: i32,
    head: Vec<(String, u8)>,
    blines: Vec<String>,
    bspans: Vec<Vec<(usize, usize, usize)>>,
}

fn modal_layout(m: &AskModal, cw: usize, h: usize) -> ModalLayout {
    let mut head: Vec<(String, u8)> =
        vec![(format!("? {} — Enter send · Esc empty", m.label), 255)];
    for l in wrap(&m.question, cw.saturating_sub(4)) {
        head.push((l, 200));
    }
    if !m.buttons {
        for (i, o) in m.options.iter().enumerate() {
            head.push((format!("  {}. {}", i + 1, clean(o)), 150));
        }
        head.push((
            clean(&format!("> {}", String::from_utf8_lossy(&m.buf))),
            255,
        ));
    }
    let (blines, bspans) = if m.buttons {
        let mut blines: Vec<String> = Vec::new();
        let mut bspans: Vec<Vec<(usize, usize, usize)>> = Vec::new();
        let mut cur = String::new();
        let mut csp: Vec<(usize, usize, usize)> = Vec::new();
        for (i, o) in m.options.iter().enumerate() {
            let o = clean(o);
            let t = if i == m.sel {
                format!("[{o}]")
            } else {
                format!(" {o} ")
            };
            let need = t.chars().count() + if cur.is_empty() { 0 } else { 2 };
            if !cur.is_empty() && cur.chars().count() + need > cw.saturating_sub(1) {
                blines.push(std::mem::take(&mut cur));
                bspans.push(std::mem::take(&mut csp));
            }
            let x0 = cur.chars().count() + if cur.is_empty() { 0 } else { 2 };
            if !cur.is_empty() {
                cur.push_str("  ");
            }
            cur.push_str(&t);
            csp.push((x0, cur.chars().count(), i));
        }
        blines.push(cur);
        bspans.push(csp);
        (blines, bspans)
    } else {
        (Vec::new(), Vec::new())
    };
    let start = (h as i32 - 4 - head.len() as i32 - blines.len() as i32).max(1);
    ModalLayout {
        start,
        head,
        blines,
        bspans,
    }
}

fn ask_modal_event(app: &mut App, ev: Ev, ctx: &mpsc::Sender<Cmd>) {
    let buttons = app.ask_modal.as_ref().is_some_and(|m| m.buttons);
    if buttons {
        let n = app.ask_modal.as_ref().map(|m| m.options.len()).unwrap_or(0);
        match ev {
            Ev::Up | Ev::Left => {
                if let Some(m) = app.ask_modal.as_mut() {
                    if n > 0 {
                        m.sel = (m.sel + n - 1) % n;
                    }
                }
            }
            Ev::Down | Ev::Right => {
                if let Some(m) = app.ask_modal.as_mut() {
                    if n > 0 {
                        m.sel = (m.sel + 1) % n;
                    }
                }
            }
            Ev::Mouse(cb, col, row, rel) => {
                if cb & 64 != 0 {
                    if let Some(m) = app.ask_modal.as_mut() {
                        if n > 0 {
                            if cb & 1 == 0 {
                                m.sel = (m.sel + n - 1) % n;
                            } else {
                                m.sel = (m.sel + 1) % n;
                            }
                        }
                    }
                    return;
                }
                if rel {
                    return;
                }
                // butona tikla = sec + onayla
                let hit = app.ask_modal.as_ref().and_then(|m| {
                    m.btn_rows.iter().find_map(|(r, spans)| {
                        if *r == row {
                            spans
                                .iter()
                                .find(|(x0, x1, _)| col >= *x0 && col < *x1)
                                .map(|(_, _, i)| *i)
                        } else {
                            None
                        }
                    })
                });
                if let Some(i) = hit {
                    let text = app
                        .ask_modal
                        .as_ref()
                        .and_then(|m| m.options.get(i).cloned())
                        .unwrap_or_default();
                    modal_submit(app, ctx, text);
                }
            }
            Ev::Key(b) => match b {
                13 | 10 => {
                    let text = app
                        .ask_modal
                        .as_ref()
                        .and_then(|m| m.options.get(m.sel).cloned())
                        .unwrap_or_default();
                    modal_submit(app, ctx, text);
                }
                0x1b => {
                    modal_submit(app, ctx, String::new());
                }
                b if (b'1'..=b'9').contains(&b) => {
                    let i = (b - b'1') as usize;
                    if let Some(m) = app.ask_modal.as_mut() {
                        if i < m.options.len() {
                            m.sel = i;
                        }
                    }
                }
                _ => {}
            },
        }
        return;
    }
    let id = match app.ask_modal.as_ref() {
        Some(m) => m.id,
        None => return,
    };
    match ev {
        Ev::Up | Ev::Down | Ev::Left | Ev::Right | Ev::Mouse(..) => {}
        Ev::Key(b) => match b {
            13 | 10 => {
                let text = app
                    .ask_modal
                    .as_ref()
                    .map(|m| String::from_utf8_lossy(&m.buf).trim().to_string())
                    .unwrap_or_default();
                let _ = ctx.send(Cmd::AskReply { id, text });
                app.ask_modal = None;
            }
            0x1b => {
                let _ = ctx.send(Cmd::AskReply {
                    id,
                    text: String::new(),
                });
                app.ask_modal = None;
            }
            127 | 8 => {
                if let Some(m) = app.ask_modal.as_mut() {
                    pop_utf8(&mut m.buf);
                }
            }
            22 => {
                if let Some(m) = app.ask_modal.as_mut() {
                    paste_into(&mut m.buf, 500, false);
                }
            }
            _ if b >= 32 => {
                if let Some(m) = app.ask_modal.as_mut() {
                    if m.buf.len() < 500 {
                        m.buf.push(b);
                    }
                }
            }
            _ => {}
        },
    }
}

fn ask_text(app: &mut App, ctx: &mpsc::Sender<Cmd>, text: String) {
    app.say(&format!("❯ {text}"), VK::User);
    app.state = AState::Thinking;
    let _ = ctx.send(Cmd::Ask {
        text,
        sess: app.sess.clone(),
        prov: prov_of(&app.cfg),
        model: app.cfg.model.clone(),
        cfg: app.cfg.clone(),
    });
}

// Kullanici kabugu: komutu ayri thread'de calistir, cikti panele aksin.
fn start_shell(app: &mut App, cmd: String) {
    if app.shell_rx.is_some() {
        app.meta("a shell command is already running");
        return;
    }
    let line = cmd.trim().to_string();
    if line.is_empty() {
        return;
    }
    app.say(&format!("$ {line}"), VK::Tool);
    app.state = AState::Working;
    let ws = std::path::PathBuf::from(&app.cfg.workspace);
    let (stx, srx) = mpsc::channel::<String>();
    app.shell_rx = Some(srx);
    std::thread::spawn(move || {
        let out = crate::tools::shell(&line, &ws, None);
        let _ = stx.send(out);
    });
}

// ── ayarlar ─────────────────────────────────────────────────────
const SET_N: usize = 8;
const MAX_TOKS: [u32; 9] = [0, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072];
const CTXS: [u32; 7] = [16000, 32000, 64000, 120000, 200000, 500000, 1000000];

struct Settings {
    sel: usize,
    typing: bool,
    type_buf: Vec<u8>,
}

fn set_label(i: usize) -> &'static str {
    const L: [&str; 8] = [
        "temperature",
        "top_p",
        "max_tokens",
        "context_limit",
        "provider",
        "tools",
        "confirm_writes",
        "thinking",
    ];
    L[i.min(7)]
}

fn onoff(b: bool) -> String {
    if b {
        "on".into()
    } else {
        "off".into()
    }
}

fn set_value(app: &App, i: usize) -> String {
    match i {
        0 => format!("{:.2}", app.cfg.temperature),
        1 => format!("{:.2}", app.cfg.top_p),
        2 => {
            if app.cfg.max_tokens == 0 {
                "auto".into()
            } else {
                app.cfg.max_tokens.to_string()
            }
        }
        3 => app.cfg.context_limit.to_string(),
        4 => app.cfg.provider.clone(),
        5 => onoff(app.cfg.tools_enabled),
        6 => onoff(app.cfg.confirm_writes),
        _ => onoff(app.show_think),
    }
}

fn cycle_u32(cur: u32, opts: &[u32], dir: i32) -> u32 {
    let i = opts.iter().position(|&x| x == cur).unwrap_or(0);
    opts[((i as i32 + dir).rem_euclid(opts.len() as i32)) as usize]
}

fn settings_cur(app: &App, x0: usize, w: usize) -> Option<(i32, i32)> {
    let s = app.settings.as_ref()?;
    if !s.typing {
        return None;
    }
    let blen = String::from_utf8_lossy(&s.type_buf).chars().count();
    let cc = (x0 as i32 + 4 + set_label(s.sel).len() as i32 + blen as i32)
        .min(w as i32 - 2)
        .max(0);
    Some((2 + s.sel as i32 + 1, cc + 1))
}

fn set_adjust(app: &mut App, dir: i32) {
    let sel = app.settings.as_ref().map(|s| s.sel).unwrap_or(0);
    match sel {
        0 => {
            app.cfg.temperature = ((app.cfg.temperature + dir as f32 * 0.1) * 10.0).round() / 10.0;
            app.cfg.temperature = app.cfg.temperature.clamp(0.0, 2.0);
        }
        1 => {
            app.cfg.top_p = ((app.cfg.top_p + dir as f32 * 0.05) * 100.0).round() / 100.0;
            app.cfg.top_p = app.cfg.top_p.clamp(0.0, 1.0);
        }
        2 => app.cfg.max_tokens = cycle_u32(app.cfg.max_tokens, &MAX_TOKS, dir),
        3 => app.cfg.context_limit = cycle_u32(app.cfg.context_limit, &CTXS, dir),
        4 => {
            let to = if app.cfg.provider == "nvidia" {
                "openrouter"
            } else {
                "nvidia"
            };
            set_provider(app, to);
            return;
        }
        5 => app.cfg.tools_enabled = !app.cfg.tools_enabled,
        6 => app.cfg.confirm_writes = !app.cfg.confirm_writes,
        _ => {
            app.show_think = !app.show_think;
            app.cfg.show_thinking = app.show_think;
        }
    }
    let _ = config::save(&app.cfg);
}

fn set_commit_type(app: &mut App) {
    let (sel, buf) = match app.settings.as_ref() {
        Some(s) => (
            s.sel,
            String::from_utf8_lossy(&s.type_buf)
                .replace(',', ".")
                .trim()
                .to_string(),
        ),
        None => return,
    };
    if sel > 1 || buf.is_empty() {
        return;
    }
    if let Ok(v) = buf.parse::<f32>() {
        if sel == 0 {
            app.cfg.temperature = v.clamp(0.0, 2.0);
        } else {
            app.cfg.top_p = v.clamp(0.0, 1.0);
        }
        let _ = config::save(&app.cfg);
    }
    if let Some(s) = app.settings.as_mut() {
        s.typing = false;
        s.type_buf.clear();
    }
}

fn settings_event(app: &mut App, ev: Ev) {
    let typing = app.settings.as_ref().is_some_and(|s| s.typing);
    match ev {
        Ev::Up => {
            if !typing {
                if let Some(s) = app.settings.as_mut() {
                    s.sel = s.sel.saturating_sub(1);
                }
            }
        }
        Ev::Down => {
            if !typing {
                if let Some(s) = app.settings.as_mut() {
                    if s.sel + 1 < SET_N {
                        s.sel += 1;
                    }
                }
            }
        }
        Ev::Left => {
            if !typing {
                set_adjust(app, -1);
            }
        }
        Ev::Right => {
            if !typing {
                set_adjust(app, 1);
            }
        }
        Ev::Mouse(cb, col, row, rel) => {
            if typing {
                return;
            }
            if cb & 64 != 0 {
                if cb & 1 == 0 {
                    if let Some(s) = app.settings.as_mut() {
                        s.sel = s.sel.saturating_sub(3);
                    }
                } else if let Some(s) = app.settings.as_mut() {
                    s.sel = (s.sel + 3).min(SET_N - 1);
                }
                return;
            }
            if rel {
                return;
            }
            let (x0, _) = app.last_panel.unwrap_or((0, 0));
            if col < x0 as i32 || row < 2 {
                return;
            }
            let idx = (row - 2) as usize;
            if idx < SET_N {
                if let Some(s) = app.settings.as_mut() {
                    s.sel = idx;
                }
            }
        }
        Ev::Key(b) => {
            if typing {
                match b {
                    13 | 10 => set_commit_type(app),
                    0x1b => {
                        if let Some(s) = app.settings.as_mut() {
                            s.typing = false;
                            s.type_buf.clear();
                        }
                    }
                    127 | 8 => {
                        if let Some(s) = app.settings.as_mut() {
                            pop_utf8(&mut s.type_buf);
                        }
                    }
                    22 => {
                        if let Some(s) = app.settings.as_mut() {
                            paste_into(&mut s.type_buf, 12, true);
                        }
                    }
                    _ if b >= 32 => {
                        if let Some(s) = app.settings.as_mut() {
                            if s.type_buf.len() < 12 {
                                s.type_buf.push(b);
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }
            match b {
                0x1b => {
                    app.settings = None;
                }
                13 | 10 => {
                    let sel = app.settings.as_ref().map(|s| s.sel).unwrap_or(0);
                    if sel < 2 {
                        if let Some(s) = app.settings.as_mut() {
                            s.typing = true;
                            s.type_buf.clear();
                        }
                    }
                }
                b'-' | b'_' => set_adjust(app, -1),
                b'=' | b'+' => set_adjust(app, 1),
                _ => {}
            }
        }
    }
}

fn help_text() -> &'static str {
    "/help /model /settings /provider openrouter|nvidia /run <cmd> /cd <dir> /init /new /sessions /resume <id> /compact /context /clear \
/key /tools /thinking /m /c /quit\n\
keys: TAB focus chat · ! shell mode · Ctrl+V paste · wheel scroll · q quit"
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
            app.status = format!(
                "enter new {} API key ({})",
                app.cfg.provider,
                config::key_hint(&app.cfg.provider)
            );
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
            app.cfg.show_thinking = app.show_think;
            let _ = config::save(&app.cfg);
            app.meta(&format!(
                "thinking stream {}",
                if app.show_think { "shown" } else { "hidden" }
            ));
        }
        "settings" => {
            app.focus = false;
            app.settings = Some(Settings {
                sel: 0,
                typing: false,
                type_buf: Vec::new(),
            });
        }
        "run" => {
            if arg.is_empty() {
                app.shell_mode = true;
                app.focus = true;
                app.buf.clear();
            } else {
                start_shell(app, arg);
            }
        }
        "cd" => {
            if arg.is_empty() {
                app.meta(&format!("workspace: {}", app.cfg.workspace));
            } else {
                let p = std::path::PathBuf::from(&arg);
                let full = if p.is_absolute() {
                    p
                } else {
                    std::path::PathBuf::from(&app.cfg.workspace).join(&p)
                };
                match full.canonicalize() {
                    Ok(c) if c.is_dir() => {
                        app.cfg.workspace = c.to_string_lossy().into_owned();
                        let _ = config::save(&app.cfg);
                        app.meta(&format!("workspace: {}", app.cfg.workspace));
                    }
                    Ok(_) => app.meta(&format!("not a directory: {arg}")),
                    Err(e) => app.meta(&format!("cannot access {arg}: {e}")),
                }
            }
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
                prov: prov_of(&app.cfg),
                model: app.cfg.model.clone(),
            });
            app.meta("compacting context…");
        }
        "models" | "model" if arg.is_empty() => {
            open_picker(app, mtx);
        }
        "model" => {
            set_model(&mut app.cfg, &arg);
            app.sess.model = arg.clone();
            let _ = config::save(&app.cfg);
            app.meta(&format!("model set: {arg}"));
        }
        "provider" => {
            if arg.is_empty() {
                app.meta(&format!(
                    "provider: {} (openrouter | nvidia)",
                    app.cfg.provider
                ));
            } else if arg == "openrouter" || arg == "nvidia" {
                set_provider(app, &arg);
            } else {
                app.meta("usage: /provider openrouter|nvidia");
            }
        }
        "init" => {
            app.say("❯ /init", VK::User);
            app.state = AState::Thinking;
            let _ = tx.send(Cmd::Ask {
                text: "Survey this workspace and report: 1) project tree (top 2 levels), \
2) languages and build systems detected, 3) entry points, READMEs and configs, \
4) how to build, test and run it. Use list_dir, glob and read_file. Keep it factual. \
Do not write any files. End with a 5-line project summary."
                    .to_string(),
                sess: app.sess.clone(),
                prov: prov_of(&app.cfg),
                model: app.cfg.model.clone(),
                cfg: app.cfg.clone(),
            });
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
            if nb == b'O' {
                // SS3 ok tuslari (Windows VT): ESC O A / ESC O B
                match term::poll_byte() {
                    Some(b'A') => {
                        let _ = etx.send(Ev::Up);
                    }
                    Some(b'B') => {
                        let _ = etx.send(Ev::Down);
                    }
                    Some(b'C') => {
                        let _ = etx.send(Ev::Right);
                    }
                    Some(b'D') => {
                        let _ = etx.send(Ev::Left);
                    }
                    _ => {}
                }
                continue;
            }
            if nb != b'[' {
                let _ = etx.send(Ev::Key(0x1b));
                let _ = etx.send(Ev::Key(nb));
                continue;
            }
            if let Some(f) = term::poll_byte() {
                if f == b'A' {
                    let _ = etx.send(Ev::Up);
                } else if f == b'B' {
                    let _ = etx.send(Ev::Down);
                } else if f == b'C' {
                    let _ = etx.send(Ev::Right);
                } else if f == b'D' {
                    let _ = etx.send(Ev::Left);
                } else if let Some((cb, col, row, rel)) = parse_sgr_mouse(f) {
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
    let wtx = agent::spawn(ctx_rx);

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
        picker: None,
        ask_modal: None,
        ask_pos: None,
        shell_mode: false,
        shell_rx: None,
        settings: None,
    };
    app.show_think = app.cfg.show_thinking;
    if no_key {
        let prov = app.cfg.provider.clone();
        let (where_, hint) = if prov == "nvidia" {
            (
                "an NVIDIA NIM API key below",
                "Get one at https://build.nvidia.com",
            )
        } else {
            (
                "your OpenRouter API key below",
                "Get one at https://openrouter.ai/keys",
            )
        };
        app.say(
            &format!(
                "Welcome to synapse-distilled.\nPaste {where_} (stored in the user config). {hint}"
            ),
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
                ev if app.ask_modal.is_some() => {
                    ask_modal_event(&mut app, ev, &ctx);
                    dirty = true;
                }
                ev if app.settings.is_some() => {
                    settings_event(&mut app, ev);
                    dirty = true;
                }
                ev if app.picker.is_some() => {
                    picker_event(&mut app, ev);
                    dirty = true;
                }
                Ev::Up | Ev::Down | Ev::Left | Ev::Right => {}
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
                        let want = if app.cfg.provider == "nvidia" {
                            "nvapi-"
                        } else {
                            "sk-or-"
                        };
                        if k.starts_with(want) || k.len() > 20 {
                            if app.cfg.provider == "nvidia" {
                                app.cfg.nvidia_key = k;
                            } else {
                                app.cfg.api_key = k;
                            }
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
                            app.cap_err = Some(format!(
                                "that does not look like a {} key (expected {})",
                                app.cfg.provider,
                                config::key_hint(&app.cfg.provider)
                            ));
                            app.buf.clear();
                        }
                        dirty = true;
                    }
                    127 | 8 => {
                        pop_utf8(&mut app.buf);
                        dirty = true;
                    }
                    3 => quit = true,
                    22 => {
                        paste_into(&mut app.buf, 256, true);
                        dirty = true;
                    }
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
                            } else if app.shell_mode {
                                start_shell(&mut app, line);
                            } else {
                                ask_text(&mut app, &ctx, line);
                            }
                        }
                        dirty = true;
                    }
                    127 | 8 => {
                        pop_utf8(&mut app.buf);
                        dirty = true;
                    }
                    0x1b => {
                        app.focus = false;
                        dirty = true;
                    }
                    22 => {
                        paste_into(&mut app.buf, 4000, false);
                        dirty = true;
                    }
                    b if b >= 32 && app.buf.len() < 4000 => {
                        app.buf.push(b);
                        dirty = true;
                    }
                    _ => {}
                },
                Ev::Key(b) => {
                    if b == b'q' || b == 3 {
                        quit = true;
                    } else if b == 0x1b {
                        if app.shell_mode {
                            app.shell_mode = false;
                            dirty = true;
                        } else {
                            quit = true;
                        }
                    } else if b == b'!' {
                        // shell modu: terminalden direkt komut calistir
                        app.shell_mode = !app.shell_mode;
                        app.focus = true;
                        app.buf.clear();
                        dirty = true;
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
                WEvent::AskUser {
                    id,
                    label,
                    question,
                    options,
                    buttons,
                } => {
                    app.ask_modal = Some(AskModal {
                        id,
                        label,
                        question,
                        options,
                        buf: Vec::new(),
                        buttons,
                        sel: 0,
                        btn_rows: Vec::new(),
                    });
                    app.focus = false;
                    dirty = true;
                }
                WEvent::PermSet { tool, value } => {
                    app.cfg.permissions.insert(tool.clone(), value.clone());
                    let _ = config::save(&app.cfg);
                    app.meta(&format!("permission: {tool} = {value}"));
                    dirty = true;
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
        // shell komutu sonucu
        if let Some(rx) = app.shell_rx.as_ref() {
            if let Ok(out) = rx.try_recv() {
                for l in wrap(out.trim_end(), 90) {
                    app.view.push(VLine {
                        text: l,
                        kind: VK::Tool,
                    });
                }
                app.shell_rx = None;
                if app.state == AState::Working {
                    app.state = AState::Waiting;
                }
                app.scroll = 0;
                dirty = true;
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
            if let Some(p) = app.picker.as_mut() {
                if list[0].starts_with("error:") {
                    p.error = Some(list[0].clone());
                } else {
                    p.fetched = list;
                    p.error = None;
                }
            } else if list[0].starts_with("error:") {
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
                "{} v{} │ {}:{} │ ctx {}% │ {} │ {}",
                NAME,
                VER,
                prov_tag(&app.cfg),
                model_short(&app.cfg.model),
                pct,
                st,
                app.sess.id
            )
            .chars()
            .take(cw.saturating_sub(1))
            .collect();
            screen.text(x0i, 0, &head, 150);
            // mesajlar (overlay aciksa altinda kalan alana sigar)
            let hist_rows = h - 4;
            let mut clip_top = hist_rows as i32 + 1;
            if app.picker.is_some() || app.settings.is_some() {
                clip_top = 1;
            }
            if let Some(m) = app.ask_modal.as_ref() {
                clip_top = clip_top.min(modal_layout(m, cw, h).start);
            }
            let msg_rows = (clip_top - 1).max(0) as usize;
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
            let maxs = lines.len().saturating_sub(msg_rows);
            if app.scroll > maxs {
                app.scroll = maxs;
            }
            let end = lines.len().saturating_sub(app.scroll);
            let start = end.saturating_sub(msg_rows);
            let shown = &lines[start..end];
            let base = msg_rows - shown.len();
            for (r, (txt, v)) in shown.iter().enumerate() {
                let s: String = txt.chars().take(cw).collect();
                screen.text(x0i, 1 + base as i32 + r as i32, &s, *v);
            }
            // model secici overlay (mesajlarin ustune)
            if let Some(p) = app.picker.as_mut() {
                let total = p.len();
                if total > 0 && p.sel >= total {
                    p.sel = total - 1;
                }
                let title: String = "Model — Enter select · Esc cancel · c custom"
                    .chars()
                    .take(cw.saturating_sub(1))
                    .collect();
                screen.text(x0i, 1, &title, 255);
                if let Some(e) = p.error.clone() {
                    let s: String = e.chars().take(cw.saturating_sub(1)).collect();
                    screen.text(x0i, 2, &s, VK::Err.v());
                    p.prompt_row = None;
                } else {
                    let room = hist_rows.saturating_sub(3).max(1);
                    let vis = total.min(room);
                    if p.sel < p.scroll {
                        p.scroll = p.sel;
                    }
                    if vis > 0 && p.sel >= p.scroll + vis {
                        p.scroll = p.sel + 1 - vis;
                    }
                    let max0 = total.saturating_sub(vis.max(1));
                    if p.scroll > max0 {
                        p.scroll = max0;
                    }
                    for r in 0..vis {
                        let i = p.scroll + r;
                        let label = match p.id_at(i) {
                            Some(id) => id,
                            None => CUSTOM_ENTRY.to_string(),
                        };
                        let sel = i == p.sel;
                        let txt = format!("{} {}", if sel { ">" } else { " " }, label);
                        let s: String = txt.chars().take(cw.saturating_sub(1)).collect();
                        screen.text(x0i, 2 + r as i32, &s, if sel { 255 } else { 150 });
                    }
                    if p.custom_mode {
                        let pr = 2 + vis as i32 + 1;
                        let prompt = format!("id: {}", String::from_utf8_lossy(&p.custom_buf));
                        let s: String = prompt.chars().take(cw.saturating_sub(1)).collect();
                        screen.text(x0i, pr, &s, 255);
                        p.prompt_row = Some(pr);
                    } else {
                        p.prompt_row = None;
                    }
                }
            }
            // izin/soru modali (en ustte)
            app.ask_pos = None;
            if let Some(m) = app.ask_modal.as_ref() {
                let lay = modal_layout(m, cw, h);
                for (r, (txt, v)) in lay.head.iter().enumerate() {
                    let s: String = clean(txt).chars().take(cw).collect();
                    screen.text(x0i, lay.start + r as i32, &s, *v);
                }
                if let Some(m) = app.ask_modal.as_mut() {
                    if !lay.blines.is_empty() {
                        m.btn_rows.clear();
                        for (r, l) in lay.blines.iter().enumerate() {
                            let gr = lay.start + lay.head.len() as i32 + r as i32;
                            screen.text(x0i, gr, l, 150);
                            if let Some((sx0, _, si)) =
                                lay.bspans[r].iter().find(|(_, _, i)| *i == m.sel)
                            {
                                if let Some(opt) = m.options.get(*si) {
                                    let st = format!("[{}]", clean(opt));
                                    screen.text(x0i + *sx0 as i32, gr, &st, 255);
                                }
                            }
                            let spans: Vec<(i32, i32, usize)> = lay.bspans[r]
                                .iter()
                                .map(|(a, b, i)| (x0i + *a as i32, x0i + *b as i32, *i))
                                .collect();
                            m.btn_rows.push((gr, spans));
                        }
                        app.ask_pos = None;
                    } else {
                        let blen = m.buf.len().min(cw.saturating_sub(4));
                        let ccx = (x0i + 2 + blen as i32).min(w as i32 - 2).max(0);
                        app.ask_pos = Some((lay.start + lay.head.len() as i32 - 1 + 1, ccx + 1));
                    }
                }
            }
            // ayarlar overlay
            if let Some(st) = app.settings.as_mut() {
                if st.sel >= SET_N {
                    st.sel = SET_N - 1;
                }
            }
            if let Some(st) = app.settings.as_ref() {
                let (sel, typing, tbuf) = (
                    st.sel,
                    st.typing,
                    String::from_utf8_lossy(&st.type_buf).into_owned(),
                );
                let title: String =
                    "Settings — Up/Down · Left/Right adjust · Enter type · Esc close"
                        .chars()
                        .take(cw.saturating_sub(1))
                        .collect();
                screen.text(x0i, 1, &title, 255);
                for r in 0..SET_N {
                    let is_sel = r == sel;
                    let mut txt = format!(
                        "{} {}: {}",
                        if is_sel { ">" } else { " " },
                        set_label(r),
                        set_value(&app, r)
                    );
                    if typing && is_sel {
                        txt = format!("> {}: {tbuf}", set_label(r));
                    }
                    let s: String = txt.chars().take(cw.saturating_sub(1)).collect();
                    screen.text(x0i, 2 + r as i32, &s, if is_sel { 255 } else { 150 });
                }
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
            } else if app.shell_mode {
                (
                    format!("$ {}", String::from_utf8_lossy(&app.buf)),
                    if app.focus { 255 } else { 200 },
                )
            } else if app.buf.is_empty() && !app.focus {
                (
                    "TAB or click to write · /help · ! shell".to_string(),
                    VK::Hint.v(),
                )
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
            let custom_cur: Option<(i32, i32)> = match app.picker.as_ref() {
                Some(p) if p.custom_mode => p.prompt_row.map(|pr| {
                    let blen = String::from_utf8_lossy(&p.custom_buf).chars().count();
                    let cx = (x0 as i32 + 5 + blen as i32).min(w as i32 - 2).max(0);
                    (pr + 1, cx + 1)
                }),
                _ => None,
            };
            if let Some((rr, ccx)) = custom_cur {
                buf.push_str(&format!("\x1b[{rr};{ccx}H\x1b[?25h"));
            } else if let Some((rr, ccx)) = app.ask_pos {
                buf.push_str(&format!("\x1b[{rr};{ccx}H\x1b[?25h"));
            } else if let Some((rr, ccx)) = settings_cur(&app, x0, w) {
                buf.push_str(&format!("\x1b[{rr};{ccx}H\x1b[?25h"));
            } else if app.capture || app.focus {
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
