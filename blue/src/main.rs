#[allow(dead_code)]
#[path = "../../tui/src/screen.rs"]
mod screen;
#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/term.rs"]
mod term;

use screen::{Screen, BG_FULL_BLUE};
use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("blue v{VERSION}");
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
            screen.reset(w.max(10), h.max(5));
            let buf = screen.render();
            let _ = out.write_all(buf.as_bytes());
            let _ = out.flush();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
