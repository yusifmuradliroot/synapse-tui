use crate::render::{draw_pass, draw_solid, Canvas, Fx, Pass};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    Standard,
    Breath,
    Spin,
    Slow,
    Reverse,
    Rock,
    Pendulum,
    Pulse,
    Heartbeat,
    Squash,
    Stretch,
    SlideX,
    SlideY,
    Orbit,
    SpinOrbit,
    CoinX,
    CoinY,
    Stairs,
    Jitter,
    Twinkle,
    Scan,
    Wipe,
    Trail,
    Counter,
    Wave,
    Blink,
    Grow,
    SpinPulse,
    Drift,
    Shiver,
    Static,
    Swing,
}

pub const ALL: [Variant; 32] = [
    Variant::Standard,
    Variant::Breath,
    Variant::Spin,
    Variant::Slow,
    Variant::Reverse,
    Variant::Rock,
    Variant::Pendulum,
    Variant::Pulse,
    Variant::Heartbeat,
    Variant::Squash,
    Variant::Stretch,
    Variant::SlideX,
    Variant::SlideY,
    Variant::Orbit,
    Variant::SpinOrbit,
    Variant::CoinX,
    Variant::CoinY,
    Variant::Stairs,
    Variant::Jitter,
    Variant::Twinkle,
    Variant::Scan,
    Variant::Wipe,
    Variant::Trail,
    Variant::Counter,
    Variant::Wave,
    Variant::Blink,
    Variant::Grow,
    Variant::SpinPulse,
    Variant::Drift,
    Variant::Shiver,
    Variant::Static,
    Variant::Swing,
];

pub const FEATURED: [Variant; 8] = [
    Variant::Standard,
    Variant::Breath,
    Variant::Spin,
    Variant::Rock,
    Variant::CoinX,
    Variant::CoinY,
    Variant::Counter,
    Variant::Trail,
];

impl Variant {
    pub fn name(self) -> &'static str {
        match self {
            Variant::Standard => "STANDARD",
            Variant::Breath => "BREATH",
            Variant::Spin => "SPIN",
            Variant::Slow => "SLOW",
            Variant::Reverse => "REVERSE",
            Variant::Rock => "ROCK",
            Variant::Pendulum => "PENDULUM",
            Variant::Pulse => "PULSE",
            Variant::Heartbeat => "HEARTBEAT",
            Variant::Squash => "SQUASH",
            Variant::Stretch => "STRETCH",
            Variant::SlideX => "SLIDE X",
            Variant::SlideY => "SLIDE Y",
            Variant::Orbit => "ORBIT",
            Variant::SpinOrbit => "SPIN+ORBIT",
            Variant::CoinX => "COIN X",
            Variant::CoinY => "COIN Y",
            Variant::Stairs => "STAIRS",
            Variant::Jitter => "JITTER",
            Variant::Twinkle => "TWINKLE",
            Variant::Scan => "SCAN",
            Variant::Wipe => "WIPE",
            Variant::Trail => "TRAIL",
            Variant::Counter => "COUNTER",
            Variant::Wave => "WAVE",
            Variant::Blink => "BLINK",
            Variant::Grow => "GROW",
            Variant::SpinPulse => "SPIN PULSE",
            Variant::Drift => "DRIFT",
            Variant::Shiver => "SHIVER",
            Variant::Static => "STATIC",
            Variant::Swing => "SWING",
        }
    }
}

pub struct Cell {
    pub theta: f64,
}

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    pub fn unit(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn rng_f(env: &mut Env) -> f64 {
    env.rng.unit()
}

pub struct Env<'a> {
    pub t: f64,
    pub dt: f64,
    pub spd: f64,
    pub frame: u32,
    pub rng: &'a mut Rng,
    pub dark_bg: bool,
}

