#[allow(dead_code)]
#[path = "../../tui/src/render.rs"]
mod render;
#[allow(dead_code)]
#[path = "../../tui/src/screen.rs"]
mod screen;
#[path = "../../tui/src/term.rs"]
mod term;

mod audio;

use render::{Canvas, Fx, Pass};
use screen::{Screen, BG_BLACK, BG_CYCLE};
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const TAU: f64 = std::f64::consts::TAU;
const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    StaticStar,
    ScanErase,
    ScanFill,
    Shrink,
    Active,
    Processing,
    Glow,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::StaticStar => "1 STATIC",
            Phase::ScanErase => "2 SCAN / ERASE",
            Phase::ScanFill => "3 SCAN / KEEP",
            Phase::Shrink => "4 SHRINK > TOP-LEFT",
            Phase::Active => "5 COIN Y + AUDIO",
            Phase::Processing => "6 PROCESSING / WHEEL",
            Phase::Glow => "7 GLOW",
        }
    }
}

const T_STATIC: f64 = 1.4;
const T_SCAN1: f64 = 2.4;
const T_SCAN2: f64 = 2.4;
const T_SHRINK: f64 = 2.8;
const T_FACE: f64 = 1.1;
const T_WHEEL: f64 = 0.6;
const T_GLOW: f64 = 1.0;

fn sample(cv: &Canvas, x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 || x >= cv.n || y >= cv.n {
        0
    } else {
        cv.px[(y * cv.n + x) as usize]
    }
}

fn blit(s: &mut Screen, cv: &Canvas, x: i32, y: i32, scale: f64) {
    if scale <= 0.02 {
        return;
    }
    let n = cv.n as f64;
    let dw = ((n * scale).round() as i32).max(1);
    let dh = ((n * scale).round() as i32).max(1);
    for j in 0..(dh + 1) / 2 {
        let sy0 = ((2.0 * j as f64) / scale).floor() as i32;
        let sy1 = (((2.0 * j as f64 + 1.0) / scale).floor() as i32).max(sy0);
        for i in 0..dw {
            let sx = ((i as f64) / scale).floor() as i32;
            s.half(x + i, y + j, sample(cv, sx, sy0), sample(cv, sx, sy1));
        }
    }
}

fn glow(cv: &mut Canvas, amount: f64) {
    if amount <= 0.001 {
        return;
    }
    let n = cv.n;
    let src = cv.px.clone();
    let a = amount.clamp(0.0, 1.0);
    for i in 0..(n * n) as usize {
        if src[i] > 0 {
            let v = src[i] as f64 + (255.0 - src[i] as f64) * a;
            cv.px[i] = v.round().max(210.0) as u8;
        } else {
            let (x, y) = ((i as i32) % n, (i as i32) / n);
            let mut near = false;
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < n && ny < n && src[(ny * n + nx) as usize] > 0 {
                        near = true;
                    }
                }
            }
            if near {
                cv.px[i] = (60.0 + 110.0 * a) as u8;
            }
        }
    }
}

