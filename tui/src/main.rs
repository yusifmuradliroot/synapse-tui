mod anim;
#[allow(dead_code)]
mod render;
mod term;

use anim::{Cell, Rng, Variant, ALL, FEATURED};
use render::Canvas;
use screen::{Screen, BG_BLACK, BG_CYCLE};
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Standby,
    Listening,
    Processing,
    Speaking,
}

impl State {
    fn speed(self) -> f64 {
        match self {
            State::Standby => 0.0,
            State::Listening => 0.84,
            State::Processing => 1.26,
            State::Speaking => 0.42,
        }
    }
    fn label(self) -> &'static str {
        match self {
            State::Standby => "STANDBY",
            State::Listening => "LISTENING",
            State::Processing => "PROCESSING",
            State::Speaking => "SPEAKING",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Grid4,
    Grid8,
    Focus,
}

enum Key {
    Char(u8),
    Up,
    Down,
    Left,
    Right,
    F11,
}

#[allow(dead_code)]
mod screen;

struct Unit {
    v: Variant,
    cell: Cell,
    cv: Canvas,
    x: i32,
    y: i32,
    label: String,
    sel: bool,
}

fn odd_clamp(mut n: i32, lo: i32, hi: i32) -> i32 {
    if n % 2 == 0 {
        n -= 1;
    }
    n.clamp(lo | 1, hi | 1)
}

const MIN_READ: i32 = 33;

fn grid(list: &[Variant], max_cols: usize, w: usize, h: usize) -> (i32, Vec<(usize, i32, i32)>) {
    let avail_h = h.saturating_sub(2);
    let min_text_rows = (MIN_READ / 2 + 1) as usize;
    let max_rows = (avail_h / min_text_rows).max(1);
    let mut best: Option<(i32, usize)> = None;
    let mut cols = max_cols.min(list.len().max(1));
    loop {
        let rows = list.len().div_ceil(cols);
        if rows <= max_rows {
            let per_cell = avail_h.saturating_sub(rows) / rows;
            let s = odd_clamp(((w / cols) as i32).min((per_cell * 2) as i32), 9, 73);
            best = Some((s, cols));
            if s >= MIN_READ {
                break;
            }
        }
        if cols <= 1 {
            break;
        }
        cols = cols.div_ceil(2);
    }
    let (s, cols) = best.unwrap_or((9, 1));
    let rows = list.len().div_ceil(cols);
    let count = (rows * cols).min(list.len());
    let sw = s as usize;
    let gap = if cols > 1 {
        ((w.saturating_sub(cols * sw)) / (cols - 1)).clamp(1, 8)
    } else {
        1
    };
    let step = sw + gap;
    let total_w = cols * sw + (cols - 1) * gap;
    let x0 = ((w.saturating_sub(total_w)) / 2) as i32;
    let cell_h = sw.div_ceil(2) + 1;
    let block_h = rows * cell_h;
    let y0 = 1 + (avail_h.saturating_sub(block_h) / 2) as i32;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let c = i % cols;
        let r = i / cols;
        out.push((i, x0 + (c * step) as i32, y0 + (r * cell_h) as i32));
    }
    (s, out)
}

fn layout(mode: Mode, w: usize, h: usize, sel: usize, hud: bool) -> (i32, Vec<(usize, i32, i32)>) {
    match mode {
        Mode::Focus => {
            let (avail_h, y0) = if hud {
                (h.saturating_sub(4), 2)
            } else {
                (h, 0)
            };
            let s = odd_clamp((w as i32).min(avail_h as i32 * 2), 9, 73);
            let x0 = ((w as i32 - s) / 2).max(0);
            let y0 = y0.max(0) + (avail_h as i32 - s / 2).max(0) / 2;
            (s, vec![(sel, x0, y0)])
        }
        Mode::Grid4 => grid(&FEATURED, 4, w, h),
        Mode::Grid8 => grid(&ALL, 8, w, h),
    }
}

fn mode_label(mode: Mode, sel: usize) -> String {
    match mode {
        Mode::Focus => format!(
            "FOCUS  {:02}/{:02}  {}",
            sel + 1,
            ALL.len(),
            ALL[sel].name()
        ),
        Mode::Grid4 => "GRID 4x2".to_string(),
        Mode::Grid8 => format!("GRID 8x4   sel {:02} {}", sel + 1, ALL[sel].name()),
    }
}

