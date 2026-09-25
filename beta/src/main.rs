#[allow(dead_code, unused_imports)]
#[path = "../../tui/src/render.rs"]
mod render;
#[allow(dead_code, unused_imports)]
mod term;

use render::{Canvas, Solid};
use std::f64::consts::{PI, TAU};
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const NAME: &str = env!("CARGO_BIN_NAME");

// Sanal canvas: 1280x720 gri ton. Yildiz kutusu sol ustte 256x256,
// cevresinde 1px beyaz cerceve. Geri kalan full siyah.
const VW: usize = 1280;
const VH: usize = 720;
const BOX: usize = 256;

const T_BLANK: f64 = 3.0;
const T_SWEEP: f64 = 2.5;
const T_HOLD: f64 = 0.5;
const T_HOLD2IDLE: f64 = 1.5;
const IDLE_R: f64 = 0.08;
const PARK_Y: f64 = -0.15;
const PROD_TILT: f64 = 0.18;
const WHEEL_CRUISE: f64 = 2.2;
const IDLE_W: f64 = 1.25663706144;
const FRAME: Duration = Duration::from_micros(16_667);

const RAMP: &[u8] = b" .:-=+*#%@";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Blank,
    Scan,
    ScanFill,
    Hold,
    Idle,
    Returning,
    Production,
}

fn angle_rate(a: f64) -> f64 {
    1.5 * (0.5 + (1.0 - a.cos().abs()))
}

fn clip_y(cv: &mut Canvas, lo: f64, hi: f64) {
    let n = cv.n;
    for y in 0..n {
        let py = y as f64 + 0.5;
        if py < lo || py >= hi {
            for x in 0..n {
                cv.px[(y * n + x) as usize] = 0;
            }
        }
    }
}

