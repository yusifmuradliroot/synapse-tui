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
            let n = (w as i32).min((h as i32) * 2).clamp(41, 121);
            if n != cv.n {
                cv = Canvas::new(n);
            }
            cv.clear();
            let mut p = Pass::base(0.0, 0);
            p.th_u = 0.0;
            p.th_d = 0.0;
            render::draw_pass(&mut cv, &p);
            let nf = n as f64;
            let max_s = ((w as f64 - 1.0) / nf).min((h as f64 * 2.0) / nf);
            let scale = (max_s * 0.82).clamp(0.25, 2.2).min(max_s.max(0.25));
            let bx = (w as i32 - (nf * scale) as i32) / 2;
            let by = ((h as i32 * 2 - (nf * scale) as i32) / 4).max(0);
            screen.reset(w, h);
            blit(&mut screen, &cv, bx, by, scale);
            let buf = screen.render();
            let _ = out.write_all(buf.as_bytes());
            let _ = out.flush();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
