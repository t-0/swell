//! Beat / tempo detection and the constant-tempo beat grid.
//!
//! Detection: spectral-flux onset strength → autocorrelation tempo estimate
//! with a log-normal prior → dynamic-programming beat tracking (Ellis 2007)
//! → least-squares constant-tempo fit → downbeat phase from low-band energy.

use crate::audio::Audio;
use anyhow::{bail, Result};
use std::f32::consts::PI;

/// A constant-tempo grid: beat `k` is at `anchor + k * frames_per_beat`,
/// beat 0 is the downbeat of the first full bar ("bar 1 beat 1" of the music).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub bpm: f64,
    pub anchor: i64,
    pub beats_per_bar: u32,
}

impl Grid {
    pub fn fpb(&self, rate: u32) -> f64 {
        60.0 * rate as f64 / self.bpm
    }

    pub fn bar_frames(&self, rate: u32) -> f64 {
        self.fpb(rate) * self.beats_per_bar as f64
    }

    pub fn beat_frame(&self, k: i64, rate: u32) -> f64 {
        self.anchor as f64 + k as f64 * self.fpb(rate)
    }

    /// Fractional beat index at a frame.
    pub fn beat_index(&self, frame: f64, rate: u32) -> f64 {
        (frame - self.anchor as f64) / self.fpb(rate)
    }

    pub fn is_downbeat(&self, k: i64) -> bool {
        k.rem_euclid(self.beats_per_bar as i64) == 0
    }

    /// Bars before the anchor, counting a partial one. The file starts at bar 1.
    pub fn lead_in_bars(&self, rate: u32) -> i64 {
        (self.anchor as f64 / self.bar_frames(rate)).ceil().max(0.0) as i64
    }

    /// Bar number as a DAW would show it with the file placed at bar 1.
    pub fn bar_number(&self, k: i64, rate: u32) -> i64 {
        k.div_euclid(self.beats_per_bar as i64) + 1 + self.lead_in_bars(rate)
    }

    /// Nearest beat frame to `frame`, clamped to `[0, n]`.
    pub fn snap(&self, frame: usize, rate: u32, n: usize) -> usize {
        let k = self.beat_index(frame as f64, rate).round() as i64;
        (self.beat_frame(k, rate).round().max(0.0) as usize).min(n)
    }

    /// Move from `frame` by `steps` beats (or bars when `bars`), landing on grid.
    pub fn step(&self, frame: usize, steps: i64, bars: bool, rate: u32, n: usize) -> usize {
        let unit = if bars { self.beats_per_bar as i64 } else { 1 };
        let idx = self.beat_index(frame as f64, rate) / unit as f64;
        let eps = 1e-4;
        let base = if steps > 0 { (idx + eps).floor() } else { (idx - eps).ceil() } as i64;
        let k = (base + steps) * unit;
        (self.beat_frame(k, rate).round().max(0.0) as usize).min(n)
    }

    /// Beats `(k, frame)` with `start <= frame < end`.
    pub fn beats_in(&self, start: usize, end: usize, rate: u32) -> Vec<(i64, f64)> {
        let fpb = self.fpb(rate);
        if fpb <= 0.0 || end <= start {
            return Vec::new();
        }
        let k0 = self.beat_index(start as f64, rate).ceil() as i64;
        let k1 = self.beat_index(end as f64, rate).ceil() as i64;
        (k0..k1).map(|k| (k, self.anchor as f64 + k as f64 * fpb)).collect()
    }

    /// Downbeats in `[start, end)`, keeping only every `every`-th bar.
    pub fn bars_in(&self, start: usize, end: usize, rate: u32, every: i64) -> Vec<(i64, f64)> {
        let every = every.max(1);
        self.beats_in(start, end, rate)
            .into_iter()
            .filter(|(k, _)| self.is_downbeat(*k) && (k / self.beats_per_bar as i64).rem_euclid(every) == 0)
            .collect()
    }
}

pub struct Detection {
    pub grid: Grid,
    /// Mean absolute deviation of tracked beats from the fitted grid, in ms.
    pub fit_ms: f64,
    pub n_beats: usize,
    /// How the user's bar marks were used, for the status message.
    pub note: String,
}

/// A bar marked by the user: its start frame, and optionally its end frame
/// (which fixes the tempo prior).
#[derive(Clone, Copy, Debug)]
pub struct BarHint {
    pub start: usize,
    pub end: Option<usize>,
}

/// In-place iterative radix-2 FFT. `re.len()` must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

