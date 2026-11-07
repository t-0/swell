//! Effects. Every effect works on a frame range of an `Audio`; some change
//! the range's length (the result reports the new end frame).

use crate::audio::Audio;
use anyhow::{bail, Result};
use std::f32::consts::TAU;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum FxKind {
    // amplitude / dynamics
    Gain,
    Normalize,
    FadeIn,
    FadeOut,
    Invert,
    Silence,
    RemoveDc,
    Compressor,
    Limiter,
    Gate,
    Tremolo,
    AutoPan,
    // filters / eq
    LowPass,
    HighPass,
    BandPass,
    Notch,
    LowShelf,
    HighShelf,
    Peaking,
    Telephone,
    // delay / reverb / modulation
    Delay,
    PingPong,
    Reverb,
    Chorus,
    Flanger,
    Phaser,
    Vibrato,
    RingMod,
    // distortion
    BitCrush,
    Overdrive,
    HardClip,
    // time / pitch / structure
    Reverse,
    ChangeSpeed,
    TimeStretch,
    PitchShift,
    Repeat,
    InsertSilence,
    TrimSilence,
    ResampleFile,
    // stereo
    SwapChannels,
    Mono,
    StereoWidth,
    Pan,
    // generators
    GenTone,
    GenNoise,
}

pub struct Param {
    pub name: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
}

const fn p(name: &'static str, default: f64, min: f64, max: f64) -> Param {
    Param { name, default, min, max }
}

pub struct FxInfo {
    pub kind: FxKind,
    pub name: &'static str,
    pub category: &'static str,
    pub params: &'static [Param],
    /// Inserts at a point: with no selection it acts at the cursor instead of the whole file.
    pub insert: bool,
    /// Always acts on the whole file.
    pub whole_file: bool,
}

const fn fx(kind: FxKind, name: &'static str, category: &'static str, params: &'static [Param]) -> FxInfo {
    FxInfo { kind, name, category, params, insert: false, whole_file: false }
}
const fn ins(kind: FxKind, name: &'static str, category: &'static str, params: &'static [Param]) -> FxInfo {
    FxInfo { kind, name, category, params, insert: true, whole_file: false }
}
const fn whole(kind: FxKind, name: &'static str, category: &'static str, params: &'static [Param]) -> FxInfo {
    FxInfo { kind, name, category, params, insert: false, whole_file: true }
}

