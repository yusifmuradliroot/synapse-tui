#[cfg(unix)]
mod imp {
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Termios {
        pub c_iflag: u32,
        pub c_oflag: u32,
        pub c_cflag: u32,
        pub c_lflag: u32,
        pub c_line: u8,
        pub c_cc: [u8; 32],
        pub c_ispeed: u32,
        pub c_ospeed: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Winsize {
        row: u16,
        col: u16,
        xpix: u16,
        ypix: u16,
    }

    extern "C" {
        fn tcgetattr(fd: i32, t: *mut Termios) -> i32;
        fn tcsetattr(fd: i32, a: i32, t: *const Termios) -> i32;
        fn ioctl(fd: i32, req: u64, ...) -> i32;
        fn read(fd: i32, buf: *mut u8, n: usize) -> isize;
    }

    pub struct Saved {
        pub t: Termios,
    }

    pub fn raw_start() -> Saved {
        unsafe {
            let mut orig: Termios = std::mem::zeroed();
            tcgetattr(0, &mut orig);
            let mut raw = orig;
            raw.c_iflag &= !(0o1 | 0o2 | 0o10 | 0o40 | 0o100 | 0o200 | 0o400 | 0o2000);
            raw.c_oflag &= !0o1;
            raw.c_lflag &= !(0o1 | 0o2 | 0o10 | 0o100 | 0o100000);
            raw.c_cflag &= !(0o60 | 0o400);
            raw.c_cflag |= 0o60;
            raw.c_cc[6] = 1;
            raw.c_cc[5] = 0;
            tcsetattr(0, 0, &raw);
            Saved { t: orig }
        }
    }

    pub fn raw_stop(s: &Saved) {
        unsafe {
            tcsetattr(0, 0, &s.t);
        }
    }

    pub fn set_fullscreen(_on: bool) {}

    pub fn setup_window() {}

    pub fn size() -> (usize, usize) {
        unsafe {
            let mut ws = Winsize::default();
            if ioctl(1, 0x5413, &mut ws as *mut Winsize) == 0 && ws.col > 0 && ws.row > 0 {
                return (ws.col as usize, ws.row as usize);
            }
            (80, 24)
        }
    }

    pub fn read_byte() -> u8 {
        unsafe {
            let mut b = 0u8;
            if read(0, &mut b, 1) == 1 {
                b
            } else {
                0
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Info {
        size: Coord,
        cursor: Coord,
        attributes: u16,
        window: SmallRect,
        maximum: Coord,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FontInfo {
        cb_size: u32,
        n_font: u32,
        size: Coord,
        family: u32,
        weight: u32,
        face: [u16; 32],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: i32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, m: *mut u32) -> i32;
        fn SetConsoleMode(h: *mut c_void, m: u32) -> i32;
        fn SetConsoleOutputCP(cp: u32) -> i32;
        fn GetConsoleScreenBufferInfo(h: *mut c_void, i: *mut Info) -> i32;
        fn SetConsoleDisplayMode(h: *mut c_void, m: u32) -> i32;
        fn GetConsoleWindow() -> *mut c_void;
        fn SetConsoleScreenBufferSize(h: *mut c_void, s: Coord) -> i32;
        fn SetConsoleWindowInfo(h: *mut c_void, a: i32, r: *const SmallRect) -> i32;
        fn SetCurrentConsoleFontEx(h: *mut c_void, m: i32, f: *const FontInfo) -> i32;
        fn GetCurrentConsoleFontEx(h: *mut c_void, m: i32, f: *mut FontInfo) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetWindowRect(h: *mut c_void, r: *mut Rect) -> i32;
        fn GetClientRect(h: *mut c_void, r: *mut Rect) -> i32;
        fn MoveWindow(h: *mut c_void, x: i32, y: i32, w: i32, hgt: i32, r: i32) -> i32;
    }

    extern "C" {
        fn _getch() -> i32;
    }

    pub struct Saved {
        out: u32,
        inp: u32,
    }

    pub fn raw_start() -> Saved {
        unsafe {
            let hout = GetStdHandle(-11);
            let hin = GetStdHandle(-10);
            let mut om = 0u32;
            let mut im = 0u32;
            GetConsoleMode(hout, &mut om);
            GetConsoleMode(hin, &mut im);
            SetConsoleOutputCP(65001);
            SetConsoleMode(hout, om | 0x0004);
            const LINE_INPUT: u32 = 0x0002;
            const ECHO_INPUT: u32 = 0x0008;
            const QUICK_EDIT: u32 = 0x0040;
            const EXTENDED_FLAGS: u32 = 0x0080;
            const VT_INPUT: u32 = 0x0200;
            let raw = (im & !(LINE_INPUT | ECHO_INPUT | QUICK_EDIT)) | EXTENDED_FLAGS | VT_INPUT;
            SetConsoleMode(hin, raw);
            Saved { out: om, inp: im }
        }
    }

    pub fn set_fullscreen(on: bool) {
        unsafe {
            SetConsoleDisplayMode(GetStdHandle(-11), if on { 1 } else { 2 });
        }
    }

    // Konsolu stabil 1280x720 istemci alanina getir: 6x12 TrueType font,
    // tampon = pencere (kaydirma cubugu yok) + dis pencere boyutu.
    // Basarisiz olursa sessizce vazgecilir, mevcut konsol kullanilir.
    pub fn setup_window() {
        unsafe {
            let hwnd = GetConsoleWindow();
            if hwnd.is_null() {
                return;
            }
            let hout = GetStdHandle(-11);
            let mut fi: FontInfo = std::mem::zeroed();
            fi.cb_size = std::mem::size_of::<FontInfo>() as u32;
            fi.size = Coord { x: 6, y: 12 };
            let name: [u16; 9] = [0x43, 0x6F, 0x6E, 0x73, 0x6F, 0x6C, 0x61, 0x73, 0];
            fi.face[..9].copy_from_slice(&name);
            SetCurrentConsoleFontEx(hout, 0, &fi);
            let mut cur: FontInfo = std::mem::zeroed();
            cur.cb_size = std::mem::size_of::<FontInfo>() as u32;
            let (mut fw, mut fh) = (6i32, 12i32);
            if GetCurrentConsoleFontEx(hout, 0, &mut cur) != 0 && cur.size.x > 0 && cur.size.y > 0 {
                fw = cur.size.x as i32;
                fh = cur.size.y as i32;
            }
            let cols = (1280 / fw).clamp(40, 600) as i16;
            let rows = (720 / fh).clamp(20, 300) as i16;
            SetConsoleScreenBufferSize(hout, Coord { x: cols, y: rows });
            let win = SmallRect {
                left: 0,
                top: 0,
                right: cols - 1,
                bottom: rows - 1,
            };
            SetConsoleWindowInfo(hout, 1, &win);
            let (mut wr, mut cr) = (Rect::default(), Rect::default());
            if GetWindowRect(hwnd, &mut wr) != 0 && GetClientRect(hwnd, &mut cr) != 0 {
                let dx = (wr.right - wr.left) - (cr.right - cr.left);
                let dy = (wr.bottom - wr.top) - (cr.bottom - cr.top);
                MoveWindow(
                    hwnd,
                    wr.left,
                    wr.top,
                    cols as i32 * fw + dx,
                    rows as i32 * fh + dy,
                    1,
                );
            }
        }
    }

    pub fn raw_stop(s: &Saved) {
        unsafe {
            SetConsoleMode(GetStdHandle(-11), s.out);
            SetConsoleMode(GetStdHandle(-10), s.inp);
        }
    }

    pub fn size() -> (usize, usize) {
        unsafe {
            let mut i = Info::default();
            if GetConsoleScreenBufferInfo(GetStdHandle(-11), &mut i) != 0 {
                (
                    (i.window.right - i.window.left + 1) as usize,
                    (i.window.bottom - i.window.top + 1) as usize,
                )
            } else {
                (80, 24)
            }
        }
    }

    pub fn read_byte() -> u8 {
        unsafe {
            let c = _getch();
            if c < 0 {
                0
            } else if c == 0 {
                let _ = _getch();
                0
            } else {
                c as u8
            }
        }
    }
}

pub use imp::{raw_start, raw_stop, read_byte, set_fullscreen, setup_window, size};
