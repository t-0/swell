//! Playback through the default output device via cpal, with linear
//! resampling from the file's rate to the device's rate, pause/resume and
//! loop support. The UI polls `position()` for the playhead.

use crate::audio::Audio;
use anyhow::{anyhow, bail, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

struct Shared {
    samples: Arc<Vec<f32>>, // interleaved
    channels: usize,
    src_rate: f64,
    pos: f64, // in source frames
    end: usize,
    loop_range: Option<(usize, usize)>,
}

pub struct Player {
    _stream: Option<cpal::Stream>,
    shared: Arc<Mutex<Shared>>,
    playing: Arc<AtomicBool>,
    pos_bits: Arc<AtomicU64>,
    speed_pct: Arc<AtomicU32>,
    keep_alive: Arc<AtomicU8>,
    idle_params: Arc<IdleParams>,
    pub error: Option<String>,
}

/// Keep-alive levels, settable at runtime (f32 bits in atomics).
pub struct IdleParams {
    dither: AtomicU32,
    tone: AtomicU32,
    tone_hz: AtomicU32,
}

impl IdleParams {
    fn new() -> IdleParams {
        IdleParams {
            dither: AtomicU32::new(KEEP_ALIVE_LEVEL.to_bits()),
            tone: AtomicU32::new(KEEP_ALIVE_TONE_LEVEL.to_bits()),
            tone_hz: AtomicU32::new(KEEP_ALIVE_TONE_HZ.to_bits()),
        }
    }
    fn get(&self) -> (f32, f32, f32) {
        (
            f32::from_bits(self.dither.load(Ordering::Relaxed)),
            f32::from_bits(self.tone.load(Ordering::Relaxed)),
            f32::from_bits(self.tone_hz.load(Ordering::Relaxed)),
        )
    }
}

/// Idle output while nothing is playing. Output devices (built-in speaker
/// amps, Bluetooth, USB DACs) mute after a stretch of digital silence and
/// fade back in on the next sound, eating the start of the first play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeepAlive {
    /// True digital silence.
    Off = 0,
    /// Triangular dither at about -96 dBFS peak.
    Dither = 1,
    /// Dither plus a 15 Hz tone at -60 dBFS: inaudible, but well above any
    /// silence-detector threshold.
    Tone = 2,
}

impl KeepAlive {
    fn from_u8(v: u8) -> KeepAlive {
        match v {
            0 => KeepAlive::Off,
            1 => KeepAlive::Dither,
            _ => KeepAlive::Tone,
        }
    }
    pub fn next(self) -> KeepAlive {
        match self {
            KeepAlive::Off => KeepAlive::Dither,
            KeepAlive::Dither => KeepAlive::Tone,
            KeepAlive::Tone => KeepAlive::Off,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            KeepAlive::Off => "off: digital silence while idle (devices may mute and fade in)",
            KeepAlive::Dither => "dither: -96 dBFS noise while idle",
            KeepAlive::Tone => "tone: 15 Hz at -60 dBFS plus dither while idle (strongest, inaudible)",
        }
    }
}

/// Peak level of the idle dither: about -96 dBFS, below 16-bit resolution.
const KEEP_ALIVE_LEVEL: f32 = 1.0 / 65536.0;
const KEEP_ALIVE_TONE_HZ: f32 = 15.0;
const KEEP_ALIVE_TONE_LEVEL: f32 = 0.001; // -60 dBFS