pub fn draw(cv: &mut Canvas, v: Variant, cell: &mut Cell, env: &mut Env) {
    let (t, dt, spd, frame) = (env.t, env.dt, env.spd, env.frame);
    cv.clear();
    match v {
        Variant::Blink => {
            if (t * 1.5) % 1.0 >= 0.7 {
                return;
            }
        }
        Variant::CoinX => {
            draw_solid(cv, t, true, env.dark_bg, None);
            return;
        }
        Variant::CoinY => {
            draw_solid(cv, t, false, env.dark_bg, None);
            return;
        }
        _ => {}
    }

    let mut sx = 1.0f64;
    let mut sy = 1.0f64;
    let (mut ox, mut oy) = (0.0f64, 0.0f64);
    let mut pv = (0.0f64, 0.0f64);
    let mut fx = Fx::None;

    match v {
        Variant::Standard => cell.theta += spd * dt,
        Variant::Breath => {
            let s = 1.0 + 0.05 * (t * 1.6).sin();
            sx = s;
            sy = s;
            cell.theta += spd * dt;
        }
        Variant::Spin => cell.theta += 0.9 * dt,
        Variant::Slow => cell.theta += 0.25 * dt,
        Variant::Reverse => cell.theta -= 0.7 * dt,
        Variant::Rock | Variant::Pendulum => cell.theta += spd * dt,
        Variant::Pulse => {
            let s = 1.0 + 0.12 * (t * 2.2).sin();
            sx = s;
            sy = s;
            cell.theta += spd * dt;
        }
        Variant::Heartbeat => {
            let a =
                (t * 3.0).sin().max(0.0).powi(3) + 0.5 * ((t * 6.0 + 1.0).sin().max(0.0)).powi(3);
            sx = 1.0 + 0.09 * a;
            sy = sx;
            cell.theta += spd * dt;
        }
        Variant::Squash => {
            let q = 0.1 * (t * 1.8).sin();
            sx = 1.0 + q;
            sy = 1.0 - q;
            cell.theta += spd * dt;
        }
        Variant::Stretch => {
            sx = 1.0 + 0.12 * (t * 1.1).sin();
            cell.theta += spd * dt;
        }
        Variant::SlideX => {
            ox = 4.0 * (t * 0.9).sin();
            cell.theta += spd * dt;
        }
        Variant::SlideY => {
            oy = 4.0 * (t * 1.1).sin();
            cell.theta += spd * dt;
        }
        Variant::Orbit => {
            ox = 5.0 * (t * 0.7).cos();
            oy = 5.0 * (t * 0.7).sin();
        }
        Variant::SpinOrbit => {
            cell.theta += 0.8 * dt;
            ox = 5.0 * (t * 0.7).cos();
            oy = 5.0 * (t * 0.7).sin();
        }
        Variant::Stairs => cell.theta += spd * dt,
        Variant::Jitter => {
            ox = (rng_f(env) - 0.5) * 2.0;
            oy = (rng_f(env) - 0.5) * 2.0;
            cell.theta += spd * dt;
        }
        Variant::Twinkle => {
            cell.theta += spd * dt;
            fx = Fx::Twinkle;
        }
        Variant::Scan => {
            cell.theta += spd * dt;
            fx = Fx::Scan;
        }
        Variant::Wipe => {
            cell.theta += spd * dt;
            fx = Fx::Wipe;
        }
        Variant::Trail => cell.theta += if spd != 0.0 { spd } else { 0.9 } * dt,
        Variant::Counter => cell.theta += 0.6 * dt,
        Variant::Wave => {
            cell.theta += spd * dt;
            fx = Fx::Wave;
        }
        Variant::Grow => {
            let g = (t * 0.25) % 1.0;
            sx = 0.75 + 0.4 * g;
            sy = sx;
            cell.theta += spd * dt;
        }
        Variant::SpinPulse => cell.theta += 0.9 * (0.5 + 0.5 * (t * 0.5).sin()) * dt,
        Variant::Drift => {
            cell.theta += 0.3 * dt;
            ox = 3.0 * (t * 0.3).sin();
            oy = ox;
        }
        Variant::Shiver => cell.theta += 0.5 * dt,
        Variant::Static => {}
        Variant::Swing => pv = (0.0, 1.0),
        Variant::CoinX | Variant::CoinY | Variant::Blink => cell.theta += spd * dt,
    }

    let (th_u, th_d) = match v {
        Variant::Rock => {
            let a = cell.theta + 0.35 * (t * 0.45).sin();
            (a, a)
        }
        Variant::Pendulum => {
            let a = cell.theta + 1.2 * (t * 0.8).sin();
            (a, a)
        }
        Variant::Stairs => {
            let q = std::f64::consts::PI / 12.0;
            let a = (cell.theta / q).floor() * q;
            (a, a)
        }
        Variant::Swing => {
            let a = cell.theta + 0.45 * (t * 0.9).sin();
            (a, a)
        }
        Variant::Counter => (cell.theta, -cell.theta),
        Variant::Shiver => {
            let a = cell.theta + 0.05 * (t * 25.0).sin();
            (a, a)
        }
        _ => (cell.theta, cell.theta),
    };

    let p = Pass {
        th_u,
        th_d,
        sx,
        sy,
        ox,
        oy,
        pv,
        fx,
        t,
        frame,
        scan: -1.0,
    };
    if v == Variant::Trail {
        for j in (0..3).rev() {
            let mut q = Pass { fx: Fx::None, ..p };
            q.th_u = th_u - 0.15 * j as f64;
            q.th_d = th_d - 0.15 * j as f64;
            draw_pass(cv, &q);
        }
    } else {
        draw_pass(cv, &p);
    }
}