pub fn detect(a: &Audio, beats_per_bar: u32, hint: Option<BarHint>) -> Result<Detection> {
    let rate = a.sample_rate;
    let fs = rate as f64;
    let n = a.frames();
    if n < (fs * 2.0) as usize {
        bail!("need at least 2 seconds of audio");
    }
    let nc = a.n_channels() as f32;
    let mono: Vec<f32> = (0..n).map(|i| a.channels.iter().map(|c| c[i]).sum::<f32>() / nc).collect();

    // ---- onset strength (spectral flux) ----
    let win = ((fs * 0.023).round() as usize).next_power_of_two();
    let hop = win / 2;
    let osr = fs / hop as f64; // onset-function sample rate
    let n_frames = (n - win) / hop + 1;
    let hann: Vec<f32> = (0..win).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / win as f32).cos()).collect();
    // Kick-drum band: below the snare/tom body so downbeats stand out.
    let low_bins = ((120.0 / fs) * win as f64).ceil().max(1.0) as usize;
    let mut re = vec![0f32; win];
    let mut im = vec![0f32; win];
    let mut prev = vec![0f32; win / 2 + 1];
    let mut osf = Vec::with_capacity(n_frames);
    let mut low = Vec::with_capacity(n_frames);
    for t in 0..n_frames {
        let off = t * hop;
        for j in 0..win {
            re[j] = mono[off + j] * hann[j];
            im[j] = 0.0;
        }
        fft(&mut re, &mut im);
        let (mut flux, mut lowe) = (0f32, 0f32);
        for k in 0..=win / 2 {
            let m = (re[k] * re[k] + im[k] * im[k]).sqrt();
            let lm = (1.0 + 10.0 * m).ln();
            let d = lm - prev[k];
            if d > 0.0 {
                flux += d;
            }
            prev[k] = lm;
            if k < low_bins {
                lowe += m;
            }
        }
        osf.push(flux);
        low.push(lowe);
    }
    // remove slow trend, rectify, normalise to unit std
    let ma = (osr * 0.5) as usize;
    let mut detr = vec![0f32; n_frames];
    for t in 0..n_frames {
        let lo = t.saturating_sub(ma);
        let hi = (t + ma + 1).min(n_frames);
        let mean = osf[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        detr[t] = (osf[t] - mean).max(0.0);
    }
    let std = (detr.iter().map(|v| v * v).sum::<f32>() / n_frames as f32).sqrt().max(1e-6);
    for v in &mut detr {
        *v /= std;
    }
    if detr.iter().all(|v| *v < 1e-3) {
        bail!("no onsets found (silence?)");
    }

    // ---- tempo via autocorrelation with a prior ----
    // A marked bar (start+end) gives a tempo prior: search only ±8% around it.
    let hint_bpm: Option<f64> = hint.and_then(|h| h.end.map(|e| (e, h.start))).and_then(|(e, st)| {
        (e > st).then(|| 60.0 * beats_per_bar.max(1) as f64 * fs / (e - st) as f64)
    });
    let (min_lag, max_lag) = match hint_bpm {
        Some(b) => {
            let p0 = osr * 60.0 / b;
            (((p0 / 1.08).floor() as usize).max(2), ((p0 * 1.08).ceil() as usize).min(n_frames / 2))
        }
        None => ((osr * 60.0 / 220.0).floor().max(2.0) as usize, ((osr * 60.0 / 50.0).ceil() as usize).min(n_frames / 2)),
    };
    if max_lag <= min_lag + 2 {
        bail!("audio too short for tempo analysis");
    }
    let mut acf = vec![0f32; max_lag + 1];
    for (lag, v) in acf.iter_mut().enumerate().take(max_lag + 1).skip(min_lag) {
        let mut s = 0f32;
        for t in 0..n_frames - lag {
            s += detr[t] * detr[t + lag];
        }
        let bpm = 60.0 * osr / lag as f64;
        let prior = if hint_bpm.is_some() { 1.0 } else { (-0.5 * ((bpm / 110.0).log2() / 0.9).powi(2)).exp() as f32 };
        *v = s / (n_frames - lag) as f32 * prior;
    }
    let mut best = min_lag;
    for lag in min_lag..=max_lag {
        if acf[lag] > acf[best] {
            best = lag;
        }
    }
    // parabolic refinement of the peak
    let period = if best > min_lag && best < max_lag {
        let (y0, y1, y2) = (acf[best - 1], acf[best], acf[best + 1]);
        let denom = y0 - 2.0 * y1 + y2;
        let d = if denom.abs() > 1e-9 { 0.5 * (y0 - y2) / denom } else { 0.0 };
        best as f64 + d.clamp(-0.5, 0.5) as f64
    } else {
        best as f64
    };

    // ---- beat tracking by dynamic programming ----
    // local score: onset strength smoothed by a Gaussian of width period/32
    let sigma = (period / 32.0).max(1.0);
    let half = (sigma * 3.0).ceil() as usize;
    let kernel: Vec<f32> = (0..=2 * half).map(|i| (-0.5 * ((i as f64 - half as f64) / sigma).powi(2)).exp() as f32).collect();
    let ksum: f32 = kernel.iter().sum();
    let mut local = vec![0f32; n_frames];
    for t in 0..n_frames {
        let mut s = 0f32;
        for (i, kv) in kernel.iter().enumerate() {
            let src = t as isize + i as isize - half as isize;
            if src >= 0 && (src as usize) < n_frames {
                s += detr[src as usize] * kv;
            }
        }
        local[t] = s / ksum;
    }
    let alpha = 400f32;
    let (lo_lag, hi_lag) = ((period / 2.0).round() as usize, (period * 2.0).round() as usize);
    let mut score = vec![0f32; n_frames];
    let mut back = vec![usize::MAX; n_frames];
    for t in 0..n_frames {
        let mut best_s = 0f32;
        let mut best_p = usize::MAX;
        if t >= lo_lag {
            for tau in lo_lag..=hi_lag.min(t) {
                let pen = -alpha * ((tau as f64 / period).ln() as f32).powi(2);
                let s = score[t - tau] + pen;
                if s > best_s || best_p == usize::MAX {
                    best_s = s;
                    best_p = t - tau;
                }
            }
            if best_s < 0.0 {
                best_s = 0.0; // allow starting fresh
                best_p = usize::MAX;
            }
        }
        score[t] = local[t] + best_s;
        back[t] = best_p;
    }
    let tail = n_frames.saturating_sub(hi_lag);
    let mut end = tail;
    for t in tail..n_frames {
        if score[t] > score[end] {
            end = t;
        }
    }
    let mut beats_t = Vec::new();
    let mut t = end;
    while t != usize::MAX {
        beats_t.push(t);
        t = back[t];
    }
    beats_t.reverse();
    if beats_t.len() < 4 {
        bail!("too few beats tracked");
    }

    // ---- constant-tempo least-squares fit ----
    let frames_of = |t: usize| (t * hop + hop / 2) as f64;
    let m = beats_t.len() as f64;
    let xm = (m - 1.0) / 2.0;
    let ym = beats_t.iter().map(|&t| frames_of(t)).sum::<f64>() / m;
    let (mut sxy, mut sxx) = (0f64, 0f64);
    for (i, &t) in beats_t.iter().enumerate() {
        let dx = i as f64 - xm;
        sxy += dx * (frames_of(t) - ym);
        sxx += dx * dx;
    }
    let slope = sxy / sxx; // frames per beat
    let intercept = ym - slope * xm;
    let fit_ms = beats_t.iter().enumerate().map(|(i, &t)| (frames_of(t) - (intercept + slope * i as f64)).abs()).sum::<f64>() / m / fs * 1000.0;
    let mut bpm = 60.0 * fs / slope;
    let mut note = String::new();
    if let Some(b0) = hint_bpm {
        if ((bpm - b0) / b0).abs() > 0.06 {
            note = format!(", kept your marked tempo ({b0:.2}) over the tracked {bpm:.2}");
            bpm = b0;
        } else {
            note = format!(", refined your marked {b0:.2} BPM");
        }
    }
    if (bpm - bpm.round()).abs() < 0.03 {
        bpm = bpm.round();
    }
    bpm = (bpm * 100.0).round() / 100.0;
    let fpb = 60.0 * fs / bpm;

    // ---- downbeat phase: strongest low-band energy + onset on average ----
    // Look a few frames past each beat so the whole attack is seen.
    let peek = |v: &[f32], t: usize| v[t..(t + 4).min(v.len())].iter().cloned().fold(0f32, f32::max);
    let low_max = low.iter().cloned().fold(1e-6f32, f32::max);
    let loc_max = local.iter().cloned().fold(1e-6f32, f32::max);
    let bpb = beats_per_bar.max(1) as usize;
    let mut phase_score = vec![0f32; bpb];
    for (i, &t) in beats_t.iter().enumerate() {
        phase_score[i % bpb] += 2.0 * peek(&low, t) / low_max + peek(&local, t) / loc_max;
    }
    let phase = (0..bpb).max_by(|&a, &b| phase_score[a].partial_cmp(&phase_score[b]).unwrap()).unwrap_or(0);
    let bar = fpb * bpb as f64;
    let anchor = match hint {
        Some(h) => {
            note += ", downbeat anchored at your bar start";
            h.start as i64
        }
        None => (intercept + slope * phase as f64).rem_euclid(bar).round() as i64,
    };

    Ok(Detection {
        grid: Grid { bpm, anchor, beats_per_bar },
        fit_ms,
        n_beats: beats_t.len(),
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Click track with a louder, bassier click on each downbeat.
    fn clicks(bpm: f64, secs: f64, offset_s: f64, rate: u32) -> Audio {
        let n = (secs * rate as f64) as usize;
        let mut a = Audio::new(rate, 1);
        a.channels[0] = vec![0.0; n];
        let fpb = 60.0 * rate as f64 / bpm;
        let mut k = 0;
        loop {
            let pos = (offset_s * rate as f64 + k as f64 * fpb).round() as usize;
            if pos >= n {
                break;
            }
            let (freq, amp, len) = if k % 4 == 0 { (80.0, 0.9, 0.08) } else { (1000.0, 0.5, 0.03) };
            for i in 0..(len * rate as f64) as usize {
                if pos + i < n {
                    let t = i as f32 / rate as f32;
                    a.channels[0][pos + i] += amp * (2.0 * PI * freq * t).sin() * (-t * 40.0).exp();
                }
            }
            k += 1;
        }
        a
    }

    #[test]
    fn detects_tempo_and_downbeat() {
        for (bpm, off) in [(120.0, 0.25), (95.0, 0.0), (140.0, 1.1)] {
            let a = clicks(bpm, 20.0, off, 22050);
            let d = detect(&a, 4, None).unwrap();
            assert!((d.grid.bpm - bpm).abs() < 0.5, "bpm {} vs {}", d.grid.bpm, bpm);
            let bar = d.grid.bar_frames(22050);
            let expect = (off * 22050.0) % bar;
            let diff = (d.grid.anchor as f64 - expect).abs();
            let diff = diff.min(bar - diff);
            assert!(diff < 22050.0 * 0.03, "anchor off by {:.1} ms at {bpm}", diff / 22.05);
            assert!(d.fit_ms < 15.0, "fit {}", d.fit_ms);
        }
    }

    #[test]
    fn hinted_detection_keeps_user_anchor_and_refines_tempo() {
        let a = clicks(120.0, 20.0, 0.25, 22050);
        // user marks a bar 3% too long starting one bar in
        let start = (0.25 * 22050.0) as usize + 4 * 11025;
        let end = start + (4.0 * 11025.0 * 1.03) as usize;
        let d = detect(&a, 4, Some(BarHint { start, end: Some(end) })).unwrap();
        assert_eq!(d.grid.anchor, start as i64);
        assert!((d.grid.bpm - 120.0).abs() < 0.5, "bpm {}", d.grid.bpm);
        assert!(d.note.contains("refined"), "{}", d.note);
        // a wildly wrong mark is kept (user authority) but reported
        let d = detect(&a, 4, Some(BarHint { start, end: Some(start + 11025 * 5) })).unwrap();
        assert!(d.note.contains("kept"), "{}", d.note);
        assert!((d.grid.bpm - 96.0).abs() < 0.5);
        // start-only hint: free tempo, fixed anchor
        let d = detect(&a, 4, Some(BarHint { start, end: None })).unwrap();
        assert_eq!(d.grid.anchor, start as i64);
        assert!((d.grid.bpm - 120.0).abs() < 0.5);
    }

    #[test]
    fn grid_math() {
        let g = Grid { bpm: 120.0, anchor: 1000, beats_per_bar: 4 };
        let r = 48000;
        assert_eq!(g.fpb(r), 24000.0);
        assert_eq!(g.snap(13500, r, 1_000_000), 25000); // idx 0.52 → k=1
        assert_eq!(g.snap(12000, r, 1_000_000), 1000); // idx 0.46 → k=0
        assert_eq!(g.snap(0, r, 1_000_000), 1000);
        assert!(g.is_downbeat(4) && !g.is_downbeat(5));
    }

    #[test]
    fn grid_step_and_bars() {
        let g = Grid { bpm: 120.0, anchor: 0, beats_per_bar: 4 };
        let r = 48000;
        assert_eq!(g.step(0, 1, false, r, 10_000_000), 24000);
        assert_eq!(g.step(24000, 1, false, r, 10_000_000), 48000);
        assert_eq!(g.step(24000, -1, false, r, 10_000_000), 0);
        assert_eq!(g.step(1000, -1, false, r, 10_000_000), 0);
        assert_eq!(g.step(0, 1, true, r, 10_000_000), 96000);
        assert_eq!(g.beats_in(0, 96000, r).len(), 4);
        assert_eq!(g.bars_in(0, 96000 * 8, r, 2).len(), 4);
        assert_eq!(g.bar_number(0, r), 1);
        let g2 = Grid { bpm: 120.0, anchor: 96000 * 4, beats_per_bar: 4 };
        assert_eq!(g2.lead_in_bars(r), 4);
        assert_eq!(g2.bar_number(0, r), 5);
    }
}