fn coin_face(cv: &mut Canvas, t: f64, ox: f64, oy: f64) {
    render::draw_solid(
        cv,
        &Solid {
            t,
            angle: Some(0.0),
            rim_gain: 1.0,
            wob_gain: 0.0,
            ox,
            oy,
            dark_bg: true,
            ..Solid::base(t)
        },
    );
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

// Sanal canvas -> konsol hucreleri. Kutu disi hep siyah; kutu
// cevresine deginen hucreler tam beyaz blok (1px cerceve gorunur kalir),
// kutu ici gri rampa ile orneklenir.
fn downsample(vb: &[u8], cols: usize, rows: usize, cells: &mut Vec<char>) {
    cells.clear();
    cells.reserve(cols * rows);
    let b = BOX as f64;
    for j in 0..rows {
        let fy0 = j as f64 * VH as f64 / rows as f64;
        let fy1 = (j + 1) as f64 * VH as f64 / rows as f64;
        for i in 0..cols {
            let fx0 = i as f64 * VW as f64 / cols as f64;
            let fx1 = (i + 1) as f64 * VW as f64 / cols as f64;
            if fx0 < b && fy0 < b && (fx0 < 1.0 || fx1 > b - 1.0 || fy0 < 1.0 || fy1 > b - 1.0) {
                cells.push('\u{2588}');
                continue;
            }
            if fx0 >= b || fy0 >= b {
                cells.push(' ');
                continue;
            }
            let ix0 = fx0.floor() as usize;
            let iy0 = fy0.floor() as usize;
            let mut ix1 = fx1.ceil() as usize;
            let mut iy1 = fy1.ceil() as usize;
            if ix1 > BOX {
                ix1 = BOX;
            }
            if iy1 > BOX {
                iy1 = BOX;
            }
            let mut sum = 0u32;
            let mut cnt = 0u32;
            for y in iy0..iy1 {
                let row = y * VW;
                for x in ix0..ix1 {
                    sum += vb[row + x] as u32;
                    cnt += 1;
                }
            }
            if cnt == 0 {
                cells.push(' ');
                continue;
            }
            let avg = (sum / cnt).min(255) as usize;
            let idx = (avg * RAMP.len() / 256).min(RAMP.len() - 1);
            cells.push(RAMP[idx] as char);
        }
    }
}

// Tum hucreler beyaz on / siyah arka plan: tek renk ayari + konum + karakter.
fn render_cells(cells: &[char], cols: usize, rows: usize, out: &mut String) {
    out.clear();
    out.push_str("\x1b[?2026h\x1b[H\x1b[38;2;255;255;255m\x1b[48;2;0;0;0m");
    for j in 0..rows {
        if j > 0 {
            out.push_str(&format!("\x1b[{};1H", j + 1));
        }
        for i in 0..cols {
            out.push(cells[j * cols + i]);
        }
    }
    out.push_str("\x1b[0m\x1b[?2026l");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{NAME} v{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let saved = term::raw_start();
    term::setup_window();
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

    let mut vb = vec![0u8; VW * VH];
    let mut cells: Vec<char> = Vec::new();
    let mut frame = String::new();
    let mut last_size = (0usize, 0usize);
    let mut cv = Canvas::new_exact(BOX as i32);
    cv.aa = 2;
    let mut phase = Phase::Blank;
    let mut pt = 0.0f64;
    let mut scan_prog = 0.0f64;
    let mut coin_angle = 0.0f64;
    let mut spin_vel = 0.0f64;
    let mut parked = false;
    let mut spin_env = 0.0f64;
    let mut park_env = 0.0f64;
    let mut prod_wheel = 0.0f64;
    let mut prod_wvel = 0.0f64;
    let mut prod_stop: Option<(Phase, f64, f64)> = None;
    let mut stop_t = 0.0f64;
    let mut hold_auto = false;
    let mut last = Instant::now();
    let t0 = last;

    loop {
        while let Ok(b) = rx.try_recv() {
            if b == b'1' {
                phase = Phase::Blank;
                pt = 0.0;
                scan_prog = 0.0;
                coin_angle = 0.0;
                spin_vel = 0.0;
                spin_env = 0.0;
                park_env = 0.0;
                parked = false;
                hold_auto = false;
                prod_wheel = 0.0;
                prod_wvel = 0.0;
                prod_stop = None;
            } else if b == b'2' && phase != Phase::Idle {
                if phase == Phase::Production {
                    let yaw_t = (coin_angle / PI).round() * PI;
                    let wheel_t = (prod_wheel / (PI / 3.0)).round() * (PI / 3.0);
                    prod_stop = Some((Phase::Idle, yaw_t, wheel_t));
                    stop_t = 0.0;
                } else {
                    phase = Phase::Idle;
                    pt = 0.0;
                }
            } else if b == b'3' {
                phase = Phase::Production;
                pt = 0.0;
                prod_stop = None;
            } else if b == b'0' {
                if phase == Phase::Production {
                    let yaw_t = (coin_angle / PI).round() * PI;
                    let wheel_t = (prod_wheel / (PI / 3.0)).round() * (PI / 3.0);
                    prod_stop = Some((Phase::Hold, yaw_t, wheel_t));
                    stop_t = 0.0;
                    parked = true;
                } else if phase == Phase::Idle {
                    phase = Phase::Returning;
                    pt = 0.0;
                    hold_auto = false;
                    parked = true;
                } else if phase != Phase::Hold {
                    phase = Phase::Hold;
                    pt = 0.0;
                    hold_auto = false;
                    parked = true;
                }
            } else if b == b'q' || b == 0x1b || b == 3 {
                term::raw_stop(&saved);
                let _ = out.write_all(b"\x1b[?25h\x1b[0m\x1b[?1049l");
                let _ = out.flush();
                return;
            }
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.05);
        last = now;
        let t = now.duration_since(t0).as_secs_f64();
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
            Phase::Hold if hold_auto && pt >= T_HOLD2IDLE => phase = Phase::Idle,
            _ => {}
        }
        if prev != phase {
            pt = 0.0;
            if phase == Phase::Scan || phase == Phase::ScanFill {
                scan_prog = 0.0;
            }
            if phase == Phase::Hold && prev == Phase::ScanFill {
                hold_auto = true;
                parked = false;
            }
        }
        if phase == Phase::Scan || phase == Phase::ScanFill {
            scan_prog = (scan_prog + dt / T_SWEEP).min(1.0);
        }
        let spin_target = if phase == Phase::Idle { 1.0 } else { 0.0 };
        let env_tau = if phase == Phase::Returning { 0.15 } else { 0.3 };
        spin_env += (spin_target - spin_env) * (1.0 - (-dt / env_tau).exp());
        let park_target = match phase {
            Phase::Hold if parked => 1.0,
            Phase::Returning | Phase::Production => 1.0,
            _ => 0.0,
        };
        let park_tau = if phase == Phase::Returning {
            0.15
        } else {
            0.25
        };
        park_env += (park_target - park_env) * (1.0 - (-dt / park_tau).exp());
        if phase == Phase::Idle {
            let cruise = angle_rate(coin_angle);
            spin_vel += (cruise - spin_vel) * (1.0 - (-dt / 0.25).exp());
            coin_angle += spin_vel * dt;
        } else if phase == Phase::Returning {
            let target = (coin_angle / PI).round() * PI;
            let servo = ((target - coin_angle) * 12.0).clamp(-14.0, 14.0);
            spin_vel += (servo - spin_vel) * (1.0 - (-dt / 0.1).exp());
            coin_angle += spin_vel * dt;
            if (coin_angle - target).abs() < 0.03
                && spin_vel.abs() < 0.6
                && spin_env < 0.05
                && park_env > 0.95
            {
                coin_angle = target;
                spin_vel = 0.0;
                phase = Phase::Hold;
                pt = 0.0;
            }
        } else if phase == Phase::Production {
            if let Some((target_phase, yaw_t, wheel_t)) = prod_stop {
                stop_t += dt;
                let yaw_servo = ((yaw_t - coin_angle) * 4.0).clamp(-8.0, 8.0);
                spin_vel += (yaw_servo - spin_vel) * (1.0 - (-dt / 0.15).exp());
                coin_angle += spin_vel * dt;
                let wheel_servo = ((wheel_t - prod_wheel) * 4.0).clamp(-8.0, 8.0);
                prod_wvel += (wheel_servo - prod_wvel) * (1.0 - (-dt / 0.15).exp());
                prod_wheel += prod_wvel * dt;
                if ((coin_angle - yaw_t).abs() < 0.03
                    && spin_vel.abs() < 0.6
                    && (prod_wheel - wheel_t).abs() < 0.05
                    && prod_wvel.abs() < 1.0)
                    || stop_t > 2.5
                {
                    coin_angle = yaw_t;
                    prod_wheel = wheel_t;
                    spin_vel = 0.0;
                    prod_wvel = 0.0;
                    prod_stop = None;
                    phase = target_phase;
                    pt = 0.0;
                    if target_phase == Phase::Hold {
                        hold_auto = false;
                    }
                }
            } else {
                let tilt_target = PROD_TILT + ((coin_angle - PROD_TILT) / TAU).round() * TAU;
                let yaw_servo = ((tilt_target - coin_angle) * 4.0).clamp(-8.0, 8.0);
                spin_vel += (yaw_servo - spin_vel) * (1.0 - (-dt / 0.15).exp());
                coin_angle += spin_vel * dt;
                prod_wvel += (WHEEL_CRUISE - prod_wvel) * (1.0 - (-dt / 0.15).exp());
                prod_wheel += prod_wvel * dt;
            }
        }

        let animated = phase == Phase::Scan
            || phase == Phase::ScanFill
            || phase == Phase::Idle
            || phase == Phase::Returning
            || phase == Phase::Production;
        if !animated && !resized && prev == phase {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }

        let w = w.max(10);
        let h = h.max(5);
        let nf = BOX as f64;
        let sweep = -14.0 + (nf + 28.0) * scan_prog;
        cv.clear();
        match phase {
            Phase::Blank => {}
            Phase::Scan => {
                let r = cv.r;
                coin_face(&mut cv, t, 0.0, park_env * PARK_Y);
                clip_y(&mut cv, sweep - 0.4 * r, sweep);
                mirror_x(&mut cv);
            }
            Phase::ScanFill => {
                coin_face(&mut cv, t, 0.0, park_env * PARK_Y);
                clip_y(&mut cv, f64::MIN, sweep + 7.0);
                mirror_x(&mut cv);
            }
            Phase::Hold => {
                coin_face(&mut cv, t, 0.0, park_env * PARK_Y);
                mirror_x(&mut cv);
            }
            Phase::Idle | Phase::Returning => {
                let wt = t * IDLE_W;
                render::draw_solid(
                    &mut cv,
                    &Solid {
                        t,
                        angle: Some(coin_angle),
                        rim_gain: 1.0,
                        wob_gain: spin_env,
                        ox: spin_env * IDLE_R * wt.cos(),
                        oy: spin_env * IDLE_R * wt.sin() + park_env * PARK_Y,
                        dark_bg: true,
                        ..Solid::base(t)
                    },
                );
            }
            Phase::Production => {
                render::draw_solid(
                    &mut cv,
                    &Solid {
                        t,
                        angle: Some(coin_angle),
                        rot_z: prod_wheel,
                        rim_gain: 1.0,
                        wob_gain: 0.0,
                        ox: spin_env * IDLE_R * (t * IDLE_W).cos(),
                        oy: spin_env * IDLE_R * (t * IDLE_W).sin() + park_env * PARK_Y,
                        dark_bg: true,
                        ..Solid::base(t)
                    },
                );
            }
        }

        // Yildiz kutusunu sanal canvasin sol ustune blitle.
        for y in 0..BOX {
            let src = &cv.px[y * BOX..(y + 1) * BOX];
            vb[y * VW..y * VW + BOX].copy_from_slice(src);
        }
        // 1px beyaz cerceve (kutu cevresi).
        for x in 0..BOX {
            vb[x] = 255;
            vb[(BOX - 1) * VW + x] = 255;
        }
        for y in 0..BOX {
            vb[y * VW] = 255;
            vb[y * VW + BOX - 1] = 255;
        }

        downsample(&vb, w, h, &mut cells);
        render_cells(&cells, w, h, &mut frame);
        let _ = out.write_all(frame.as_bytes());
        let _ = out.flush();

        if animated {
            let spent = now.elapsed();
            if spent < FRAME {
                std::thread::sleep(FRAME - spent);
            }
        }
    }
}
