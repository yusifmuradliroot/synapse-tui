#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/render.rs"]
mod render;
#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/screen.rs"]
mod screen;
#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/term.rs"]
mod term;

use render::{Canvas, Pass};
use screen::{Screen, BG_FULL_BLUE};
use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = env!("CARGO_BIN_NAME");

fn symmetrize(cv: &mut Canvas) {
    let n = cv.n;
    for y in 0..(n + 1) / 2 {
        for x in 0..(n + 1) / 2 {
            let mx = n - 1 - x;
            let my = n - 1 - y;
            let m = cv.px[(y * n + x) as usize]
                .max(cv.px[(y * n + mx) as usize])
                .max(cv.px[(my * n + x) as usize])
                .max(cv.px[(my * n + mx) as usize]);
            cv.px[(y * n + x) as usize] = m;
            cv.px[(y * n + mx) as usize] = m;
            cv.px[(my * n + x) as usize] = m;
            cv.px[(my * n + mx) as usize] = m;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{NAME} v{VERSION}");
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

    loop {
        while let Ok(b) = rx.try_recv() {
            if b == b'q' || b == 0x1b || b == 3 {
                term::raw_stop(&saved);
                let _ = out.write_all(b"\x1b[?25h\x1b[0m\x1b[?1049l");
                let _ = out.flush();
                return;
            }
        }
        let (w, h) = term::size();
        if (w, h) != last_size {
            last_size = (w, h);
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
            cv.clear();
            let mut p = Pass::base(0.0, 0);
            p.th_u = 0.0;
            p.th_d = 0.0;
            render::draw_pass(&mut cv, &p);
            symmetrize(&mut cv);
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
            let buf = screen.render();
            let _ = out.write_all(buf.as_bytes());
            let _ = out.flush();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
