#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    const WAVE_FORMAT_PCM: u32 = 1;
    const WAVE_MAPPER: u32 = 0xFFFF_FFFF;
    const CALLBACK_NULL: usize = 0;
    const RATE: usize = 48_000;
    const BUF_FRAMES: usize = 1024;
    const NBUF: usize = 4;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WaveFormatEx {
        w_format_tag: u16,
        n_channels: u16,
        n_samples_per_sec: u32,
        n_avg_bytes_per_sec: u32,
        n_block_align: u16,
        w_bits_per_sample: u16,
        cb_size: u16,
    }

    impl Default for WaveFormatEx {
        fn default() -> WaveFormatEx {
            WaveFormatEx {
                w_format_tag: 0,
                n_channels: 0,
                n_samples_per_sec: 0,
                n_avg_bytes_per_sec: 0,
                n_block_align: 0,
                w_bits_per_sample: 0,
                cb_size: 0,
            }
        }
    }

    #[repr(C)]
    struct WaveHdr {
        lp_data: *mut c_void,
        dw_buffer_length: u32,
        bytes_recorded: u32,
        dw_user: usize,
        dw_flags: u32,
        dw_loops: u32,
        lp_next: *mut WaveHdr,
        reserved: usize,
    }

    impl Default for WaveHdr {
        fn default() -> WaveHdr {
            WaveHdr {
                lp_data: std::ptr::null_mut(),
                dw_buffer_length: 0,
                bytes_recorded: 0,
                dw_user: 0,
                dw_flags: 0,
                dw_loops: 0,
                lp_next: std::ptr::null_mut(),
                reserved: 0,
            }
        }
    }

    const HDR_SIZE: u32 = std::mem::size_of::<WaveHdr>() as u32;
    const WHDR_DONE: u32 = 0x0000_0001;

    #[link(name = "winmm")]
    extern "system" {
        fn waveInOpen(
            phwi: *mut usize,
            id: u32,
            format: *mut WaveFormatEx,
            callback: usize,
            instance: usize,
            flags: u32,
        ) -> i32;
        fn waveInPrepareHeader(hwi: usize, hdr: *mut WaveHdr, size: u32) -> i32;
        fn waveInAddBuffer(hwi: usize, hdr: *mut WaveHdr, size: u32) -> i32;
        fn waveInStart(hwi: usize) -> i32;
    }

    pub struct Mic {
        level: Arc<AtomicU32>,
        failed: Arc<AtomicU32>,
    }

    pub fn start() -> Option<Mic> {
        let level = Arc::new(AtomicU32::new(0));
        let failed = Arc::new(AtomicU32::new(0));
        let l = level.clone();
        let f = failed.clone();
        let ok = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || capture(l, f))
            .is_ok();
        if ok {
            Some(Mic { level, failed })
        } else {
            None
        }
    }

    impl Mic {
        pub fn level(&self) -> f64 {
            f32::from_bits(self.level.load(Ordering::Relaxed)) as f64
        }
        pub fn failed(&self) -> bool {
            self.failed.load(Ordering::Relaxed) != 0
        }
    }

    fn capture(level: Arc<AtomicU32>, failed: Arc<AtomicU32>) {
        unsafe {
            let mut wf = WaveFormatEx {
                w_format_tag: WAVE_FORMAT_PCM as u16,
                n_channels: 1,
                n_samples_per_sec: RATE as u32,
                n_avg_bytes_per_sec: (RATE * 2) as u32,
                n_block_align: 2,
                w_bits_per_sample: 16,
                cb_size: 0,
            };
            let mut hwi: usize = 0;
            if waveInOpen(&mut hwi, WAVE_MAPPER, &mut wf, CALLBACK_NULL, 0, 0) != 0 {
                failed.store(1, Ordering::Relaxed);
                return;
            }
            let mut raw: Vec<i16> = vec![0; BUF_FRAMES * NBUF];
            let base = raw.as_mut_ptr() as *mut u8;
            let mut hdrs: [WaveHdr; NBUF] = Default::default();
            let mut prepared = 0usize;
            for (i, hdr) in hdrs.iter_mut().enumerate() {
                hdr.lp_data = base.add(i * BUF_FRAMES * 2) as *mut c_void;
                hdr.dw_buffer_length = (BUF_FRAMES * 2) as u32;
                if waveInPrepareHeader(hwi, hdr, HDR_SIZE) != 0 {
                    failed.store(1, Ordering::Relaxed);
                    break;
                }
                prepared += 1;
            }
            if prepared != NBUF {
                failed.store(1, Ordering::Relaxed);
                return;
            }
            for hdr in hdrs.iter_mut() {
                if waveInAddBuffer(hwi, hdr, HDR_SIZE) != 0 {
                    failed.store(1, Ordering::Relaxed);
                    return;
                }
            }
            if waveInStart(hwi) != 0 {
                failed.store(1, Ordering::Relaxed);
                return;
            }
            let mut smooth = 0.0f64;
            loop {
                let mut sum = 0.0f64;
                let mut count = 0u32;
                for idx in 0..NBUF {
                    let hdr = &mut hdrs[idx];
                    if hdr.dw_flags & WHDR_DONE != 0 {
                        let samples = hdr.dw_buffer_length as usize / 2;
                        let p = hdr.lp_data as *const i16;
                        for i in 0..samples {
                            let v = *p.add(i) as f64 / 32768.0;
                            sum += v * v;
                        }
                        count += samples as u32;
                        hdr.dw_flags &= !WHDR_DONE;
                        waveInAddBuffer(hwi, hdr, HDR_SIZE);
                    }
                }
                if count > 0 {
                    let rms = ((sum / count as f64).sqrt() * 5.5).clamp(0.0, 1.0);
                    smooth += (rms - smooth) * if rms > smooth { 0.55 } else { 0.10 };
                    level.store((smooth as f32).to_bits(), Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    pub struct Mic {
        level: Arc<AtomicU32>,
    }

    pub fn start() -> Option<Mic> {
        let level = Arc::new(AtomicU32::new(0));
        let out = level.clone();
        std::thread::spawn(move || {
            let mut t = 0.0f64;
            let mut smooth = 0.0f64;
            loop {
                t += 0.01;
                let wobble =
                    (0.20 + 0.70 * (t * 1.7).sin().abs() * (t * 0.53).sin().abs()).clamp(0.0, 1.0);
                smooth += (wobble - smooth) * 0.25;
                out.store((smooth as f32).to_bits(), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        Some(Mic { level })
    }

    impl Mic {
        pub fn level(&self) -> f64 {
            f32::from_bits(self.level.load(Ordering::Relaxed)) as f64
        }
        pub fn failed(&self) -> bool {
            false
        }
    }
}

pub use imp::Mic;

pub fn start_mic() -> Option<Mic> {
    imp::start()
}