pub const ALL: &[FxInfo] = &[
    fx(FxKind::Gain, "Gain", "amplitude", &[p("gain dB", 0.0, -60.0, 60.0)]),
    fx(FxKind::Normalize, "Normalize", "amplitude", &[p("peak dB", 0.0, -60.0, 0.0)]),
    fx(FxKind::FadeIn, "Fade in", "amplitude", &[p("curve (1 linear, 2 slow start, 0.5 fast start)", 1.0, 0.1, 10.0)]),
    fx(FxKind::FadeOut, "Fade out", "amplitude", &[p("curve (1 linear, 2 slow end, 0.5 fast end)", 1.0, 0.1, 10.0)]),
    fx(FxKind::Invert, "Invert polarity", "amplitude", &[]),
    fx(FxKind::Silence, "Silence (zero out)", "amplitude", &[]),
    fx(FxKind::RemoveDc, "Remove DC offset", "amplitude", &[]),
    fx(FxKind::Compressor, "Compressor", "dynamics", &[
        p("threshold dB", -20.0, -60.0, 0.0), p("ratio", 4.0, 1.0, 100.0),
        p("attack ms", 10.0, 0.1, 500.0), p("release ms", 100.0, 1.0, 5000.0), p("makeup dB", 0.0, -20.0, 40.0),
    ]),
    fx(FxKind::Limiter, "Limiter", "dynamics", &[p("ceiling dB", -1.0, -30.0, 0.0), p("release ms", 50.0, 1.0, 2000.0)]),
    fx(FxKind::Gate, "Noise gate", "dynamics", &[p("threshold dB", -40.0, -90.0, 0.0), p("attack ms", 1.0, 0.1, 500.0), p("release ms", 100.0, 1.0, 5000.0)]),
    fx(FxKind::Tremolo, "Tremolo", "modulation", &[p("rate Hz", 5.0, 0.01, 50.0), p("depth 0-1", 0.6, 0.0, 1.0)]),
    fx(FxKind::AutoPan, "Auto-pan (stereo)", "modulation", &[p("rate Hz", 0.5, 0.01, 20.0), p("depth 0-1", 1.0, 0.0, 1.0)]),
    fx(FxKind::LowPass, "Low-pass filter", "filter", &[p("cutoff Hz", 2000.0, 10.0, 22000.0), p("Q", 0.707, 0.1, 20.0)]),
    fx(FxKind::HighPass, "High-pass filter", "filter", &[p("cutoff Hz", 200.0, 10.0, 22000.0), p("Q", 0.707, 0.1, 20.0)]),
    fx(FxKind::BandPass, "Band-pass filter", "filter", &[p("center Hz", 1000.0, 10.0, 22000.0), p("Q", 1.0, 0.1, 50.0)]),
    fx(FxKind::Notch, "Notch filter", "filter", &[p("center Hz", 60.0, 10.0, 22000.0), p("Q", 10.0, 0.1, 100.0)]),
    fx(FxKind::LowShelf, "Low shelf EQ", "filter", &[p("freq Hz", 200.0, 10.0, 22000.0), p("gain dB", 3.0, -30.0, 30.0)]),
    fx(FxKind::HighShelf, "High shelf EQ", "filter", &[p("freq Hz", 4000.0, 10.0, 22000.0), p("gain dB", 3.0, -30.0, 30.0)]),
    fx(FxKind::Peaking, "Peaking EQ", "filter", &[p("freq Hz", 1000.0, 10.0, 22000.0), p("gain dB", 3.0, -30.0, 30.0), p("Q", 1.0, 0.1, 50.0)]),
    fx(FxKind::Telephone, "Telephone (300-3400 Hz)", "filter", &[]),
    fx(FxKind::Delay, "Delay / echo", "delay", &[p("time ms", 300.0, 1.0, 5000.0), p("feedback 0-1", 0.4, 0.0, 0.95), p("mix 0-1", 0.4, 0.0, 1.0)]),
    fx(FxKind::PingPong, "Ping-pong delay (stereo)", "delay", &[p("time ms", 300.0, 1.0, 5000.0), p("feedback 0-1", 0.5, 0.0, 0.95), p("mix 0-1", 0.4, 0.0, 1.0)]),
    fx(FxKind::Reverb, "Reverb", "delay", &[p("room 0-1", 0.7, 0.0, 1.0), p("damping 0-1", 0.5, 0.0, 1.0), p("mix 0-1", 0.3, 0.0, 1.0), p("tail s", 2.0, 0.0, 20.0)]),
    fx(FxKind::Chorus, "Chorus", "modulation", &[p("rate Hz", 0.8, 0.01, 10.0), p("depth ms", 4.0, 0.1, 20.0), p("mix 0-1", 0.5, 0.0, 1.0)]),
    fx(FxKind::Flanger, "Flanger", "modulation", &[p("rate Hz", 0.3, 0.01, 10.0), p("depth ms", 2.0, 0.1, 10.0), p("feedback 0-1", 0.5, 0.0, 0.95), p("mix 0-1", 0.7, 0.0, 1.0)]),
    fx(FxKind::Phaser, "Phaser", "modulation", &[p("rate Hz", 0.5, 0.01, 10.0), p("stages", 4.0, 2.0, 12.0), p("feedback 0-1", 0.3, 0.0, 0.95), p("mix 0-1", 0.5, 0.0, 1.0)]),
    fx(FxKind::Vibrato, "Vibrato", "modulation", &[p("rate Hz", 5.0, 0.01, 20.0), p("depth ms", 1.0, 0.05, 10.0)]),
    fx(FxKind::RingMod, "Ring modulator", "modulation", &[p("freq Hz", 440.0, 0.1, 20000.0), p("mix 0-1", 1.0, 0.0, 1.0)]),
    fx(FxKind::BitCrush, "Bit crusher", "distortion", &[p("bits", 8.0, 1.0, 16.0), p("downsample factor", 1.0, 1.0, 64.0)]),
    fx(FxKind::Overdrive, "Overdrive (soft clip)", "distortion", &[p("drive 1-50", 5.0, 1.0, 50.0), p("output dB", 0.0, -30.0, 12.0)]),
    fx(FxKind::HardClip, "Hard clip", "distortion", &[p("threshold dB", -6.0, -40.0, 0.0), p("output dB", 0.0, -30.0, 12.0)]),
    fx(FxKind::Reverse, "Reverse", "time", &[]),
    fx(FxKind::ChangeSpeed, "Change speed (pitch follows, like the slider)", "time", &[p("speed %", 100.0, 10.0, 400.0)]),
    fx(FxKind::TimeStretch, "Time stretch (keep pitch)", "time", &[p("length %", 100.0, 25.0, 400.0)]),
    fx(FxKind::PitchShift, "Pitch shift (keep length)", "time", &[p("semitones", 0.0, -24.0, 24.0)]),
    fx(FxKind::Repeat, "Repeat selection", "time", &[p("extra copies", 1.0, 1.0, 100.0)]),
    ins(FxKind::InsertSilence, "Insert silence", "time", &[p("seconds", 1.0, 0.001, 600.0)]),
    fx(FxKind::TrimSilence, "Trim silence from ends", "time", &[p("threshold dB", -50.0, -100.0, 0.0)]),
    whole(FxKind::ResampleFile, "Resample file (change sample rate)", "time", &[p("sample rate Hz", 44100.0, 4000.0, 192000.0)]),
    fx(FxKind::SwapChannels, "Swap channels (stereo)", "stereo", &[]),
    fx(FxKind::Mono, "Mix to mono", "stereo", &[]),
    fx(FxKind::StereoWidth, "Stereo width (stereo)", "stereo", &[p("width 0-2", 1.5, 0.0, 2.0)]),
    fx(FxKind::Pan, "Pan (stereo)", "stereo", &[p("pan -1 left .. 1 right", 0.0, -1.0, 1.0)]),
    ins(FxKind::GenTone, "Generate tone", "generate", &[p("freq Hz", 440.0, 1.0, 20000.0), p("seconds", 1.0, 0.001, 600.0), p("level dB", -6.0, -60.0, 0.0)]),
    ins(FxKind::GenNoise, "Generate white noise", "generate", &[p("seconds", 1.0, 0.001, 600.0), p("level dB", -12.0, -60.0, 0.0)]),
];

