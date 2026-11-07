//! Audio buffer, file I/O (WAV/MP3/FLAC/AIFF decode, WAV/MP3 encode) and effects.

use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

/// Number of frames summarised per peak block (used for fast zoomed-out drawing).
pub const PEAK_BLOCK: usize = 256;

/// Planar floating point audio: one `Vec<f32>` per channel, all the same length.
#[derive(Clone, Debug)]
pub struct Audio {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Audio {
    pub fn new(sample_rate: u32, n_channels: usize) -> Self {
        Audio {
            sample_rate,
            channels: vec![Vec::new(); n_channels.max(1)],
        }
    }

    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, |c| c.len())
    }

    pub fn n_channels(&self) -> usize {
        self.channels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames() == 0
    }

    pub fn clamp_range(&self, start: usize, end: usize) -> (usize, usize) {
        let n = self.frames();
        let s = start.min(n);
        let e = end.min(n).max(s);
        (s, e)
    }

    pub fn interleaved(&self) -> Vec<f32> {
        let n = self.frames();
        let nc = self.n_channels();
        let mut out = Vec::with_capacity(n * nc);
        for i in 0..n {
            for ch in &self.channels {
                out.push(ch[i]);
            }
        }
        out
    }

    pub fn slice(&self, start: usize, end: usize) -> Audio {
        let (s, e) = self.clamp_range(start, end);
        Audio {
            sample_rate: self.sample_rate,
            channels: self.channels.iter().map(|c| c[s..e].to_vec()).collect(),
        }
    }

    pub fn delete(&mut self, start: usize, end: usize) {
        let (s, e) = self.clamp_range(start, end);
        for c in &mut self.channels {
            c.drain(s..e);
        }
    }

    /// Insert `other` at frame `at`. Channel counts are mapped modulo, so mono
    /// pastes into every channel of a stereo file.
    pub fn insert(&mut self, at: usize, other: &Audio) {
        let at = at.min(self.frames());
        let onc = other.n_channels();
        for (i, c) in self.channels.iter_mut().enumerate() {
            let src = &other.channels[i % onc];
            c.splice(at..at, src.iter().copied());
        }
    }

    pub fn peak(&self, start: usize, end: usize) -> f32 {
        let (s, e) = self.clamp_range(start, end);
        self.channels
            .iter()
            .flat_map(|c| c[s..e].iter())
            .fold(0.0f32, |m, v| m.max(v.abs()))
    }

    /// Scale the range so its peak hits `target` (linear, e.g. 1.0 = 0 dBFS).
    /// Returns the gain applied.
    pub fn normalize(&mut self, start: usize, end: usize, target: f32) -> f32 {
        let (s, e) = self.clamp_range(start, end);
        let p = self.peak(s, e);
        if p <= 0.0 {
            return 1.0;
        }
        let g = target / p;
        for c in &mut self.channels {
            for v in &mut c[s..e] {
                *v *= g;
            }
        }
        g
    }

    pub fn reverse(&mut self, start: usize, end: usize) {
        let (s, e) = self.clamp_range(start, end);
        for c in &mut self.channels {
            c[s..e].reverse();
        }
    }
}

/// Per-channel min/max summary over blocks of `PEAK_BLOCK` frames.
#[derive(Clone, Debug, Default)]
pub struct Peaks {
    pub min: Vec<Vec<f32>>,
    pub max: Vec<Vec<f32>>,
}

impl Peaks {
    pub fn build(a: &Audio) -> Peaks {
        let mut min = Vec::with_capacity(a.n_channels());
        let mut max = Vec::with_capacity(a.n_channels());
        for c in &a.channels {
            let mut mn = Vec::with_capacity(c.len() / PEAK_BLOCK + 1);
            let mut mx = Vec::with_capacity(c.len() / PEAK_BLOCK + 1);
            for blk in c.chunks(PEAK_BLOCK) {
                let (lo, hi) = blk
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
                mn.push(lo);
                mx.push(hi);
            }
            min.push(mn);
            max.push(mx);
        }
        Peaks { min, max }
    }
}

