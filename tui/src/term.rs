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

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: i32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, m: *mut u32) -> i32;
        fn SetConsoleMode(h: *mut c_void, m: u32) -> i32;
        fn SetConsoleOutputCP(cp: u32) -> i32;
        fn GetConsoleScreenBufferInfo(h: *mut c_void, i: *mut Info) -> i32;
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
            SetConsoleMode(hin, (im & !(0x0002 | 0x0008)) | 0x0200);
            Saved { out: om, inp: im }
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
            } else {
                c as u8
            }
        }
    }
}

pub use imp::{raw_start, raw_stop, read_byte, size};
