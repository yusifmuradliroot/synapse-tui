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
use screen::{Screen, BG_BLACK, BG_CYCLE, BG_FULL_BLUE};
use std::io::Write;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(u: u32) -> u32;
    fn timeEndPeriod(u: u32) -> u32;
}

#[cfg(windows)]
fn hires(on: bool) {
    unsafe {
        if on {
            timeBeginPeriod(1);
        } else {
            timeEndPeriod(1);
        }
    }
}

#[cfg(not(windows))]
fn hires(_on: bool) {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Blank,
    Scan,
    ScanFill,
    CoinY,
    Processing,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::Blank => "0 BLANK",
            Phase::Scan => "1 SCAN",
            Phase::ScanFill => "2 SCAN KEEP",
            Phase::CoinY => "3 COIN Y + AUDIO",
            Phase::Processing => "4 PROCESSING / WHEEL",
        }
    }
}

const T_BLANK: f64 = 1.0;
const T_SWEEP: f64 = 2.2;
const T_SCANFILL: f64 = 2.6;
const T_FACE: f64 = 1.0;
const T_XFADE: f64 = 0.3;
const T_DEMO_WHEEL: f64 = 4.0;
const WHEEL_RATE: f64 = std::f64::consts::TAU;
const FRAME: Duration = Duration::from_micros(16_667);

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

fn audio_flash(cv: &mut Canvas, amount: f64, seed: u32) {
    let a = (amount * amount).clamp(0.0, 1.0);
    if a <= 0.01 {
        return;
    }
    for v in cv.px.iter_mut() {
        if *v > 0 {
            *v = (*v as f64 + (255.0 - *v as f64) * a).round() as u8;
        }
    }
    if a > 0.35 {
        let n = cv.n;
        let mut h = seed.wrapping_mul(0x9E37_79B1).wrapping_add(0x85EB_CA6B);
        let spikes = 2 + (a * 6.0) as usize;
        for _ in 0..spikes {
            h = h.wrapping_mul(0x85EB_CA6B).wrapping_add(0xC2B2_AE35);
            let i = (h as usize) % (n * n) as usize;
            if cv.px[i] > 0 {
                cv.px[i] = 255;
                let (x, y) = ((i as i32) % n, (i as i32) / n);
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < n && ny < n {
                        let j = (ny * n + nx) as usize;
                        if cv.px[j] < 200 {
                            cv.px[j] = (200.0 + 55.0 * a).min(255.0).max(cv.px[j] as f64) as u8;
                        }
                    }
                }
            }
        }
    }
}