pub fn info(kind: FxKind) -> &'static FxInfo {
    ALL.iter().find(|i| i.kind == kind).expect("every FxKind is listed in ALL")
}

pub struct FxOut {
    pub note: String,
    pub new_end: usize,
}

fn ok(new_end: usize) -> Result<FxOut> {
    Ok(FxOut { note: String::new(), new_end })
}

fn db(x: f64) -> f32 {
    10f64.powf(x / 20.0) as f32
}

fn to_db(x: f32) -> f32 {
    20.0 * (x.max(1e-9)).log10()
}

/// One-pole smoothing coefficient for a time constant in ms.
fn coef(ms: f64, fs: f64) -> f32 {
    if ms <= 0.0 {
        1.0
    } else {
        (1.0 - (-1.0 / (ms * 0.001 * fs)).exp()) as f32
    }
}

fn need_stereo(a: &Audio) -> Result<()> {
    if a.n_channels() < 2 {
        bail!("needs a stereo file");
    }
    Ok(())
}

/// Apply `kind` to frames `[s, e)` with parameters `prm` (already clamped).
pub fn apply(kind: FxKind, a: &mut Audio, s: usize, e: usize, prm: &[f64]) -> Result<FxOut> {
    let fs = a.sample_rate as f64;
    let nc = a.n_channels();
    let len = e - s;
    let pr = |i: usize| prm.get(i).copied().unwrap_or(info(kind).params[i].default);
    let generator = matches!(kind, FxKind::GenTone | FxKind::GenNoise | FxKind::InsertSilence);
    if len == 0 && !generator && !info(kind).whole_file {
        bail!("empty range");
    }

    match kind {
        FxKind::Gain => {
            let g = db(pr(0));
            for c in &mut a.channels {
                for v in &mut c[s..e] {
                    *v *= g;
                }
            }
            ok(e)
        }
        FxKind::Normalize => {
            let g = a.normalize(s, e, db(pr(0)));
            Ok(FxOut { note: format!(" (gain {:+.2} dB)", to_db(g)), new_end: e })
        }
        FxKind::FadeIn | FxKind::FadeOut => {
            let curve = pr(0) as f32;
            let fade_in = kind == FxKind::FadeIn;
            for c in &mut a.channels {
                for (k, v) in c[s..e].iter_mut().enumerate() {
                    let t = k as f32 / len as f32;
                    let t = if fade_in { t } else { 1.0 - t };
                    *v *= t.powf(curve);
                }
            }
            ok(e)
        }
        FxKind::Invert => {
            for c in &mut a.channels {
                for v in &mut c[s..e] {
                    *v = -*v;
                }
            }
            ok(e)
        }
        FxKind::Silence => {
            for c in &mut a.channels {
                c[s..e].fill(0.0);
            }
            ok(e)
        }
        FxKind::RemoveDc => {
            let mut worst = 0f32;
            for c in &mut a.channels {
                let mean = c[s..e].iter().sum::<f32>() / len as f32;
                worst = worst.max(mean.abs());
                for v in &mut c[s..e] {
                    *v -= mean;
                }
            }
            Ok(FxOut { note: format!(" (offset {:.4})", worst), new_end: e })
        }
        FxKind::Compressor => {
            compress(a, s, e, pr(0), pr(1), pr(2), pr(3), pr(4));
            ok(e)
        }
        FxKind::Limiter => {
            compress(a, s, e, pr(0), 100.0, 0.3, pr(1), 0.0);
            ok(e)
        }
        FxKind::Gate => {
            let thr = db(pr(0));
            let att = coef(pr(1), fs);
            let rel = coef(pr(2), fs);
            let mut gain = 1f32;
            for i in s..e {
                let lvl = a.channels.iter().map(|c| c[i].abs()).fold(0f32, f32::max);
                let target = if lvl > thr { 1.0 } else { 0.0 };
                gain += (target - gain) * if target > gain { att } else { rel };
                for c in &mut a.channels {
                    c[i] *= gain;
                }
            }
            ok(e)
        }
        FxKind::Tremolo => {
            let (rate, depth) = (pr(0) as f32, pr(1) as f32);
            for c in &mut a.channels {
                for (k, v) in c[s..e].iter_mut().enumerate() {
                    let lfo = 0.5 - 0.5 * (TAU * rate * k as f32 / fs as f32).cos();
                    *v *= 1.0 - depth * lfo;
                }
            }
            ok(e)
        }
        FxKind::AutoPan => {
            need_stereo(a)?;
            let (rate, depth) = (pr(0) as f32, pr(1) as f32);
            for i in s..e {
                let pan = depth * (TAU * rate * (i - s) as f32 / fs as f32).sin(); // -1..1
                let ang = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
                a.channels[0][i] *= ang.cos() * std::f32::consts::SQRT_2;
                a.channels[1][i] *= ang.sin() * std::f32::consts::SQRT_2;
            }
            ok(e)
        }
        FxKind::LowPass => filter(a, s, e, |fs| Biquad::lowpass(fs, pr(0), pr(1))),
        FxKind::HighPass => filter(a, s, e, |fs| Biquad::highpass(fs, pr(0), pr(1))),
        FxKind::BandPass => filter(a, s, e, |fs| Biquad::bandpass(fs, pr(0), pr(1))),
        FxKind::Notch => filter(a, s, e, |fs| Biquad::notch(fs, pr(0), pr(1))),
        FxKind::LowShelf => filter(a, s, e, |fs| Biquad::lowshelf(fs, pr(0), pr(1))),
        FxKind::HighShelf => filter(a, s, e, |fs| Biquad::highshelf(fs, pr(0), pr(1))),
        FxKind::Peaking => filter(a, s, e, |fs| Biquad::peaking(fs, pr(0), pr(1), pr(2))),
        FxKind::Telephone => {
            filter(a, s, e, |fs| Biquad::highpass(fs, 300.0, 0.707))?;
            filter(a, s, e, |fs| Biquad::highpass(fs, 300.0, 0.707))?;
            filter(a, s, e, |fs| Biquad::lowpass(fs, 3400.0, 0.707))?;
            filter(a, s, e, |fs| Biquad::lowpass(fs, 3400.0, 0.707))
        }
        FxKind::Delay => {
            delay(a, s, e, pr(0), pr(1), pr(2), false);
            ok(e)
        }
        FxKind::PingPong => {
            need_stereo(a)?;
            delay(a, s, e, pr(0), pr(1), pr(2), true);
            ok(e)
        }
        FxKind::Reverb => {
            reverb(a, s, e, pr(0), pr(1), pr(2), pr(3));
            ok(e)
        }
        FxKind::Chorus => {
            mod_delay(a, s, e, 20.0, pr(1), pr(0), 0.0, pr(2), false);
            ok(e)
        }
        FxKind::Flanger => {
            mod_delay(a, s, e, 1.0, pr(1), pr(0), pr(2), pr(3), false);
            ok(e)
        }
        FxKind::Vibrato => {
            mod_delay(a, s, e, pr(1), pr(1), pr(0), 0.0, 1.0, true);
            ok(e)
        }
        FxKind::Phaser => {
            phaser(a, s, e, pr(0), pr(1) as usize, pr(2), pr(3));
            ok(e)
        }
        FxKind::RingMod => {
            let (f, mix) = (pr(0) as f32, pr(1) as f32);
            for c in &mut a.channels {
                for (k, v) in c[s..e].iter_mut().enumerate() {
                    let m = (TAU * f * k as f32 / fs as f32).sin();
                    *v = *v * (1.0 - mix) + *v * m * mix;
                }
            }
            ok(e)
        }
        FxKind::BitCrush => {
            let q = 2f32.powf(pr(0) as f32 - 1.0);
            let ds = (pr(1) as usize).max(1);
            for c in &mut a.channels {
                let mut held = 0f32;
                for (k, v) in c[s..e].iter_mut().enumerate() {
                    if k % ds == 0 {
                        held = (*v * q).round() / q;
                    }
                    *v = held;
                }
            }
            ok(e)
        }
        FxKind::Overdrive => {
            let drive = pr(0) as f32;
            let out = db(pr(1)) / drive.tanh();
            for c in &mut a.channels {
                for v in &mut c[s..e] {
                    *v = (*v * drive).tanh() * out;
                }
            }
            ok(e)
        }
        FxKind::HardClip => {
            let t = db(pr(0));
            let out = db(pr(1));
            for c in &mut a.channels {
                for v in &mut c[s..e] {
                    *v = v.clamp(-t, t) * out;
                }
            }
            ok(e)
        }
        FxKind::Reverse => {
            a.reverse(s, e);
            ok(e)
        }
        FxKind::ChangeSpeed => {
            let step = pr(0) / 100.0;
            let new: Vec<Vec<f32>> = a.channels.iter().map(|c| resample(&c[s..e], step)).collect();
            replace_range(a, s, e, &new);
            ok(s + new[0].len())
        }
        FxKind::TimeStretch => {
            let factor = pr(0) / 100.0;
            let input: Vec<Vec<f32>> = a.channels.iter().map(|c| c[s..e].to_vec()).collect();
            let new = wsola(&input, a.sample_rate, factor);
            replace_range(a, s, e, &new);
            ok(s + new[0].len())
        }
        FxKind::PitchShift => {
            let r = 2f64.powf(pr(0) / 12.0);
            let input: Vec<Vec<f32>> = a.channels.iter().map(|c| c[s..e].to_vec()).collect();
            let stretched = wsola(&input, a.sample_rate, r);
            let new: Vec<Vec<f32>> = stretched.iter().map(|c| resample_to(c, len)).collect();
            replace_range(a, s, e, &new);
            ok(s + new[0].len())
        }
        FxKind::Repeat => {
            let copies = pr(0) as usize;
            let clip = a.slice(s, e);
            for k in 0..copies {
                a.insert(e + k * len, &clip);
            }
            ok(e + copies * len)
        }
        FxKind::InsertSilence => {
            let n = (pr(0) * fs).round() as usize;
            let mut sil = Audio::new(a.sample_rate, nc);
            for c in &mut sil.channels {
                c.resize(n, 0.0);
            }
            a.insert(s, &sil);
            ok(s + n)
        }
        FxKind::TrimSilence => {
            let thr = db(pr(0));
            let loud = |i: usize| a.channels.iter().any(|c| c[i].abs() > thr);
            let first = (s..e).find(|&i| loud(i));
            let Some(first) = first else { bail!("range is entirely below the threshold") };
            let last = (s..e).rev().find(|&i| loud(i)).unwrap();
            a.delete(last + 1, e);
            a.delete(s, first);
            Ok(FxOut { note: format!(" (removed {} frames)", len - (last + 1 - first)), new_end: s + (last + 1 - first) })
        }
        FxKind::ResampleFile => {
            let new_rate = pr(0).round() as u32;
            if new_rate == a.sample_rate {
                bail!("already at {} Hz", new_rate);
            }
            let step = fs / new_rate as f64;
            for c in &mut a.channels {
                *c = resample(c, step);
            }
            a.sample_rate = new_rate;
            Ok(FxOut { note: format!(" to {} Hz", new_rate), new_end: a.frames() })
        }
        FxKind::SwapChannels => {
            need_stereo(a)?;
            let (l, r) = a.channels.split_at_mut(1);
            l[0][s..e].swap_with_slice(&mut r[0][s..e]);
            ok(e)
        }
        FxKind::Mono => {
            if nc < 2 {
                bail!("already mono");
            }
            for i in s..e {
                let m = a.channels.iter().map(|c| c[i]).sum::<f32>() / nc as f32;
                for c in &mut a.channels {
                    c[i] = m;
                }
            }
            ok(e)
        }
        FxKind::StereoWidth => {
            need_stereo(a)?;
            let w = pr(0) as f32;
            for i in s..e {
                let (l, r) = (a.channels[0][i], a.channels[1][i]);
                let m = (l + r) * 0.5;
                let sd = (l - r) * 0.5 * w;
                a.channels[0][i] = m + sd;
                a.channels[1][i] = m - sd;
            }
            ok(e)
        }
        FxKind::Pan => {
            need_stereo(a)?;
            let ang = (pr(0) as f32 + 1.0) * std::f32::consts::FRAC_PI_4;
            let (gl, gr) = (ang.cos() * std::f32::consts::SQRT_2, ang.sin() * std::f32::consts::SQRT_2);
            for v in &mut a.channels[0][s..e] {
                *v *= gl;
            }
            for v in &mut a.channels[1][s..e] {
                *v *= gr;
            }
            ok(e)
        }
        FxKind::GenTone => {
            let (f, secs, amp) = (pr(0) as f32, pr(1), db(pr(2)));
            let n = (secs * fs).round() as usize;
            let mut g = Audio::new(a.sample_rate, nc);
            for c in &mut g.channels {
                *c = (0..n).map(|i| amp * (TAU * f * i as f32 / fs as f32).sin()).collect();
            }
            a.insert(s, &g);
            ok(s + n)
        }
        FxKind::GenNoise => {
            let (secs, amp) = (pr(0), db(pr(1)));
            let n = (secs * fs).round() as usize;
            let mut g = Audio::new(a.sample_rate, nc);
            let mut seed: u32 = 0x9E37_79B9 ^ (n as u32);
            for c in &mut g.channels {
                *c = (0..n)
                    .map(|_| {
                        // xorshift32
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        amp * ((seed as f32 / u32::MAX as f32) * 2.0 - 1.0)
                    })
                    .collect();
            }
            a.insert(s, &g);
            ok(s + n)
        }
    }
}