/// Min/max of channel `ch` over frames `[start, end)`, using the peak
/// summary for the interior whole blocks.
pub fn column_minmax(a: &Audio, peaks: &Peaks, ch: usize, start: usize, end: usize) -> (f32, f32) {
    let n = a.frames();
    let s = start.min(n);
    let e = end.max(s + 1).min(n);
    if s >= e {
        return (0.0, 0.0);
    }
    let data = &a.channels[ch];
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    let fold = |lo: &mut f32, hi: &mut f32, sl: &[f32]| {
        for &v in sl {
            if v < *lo {
                *lo = v;
            }
            if v > *hi {
                *hi = v;
            }
        }
    };
    let sb = s.div_ceil(PEAK_BLOCK);
    let eb = e / PEAK_BLOCK;
    if eb > sb && ch < peaks.min.len() {
        fold(&mut lo, &mut hi, &data[s..(sb * PEAK_BLOCK).min(e)]);
        fold(&mut lo, &mut hi, &peaks.min[ch][sb..eb]);
        fold(&mut lo, &mut hi, &peaks.max[ch][sb..eb]);
        fold(&mut lo, &mut hi, &data[(eb * PEAK_BLOCK).max(s)..e]);
    } else {
        fold(&mut lo, &mut hi, &data[s..e]);
    }
    (lo, hi)
}

pub struct Loaded {
    pub audio: Audio,
    /// Bit depth of the source (16/24/32); used to pick the WAV output format.
    pub bits: u16,
}

/// Decode any format symphonia understands (wav, mp3, flac, aiff) to f32.
pub fn load(path: &Path) -> Result<Loaded> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error as SymErr;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let fmt_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &MetadataOptions::default())
        .with_context(|| format!("unrecognised audio format: {}", path.display()))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no audio track found"))?;
    let track_id = track.id;
    let bits = if track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_PCM_F32LE
        || track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_PCM_F32BE
    {
        32
    } else {
        track.codec_params.bits_per_sample.unwrap_or(16) as u16
    };
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("unsupported codec")?;

    let mut audio: Option<Audio> = None;
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut buf_frames = 0u64;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymErr::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let cap = decoded.capacity() as u64;
                let a = audio.get_or_insert_with(|| Audio::new(spec.rate, spec.channels.count()));
                if sample_buf.is_none() || cap > buf_frames {
                    sample_buf = Some(SampleBuffer::<f32>::new(cap, spec));
                    buf_frames = cap;
                }
                let sb = sample_buf.as_mut().unwrap();
                sb.copy_interleaved_ref(decoded);
                let nc = a.n_channels();
                for (i, v) in sb.samples().iter().enumerate() {
                    a.channels[i % nc].push(*v);
                }
            }
            Err(SymErr::DecodeError(_)) => continue, // skip corrupt frame
            Err(e) => return Err(e.into()),
        }
    }

    let audio = audio.ok_or_else(|| anyhow!("file contains no decodable audio"))?;
    if audio.is_empty() {
        bail!("file contains no audio frames");
    }
    Ok(Loaded { audio, bits })
}

/// ACID chunk contents: lets DAWs (Ableton, FL, Reaper, Cubase, Logic) read the tempo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcidInfo {
    pub bpm: f64,
    pub beats: u32,
    pub num: u16,
    pub denom: u16,
}

/// Extra WAV metadata: ACID tempo and labelled cue markers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WavMeta {
    pub acid: Option<AcidInfo>,
    /// (frame, label)
    pub cues: Vec<(usize, String)>,
}

/// Write by file extension: `.wav` or `.mp3`. MP3 cannot carry the metadata.
#[cfg(test)]
pub fn save(audio: &Audio, path: &Path, bits: u16, meta: &WavMeta) -> Result<()> {
    save_with(audio, path, bits, meta, 192)
}

/// Write by file extension: `.wav` or `.mp3`, with the MP3 bitrate in kbps.
pub fn save_with(audio: &Audio, path: &Path, bits: u16, meta: &WavMeta, mp3_kbps: u32) -> Result<()> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    match ext.as_str() {
        "wav" | "wave" => save_wav(audio, path, bits, meta),
        "mp3" => save_mp3(audio, path, mp3_kbps),
        "" => bail!("no file extension; use .wav or .mp3"),
        other => bail!("unsupported output format .{other}; use .wav or .mp3"),
    }
}

