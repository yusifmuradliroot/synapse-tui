pub const C30: f64 = 0.866_025_4;
pub const TAU: f64 = std::f64::consts::PI * 2.0;

pub const TRI_U: [(f64, f64); 3] = [(0.0, -1.0), (C30, 0.5), (-C30, 0.5)];
pub const TRI_D: [(f64, f64); 3] = [(0.0, 1.0), (C30, -0.5), (-C30, -0.5)];

const HX: f64 = C30 / 3.0;
pub const SOLID_PTS: [(f64, f64); 12] = [
    (0.0, -1.0),
    (C30, 0.5),
    (-C30, 0.5),
    (0.0, 1.0),
    (C30, -0.5),
    (-C30, -0.5),
    (-HX, -0.5),
    (HX, -0.5),
    (2.0 * HX, 0.0),
    (HX, 0.5),
    (-HX, 0.5),
    (-2.0 * HX, 0.0),
];

pub const HALF_EDGES: [(usize, usize); 18] = [
    (0, 7),
    (7, 8),
    (8, 1),
    (1, 9),
    (9, 10),
    (10, 2),
    (2, 11),
    (11, 6),
    (6, 0),
    (3, 9),
    (9, 8),
    (8, 4),
    (4, 7),
    (7, 6),
    (6, 5),
    (5, 11),
    (11, 10),
    (10, 3),
];

const HD: f64 = 0.16;
const CAM: f64 = 6.2;
const RIM_SH: [u8; 6] = [200, 150, 105, 70, 120, 175];
const RIM_SH_BRIGHT: [u8; 6] = [245, 205, 175, 150, 190, 225];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fx {
    None,
    Twinkle,
    Scan,
    Wipe,
    Wave,
    ScanFill,
    ScanWindow,
}

pub struct Pass {
    pub th_u: f64,
    pub th_d: f64,
    pub sx: f64,
    pub sy: f64,
    pub ox: f64,
    pub oy: f64,
    pub pv: (f64, f64),
    pub fx: Fx,
    pub t: f64,
    pub frame: u32,
    pub scan: f64,
}

impl Pass {
    pub fn base(t: f64, frame: u32) -> Pass {
        Pass {
            th_u: 0.0,
            th_d: 0.0,
            sx: 1.0,
            sy: 1.0,
            ox: 0.0,
            oy: 0.0,
            pv: (0.0, 0.0),
            fx: Fx::None,
            t,
            frame,
            scan: -1.0,
        }
    }
}

pub struct Canvas {
    pub n: i32,
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
    pub px: Vec<u8>,
    pub aa: u8,
}

const SUB: [(f64, f64); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];

impl Canvas {
    pub fn new(n: i32) -> Canvas {
        let n = if n % 2 == 0 { n - 1 } else { n };
        Canvas {
            n,
            cx: (n as f64 - 1.0) / 2.0,
            cy: (n as f64 - 1.0) / 2.0,
            r: n as f64 * 20.0 / 73.0,
            px: vec![0; (n * n) as usize],
            aa: 1,
        }
    }

    pub fn new_exact(n: i32) -> Canvas {
        let n = n.max(9);
        Canvas {
            n,
            cx: (n as f64 - 1.0) / 2.0,
            cy: (n as f64 - 1.0) / 2.0,
            r: n as f64 * 20.0 / 73.0,
            px: vec![0; (n * n) as usize],
            aa: 1,
        }
    }

    pub fn clear(&mut self) {
        for v in self.px.iter_mut() {
            *v = 0;
        }
    }

    pub fn tri(&mut self, t: [(f64, f64); 3], v: u8) {
        self.scan(&[t[0], t[1], t[2]], v, point_in_tri);
    }

    pub fn poly(&mut self, p: &[(f64, f64)], v: u8) {
        self.scan(p, v, point_in_poly);
    }

    fn scan(&mut self, p: &[(f64, f64)], v: u8, test: fn(f64, f64, &[(f64, f64)]) -> bool) {
        let (mut x0, mut y0) = (f64::MAX, f64::MAX);
        let (mut x1, mut y1) = (f64::MIN, f64::MIN);
        for q in p {
            x0 = x0.min(q.0);
            x1 = x1.max(q.0);
            y0 = y0.min(q.1);
            y1 = y1.max(q.1);
        }
        let ix0 = x0.floor().max(0.0) as i32;
        let iy0 = y0.floor().max(0.0) as i32;
        let ix1 = (x1.ceil() as i32).min(self.n - 1);
        let iy1 = (y1.ceil() as i32).min(self.n - 1);
        for y in iy0..=iy1 {
            for x in ix0..=ix1 {
                let idx = (y * self.n + x) as usize;
                if self.aa <= 1 {
                    if test(x as f64 + 0.5, y as f64 + 0.5, p) {
                        self.px[idx] = v;
                    }
                } else {
                    let mut hits = 0u32;
                    for (dx, dy) in SUB {
                        if test(x as f64 + dx, y as f64 + dy, p) {
                            hits += 1;
                        }
                    }
                    if hits > 0 {
                        let nv = (v as u32 * hits / 4) as u8;
                        if nv > self.px[idx] {
                            self.px[idx] = nv;
                        }
                    }
                }
            }
        }
    }
}