impl Player {
    pub fn new() -> Player {
        let shared = Arc::new(Mutex::new(Shared {
            samples: Arc::new(Vec::new()),
            channels: 1,
            src_rate: 44100.0,
            pos: 0.0,
            end: 0,
            loop_range: None,
        }));
        let playing = Arc::new(AtomicBool::new(false));
        let pos_bits = Arc::new(AtomicU64::new(0));
        let speed_pct = Arc::new(AtomicU32::new(100));
        let keep_alive = Arc::new(AtomicU8::new(KeepAlive::Tone as u8));
        let idle_params = Arc::new(IdleParams::new());
        let (stream, error) = match open_stream(shared.clone(), playing.clone(), pos_bits.clone(), speed_pct.clone(), keep_alive.clone(), idle_params.clone()) {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(format!("audio output unavailable: {e}"))),
        };
        Player {
            _stream: stream,
            shared,
            playing,
            pos_bits,
            speed_pct,
            keep_alive,
            idle_params,
            error,
        }
    }

    /// Keep-alive levels in dBFS (peak) and tone frequency in Hz.
    pub fn set_keep_alive_levels(&self, dither_db: f64, tone_db: f64, tone_hz: f64) {
        let amp = |db: f64| 10f64.powf(db.clamp(-150.0, -20.0) / 20.0) as f32;
        self.idle_params.dither.store(amp(dither_db).to_bits(), Ordering::Relaxed);
        self.idle_params.tone.store(amp(tone_db).to_bits(), Ordering::Relaxed);
        self.idle_params.tone_hz.store((tone_hz.clamp(1.0, 60.0) as f32).to_bits(), Ordering::Relaxed);
    }

    pub fn set_keep_alive(&self, mode: KeepAlive) {
        self.keep_alive.store(mode as u8, Ordering::Relaxed);
    }

    pub fn keep_alive(&self) -> KeepAlive {
        KeepAlive::from_u8(self.keep_alive.load(Ordering::Relaxed))
    }

    /// Playback rate in percent (100 = normal). Takes effect immediately.
    pub fn set_speed_pct(&self, pct: u32) {
        self.speed_pct.store(pct.max(1), Ordering::Relaxed);
    }

    pub fn available(&self) -> bool {
        self._stream.is_some()
    }

    /// Replace the buffer being played (call after any edit). Stops playback.
    pub fn set_audio(&mut self, audio: &Audio) {
        self.stop();
        let mut sh = self.shared.lock().unwrap();
        sh.samples = Arc::new(audio.interleaved());
        sh.channels = audio.n_channels().max(1);
        sh.src_rate = audio.sample_rate as f64;
        sh.pos = 0.0;
        sh.end = 0;
        sh.loop_range = None;
        self.pos_bits.store(0f64.to_bits(), Ordering::Relaxed);
    }

    pub fn play(&mut self, start: usize, end: usize, loop_range: Option<(usize, usize)>) {
        {
            let mut sh = self.shared.lock().unwrap();
            sh.pos = start as f64;
            sh.end = end;
            sh.loop_range = loop_range;
        }
        self.pos_bits.store((start as f64).to_bits(), Ordering::Relaxed);
        self.playing.store(true, Ordering::SeqCst);
    }

    /// Change the stop point and loop region live, without touching position.
    pub fn set_range(&self, end: usize, loop_range: Option<(usize, usize)>) {
        let mut sh = self.shared.lock().unwrap();
        sh.end = end;
        sh.loop_range = loop_range;
    }

    pub fn resume(&self) {
        self.playing.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::SeqCst);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }

    pub fn position(&self) -> usize {
        f64::from_bits(self.pos_bits.load(Ordering::Relaxed)) as usize
    }
}