fn ease(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

fn blend(cv: &mut Canvas, snap: &[u8], k: f64) {
    let k = k.clamp(0.0, 1.0);
    for (v, s) in cv.px.iter_mut().zip(snap.iter()) {
        *v = (*s as f64 * (1.0 - k) + *v as f64 * k).round() as u8;
    }
}

fn angle_rate(a: f64) -> f64 {
    1.5 * (0.5 + (1.0 - a.cos().abs()))
}

enum Key {
    Char(u8),
    F11,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("animation-production v{VERSION}");
        return;
    }
    let mut auto_demo = args.iter().any(|a| a == "--demo");

    let saved = term::raw_start();
    hires(true);
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J");
    let _ = out.flush();

    let shared: Arc<Mutex<String>> = Arc::new(Mutex::new(String::with_capacity(1 << 18)));
    let (wtx, wrx) = mpsc::sync_channel::<()>(1);
    let wshared = shared.clone();
    std::thread::spawn(move || {
        let so = std::io::stdout();
        let mut lock = so.lock();
        while wrx.recv().is_ok() {
            let s = wshared.lock().unwrap().clone();
            if lock.write_all(s.as_bytes()).is_err() {
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
    screen.set_bg(BG_FULL_BLUE);
    let mut phase = Phase::Blank;
    let mut pt = 0.0f64;
    let mut audio = 0.0f64;
    let mut disp = 0.0f64;
    let mut coin_angle = 0.0f64;
    let mut enter_angle = 0.0f64;
    let mut scan_prog = 0.0f64;
    let mut wheel_th = 0.0f64;
    let mut flat = false;
    let mut prev_pt = 0.0f64;
    let mut xfade: Option<(Vec<u8>, f64)> = None;
    let mut rim_t = 0.0f64;
    let mut flash_w = 0.0f64;
    let mut paused = false;
    let mut fullscreen = false;
    let mut hud = true;
    let mut bg_idx = 0usize;
    let mut size_mul = 1.0f64;
    let mut started = false;
    let mut prev_w = 0usize;
    let mut prev_h = 0usize;
    let mut last_hash = 0u64;
    let mut n = 73i32;
    let mut cv = Canvas::new(n);
    let mut frame_no = 0u32;

    let t0 = Instant::now();
    let mut last = t0;

    loop {
        let mut touched = false;
        while let Ok(k) = rx.try_recv() {
            touched = true;
            let mut quit = false;
            match k {
                Key::Char(b'q') | Key::Char(3) => quit = true,
                Key::Char(b' ') => paused = !paused,
                Key::Char(b'r') => {
                    phase = Phase::Blank;
                    pt = 0.0;
                    prev_pt = 0.0;
                    scan_prog = 0.0;
                    coin_angle = 0.0;
                    wheel_th = 0.0;
                    flat = false;
                    xfade = None;
                    rim_t = 0.0;
                    flash_w = 0.0;
                    auto_demo = false;
                }
                Key::Char(b'p') => {
                    if phase == Phase::Processing {
                        phase = Phase::CoinY;
                        xfade = Some((cv.px.clone(), 0.0));
                    } else {
                        phase = Phase::Processing;
                        enter_angle = coin_angle;
                        wheel_th = 0.0;
                        flat = false;
                        xfade = Some((cv.px.clone(), 0.0));
                    }
                    pt = 0.0;
                    prev_pt = 0.0;
                }
                Key::Char(b'm') => mic_on = !mic_on,
                Key::Char(b'h') => hud = !hud,
                Key::Char(b'b') => {
                    bg_idx = (bg_idx + 1) % BG_CYCLE.len();
                    screen.set_bg(BG_CYCLE[bg_idx]);
                }
                Key::Char(b'=') | Key::Char(b'+') => size_mul = (size_mul + 0.05).min(1.5),
                Key::Char(b'-') | Key::Char(b'_') => size_mul = (size_mul - 0.05).max(0.3),
                Key::Char(b'd') => {
                    auto_demo = !auto_demo;
                    if auto_demo {
                        phase = Phase::Processing;
                        enter_angle = coin_angle;
                        wheel_th = 0.0;
                        flat = false;
                        xfade = Some((cv.px.clone(), 0.0));
                        pt = 0.0;
                        prev_pt = 0.0;
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
                hires(false);
                let _ = out.write_all(b"\x1b[?25h\x1b[0m\x1b[?1049l");
                let _ = out.flush();
                std::process::exit(0);
            }
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.05);
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
            touched = true;
        }
        let want_n = ((w as i32).min((h as i32 - 2) * 2)).clamp(41, 121);
        if want_n != n {
            n = want_n;
            cv = Canvas::new(n);
            xfade = None;
        }

        if mic_on {
            if let Some(m) = &mic {
                if !m.failed() {
                    let v = m.level();
                    let tau = if v > audio { 0.03 } else { 0.25 };
                    audio += (v - audio) * (1.0 - (-dt / tau).exp());
                }
            }
        } else {
            audio *= (-dt / 0.4).exp();
        }
        if !paused {
            disp = audio;
        }

        if auto_demo && phase == Phase::Processing && pt > T_FACE + T_DEMO_WHEEL {
            phase = Phase::CoinY;
            xfade = Some((cv.px.clone(), 0.0));
            pt = 0.0;
        }

        let prev_phase = phase;
        match phase {
            Phase::Blank if pt >= T_BLANK => phase = Phase::Scan,
            Phase::Scan if scan_prog >= 1.0 => phase = Phase::ScanFill,
            Phase::ScanFill if scan_prog >= 1.0 && pt >= T_SCANFILL => phase = Phase::CoinY,
            _ => {}
        }
        if prev_phase != phase {
            xfade = Some((cv.px.clone(), 0.0));
            pt = 0.0;
            prev_pt = 0.0;
            if phase == Phase::Scan || phase == Phase::ScanFill {
                scan_prog = 0.0;
            }
        }

        if paused && !touched && !resized {
            std::thread::sleep(FRAME);
            continue;
        }

        if !paused && (phase == Phase::Scan || phase == Phase::ScanFill) {
            scan_prog = (scan_prog + dt / T_SWEEP).min(1.0);
        }
        if !paused {
            match phase {
                Phase::CoinY => {
                    coin_angle += angle_rate(coin_angle) * dt;
                    rim_t += dt;
                    flash_w += (1.0 - flash_w) * (1.0 - (-dt / 0.4).exp());
                }
                Phase::Processing => {
                    coin_angle += angle_rate(coin_angle) * dt;
                    if flat {
                        wheel_th += WHEEL_RATE * dt;
                    }
                }
                _ => {}
            }
        }
        if !flat && phase == Phase::Processing && prev_pt < T_FACE && pt >= T_FACE {
            xfade = Some((cv.px.clone(), pt));
            flat = true;
        }
        prev_pt = pt;

        let dark = screen.bg == BG_BLACK;
        let rim_gain = ease((rim_t / 0.5).min(1.0));
        cv.clear();
        let nf = n as f64;
        let sweep = |p: f64| -14.0 + (nf + 28.0) * p.clamp(0.0, 1.0);
        match phase {
            Phase::Blank => {}
            Phase::Scan => {
                let mut p = Pass::base(t, frame_no);
                p.th_u = 0.0;
                p.th_d = 0.0;
                p.fx = Fx::ScanWindow;
                p.scan = sweep(scan_prog);
                render::draw_pass(&mut cv, &p);
            }
            Phase::ScanFill => {
                let mut p = Pass::base(t, frame_no);
                p.th_u = 0.0;
                p.th_d = 0.0;
                p.fx = Fx::ScanFill;
                p.scan = sweep(scan_prog);
                render::draw_pass(&mut cv, &p);
            }
            Phase::CoinY => {
                render::draw_solid(&mut cv, &render::Solid { t, dark_bg: dark, angle: Some(coin_angle), rim_gain, ..render::Solid::base(t) });
                audio_flash(&mut cv, disp * disp, frame_no);
            }
            Phase::Processing => {
                if flat {
                    let mut p = Pass::base(t, frame_no);
                    p.th_u = wheel_th;
                    p.th_d = wheel_th;
                    render::draw_pass(&mut cv, &p);
                } else {
                    let a = if pt < T_FACE {
                        let target =
                            (enter_angle / std::f64::consts::PI).round() * std::f64::consts::PI;
                        enter_angle + (target - enter_angle) * ease(pt / T_FACE)
                    } else {
                        coin_angle
                    };
                    render::draw_solid(&mut cv, &render::Solid { t, dark_bg: dark, angle: Some(a), ..render::Solid::base(t) });
                }
            }
        }

        if let Some((snap, t0)) = &xfade {
            let k = ease((pt - t0) / T_XFADE);
            if k >= 1.0 {
                xfade = None;
            } else {
                blend(&mut cv, snap, k);
            }
        }

        frame_no = frame_no.wrapping_add(1);

        let audio_mul = 1.0 + 0.38 * disp * flash_w;
        let fill = (((h as f64 - 2.0) * 2.0) / (n as f64 * 0.548)).clamp(1.0, 2.2);
        let scale = fill * size_mul * audio_mul;
        let full_w = (n as f64 * scale) as i32;
        let bx = (w as i32 - full_w) / 2;
        let by = 1 + (h as i32 - 2 - ((n as f64 * scale / 2.0) as i32).max(1)) / 2;

        screen.reset(w, h);
        blit(&mut screen, &cv, bx, by, scale);

        if hud && phase != Phase::Blank {
            screen.clear_row(0);
            screen.clear_row(h - 1);
            let bar = 22usize;
            let filled = ((disp * bar as f64).round() as usize).clamp(0, bar);
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
                    " ANIMATION-PRODUCTION v{VERSION}  {}  mic {}  [{}] {:.2}{}",
                    phase.label(),
                    mic_txt,
                    meter,
                    disp,
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

        let hh = screen.hash();
        if hh != last_hash || resized || touched {
            last_hash = hh;
            {
                let mut f = shared.lock().unwrap();
                screen.render_into(&mut f);
                if resized {
                    f.insert_str(0, "\x1b[2J");
                }
            }
            let _ = wtx.try_send(());
        }

        let spent = now.elapsed();
        if spent < FRAME {
            std::thread::sleep(FRAME - spent);
        }
    }
}