// ----- helpers --------------------------------------------------------------

fn replace_range(a: &mut Audio, s: usize, e: usize, new: &[Vec<f32>]) {
    for (c, n) in a.channels.iter_mut().zip(new) {
        c.splice(s..e, n.iter().copied());
    }
}

/// 4-point cubic Hermite interpolation at fractional position `pos`.
fn interp(v: &[f32], pos: f64) -> f32 {
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    let i = pos.floor() as isize;
    let t = (pos - i as f64) as f32;
    let at = |k: isize| v[k.clamp(0, n as isize - 1) as usize];
    let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (p2 - p0);
    let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
    ((c3 * t + c2) * t + c1) * t + p1
}

/// Resample by reading `step` input frames per output frame.
fn resample(v: &[f32], step: f64) -> Vec<f32> {
    let n_out = ((v.len() as f64) / step).round().max(1.0) as usize;
    (0..n_out).map(|i| interp(v, i as f64 * step)).collect()
}

fn resample_to(v: &[f32], n_out: usize) -> Vec<f32> {
    let step = v.len() as f64 / n_out.max(1) as f64;
    (0..n_out).map(|i| interp(v, i as f64 * step)).collect()
}

/// RBJ cookbook biquad, transposed direct form II.
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn from_raw(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Biquad {
        Biquad {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
            z1: 0.0,
            z2: 0.0,
        }
    }
    fn w(fs: f64, f0: f64) -> (f64, f64) {
        let f0 = f0.clamp(1.0, fs * 0.49);
        let w0 = TAU as f64 * f0 / fs;
        (w0.cos(), w0.sin())
    }
    fn lowpass(fs: f64, f0: f64, q: f64) -> Biquad {
        let (c, sn) = Self::w(fs, f0);
        let al = sn / (2.0 * q);
        Self::from_raw((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }
    fn highpass(fs: f64, f0: f64, q: f64) -> Biquad {
        let (c, sn) = Self::w(fs, f0);
        let al = sn / (2.0 * q);
        Self::from_raw((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }
    fn bandpass(fs: f64, f0: f64, q: f64) -> Biquad {
        let (c, sn) = Self::w(fs, f0);
        let al = sn / (2.0 * q);
        Self::from_raw(al, 0.0, -al, 1.0 + al, -2.0 * c, 1.0 - al)
    }
    fn notch(fs: f64, f0: f64, q: f64) -> Biquad {
        let (c, sn) = Self::w(fs, f0);
        let al = sn / (2.0 * q);
        Self::from_raw(1.0, -2.0 * c, 1.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }
    fn peaking(fs: f64, f0: f64, gain_db: f64, q: f64) -> Biquad {
        let (c, sn) = Self::w(fs, f0);
        let a = 10f64.powf(gain_db / 40.0);
        let al = sn / (2.0 * q);
        Self::from_raw(1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a)
    }
    fn lowshelf(fs: f64, f0: f64, gain_db: f64) -> Biquad {
        let (c, sn) = Self::w(fs, f0);
        let a = 10f64.powf(gain_db / 40.0);
        let al = sn / 2.0 * std::f64::consts::SQRT_2; // shelf slope S = 1
        let beta = 2.0 * a.sqrt() * al;
        Self::from_raw(
            a * ((a + 1.0) - (a - 1.0) * c + beta),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - beta),
            (a + 1.0) + (a - 1.0) * c + beta,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - beta,
        )
    }
    fn highshelf(fs: f64, f0: f64, gain_db: f64) -> Biquad {
        let (c, sn) = Self::w(fs, f0);
        let a = 10f64.powf(gain_db / 40.0);
        let al = sn / 2.0 * std::f64::consts::SQRT_2; // shelf slope S = 1
        let beta = 2.0 * a.sqrt() * al;
        Self::from_raw(
            a * ((a + 1.0) + (a - 1.0) * c + beta),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - beta),
            (a + 1.0) - (a - 1.0) * c + beta,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - beta,
        )
    }
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

fn filter(a: &mut Audio, s: usize, e: usize, make: impl Fn(f64) -> Biquad) -> Result<FxOut> {
    let fs = a.sample_rate as f64;
    for c in &mut a.channels {
        let mut bq = make(fs);
        for v in &mut c[s..e] {
            *v = bq.run(*v);
        }
    }
    ok(e)
}

#[allow(clippy::too_many_arguments)]
fn compress(a: &mut Audio, s: usize, e: usize, thr_db: f64, ratio: f64, att_ms: f64, rel_ms: f64, makeup_db: f64) {
    let fs = a.sample_rate as f64;
    let att = coef(att_ms, fs);
    let rel = coef(rel_ms, fs);
    let makeup = db(makeup_db);
    let slope = (1.0 - 1.0 / ratio) as f32;
    let thr = thr_db as f32;
    let mut env = 0f32;
    for i in s..e {
        let lvl = a.channels.iter().map(|c| c[i].abs()).fold(0f32, f32::max);
        env += (lvl - env) * if lvl > env { att } else { rel };
        let over = to_db(env) - thr;
        let g = if over > 0.0 { db((-over * slope) as f64) } else { 1.0 } * makeup;
        for c in &mut a.channels {
            c[i] *= g;
        }
    }
}

/// Feedback delay. The echo tail keeps ringing past `e` into existing audio
/// (mixed in, never extending the file) until it has decayed.
fn delay(a: &mut Audio, s: usize, e: usize, time_ms: f64, fb: f64, mix: f64, pingpong: bool) {
    let fs = a.sample_rate as f64;
    let n = a.frames();
    let nc = a.n_channels();
    let d = ((time_ms * 0.001 * fs).round() as usize).max(1);
    let repeats = if fb <= 0.0 { 1.0 } else { (0.001f64.ln() / fb.ln()).ceil().clamp(1.0, 60.0) };
    let end = (e + (repeats as usize) * d).min(n);
    let (fb, mix) = (fb as f32, mix as f32);
    let mut bufs = vec![vec![0f32; d]; nc];
    let mut idx = 0usize;
    let mut taps = vec![0f32; nc];
    for i in s..end {
        for ch in 0..nc {
            taps[ch] = bufs[ch][idx];
        }
        for ch in 0..nc {
            let x = if i < e { a.channels[ch][i] } else { 0.0 };
            let wet = taps[ch];
            let write_to = if pingpong { (ch + 1) % nc } else { ch };
            bufs[write_to][idx] = x + fb * wet;
            a.channels[ch][i] += mix * wet;
        }
        idx = (idx + 1) % d;
    }
}

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
    feedback: f32,
    damp: f32,
}
impl Comb {
    fn run(&mut self, x: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - self.damp) + self.store * self.damp;
        self.buf[self.idx] = x + self.store * self.feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}
struct AllPass {
    buf: Vec<f32>,
    idx: usize,
}
impl AllPass {
    fn run(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        let out = -x + b;
        self.buf[self.idx] = x + b * 0.5;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

/// Freeverb-style reverb (8 parallel combs into 4 series allpasses per channel).
fn reverb(a: &mut Audio, s: usize, e: usize, room: f64, damping: f64, mix: f64, tail_s: f64) {
    const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLP: [usize; 4] = [556, 441, 341, 225];
    let fs = a.sample_rate as f64;
    let scale = fs / 44100.0;
    let n = a.frames();
    let end = (e + (tail_s * fs) as usize).min(n);
    let feedback = (0.7 + 0.28 * room) as f32;
    let damp = (0.4 * damping) as f32;
    let wet = (mix * 3.0) as f32;
    for (ch, c) in a.channels.iter_mut().enumerate() {
        let spread = 23 * ch;
        let mut combs: Vec<Comb> = COMBS
            .iter()
            .map(|&l| Comb { buf: vec![0.0; ((l + spread) as f64 * scale) as usize], idx: 0, store: 0.0, feedback, damp })
            .collect();
        let mut aps: Vec<AllPass> = ALLP
            .iter()
            .map(|&l| AllPass { buf: vec![0.0; ((l + spread) as f64 * scale) as usize], idx: 0 })
            .collect();
        for i in s..end {
            let x = if i < e { c[i] } else { 0.0 } * 0.015;
            let mut out: f32 = combs.iter_mut().map(|cb| cb.run(x)).sum();
            for ap in &mut aps {
                out = ap.run(out);
            }
            c[i] += out * wet;
        }
    }
}

/// LFO-modulated delay line: chorus, flanger, vibrato.
#[allow(clippy::too_many_arguments)]
fn mod_delay(a: &mut Audio, s: usize, e: usize, base_ms: f64, depth_ms: f64, rate: f64, fb: f64, mix: f64, wet_only: bool) {
    let fs = a.sample_rate as f64;
    let max_d = ((base_ms + depth_ms) * 0.001 * fs) as usize + 4;
    let (fb, mix) = (fb as f32, mix as f32);
    for c in &mut a.channels {
        let mut buf = vec![0f32; max_d];
        let mut w = 0usize;
        for (k, v) in c[s..e].iter_mut().enumerate() {
            let lfo = 0.5 + 0.5 * (TAU as f64 * rate * k as f64 / fs).sin();
            let dly = (base_ms + depth_ms * lfo) * 0.001 * fs;
            let rp = (w as f64 + max_d as f64 - dly) % max_d as f64;
            let i0 = rp.floor() as usize % max_d;
            let i1 = (i0 + 1) % max_d;
            let t = (rp - rp.floor()) as f32;
            let d = buf[i0] * (1.0 - t) + buf[i1] * t;
            let x = *v;
            buf[w] = x + fb * d;
            w = (w + 1) % max_d;
            *v = if wet_only { d } else { x + mix * d };
        }
    }
}

fn phaser(a: &mut Audio, s: usize, e: usize, rate: f64, stages: usize, fb: f64, mix: f64) {
    let fs = a.sample_rate as f64;
    let stages = stages.clamp(2, 12);
    let (fb, mix) = (fb as f32, mix as f32);
    let (fmin, fmax) = (200.0f64, (2000.0f64).min(fs * 0.45));
    for c in &mut a.channels {
        let mut xz = vec![0f32; stages];
        let mut yz = vec![0f32; stages];
        let mut last = 0f32;
        for (k, v) in c[s..e].iter_mut().enumerate() {
            let lfo = 0.5 + 0.5 * (TAU as f64 * rate * k as f64 / fs).sin();
            let f = fmin * (fmax / fmin).powf(lfo);
            let tn = (std::f64::consts::PI * f / fs).tan();
            let g = ((tn - 1.0) / (tn + 1.0)) as f32;
            let x = *v;
            let mut y = x + fb * last;
            for st in 0..stages {
                let out = g * y + xz[st] - g * yz[st];
                xz[st] = y;
                yz[st] = out;
                y = out;
            }
            last = y;
            *v = x + mix * y;
        }
    }
}

/// WSOLA time stretch: output length = input length × `factor`, pitch kept.
fn wsola(input: &[Vec<f32>], fs: u32, factor: f64) -> Vec<Vec<f32>> {
    let nc = input.len();
    let n_in = input[0].len();
    if (factor - 1.0).abs() < 1e-6 || n_in < 64 {
        return input.to_vec();
    }
    let win = (((fs as f64 * 0.04) as usize).max(256)) & !1;
    let hop_out = win / 2;
    let hop_in = hop_out as f64 / factor;
    let tol = (win / 8).max(8) as isize;
    let cmp = win / 2;
    let n_out = (n_in as f64 * factor).round() as usize;
    let hann: Vec<f32> = (0..win).map(|i| 0.5 - 0.5 * (TAU * i as f32 / win as f32).cos()).collect();
    let guide: Vec<f32> = (0..n_in).map(|i| input.iter().map(|c| c[i]).sum::<f32>()).collect();
    let mut out = vec![vec![0f32; n_out + win]; nc];
    let mut norm = vec![0f32; n_out + win];
    let mut prev: Option<usize> = None;
    let mut k = 0usize;
    loop {
        let out_pos = k * hop_out;
        if out_pos >= n_out {
            break;
        }
        let nominal = (k as f64 * hop_in).round() as isize;
        let pos = match prev {
            None => nominal.max(0) as usize,
            Some(pp) => {
                let natural = pp + hop_out;
                let mut best = nominal;
                let mut best_score = f32::MIN;
                if natural + cmp <= n_in {
                    for d in -tol..=tol {
                        let c = nominal + d;
                        if c < 0 || c as usize + cmp > n_in {
                            continue;
                        }
                        let c = c as usize;
                        let sc: f32 = guide[c..c + cmp].iter().zip(&guide[natural..natural + cmp]).map(|(x, y)| x * y).sum();
                        if sc > best_score {
                            best_score = sc;
                            best = c as isize;
                        }
                    }
                }
                best.clamp(0, n_in as isize - 1) as usize
            }
        };
        for ch in 0..nc {
            for j in 0..win {
                if pos + j < n_in {
                    out[ch][out_pos + j] += input[ch][pos + j] * hann[j];
                }
            }
        }
        for j in 0..win {
            norm[out_pos + j] += hann[j];
        }
        prev = Some(pos);
        k += 1;
    }
    for c in &mut out {
        for (v, nm) in c.iter_mut().zip(&norm) {
            if *nm > 1e-3 {
                *v /= *nm;
            }
        }
        c.truncate(n_out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, nc: usize, f: f32) -> Audio {
        let mut a = Audio::new(8000, nc);
        for c in 0..nc {
            a.channels[c] = (0..frames).map(|i| 0.5 * (TAU * f * i as f32 / 8000.0).sin()).collect();
        }
        a
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
    }

    #[test]
    fn every_effect_runs_with_defaults() {
        for i in ALL {
            let mut a = tone(8000, 2, 220.0);
            let prm: Vec<f64> = i.params.iter().map(|p| p.default).collect();
            let (s, e) = if i.insert { (4000, 4000) } else { (1000, 7000) };
            let r = apply(i.kind, &mut a, s, e, &prm);
            match i.kind {
                FxKind::ResampleFile | FxKind::TrimSilence => {} // may legitimately bail (same rate / nothing to trim)
                _ => assert!(r.is_ok(), "{} failed: {:?}", i.name, r.err()),
            }
            assert!(a.channels.iter().all(|c| c.iter().all(|v| v.is_finite())), "{} produced NaN", i.name);
            assert_eq!(a.channels[0].len(), a.channels[1].len(), "{} unbalanced channels", i.name);
        }
    }

    #[test]
    fn lowpass_attenuates_highs() {
        let mut hi = tone(8000, 1, 3000.0);
        let before = rms(&hi.channels[0]);
        apply(FxKind::LowPass, &mut hi, 0, 8000, &[300.0, 0.707]).unwrap();
        assert!(rms(&hi.channels[0][2000..]) < before * 0.05);
        let mut lo = tone(8000, 1, 50.0);
        let before = rms(&lo.channels[0]);
        apply(FxKind::LowPass, &mut lo, 0, 8000, &[300.0, 0.707]).unwrap();
        assert!(rms(&lo.channels[0][2000..]) > before * 0.9);
    }

    #[test]
    fn stretch_and_pitch_lengths() {
        let mut a = tone(8000, 2, 220.0);
        let r = apply(FxKind::TimeStretch, &mut a, 0, 8000, &[150.0]).unwrap();
        assert_eq!(r.new_end, 12000);
        assert_eq!(a.frames(), 12000);
        let mut b = tone(8000, 1, 220.0);
        let r = apply(FxKind::PitchShift, &mut b, 1000, 5000, &[7.0]).unwrap();
        assert_eq!(r.new_end, 5000);
        assert_eq!(b.frames(), 8000);
        assert!(rms(&b.channels[0][1500..4500]) > 0.2);
        let mut c = tone(8000, 1, 220.0);
        let r = apply(FxKind::ChangeSpeed, &mut c, 0, 8000, &[200.0]).unwrap();
        assert_eq!(r.new_end, 4000);
    }

    #[test]
    fn generators_and_structure() {
        let mut a = tone(8000, 2, 220.0);
        let r = apply(FxKind::InsertSilence, &mut a, 4000, 4000, &[0.5]).unwrap();
        assert_eq!((r.new_end, a.frames()), (8000, 12000));
        assert!(a.channels[0][4000..8000].iter().all(|v| *v == 0.0));
        // range starts inside the silence: the silent head is removed, the loud tail kept
        let r = apply(FxKind::TrimSilence, &mut a, 4000, 8500, &[-50.0]).unwrap();
        // the sine crosses zero near the boundary, so a sample or two more may go
        assert!((7990..=8000).contains(&a.frames()), "{}", a.frames());
        assert_eq!(r.new_end, a.frames() - 3500);
        assert!(apply(FxKind::TrimSilence, &mut a, 0, 1, &[20.0]).is_err());
        let n0 = a.frames();
        let r = apply(FxKind::Repeat, &mut a, 0, 1000, &[2.0]).unwrap();
        assert_eq!((r.new_end, a.frames()), (3000, n0 + 2000));
        let r = apply(FxKind::GenTone, &mut a, 0, 0, &[440.0, 0.25, -6.0]).unwrap();
        assert_eq!((r.new_end, a.frames()), (2000, n0 + 4000));
        let r = apply(FxKind::ResampleFile, &mut a, 0, 0, &[16000.0]).unwrap();
        assert_eq!((r.new_end, a.sample_rate, a.frames()), ((n0 + 4000) * 2, 16000, (n0 + 4000) * 2));
    }

    #[test]
    fn compressor_reduces_peaks() {
        let mut a = tone(8000, 1, 220.0);
        apply(FxKind::Gain, &mut a, 0, 8000, &[6.0]).unwrap(); // peak ~1.0
        let before = a.peak(2000, 8000);
        apply(FxKind::Compressor, &mut a, 0, 8000, &[-20.0, 4.0, 1.0, 100.0, 0.0]).unwrap();
        assert!(a.peak(2000, 8000) < before * 0.6);
    }
}