fn open_stream(
    shared: Arc<Mutex<Shared>>,
    playing: Arc<AtomicBool>,
    pos_bits: Arc<AtomicU64>,
    speed_pct: Arc<AtomicU32>,
    keep_alive: Arc<AtomicU8>,
    idle_params: Arc<IdleParams>,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no default output device"))?;
    let cfg = device.default_output_config()?;
    let rate = cfg.sample_rate().0 as f64;
    let out_ch = cfg.channels() as usize;
    let config: cpal::StreamConfig = cfg.clone().into();
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, shared, playing, pos_bits, speed_pct, keep_alive, idle_params, rate, out_ch)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, shared, playing, pos_bits, speed_pct, keep_alive, idle_params, rate, out_ch)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, shared, playing, pos_bits, speed_pct, keep_alive, idle_params, rate, out_ch)?,
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, shared, playing, pos_bits, speed_pct, keep_alive, idle_params, rate, out_ch)?,
        f => bail!("unsupported device sample format {f:?}"),
    };
    stream.play()?;
    Ok(stream)
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Mutex<Shared>>,
    playing: Arc<AtomicBool>,
    pos_bits: Arc<AtomicU64>,
    speed_pct: Arc<AtomicU32>,
    keep_alive: Arc<AtomicU8>,
    idle_params: Arc<IdleParams>,
    dev_rate: f64,
    out_ch: usize,
) -> Result<cpal::Stream> {
    let mut idle = Idle::new(dev_rate as f32);
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let speed = speed_pct.load(Ordering::Relaxed) as f64 / 100.0;
            let mode = KeepAlive::from_u8(keep_alive.load(Ordering::Relaxed));
            let (d, t, hz) = idle_params.get();
            idle.set(d, t, hz);
            fill(data, out_ch, dev_rate / speed, &shared, &playing, &pos_bits, mode, &mut idle)
        },
        |_err| {},
        None,
    )?;
    Ok(stream)
}