fn ease(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

enum Key {
    Char(u8),
    F11,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut auto_demo = args.iter().any(|a| a == "--demo");

    let saved = term::raw_start();
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J");
    let _ = out.flush();

    let (wtx, wrx) = mpsc::sync_channel::<String>(1);
    std::thread::spawn(move || {
        let so = std::io::stdout();
        let mut lock = so.lock();
        while let Ok(buf) = wrx.recv() {
            if lock.write_all(buf.as_bytes()).is_err() {
                break;
            }
            let _ = lock.flush();
        }
    });

    let (tx, rx) = mpsc::channel::<Key>();
    std::thread::spawn(move || {
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let b = term::read_byte();
            if b == 0 {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            if b == 0x1b {
                pending.clear();
                pending.push(b);
                continue;
            }
            if pending.first() == Some(&0x1b) {
                if pending.len() == 1 {
                    if b == b'[' || b == b'O' {
                        pending.push(b);
                    } else {
                        pending.clear();
                    }
                    continue;
                }
                if (0x40..=0x7e).contains(&b) {
                    let seq: Vec<u8> = std::mem::take(&mut pending);
                    if seq[1] == b'['
                        && b == b'~'
                        && seq.get(2) == Some(&23)
                        && tx.send(Key::F11).is_err()
                    {
                        return;
                    }
                } else {
                    pending.push(b);
                    if pending.len() > 24 {
                        pending.clear();
                    }
                }
                continue;
            }
            if tx.send(Key::Char(b)).is_err() {
                return;
            }
        }
    });

    let mic = audio::start_mic();
    let mut mic_on = mic.is_some();

    let mut screen = Screen::new(80, 24);
    screen.set_bg(BG_BLACK);
    let mut phase = Phase::StaticStar;
    let mut pt = 0.0f64;
    let mut audio = 0.0f64;
    let mut paused = false;
    let mut fullscreen = false;
    let mut hud = true;
    let mut bg_idx = 0usize;
    let mut base_scale = 0.5f64;
    let mut started = false;
    let mut prev_w = 0usize;
    let mut prev_h = 0usize;
    let mut n = 73i32;
    let mut cv = Canvas::new(n);

    let t0 = Instant::now();
    let mut last = t0;

    loop {
        while let Ok(k) = rx.try_recv() {
            let mut quit = false;
            match k {
                Key::Char(b'q') | Key::Char(3) => quit = true,
                Key::Char(b' ') => paused = !paused,
                Key::Char(b'r') => {
                    phase = Phase::StaticStar;
                    pt = 0.0;
                    auto_demo = false;
                }
                Key::Char(b'p') => {
                    if phase == Phase::Processing {
                        phase = Phase::Glow;
                    } else {
                        phase = Phase::Processing;
                    }
                    pt = 0.0;
                }
                Key::Char(b'm') => mic_on = !mic_on,
                Key::Char(b'h') => hud = !hud,
                Key::Char(b'b') => {
                    bg_idx = (bg_idx + 1) % BG_CYCLE.len();
                    screen.set_bg(BG_CYCLE[bg_idx]);
                }
                Key::Char(b'=') | Key::Char(b'+') => base_scale = (base_scale + 0.04).min(1.0),
                Key::Char(b'-') | Key::Char(b'_') => base_scale = (base_scale - 0.04).max(0.08),
                Key::Char(b'd') => {
                    auto_demo = !auto_demo;
                    if auto_demo {
                        phase = Phase::Processing;
                        pt = 0.0;
                    }
                }
                Key::F11 => {
                    fullscreen = !fullscreen;
                    term::set_fullscreen(fullscreen);
                }
                _ => {}
            }
            if quit {
                term::set_fullscreen(false);
                term::raw_stop(&saved);
                let _ = out.write_all(b"\x1b[?25h\x1b[0m\x1b[?1049l");
                let _ = out.flush();
                std::process::exit(0);
            }
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.1);
        last = now;
        let t = now.duration_since(t0).as_secs_f64();
        if !paused {
            pt += dt;
        }

        let (tw, th) = term::size();
        let w = tw.max(30);
        let h = th.max(10);
        if !started {
            started = true;
            prev_w = w;
            prev_h = h;
        }
        let resized = w != prev_w || h != prev_h;
        if resized {
            prev_w = w;
            prev_h = h;
        }
        let want_n = ((w as i32).min((h as i32 - 2) * 2)).clamp(41, 145);
        if want_n != n {
            n = want_n;
            cv = Canvas::new(n);
        }

        if mic_on {
            if let Some(m) = &mic {
                if !m.failed() {
                    let v = m.level();
                    audio += (v - audio) * if v > audio { 0.5 } else { 0.12 };
                }
            }
        } else {
            audio *= 0.92;
        }

        if auto_demo && phase == Phase::Processing && pt > T_FACE + 8.0 * T_WHEEL {
            phase = Phase::Glow;
            pt = 0.0;
        }

        let prev_phase = phase;
        match phase {
            Phase::StaticStar if pt >= T_STATIC => phase = Phase::ScanErase,
            Phase::ScanErase if pt >= T_SCAN1 => phase = Phase::ScanFill,
            Phase::ScanFill if pt >= T_SCAN2 => phase = Phase::Shrink,
            Phase::Shrink if pt >= T_SHRINK => phase = Phase::Active,
            Phase::Glow if pt >= T_GLOW => phase = Phase::Active,
            _ => {}
        }
        if prev_phase != phase {
            pt = 0.0;
        }

        cv.clear();
        let nf = n as f64;
        let dark = screen.bg == BG_BLACK;
        match phase {
            Phase::StaticStar => {
                let p = Pass::base(t, 0);
                render::draw_pass(&mut cv, &p);
            }
            Phase::ScanErase => {
                let mut p = Pass::base(t, 0);
                p.th_u = 0.10 * pt;
                p.th_d = 0.10 * pt;
                p.fx = Fx::Scan;
                p.scan = -14.0 + (nf + 28.0) * (pt / T_SCAN1).clamp(0.0, 1.0);
                render::draw_pass(&mut cv, &p);
            }
            Phase::ScanFill => {
                let mut p = Pass::base(t, 0);
                p.th_u = 0.10 * (T_SCAN1 + pt);
                p.th_d = p.th_u;
                p.fx = Fx::ScanFill;
                p.scan = -14.0 + (nf + 28.0) * (pt / T_SCAN2).clamp(0.0, 1.0);
                render::draw_pass(&mut cv, &p);
            }
            Phase::Shrink => {
                let mut p = Pass::base(t, 0);
                p.th_u = 0.14 * (T_SCAN1 + T_SCAN2 + pt);
                p.th_d = p.th_u;
                render::draw_pass(&mut cv, &p);
            }
            Phase::Active => {
                render::draw_solid(&mut cv, t, false, dark, None);
            }
            Phase::Processing => {
                let a = if pt < T_FACE {
                    FRAC_PI_2 * (1.0 - ease(pt / T_FACE))
                } else {
                    ((pt - T_FACE) / T_WHEEL) * TAU
                };
                render::draw_solid(&mut cv, t, false, dark, Some(a));
            }
            Phase::Glow => {
                render::draw_solid(&mut cv, t, false, dark, None);
                let k = if pt < 0.18 {
                    pt / 0.18
                } else if pt < 0.55 {
                    1.0
                } else {
                    1.0 - (pt - 0.55) / 0.45
                };
                glow(&mut cv, k.clamp(0.0, 1.0));
            }
        }

        let audio_scale = base_scale * (1.0 + 0.38 * audio);
        let fill = (((h as f64 - 2.0) * 2.0) / (n as f64 * 0.548)).clamp(1.0, 2.2);
        let full_w = (n as f64 * fill) as i32;
        let full_x = (w as i32 - full_w) / 2;
        let full_y = 1 + (h as i32 - 2 - (n as f64 * fill / 2.0) as i32) / 2;
        let corner_x = 2i32;
        let corner_y = 1i32;
        let (bx, by, scale) = match phase {
            Phase::Shrink => {
                let k = ease(pt / T_SHRINK);
                let s = fill + (audio_scale - fill) * k;
                let x = full_x as f64 * (1.0 - k) + corner_x as f64 * k;
                let y = full_y as f64 * (1.0 - k) + corner_y as f64 * k;
                (x as i32, y as i32, s)
            }
            Phase::Active | Phase::Processing | Phase::Glow => (corner_x, corner_y, audio_scale),
            _ => (full_x, full_y, fill),
        };

        screen.reset(w, h);
        blit(&mut screen, &cv, bx, by, scale);

        if hud {
            screen.clear_row(0);
            screen.clear_row(h - 1);
            let bar = 22usize;
            let filled = ((audio * bar as f64).round() as usize).clamp(0, bar);
            let meter = format!("{}{}", "#".repeat(filled), ".".repeat(bar - filled));
            let mic_txt = if !mic_on {
                "OFF"
            } else if mic.as_ref().map(|m| m.failed()).unwrap_or(true) {
                "N/A"
            } else {
                "LIVE"
            };
            screen.text(
                0,
                0,
                &format!(
                    " ANIMATION-PRODUCTION  {}  mic {}  [{}] {:.2}{}",
                    phase.label(),
                    mic_txt,
                    meter,
                    audio,
                    if auto_demo { "  [demo]" } else { "" }
                ),
                255,
            );
            screen.text(
                0,
                h as i32 - 1,
                " p processing  r restart  m mic  h hud  b bg  +/- size  d demo  F11 full  q quit",
                210,
            );
        }

        let mut buf = screen.render();
        if resized {
            buf.insert_str(0, "\x1b[2J");
        }
        let _ = wtx.try_send(buf);

        let spent = now.elapsed();
        if spent < Duration::from_millis(33) {
            std::thread::sleep(Duration::from_millis(33) - spent);
        }
    }
}