fn point_in_tri(px: f64, py: f64, p: &[(f64, f64)]) -> bool {
    let (a, b, c) = (p[0], p[1], p[2]);
    let d = (b.1 - c.1) * (a.0 - c.0) + (c.0 - b.0) * (a.1 - c.1);
    let d = if d.abs() < 1e-12 { 1e-9 } else { d };
    let u = ((b.1 - c.1) * (px - c.0) + (c.0 - b.0) * (py - c.1)) / d;
    let v = ((c.1 - a.1) * (px - c.0) + (a.0 - c.0) * (py - c.1)) / d;
    u >= 0.0 && v >= 0.0 && u + v <= 1.0
}

fn point_in_poly(px: f64, py: f64, p: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = p.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = p[i];
        let (xj, yj) = p[j];
        if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn draw_pass(cv: &mut Canvas, p: &Pass) {
    let k = cv.r / 20.0;
    let (cx, cy, r) = (cv.cx, cv.cy, cv.r);
    let (cu, su) = (p.th_u.cos(), p.th_u.sin());
    let (cd, sd) = (p.th_d.cos(), p.th_d.sin());
    let map = |pt: (f64, f64), c: f64, s: f64| -> (f64, f64) {
        let x = (pt.0 - p.pv.0) * p.sx * c - (pt.1 - p.pv.1) * p.sy * s;
        let y = (pt.0 - p.pv.0) * p.sx * s + (pt.1 - p.pv.1) * p.sy * c;
        (
            cx + p.pv.0 * r + x * r + p.ox * k,
            cy + p.pv.1 * r + y * r + p.oy * k,
        )
    };
    let ru = [
        map(TRI_U[0], cu, su),
        map(TRI_U[1], cu, su),
        map(TRI_U[2], cu, su),
    ];
    let rd = [
        map(TRI_D[0], cd, sd),
        map(TRI_D[1], cd, sd),
        map(TRI_D[2], cd, sd),
    ];
    let search = (r * p.sx.abs().max(p.sy.abs())).ceil() as i32 + 8;
    let x0 = (cx + p.ox * k - search as f64).floor().max(0.0) as i32;
    let x1 = ((cx + p.ox * k + search as f64).ceil() as i32).min(cv.n - 1);
    let y0 = (cy + p.oy * k - search as f64).floor().max(0.0) as i32;
    let y1 = ((cy + p.oy * k + search as f64).ceil() as i32).min(cv.n - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            match p.fx {
                Fx::Twinkle => {
                    let h = ((x as u32)
                        .wrapping_mul(73)
                        .wrapping_add(y as u32)
                        .wrapping_mul(2_654_435_761))
                        ^ p.frame.wrapping_mul(97).wrapping_add(13);
                    if h % 100 < 15 {
                        continue;
                    }
                }
                Fx::Scan | Fx::ScanFill => {
                    let sc = if p.scan >= 0.0 {
                        p.scan
                    } else {
                        (p.t * 30.0).rem_euclid(cv.n as f64 + 28.0) - 14.0
                    };
                    let cut = if p.fx == Fx::ScanFill {
                        py > sc + 7.0
                    } else {
                        (py - sc).abs() > 7.0
                    };
                    if cut {
                        continue;
                    }
                }
                Fx::ScanWindow => {
                    let sc = if p.scan >= 0.0 {
                        p.scan
                    } else {
                        (p.t * 30.0).rem_euclid(cv.n as f64 + 28.0) - 14.0
                    };
                    let win = 0.4 * cv.r;
                    if py > sc || py < sc - win {
                        continue;
                    }
                }
                Fx::Wipe => {
                    let mut rel = ((py - cy).atan2(px - cx) - p.t * 1.5).rem_euclid(TAU);
                    if rel < 0.0 {
                        rel += TAU;
                    }
                    if rel > 4.6 {
                        continue;
                    }
                }
                Fx::Wave | Fx::None => {}
            }
            let qx = if p.fx == Fx::Wave {
                px + 3.0 * (py * 0.3 + p.t * 3.0).sin()
            } else {
                px
            };
            let tri = [ru, rd];
            let idx = (y * cv.n + x) as usize;
            if cv.aa <= 1 {
                if !tri.iter().any(|t| point_in_tri(qx, py, t)) {
                    continue;
                }
                cv.px[idx] = 255;
            } else {
                let mut hits = 0u32;
                for (dx, dy) in SUB {
                    let sx = x as f64 + dx;
                    let sy = y as f64 + dy;
                    if tri.iter().any(|t| point_in_tri(sx, sy, t)) {
                        hits += 1;
                    }
                }
                if hits == 0 {
                    continue;
                }
                let nv = (255 * hits / 4) as u8;
                if nv > cv.px[idx] {
                    cv.px[idx] = nv;
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct Solid {
    pub t: f64,
    pub axis_x: bool,
    pub dark_bg: bool,
    pub angle: Option<f64>,
    pub rim_gain: f64,
    pub wob_gain: f64,
    pub ox: f64,
    pub oy: f64,
}

impl Solid {
    pub fn base(t: f64) -> Solid {
        Solid {
            t,
            axis_x: false,
            dark_bg: false,
            angle: None,
            rim_gain: 1.0,
            wob_gain: 1.0,
            ox: 0.0,
            oy: 0.0,
        }
    }
}

pub fn draw_solid(cv: &mut Canvas, s: &Solid) {
    let (t, axis_x, dark_bg) = (s.t, s.axis_x, s.dark_bg);
    let (ox, oy) = (s.ox, s.oy);
    let auto = t * 1.5;
    let mut a = match s.angle {
        Some(v) => v,
        None => auto,
    };
    for _ in 0..4 {
        if s.angle.is_none() {
            a += 0.004 * (0.5 + (1.0 - a.cos().abs()));
        }
    }
    let wob = s.wob_gain * 0.16 * (t * 1.15).sin();
    let wob2 = s.wob_gain * 0.12 * ((t * 0.83) + 1.4).sin();
    let (ca, sa) = (a.cos(), a.sin());
    let (cw, sw) = (wob.cos(), wob.sin());
    let (cw2, sw2) = (wob2.cos(), wob2.sin());

    let mv = |p: (f64, f64), z: f64| -> (f64, f64, f64) {
        let (x, y, zz) = if axis_x {
            (p.0, p.1 * ca + z * sa, -p.1 * sa + z * ca)
        } else {
            (p.0 * ca + z * sa, p.1, -p.0 * sa + z * ca)
        };
        let y2 = y * cw - zz * sw;
        let z2 = y * sw + zz * cw;
        (x * cw2 - y2 * sw2, x * sw2 + y2 * cw2, z2)
    };
    let proj = |v: (f64, f64, f64)| -> (f64, f64, f64) {
        let f = CAM / (CAM - v.2);
        (
            cv.cx + (v.0 + ox) * cv.r * f,
            cv.cy + (v.1 + oy) * cv.r * f,
            v.2,
        )
    };
    let front: Vec<(f64, f64, f64)> = SOLID_PTS.iter().map(|p| proj(mv(*p, HD))).collect();
    let back: Vec<(f64, f64, f64)> = SOLID_PTS.iter().map(|p| proj(mv(*p, -HD))).collect();

    let p3 = |v: &Vec<(f64, f64, f64)>, i: usize| (v[i].0, v[i].1);
    let gain = s.rim_gain.clamp(0.0, 1.0);
    if gain > 0.02 && dark_bg {
        let back_v = (55.0 * gain) as u8;
        cv.tri([p3(&back, 0), p3(&back, 1), p3(&back, 2)], back_v);
        cv.tri([p3(&back, 3), p3(&back, 4), p3(&back, 5)], back_v);
    }
    let rim: [u8; 6] = if dark_bg { RIM_SH } else { RIM_SH_BRIGHT };

    type Quad = (f64, [(f64, f64); 4], u8);
    let mut quads: Vec<Quad> = Vec::with_capacity(18);
    for (n, (i, j)) in HALF_EDGES.iter().enumerate() {
        let (i, j) = (*i, *j);
        let pts = [
            (front[i].0, front[i].1),
            (front[j].0, front[j].1),
            (back[j].0, back[j].1),
            (back[i].0, back[i].1),
        ];
        let z = (front[i].2 + front[j].2 + back[i].2 + back[j].2) / 4.0;
        let base = rim[n % 6] as f64;
        let v = (255.0 - (255.0 - base) * gain).round() as u8;
        quads.push((z, pts, v));
    }
    quads.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    for (_, pts, v) in quads {
        cv.poly(&pts, v);
    }

    cv.tri([p3(&front, 0), p3(&front, 1), p3(&front, 2)], 255);
    cv.tri([p3(&front, 3), p3(&front, 4), p3(&front, 5)], 255);
}
