pub const BG_FULL_BLUE: (u8, u8, u8) = (0, 0, 255);
pub const BG_BLACK: (u8, u8, u8) = (0, 0, 0);
pub const BG_DEEP_BLUE: (u8, u8, u8) = (12, 18, 96);
pub const BG_CYCLE: [(u8, u8, u8); 3] = [BG_FULL_BLUE, BG_BLACK, BG_DEEP_BLUE];

pub struct Screen {
    w: usize,
    h: usize,
    top: Vec<u8>,
    bot: Vec<u8>,
    ch: Vec<char>,
    pub bg: (u8, u8, u8),
}

impl Screen {
    pub fn new(w: usize, h: usize) -> Screen {
        Screen {
            w,
            h,
            top: vec![0; w * h],
            bot: vec![0; w * h],
            ch: vec![' '; w * h],
            bg: BG_FULL_BLUE,
        }
    }

    pub fn set_bg(&mut self, bg: (u8, u8, u8)) {
        self.bg = bg;
    }

    pub fn reset(&mut self, w: usize, h: usize) {
        if w != self.w || h != self.h {
            let bg = self.bg;
            *self = Screen::new(w, h);
            self.bg = bg;
        } else {
            self.top.iter_mut().for_each(|v| *v = 0);
            self.bot.iter_mut().for_each(|v| *v = 0);
            self.ch.iter_mut().for_each(|v| *v = ' ');
        }
    }

    pub fn half(&mut self, x: i32, y: i32, top: u8, bot: u8) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = y as usize * self.w + x as usize;
        self.top[i] = top;
        self.bot[i] = bot;
        self.ch[i] = '\u{2580}';
    }

    pub fn text(&mut self, x: i32, y: i32, s: &str, v: u8) {
        for (i, b) in s.bytes().enumerate() {
            if b == b' ' {
                continue;
            }
            let xi = x + i as i32;
            if xi < 0 || y < 0 || xi as usize >= self.w || y as usize >= self.h {
                continue;
            }
            let idx = y as usize * self.w + xi as usize;
            self.top[idx] = v;
            self.bot[idx] = 0;
            self.ch[idx] = b as char;
        }
    }

    pub fn clear_row(&mut self, y: usize) {
        if y >= self.h {
            return;
        }
        for x in 0..self.w {
            let i = y * self.w + x;
            self.top[i] = 0;
            self.bot[i] = 0;
            self.ch[i] = ' ';
        }
    }

    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for v in self.top.iter().chain(self.bot.iter()) {
            h ^= *v as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        for c in self.ch.iter() {
            h ^= *c as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        for v in [self.bg.0, self.bg.1, self.bg.2] {
            h ^= v as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    pub fn render_into(&self, out: &mut String) {
        out.clear();
        out.push_str("\x1b[?2026h\x1b[H");
        let bg = self.bg;
        let mut last_f: (i32, i32, i32) = (-1, -1, -1);
        let mut last_b: (i32, i32, i32) = (-1, -1, -1);
        for y in 0..self.h {
            if y > 0 {
                out.push_str(&format!("\x1b[{};1H", y + 1));
            }
            for x in 0..self.w {
                let i = y * self.w + x;
                let t = self.top[i];
                let b = self.bot[i];
                let fc = if t > 0 {
                    (t as i32, t as i32, t as i32)
                } else {
                    (bg.0 as i32, bg.1 as i32, bg.2 as i32)
                };
                let bc = if b > 0 {
                    (b as i32, b as i32, b as i32)
                } else {
                    (bg.0 as i32, bg.1 as i32, bg.2 as i32)
                };
                if fc != last_f {
                    if t > 0 {
                        out.push_str(&format!("\x1b[38;2;{};{};{}m", fc.0, fc.1, fc.2));
                    } else {
                        out.push_str(&format!("\x1b[38;2;{};{};{}m", bg.0, bg.1, bg.2));
                    }
                    last_f = fc;
                }
                if bc != last_b {
                    if b > 0 {
                        out.push_str(&format!("\x1b[48;2;{};{};{}m", bc.0, bc.1, bc.2));
                    } else {
                        out.push_str(&format!("\x1b[48;2;{};{};{}m", bg.0, bg.1, bg.2));
                    }
                    last_b = bc;
                }
                out.push(self.ch[i]);
            }
            out.push_str("\x1b[0m");
            last_f = (-1, -1, -1);
            last_b = (-1, -1, -1);
        }
        out.push_str("\x1b[0m\x1b[?2026l");
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(self.w * self.h * 6);
        self.render_into(&mut out);
        out
    }
}