/// Idle keep-alive signal generator: triangular-PDF dither (xorshift) and a
/// sub-audible tone. One value per frame; copied to every channel.
struct Idle {
    seed: u32,
    phase: f32,
    step: f32,
    dev_rate: f32,
    dither: f32,
    tone: f32,
}
impl Idle {
    fn new(dev_rate: f32) -> Idle {
        Idle {
            seed: 0x2545_F491,
            phase: 0.0,
            step: std::f32::consts::TAU * KEEP_ALIVE_TONE_HZ / dev_rate.max(1.0),
            dev_rate: dev_rate.max(1.0),
            dither: KEEP_ALIVE_LEVEL,
            tone: KEEP_ALIVE_TONE_LEVEL,
        }
    }
    fn set(&mut self, dither: f32, tone: f32, hz: f32) {
        self.dither = dither;
        self.tone = tone;
        self.step = std::f32::consts::TAU * hz / self.dev_rate;
    }
    #[inline]
    fn uniform(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32 // 0..1
    }
    #[inline]
    fn next(&mut self, mode: KeepAlive) -> f32 {
        let dither = (self.uniform() - self.uniform()) * self.dither;
        match mode {
            KeepAlive::Off => 0.0,
            KeepAlive::Dither => dither,
            KeepAlive::Tone => {
                self.phase += self.step;
                if self.phase > std::f32::consts::TAU {
                    self.phase -= std::f32::consts::TAU;
                }
                dither + self.phase.sin() * self.tone
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill<T: SizedSample + FromSample<f32>>(
    data: &mut [T],
    out_ch: usize,
    dev_rate: f64,
    shared: &Mutex<Shared>,
    playing: &AtomicBool,
    pos_bits: &AtomicU64,
    keep_alive: KeepAlive,
    idle: &mut Idle,
) {
    let silence = T::from_sample(0.0f32);
    if !playing.load(Ordering::Relaxed) {
        if keep_alive == KeepAlive::Off {
            data.fill(silence);
        } else {
            for frame in data.chunks_mut(out_ch.max(1)) {
                let v = T::from_sample(idle.next(keep_alive));
                frame.fill(v);
            }
        }
        return;
    }
    let mut sh = shared.lock().unwrap();
    let nch = sh.channels.max(1);
    let nframes = sh.samples.len() / nch;
    let step = sh.src_rate / dev_rate;
    let out_ch = out_ch.max(1);
    let mut frames = data.chunks_mut(out_ch);
    let mut stopped = false;
    for frame in &mut frames {
        let end = sh.end.min(nframes);
        if sh.pos as usize >= end {
            match sh.loop_range {
                Some((ls, le)) if le > ls && ls < nframes => sh.pos = ls as f64,
                _ => {
                    stopped = true;
                    frame.fill(silence);
                    break;
                }
            }
        }
        let i = (sh.pos as usize).min(nframes.saturating_sub(1));
        let i2 = (i + 1).min(nframes.saturating_sub(1));
        let frac = (sh.pos - i as f64) as f32;
        for (c, out) in frame.iter_mut().enumerate() {
            let sc = c % nch;
            let a = sh.samples[i * nch + sc];
            let b = sh.samples[i2 * nch + sc];
            *out = T::from_sample(a + (b - a) * frac);
        }
        sh.pos += step;
    }
    if stopped {
        for frame in frames {
            frame.fill(silence);
        }
        sh.pos = sh.end.min(nframes) as f64;
        playing.store(false, Ordering::SeqCst);
    }
    pos_bits.store(sh.pos.to_bits(), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_output_is_dither_not_silence() {
        let shared = Mutex::new(Shared { samples: Arc::new(Vec::new()), channels: 1, src_rate: 44100.0, pos: 0.0, end: 0, loop_range: None });
        let playing = AtomicBool::new(false);
        let pos = AtomicU64::new(0);
        let mut d = Idle::new(44100.0);
        let mut buf = vec![0f32; 4096];
        fill(&mut buf, 2, 44100.0, &shared, &playing, &pos, KeepAlive::Dither, &mut d);
        let peak = buf.iter().fold(0f32, |m, v| m.max(v.abs()));
        let mean = buf.iter().sum::<f32>() / buf.len() as f32;
        assert!(peak > 0.0 && peak <= KEEP_ALIVE_LEVEL, "peak {peak}");
        assert!(mean.abs() < KEEP_ALIVE_LEVEL / 10.0, "dc {mean}");
        assert!(buf.iter().filter(|v| **v != 0.0).count() > buf.len() / 2);
        // tone mode: -60 dBFS peak, 15 Hz (one cycle = 2940 frames at 44.1k), DC-free over whole cycles
        let mut big = vec![0f32; 2940 * 2 * 10];
        fill(&mut big, 2, 44100.0, &shared, &playing, &pos, KeepAlive::Tone, &mut d);
        let peak = big.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.0009 && peak < 0.0012, "tone peak {peak}");
        let mean = big.iter().sum::<f32>() / big.len() as f32;
        assert!(mean.abs() < 1e-5, "tone dc {mean}");
        fill(&mut buf, 2, 44100.0, &shared, &playing, &pos, KeepAlive::Off, &mut d);
        assert!(buf.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn playback_starts_at_full_level_from_the_first_sample() {
        // 1 kHz tone, play from frame 0: the first output frames must already carry it.
        let n = 4410;
        let samples: Vec<f32> = (0..n).map(|i| (i as f32 * std::f32::consts::TAU * 1000.0 / 44100.0).sin()).collect();
        let shared = Mutex::new(Shared { samples: Arc::new(samples.clone()), channels: 1, src_rate: 44100.0, pos: 0.0, end: n, loop_range: Some((0, n)) });
        let playing = AtomicBool::new(true);
        let pos = AtomicU64::new(0);
        let mut d = Idle::new(44100.0);
        let mut buf = vec![0f32; 512 * 2];
        fill(&mut buf, 2, 44100.0, &shared, &playing, &pos, KeepAlive::Tone, &mut d);
        for i in 0..512 {
            assert!((buf[i * 2] - samples[i]).abs() < 1e-5, "frame {i}");
            assert_eq!(buf[i * 2], buf[i * 2 + 1]);
        }
        // and it wraps the loop without a gap (this buffer starts at frame 512)
        let mut big = vec![0f32; n * 2 + 64];
        fill(&mut big, 2, 44100.0, &shared, &playing, &pos, KeepAlive::Tone, &mut d);
        for i in 0..32 {
            assert!((big[(n - 512 + i) * 2] - samples[i]).abs() < 1e-5, "wrap frame {i}");
        }
    }
}
