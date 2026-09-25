#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/render.rs"]
mod render;
#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/screen.rs"]
mod screen;
#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/term.rs"]
mod term;

use render::{Canvas, Fx, Pass};
use screen::{Screen, BG_FULL_BLUE};
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

// Surum semasi: 1.3 sabit; 3. kisim guncellemede artar (Cargo),
// 4. kisim hotfix sayar ve 3. artinca sifirlanir.
const HOTFIX: u32 = 2;
const NAME: &str = env!("CARGO_BIN_NAME");

const T_BLANK: f64 = 3.0;
const T_SWEEP: f64 = 2.5;
const T_HOLD: f64 = 0.5;
const FRAME: Duration = Duration::from_micros(16_667);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Blank,
    Scan,
    ScanFill,
    Hold,
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

fn main() {
    let ver = format!("{}.{}", env!("CARGO_PKG_VERSION"), HOTFIX);
    let vlabel = format!("v{ver}");
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{NAME} v{ver}");
        return;
    }

    let saved = term::raw_start();
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J");
    let _ = out.flush();

    let (tx, rx) = mpsc::channel::<u8>();
    std::thread::spawn(move || loop {
        let b = term::read_byte();
        if b == 0 {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        if tx.send(b).is_err() {
            return;
        }
    });

    let mut screen = Screen::new(80, 24);
    screen.set_bg(BG_FULL_BLUE);
    let mut last_size = (0usize, 0usize);
    let mut cv = Canvas::new(73);
    let mut phase = Phase::Blank;
    let mut pt = 0.0f64;
    let mut scan_prog = 0.0f64;
    let mut frame_no = 0u32;
    let mut last = Instant::now();

    loop {
        while let Ok(b) = rx.try_recv() {
            if b == b'q' || b == 0x1b || b == 3 {
                term::raw_stop(&saved);
                let _ = out.write_all(b"\x1b[?25h\x1b[0m\x1b[?1049l");
                let _ = out.flush();
                return;
            }
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.05);
        last = now;
        pt += dt;

        let (w, h) = term::size();
        let resized = (w, h) != last_size;
        if resized {
            last_size = (w, h);
        }

        let prev = phase;
        match phase {
            Phase::Blank if pt >= T_BLANK => phase = Phase::Scan,
            Phase::Scan if scan_prog >= 1.0 => phase = Phase::ScanFill,
            Phase::ScanFill if scan_prog >= 1.0 && pt >= T_SWEEP + T_HOLD => phase = Phase::Hold,
            _ => {}
        }
        if prev != phase {
            pt = 0.0;
            if phase == Phase::Scan || phase == Phase::ScanFill {
                scan_prog = 0.0;
            }
        }
        if phase == Phase::Scan || phase == Phase::ScanFill {
            scan_prog = (scan_prog + dt / T_SWEEP).min(1.0);
        }

        let animated = phase == Phase::Scan || phase == Phase::ScanFill;
        if !animated && !resized && prev == phase {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }

        let w = w.max(10);
        let h = h.max(5);
        let mut n = ((0.9 * (w as f64 - 1.0) / 0.50).min(0.9 * 2.0 * (h as f64 - 2.0) / 0.55)
            as i32)
            .clamp(41, 201);
        if (w as i32 - n) % 2 != 0 {
            n = if n < 201 { n + 1 } else { n - 1 };
        }
        if n != cv.n {
            cv = Canvas::new_exact(n);
        }
        let nf = n as f64;
        let sweep = -14.0 + (nf + 28.0) * scan_prog;
        cv.clear();
        match phase {
            Phase::Blank => {}
            Phase::Scan => {
                let mut p = Pass::base(0.0, frame_no);
                p.th_u = 0.0;
                p.th_d = 0.0;
                p.fx = Fx::ScanWindow;
                p.scan = sweep;
                render::draw_pass(&mut cv, &p);
                mirror_x(&mut cv);
            }
            Phase::ScanFill => {
                let mut p = Pass::base(0.0, frame_no);
                p.th_u = 0.0;
                p.th_d = 0.0;
                p.fx = Fx::ScanFill;
                p.scan = sweep;
                render::draw_pass(&mut cv, &p);
                mirror_x(&mut cv);
            }
            Phase::Hold => {
                let mut p = Pass::base(0.0, frame_no);
                p.th_u = 0.0;
                p.th_d = 0.0;
                p.fx = Fx::ScanFill;
                p.scan = -14.0 + (nf + 28.0);
                render::draw_pass(&mut cv, &p);
                mirror_x(&mut cv);
            }
        }
        frame_no = frame_no.wrapping_add(1);

        let bx = (w as i32 - cv.n) / 2;
        let rows = (cv.n + 1) / 2;
        let by = 1 + (h as i32 - 2 - rows) / 2;
        screen.reset(w, h);
        for j in 0..rows {
            let y0 = 2 * j;
            let y1 = y0 + 1;
            for i in 0..cv.n {
                let t = if y0 < cv.n {
                    cv.px[(y0 * cv.n + i) as usize]
                } else {
                    0
                };
                let b = if y1 < cv.n {
                    cv.px[(y1 * cv.n + i) as usize]
                } else {
                    0
                };
                screen.half(bx + i, by + j, t, b);
            }
        }
        screen.text(0, 0, &vlabel, 255);
        screen.text(0, h as i32 - 3, "1-Restart from 0 to all", 255);
        screen.text(0, h as i32 - 2, "2-Idle", 255);
        screen.text(0, h as i32 - 1, "3-Production", 255);
        let buf = screen.render();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();

        if animated {
            let spent = now.elapsed();
            if spent < FRAME {
                std::thread::sleep(FRAME - spent);
            }
        }
    }
}