fn help_text(w: usize, paused: bool, auto: bool) -> String {
    let full = format!(
        " 1-8 pick  arrows cycle  g grid  f focus  h hud  b bg  F11 full  space {}  a auto-state {}  q quit ",
        if paused { "resume" } else { "pause" },
        if auto { "on" } else { "off" }
    );
    let short = " q quit ".to_string();
    if full.len() <= w {
        full
    } else if short.len() <= w {
        short
    } else {
        String::new()
    }
}

fn dump(dir: &str) {
    let _ = std::fs::create_dir_all(dir);
    for v in ALL.iter() {
        let mut cv = Canvas::new(73);
        let mut cell = Cell { theta: 0.0 };
        let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
        for f in 0..=52u32 {
            let mut env = anim::Env {
                t: f as f64 * 0.033,
                dt: 0.033,
                spd: 1.26,
                frame: f,
                rng: &mut rng,
                dark_bg: true,
            };
            anim::draw(&mut cv, *v, &mut cell, &mut env);
        }
        let mut buf: Vec<u8> = Vec::with_capacity(cv.px.len() + 15);
        buf.extend_from_slice(b"P5\n73 73\n255\n");
        buf.extend_from_slice(&cv.px);
        let name: String = v
            .name()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let path = format!("{}/{}.pgm", dir, name);
        let _ = std::fs::write(path, buf);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--dump" {
        dump(&args[2]);
        return;
    }

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
                    let key = match (seq[1], b) {
                        (b'[', b'A') => Some(Key::Up),
                        (b'[', b'B') => Some(Key::Down),
                        (b'[', b'C') => Some(Key::Right),
                        (b'[', b'D') => Some(Key::Left),
                        (b'[', b'~') => match seq.get(2) {
                            Some(23) => Some(Key::F11),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(k) = key {
                        if tx.send(k).is_err() {
                            return;
                        }
                    }
                } else {
                    pending.push(b);
                    if pending.len() > 24 {
                        pending.clear();
                    }
                }
                continue;
            }
            let k = Key::Char(b);
            if tx.send(k).is_err() {
                return;
            }
        }
    });

    let mut state = State::Processing;
    let mut mode = Mode::Focus;
    let mut sel = 0usize;
    let mut paused = false;
    let mut auto = false;
    let mut hud = true;
    let mut bg_idx = 0usize;
    let mut fullscreen = false;
    let mut auto_next = Instant::now() + Duration::from_millis(2200);
    let mut rng = Rng::new(0xA11C_E001_7BEE_7A11);
    let mut units: Vec<Unit> = Vec::new();
    let mut cur_s = -1i32;
    let mut cur_list: Vec<Variant> = Vec::new();

    let start = Instant::now();
    let mut last = start;
    let mut t = 0.0f64;
    let mut frame = 0u32;
    let mut screen = Screen::new(80, 24);
    let mut started = false;
    let mut prev_w = 0usize;
    let mut prev_h = 0usize;

    loop {
        let mut touched = false;
        let mut quit = false;
        while let Ok(k) = rx.try_recv() {
            touched = true;
            match k {
                Key::Char(b'q') | Key::Char(3) => quit = true,
                Key::Char(c @ b'1'..=b'8') => {
                    sel = (c - b'1') as usize;
                    mode = Mode::Focus;
                }
                Key::Char(b'g') => {
                    mode = match mode {
                        Mode::Grid4 => Mode::Grid8,
                        Mode::Grid8 => Mode::Focus,
                        Mode::Focus => Mode::Grid4,
                    };
                }
                Key::Char(b'f') => {
                    mode = if mode == Mode::Focus {
                        Mode::Grid4
                    } else {
                        Mode::Focus
                    };
                }
                Key::Char(b' ') => paused = !paused,
                Key::Char(b'a') => auto = !auto,
                Key::Char(b'h') => hud = !hud,
                Key::Char(b'b') => {
                    bg_idx = (bg_idx + 1) % BG_CYCLE.len();
                    screen.set_bg(BG_CYCLE[bg_idx]);
                }
                Key::F11 => {
                    fullscreen = !fullscreen;
                    term::set_fullscreen(fullscreen);
                }
                Key::Char(b'[') => sel = if sel == 0 { ALL.len() - 1 } else { sel - 1 },
                Key::Char(b']') => {
                    sel = (sel + 1) % ALL.len();
                }
                Key::Left | Key::Down => {
                    sel = if sel == 0 { ALL.len() - 1 } else { sel - 1 };
                    mode = Mode::Focus;
                }
                Key::Right | Key::Up => {
                    sel = (sel + 1) % ALL.len();
                    mode = Mode::Focus;
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

        if auto && now > auto_next {
            auto_next = now + Duration::from_millis(2200);
            state = match state {
                State::Standby => State::Listening,
                State::Listening => State::Processing,
                State::Processing => State::Speaking,
                State::Speaking => State::Standby,
            };
            touched = true;
        }

        let (tw, th) = term::size();
        let w = tw.max(20);
        let h = th.max(8);
        if !started {
            started = true;
            if grid(&FEATURED, 4, w, h).0 >= MIN_READ {
                mode = Mode::Grid4;
            }
        }
        let resized = w != prev_w || h != prev_h;
        if resized {
            prev_w = w;
            prev_h = h;
            touched = true;
        }
        let (s, places) = layout(mode, w, h, sel, hud);
        let list: Vec<Variant> = match mode {
            Mode::Focus => vec![ALL[sel]],
            Mode::Grid4 => FEATURED.to_vec(),
            Mode::Grid8 => ALL.to_vec(),
        };
        let mut rebuilt = false;
        if s != cur_s || list != cur_list {
            cur_s = s;
            cur_list = list.clone();
            rebuilt = true;
            units = list
                .iter()
                .enumerate()
                .map(|(i, v)| Unit {
                    v: *v,
                    cell: Cell { theta: 0.0 },
                    cv: Canvas::new(s),
                    x: 0,
                    y: 0,
                    label: format!("{:02} {}", i + 1, v.name()),
                    sel: (mode == Mode::Grid4 && i == sel) || (mode == Mode::Grid8 && i == sel),
                })
                .collect();
        }

        let header = format!(
            " AURION TUI   {}   {}",
            state.label(),
            mode_label(mode, sel)
        );

        if paused && !rebuilt {
            if touched {
                if hud {
                    screen.clear_row(0);
                    screen.text(0, 0, &header, 255);
                    if mode == Mode::Focus {
                        screen.clear_row(h - 2);
                        screen.text(1, h as i32 - 2, ALL[sel].name(), 255);
                    }
                    screen.clear_row(h - 1);
                    screen.text(0, h as i32 - 1, &help_text(w, paused, auto), 235);
                }
                let buf = screen.render();
                let _ = wtx.try_send(buf);
            }
            std::thread::sleep(Duration::from_millis(33));
            continue;
        }

        if !paused {
            t += dt;
        }
        frame = frame.wrapping_add(1);
        screen.reset(w, h);
        let spd = state.speed();
        let draw_dt = if paused { 0.0 } else { dt };
        for (u, (_, x, y)) in units.iter_mut().zip(places.iter()) {
            u.x = *x;
            u.y = *y;
            let dark_bg = screen.bg == BG_BLACK;
            let mut env = anim::Env {
                t,
                dt: draw_dt,
                spd,
                frame,
                rng: &mut rng,
                dark_bg,
            };
            anim::draw(&mut u.cv, u.v, &mut u.cell, &mut env);
            let rows = (u.cv.n as usize).div_ceil(2);
            for j in 0..rows {
                let y0 = 2 * j as i32;
                let y1 = y0 + 1;
                for i in 0..u.cv.n as usize {
                    let t0 = if y0 < u.cv.n {
                        u.cv.px[y0 as usize * u.cv.n as usize + i]
                    } else {
                        0
                    };
                    let b0 = if y1 < u.cv.n {
                        u.cv.px[y1 as usize * u.cv.n as usize + i]
                    } else {
                        0
                    };
                    screen.half(*x + i as i32, *y + j as i32, t0, b0);
                }
            }
            if hud && mode != Mode::Focus {
                let v = if u.sel { 255 } else { 220 };
                screen.text(*x, *y + rows as i32, &u.label, v);
            }
        }

        if hud {
            screen.text(0, 0, &header, 255);
            if mode == Mode::Focus {
                screen.text(1, h as i32 - 2, ALL[sel].name(), 255);
            }
            screen.text(0, h as i32 - 1, &help_text(w, paused, auto), 235);
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