pub fn save_wav(audio: &Audio, path: &Path, bits: u16, meta: &WavMeta) -> Result<()> {
    use std::io::Write;
    let bits = match bits {
        24 => 24u16,
        32 => 32,
        _ => 16,
    };
    let nc = audio.n_channels() as u16;
    let n = audio.frames();
    let block = nc * bits / 8;
    let data_len = n * block as usize;
    let fmt_tag: u16 = if bits == 32 { 3 } else { 1 };

    let mut body: Vec<u8> = Vec::with_capacity(data_len + 1024);
    let chunk = |body: &mut Vec<u8>, id: &[u8; 4], payload: &[u8]| {
        body.extend_from_slice(id);
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            body.push(0);
        }
    };
    // fmt
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&fmt_tag.to_le_bytes());
    fmt.extend_from_slice(&nc.to_le_bytes());
    fmt.extend_from_slice(&audio.sample_rate.to_le_bytes());
    fmt.extend_from_slice(&(audio.sample_rate * block as u32).to_le_bytes());
    fmt.extend_from_slice(&block.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    chunk(&mut body, b"fmt ", &fmt);
    if fmt_tag == 3 {
        chunk(&mut body, b"fact", &(n as u32).to_le_bytes());
    }
    // data
    let mut data = Vec::with_capacity(data_len);
    for i in 0..n {
        for c in &audio.channels {
            let v = c[i];
            match bits {
                16 => data.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()),
                24 => {
                    let q = (v.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                    data.extend_from_slice(&q.to_le_bytes()[..3]);
                }
                _ => data.extend_from_slice(&v.to_le_bytes()),
            }
        }
    }
    chunk(&mut body, b"data", &data);
    drop(data);
    // acid
    if let Some(a) = meta.acid {
        let mut acid = Vec::with_capacity(24);
        acid.extend_from_slice(&0x04u32.to_le_bytes()); // loop, stretchable
        acid.extend_from_slice(&60u16.to_le_bytes()); // root note (unused flag)
        acid.extend_from_slice(&0x8000u16.to_le_bytes());
        acid.extend_from_slice(&0f32.to_le_bytes());
        acid.extend_from_slice(&a.beats.to_le_bytes());
        acid.extend_from_slice(&a.denom.to_le_bytes());
        acid.extend_from_slice(&a.num.to_le_bytes());
        acid.extend_from_slice(&(a.bpm as f32).to_le_bytes());
        chunk(&mut body, b"acid", &acid);
    }
    // cue + labels
    if !meta.cues.is_empty() {
        let mut cue = Vec::new();
        cue.extend_from_slice(&(meta.cues.len() as u32).to_le_bytes());
        let mut adtl = Vec::new();
        adtl.extend_from_slice(b"adtl");
        for (i, (frame, label)) in meta.cues.iter().enumerate() {
            let id = i as u32 + 1;
            cue.extend_from_slice(&id.to_le_bytes());
            cue.extend_from_slice(&(*frame as u32).to_le_bytes());
            cue.extend_from_slice(b"data");
            cue.extend_from_slice(&0u32.to_le_bytes());
            cue.extend_from_slice(&0u32.to_le_bytes());
            cue.extend_from_slice(&(*frame as u32).to_le_bytes());
            let mut labl = Vec::new();
            labl.extend_from_slice(&id.to_le_bytes());
            labl.extend_from_slice(label.as_bytes());
            labl.push(0);
            chunk(&mut adtl, b"labl", &labl);
        }
        chunk(&mut body, b"cue ", &cue);
        chunk(&mut body, b"LIST", &adtl);
    }

    let mut f = std::io::BufWriter::new(std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?);
    f.write_all(b"RIFF")?;
    f.write_all(&((body.len() + 4) as u32).to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(&body)?;
    f.flush()?;
    Ok(())
}

/// Read ACID tempo and cue markers from a WAV file; silently empty for
/// non-WAV files or files without them.
pub fn read_wav_meta(path: &Path) -> WavMeta {
    use std::io::{Read, Seek, SeekFrom};
    let mut meta = WavMeta::default();
    let Ok(mut f) = std::fs::File::open(path) else { return meta };
    let mut hdr = [0u8; 12];
    if f.read_exact(&mut hdr).is_err() || &hdr[0..4] != b"RIFF" || &hdr[8..12] != b"WAVE" {
        return meta;
    }
    let u32le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let u16le = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]);
    let mut cues: Vec<(u32, usize)> = Vec::new();
    let mut labels: Vec<(u32, String)> = Vec::new();
    let mut ch = [0u8; 8];
    while f.read_exact(&mut ch).is_ok() {
        let id = [ch[0], ch[1], ch[2], ch[3]];
        let size = u32le(&ch[4..8]) as u64;
        let padded = size + (size & 1);
        let interesting = matches!(&id, b"acid" | b"cue " | b"LIST") && size < 16 * 1024 * 1024;
        if !interesting {
            if f.seek(SeekFrom::Current(padded as i64)).is_err() {
                break;
            }
            continue;
        }
        let mut buf = vec![0u8; padded as usize];
        if f.read_exact(&mut buf).is_err() {
            break;
        }
        let buf = &buf[..size as usize];
        match &id {
            b"acid" if buf.len() >= 24 => {
                meta.acid = Some(AcidInfo {
                    bpm: f32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]) as f64,
                    beats: u32le(&buf[12..16]),
                    denom: u16le(&buf[16..18]),
                    num: u16le(&buf[18..20]),
                });
            }
            b"cue " if buf.len() >= 4 => {
                let count = u32le(&buf[0..4]) as usize;
                for i in 0..count {
                    let o = 4 + i * 24;
                    if o + 24 > buf.len() {
                        break;
                    }
                    cues.push((u32le(&buf[o..o + 4]), u32le(&buf[o + 20..o + 24]) as usize));
                }
            }
            b"LIST" if buf.len() >= 4 && &buf[0..4] == b"adtl" => {
                let mut o = 4;
                while o + 8 <= buf.len() {
                    let sid = &buf[o..o + 4];
                    let ssz = u32le(&buf[o + 4..o + 8]) as usize;
                    let body_end = (o + 8 + ssz).min(buf.len());
                    if sid == b"labl" && ssz >= 4 {
                        let id = u32le(&buf[o + 8..o + 12]);
                        let text = &buf[o + 12..body_end];
                        let text = text.split(|&b| b == 0).next().unwrap_or(&[]);
                        labels.push((id, String::from_utf8_lossy(text).into_owned()));
                    }
                    o += 8 + ssz + (ssz & 1);
                }
            }
            _ => {}
        }
    }
    cues.sort_by_key(|c| c.1);
    meta.cues = cues
        .into_iter()
        .map(|(id, frame)| {
            let label = labels.iter().find(|(lid, _)| *lid == id).map(|(_, l)| l.clone()).unwrap_or_default();
            (frame, label)
        })
        .collect();
    meta
}

pub fn save_mp3(audio: &Audio, path: &Path, kbps: u32) -> Result<()> {
    use mp3lame_encoder::{Bitrate, Builder, DualPcm, FlushNoGap, MonoPcm, Quality};

    const RATES: [(u32, Bitrate); 16] = [
        (8, Bitrate::Kbps8), (16, Bitrate::Kbps16), (24, Bitrate::Kbps24), (32, Bitrate::Kbps32),
        (40, Bitrate::Kbps40), (48, Bitrate::Kbps48), (64, Bitrate::Kbps64), (80, Bitrate::Kbps80),
        (96, Bitrate::Kbps96), (112, Bitrate::Kbps112), (128, Bitrate::Kbps128), (160, Bitrate::Kbps160),
        (192, Bitrate::Kbps192), (224, Bitrate::Kbps224), (256, Bitrate::Kbps256), (320, Bitrate::Kbps320),
    ];
    let bitrate = RATES.iter().min_by_key(|(k, _)| k.abs_diff(kbps)).map(|(_, b)| *b).unwrap_or(Bitrate::Kbps192);

    let nc = audio.n_channels();
    let mut b = Builder::new().ok_or_else(|| anyhow!("could not create LAME encoder"))?;
    let lame = |e: mp3lame_encoder::BuildError| anyhow!("LAME: {e}");
    b.set_num_channels(if nc == 1 { 1 } else { 2 }).map_err(lame)?;
    b.set_sample_rate(audio.sample_rate).map_err(lame)?;
    b.set_brate(bitrate).map_err(lame)?;
    b.set_quality(Quality::Best).map_err(lame)?;
    let mut enc = b.build().map_err(lame)?;

    let mut out: Vec<u8> = Vec::new();
    out.reserve(mp3lame_encoder::max_required_buffer_size(audio.frames()) + 8192);
    let enc_err = |e: mp3lame_encoder::EncodeError| anyhow!("LAME encode: {e}");
    if nc == 1 {
        enc.encode_to_vec(MonoPcm(&audio.channels[0]), &mut out).map_err(enc_err)?;
    } else {
        // Downmix anything beyond stereo into L/R.
        let (left, right): (Vec<f32>, Vec<f32>) = if nc == 2 {
            (audio.channels[0].clone(), audio.channels[1].clone())
        } else {
            let n = audio.frames();
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            for (ci, c) in audio.channels.iter().enumerate() {
                let dst = if ci % 2 == 0 { &mut l } else { &mut r };
                for (d, v) in dst.iter_mut().zip(c) {
                    *d += *v / (nc as f32 / 2.0);
                }
            }
            (l, r)
        };
        enc.encode_to_vec(DualPcm { left: &left, right: &right }, &mut out).map_err(enc_err)?;
    }
    enc.flush_to_vec::<FlushNoGap>(&mut out).map_err(enc_err)?;
    std::fs::write(path, &out).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Format a frame position as `m:ss.mmm`.
pub fn fmt_time(frames: usize, rate: u32) -> String {
    let secs = frames as f64 / rate.max(1) as f64;
    let m = (secs / 60.0).floor() as u64;
    let s = secs - m as f64 * 60.0;
    format!("{m}:{s:06.3}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, nc: usize) -> Audio {
        let mut a = Audio::new(8000, nc);
        for c in 0..nc {
            for i in 0..frames {
                let t = i as f32 / 8000.0;
                a.channels[c].push(0.5 * (t * 440.0 * (c as f32 + 1.0) * std::f32::consts::TAU).sin());
            }
        }
        a
    }

    #[test]
    fn edit_ops() {
        let mut a = tone(1000, 2);
        let clip = a.slice(100, 200);
        assert_eq!(clip.frames(), 100);
        a.delete(100, 200);
        assert_eq!(a.frames(), 900);
        a.insert(0, &clip);
        assert_eq!(a.frames(), 1000);
        a.reverse(0, 1000);
        let g = a.normalize(0, 1000, 1.0);
        assert!(g > 1.0);
        assert!((a.peak(0, 1000) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn peaks_match_raw() {
        let a = tone(5000, 1);
        let p = Peaks::build(&a);
        for (s, e) in [(0, 5000), (10, 2000), (300, 301), (4990, 6000), (257, 1023)] {
            let (lo, hi) = column_minmax(&a, &p, 0, s, e);
            let sl = &a.channels[0][s.min(5000)..e.min(5000)];
            let rlo = sl.iter().cloned().fold(f32::MAX, f32::min);
            let rhi = sl.iter().cloned().fold(f32::MIN, f32::max);
            assert_eq!((lo, hi), (rlo, rhi), "range {s}..{e}");
        }
    }

    #[test]
    fn wav_roundtrip() {
        let dir = std::env::temp_dir().join(format!("swell-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = tone(4000, 2);
        for bits in [16u16, 24, 32] {
            let p = dir.join(format!("t{bits}.wav"));
            save(&a, &p, bits, &WavMeta::default()).unwrap();
            let l = load(&p).unwrap();
            assert_eq!(l.audio.sample_rate, 8000);
            assert_eq!(l.audio.n_channels(), 2);
            assert_eq!(l.audio.frames(), 4000);
            assert_eq!(l.bits, bits);
            let err = l.audio.channels[1]
                .iter()
                .zip(&a.channels[1])
                .map(|(x, y)| (x - y).abs())
                .fold(0.0f32, f32::max);
            assert!(err < 1e-3, "bits {bits} err {err}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mp3_roundtrip() {
        let dir = std::env::temp_dir().join(format!("swell-test-mp3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = tone(16000, 2);
        a.sample_rate = 16000;
        let p = dir.join("t.mp3");
        save(&a, &p, 16, &WavMeta::default()).unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.audio.sample_rate, 16000);
        assert_eq!(l.audio.n_channels(), 2);
        // Lossy + encoder padding: just check length is close and signal is present.
        let n = l.audio.frames() as i64;
        assert!((n - 16000).abs() < 2500, "frames {n}");
        assert!(l.audio.peak(0, l.audio.frames()) > 0.3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wav_meta_roundtrip() {
        let dir = std::env::temp_dir().join(format!("swell-test-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = tone(4000, 2);
        let meta = WavMeta {
            acid: Some(AcidInfo { bpm: 128.5, beats: 16, num: 4, denom: 4 }),
            cues: vec![(0, "Bar 1".into()), (1234, "Slice 1".into()), (3999, "odd".into())],
        };
        let p = dir.join("m.wav");
        save(&a, &p, 24, &meta).unwrap();
        let back = read_wav_meta(&p);
        assert_eq!(back.cues, meta.cues);
        let acid = back.acid.unwrap();
        assert!((acid.bpm - 128.5).abs() < 1e-4);
        assert_eq!((acid.beats, acid.num, acid.denom), (16, 4, 4));
        // audio still decodes fine with the extra chunks present
        let l = load(&p).unwrap();
        assert_eq!(l.audio.frames(), 4000);
        assert_eq!(l.bits, 24);
        assert_eq!(read_wav_meta(&dir.join("missing.wav")), WavMeta::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn time_format() {
        assert_eq!(fmt_time(0, 44100), "0:00.000");
        assert_eq!(fmt_time(44100 * 61 + 22050, 44100), "1:01.500");
    }
}
