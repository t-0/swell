//! Editor state and input handling.

use crate::audio::{self, fmt_time, AcidInfo, Audio, Peaks, WavMeta};
use crate::beats::{self, BarHint, Grid};
use crate::commands::{self, Cmd, KeyChord};
use crate::config::{self, Config};
use crate::player::KeepAlive;
use std::collections::HashMap;
use crate::fx::{self, FxKind};
use crate::player::Player;
use anyhow::{anyhow, bail, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::path::{Path, PathBuf};

pub const SPEED_MIN: u32 = 50;
pub const SPEED_MAX: u32 = 200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromptKind {
    Open,
    SaveAs,
    ExportPieces,
    ExportSelection,
    /// Comma-separated numeric parameters for an effect.
    FxParams(FxKind),
    /// Comma-separated numbers for a grid command.
    Numbers(NumCmd),
    ExportMarkers,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NumCmd {
    Bpm,
    BeatsPerBar,
    Offset,
    Conform,
    SelectBars,
    SliceBars,
}

pub struct Palette {
    pub input: String,
    pub selected: usize,
    /// Indices into `commands::all()`.
    pub matches: Vec<usize>,
}

pub struct Prompt {
    pub kind: PromptKind,
    pub label: String,
    pub input: String,
}

pub enum ConfirmAction {
    Quit,
    Open(PathBuf),
}

pub enum Mode {
    Normal,
    Prompt(Prompt),
    Palette(Palette),
    Help,
    Confirm { question: String, action: ConfirmAction },
}

pub struct Msg {
    pub text: String,
    pub is_error: bool,
}

struct Snap {
    audio: Audio,
    cursor: usize,
    selection: Option<(usize, usize)>,
    loop_range: Option<(usize, usize)>,
    slices: Vec<usize>,
    grid: Option<Grid>,
    bar_marks: (Option<usize>, Option<usize>),
}

#[derive(PartialEq, Clone, Copy)]
struct WaveKey {
    view_start: usize,
    fpc_bits: u64,
    width: u16,
    version: u64,
}

pub struct App {
    pub audio: Audio,
    pub peaks: Peaks,
    pub path: Option<PathBuf>,
    pub bits: u16,
    pub dirty: bool,
    pub cursor: usize,
    pub selection: Option<(usize, usize)>,
    sel_anchor: Option<usize>,
    pub loop_range: Option<(usize, usize)>,
    pub slices: Vec<usize>,
    pub grid: Option<Grid>,
    /// User-marked bar: start (downbeat) and optional end, frames.
    pub bar_start: Option<usize>,
    pub bar_end: Option<usize>,
    pub snap: bool,
    pub view_start: usize,
    pub fpc: f64,
    pub wave_area: Rect,
    /// Track rectangle of the speed slider, set by the UI for mouse hits.
    pub slider_area: Rect,
    /// Playback speed in percent, SPEED_MIN..=SPEED_MAX.
    pub speed: u32,
    needs_fit: bool,
    pub mode: Mode,
    pub msg: Option<Msg>,
    pub should_quit: bool,
    pub player: Player,
    pub paused: bool,
    clipboard: Option<Audio>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    version: u64,
    wave_cache: Option<(WaveKey, Vec<Vec<(f32, f32)>>)>,
    mouse_drag: bool,
    slider_drag: bool,
    pub config: Config,
    pub config_path: PathBuf,
    pub keymap: HashMap<KeyChord, Cmd>,
}

impl App {
    pub fn new(path: Option<PathBuf>) -> App {
        let loaded = config::load();
        let mut player = Player::new();
        let audio = Audio::new(44100, 2);
        player.set_audio(&audio);
        let mut app = App {
            keymap: config::keymap(&loaded.config),
            config: loaded.config.clone(),
            config_path: loaded.path.clone(),
            peaks: Peaks::build(&audio),
            audio,
            path: None,
            bits: 16,
            dirty: false,
            cursor: 0,
            selection: None,
            sel_anchor: None,
            loop_range: None,
            slices: Vec::new(),
            grid: None,
            bar_start: None,
            bar_end: None,
            snap: false,
            view_start: 0,
            fpc: 1.0,
            wave_area: Rect::default(),
            slider_area: Rect::default(),
            speed: 100,
            needs_fit: true,
            mode: Mode::Normal,
            msg: None,
            should_quit: false,
            player,
            paused: false,
            clipboard: None,
            undo: Vec::new(),
            redo: Vec::new(),
            version: 0,
            wave_cache: None,
            mouse_drag: false,
            slider_drag: false,
        };
        app.apply_config();
        match path {
            Some(p) => {
                let r = app.load_file(&p);
                app.report(r);
            }
            None => app.info("No file loaded. Press o to open a file, ? for help."),
        }
        if loaded.created {
            app.info(format!("Created {} — keys, output keep-alive and defaults, commented", loaded.path.display()));
        }
        for w in &loaded.warnings {
            app.error(format!("settings: {w}"));
        }
        if let Some(e) = app.player.error.clone() {
            app.error(e);
        }
        app
    }

    /// Push the settings into the player and editor state.
    fn apply_config(&mut self) {
        let c = self.config.clone();
        let mode = match c.output.keep_alive.as_str() {
            "off" => KeepAlive::Off,
            "dither" => KeepAlive::Dither,
            _ => KeepAlive::Tone,
        };
        self.player.set_keep_alive(mode);
        self.player.set_keep_alive_levels(c.output.dither_db, c.output.tone_db, c.output.tone_hz);
        self.speed = c.editor.speed.clamp(SPEED_MIN, SPEED_MAX);
        self.player.set_speed_pct(self.speed);
        if c.editor.snap && self.grid.is_some() {
            self.snap = true;
        }
        self.undo.truncate(c.editor.undo_levels.max(1));
    }

    fn reload_config(&mut self) {
        let loaded = config::load();
        if !loaded.parsed {
            // keep the last good settings and bindings
            self.error(format!("Settings unchanged, file has errors: {}", loaded.warnings.join("; ")));
            return;
        }
        self.config = loaded.config;
        self.config_path = loaded.path;
        self.keymap = config::keymap(&self.config);
        self.apply_config();
        if loaded.warnings.is_empty() {
            self.info(format!("Reloaded {}", self.config_path.display()));
        } else {
            self.error(format!("Reloaded with problems: {}", loaded.warnings.join("; ")));
        }
    }

    fn max_undo(&self) -> usize {
        self.config.editor.undo_levels.max(1)
    }

    /// WAV bit depth to write: the source's, or the configured override.
    fn save_bits(&self) -> u16 {
        match self.config.editor.wav_bits.as_str() {
            "16" => 16,
            "24" => 24,
            "32" => 32,
            _ => self.bits,
        }
    }

    // ----- messages -------------------------------------------------------

    pub fn info(&mut self, s: impl Into<String>) {
        self.msg = Some(Msg { text: s.into(), is_error: false });
    }

    pub fn error(&mut self, s: impl Into<String>) {
        self.msg = Some(Msg { text: s.into(), is_error: true });
    }

    fn report(&mut self, r: Result<String>) {
        match r {
            Ok(s) => self.info(s),
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    // ----- file I/O -------------------------------------------------------

    pub fn load_file(&mut self, path: &Path) -> Result<String> {
        let loaded = audio::load(path)?;
        self.audio = loaded.audio;
        self.bits = loaded.bits;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        self.cursor = 0;
        self.selection = None;
        self.sel_anchor = None;
        self.loop_range = None;
        self.slices.clear();
        self.grid = None;
        self.bar_start = None;
        self.bar_end = None;
        self.undo.clear();
        self.redo.clear();
        self.paused = false;
        self.needs_fit = true;
        self.view_start = 0;
        let mut note = String::new();
        // Embedded markers / tempo (WAV only)
        let meta = audio::read_wav_meta(path);
        if let Some(acid) = meta.acid.filter(|a| a.bpm > 20.0 && a.bpm < 400.0) {
            let anchor = meta.cues.iter().find(|(_, l)| l == "Bar 1").map(|(f, _)| *f as i64).unwrap_or(0);
            self.grid = Some(Grid { bpm: acid.bpm as f64, anchor, beats_per_bar: acid.num.clamp(1, 32) as u32 });
            note = format!(", {:.2} BPM from ACID chunk", acid.bpm);
        }
        let n = self.audio.frames();
        for (f, l) in &meta.cues {
            if !l.starts_with("Bar ") && *f > 0 && *f < n {
                self.slices.push(*f);
            }
        }
        if !meta.cues.is_empty() {
            note += &format!(", {} cue markers", meta.cues.len());
        }
        self.rebuild();
        Ok(format!(
            "Loaded {} ({}, {} Hz, {} ch, {}-bit{})",
            file_name(path),
            fmt_time(self.audio.frames(), self.audio.sample_rate),
            self.audio.sample_rate,
            self.audio.n_channels(),
            self.bits,
            note
        ))
    }

    /// Metadata written with the whole file: ACID tempo, bar markers, slice markers.
    pub fn wav_meta(&self, with_markers: bool) -> WavMeta {
        let rate = self.audio.sample_rate;
        let n = self.audio.frames();
        let mut meta = WavMeta::default();
        if let Some(g) = self.grid {
            meta.acid = Some(AcidInfo {
                bpm: g.bpm,
                beats: (n as f64 / g.fpb(rate)).round().max(1.0) as u32,
                num: g.beats_per_bar as u16,
                denom: 4,
            });
            if with_markers {
                for (k, f) in g.bars_in(0, n, rate, 1) {
                    meta.cues.push((f.round() as usize, format!("Bar {}", g.bar_number(k, rate))));
                }
            }
        }
        if with_markers {
            for (i, s) in self.slices.iter().enumerate() {
                meta.cues.push((*s, format!("Slice {}", i + 1)));
            }
            meta.cues.sort_by_key(|c| c.0);
        }
        meta
    }

    /// Metadata for an exported piece of `len` frames: tempo only.
    fn export_meta(&self, len: usize) -> WavMeta {
        let mut meta = WavMeta::default();
        if let Some(g) = self.grid {
            meta.acid = Some(AcidInfo {
                bpm: g.bpm,
                beats: (len as f64 / g.fpb(self.audio.sample_rate)).round().max(1.0) as u32,
                num: g.beats_per_bar as u16,
                denom: 4,
            });
        }
        meta
    }

    fn grid_warning(&self) -> Option<String> {
        let g = self.grid?;
        let rate = self.audio.sample_rate;
        let idx = g.beat_index(0.0, rate);
        let off_beat = (idx - idx.round()).abs() * g.fpb(rate) / rate as f64 * 1000.0;
        (off_beat > 2.0).then(|| format!(" (file start is {off_beat:.0} ms off the beat grid: run Conform to bars for DAW alignment)"))
    }

    fn save(&mut self) {
        match self.path.clone() {
            Some(p) => {
                let r = self.save_as(p);
                self.report(r);
            }
            None => self.open_prompt(PromptKind::SaveAs, "Save as", String::new()),
        }
    }

    fn save_as(&mut self, path: PathBuf) -> Result<String> {
        if self.audio.is_empty() {
            bail!("nothing to save");
        }
        audio::save_with(&self.audio, &path, self.save_bits(), &self.wav_meta(true), self.config.editor.mp3_kbps)?;
        self.path = Some(path.clone());
        self.dirty = false;
        let is_wav = path.extension().and_then(|e| e.to_str()).map_or(false, |e| e.eq_ignore_ascii_case("wav"));
        let extra = if is_wav {
            match self.grid {
                Some(g) => format!(" with {:.2} BPM ACID tempo + markers{}", g.bpm, self.grid_warning().unwrap_or_default()),
                None => String::new(),
            }
        } else if self.grid.is_some() {
            " (MP3 cannot carry tempo/markers; save a WAV for DAW import)".into()
        } else {
            String::new()
        };
        Ok(format!("Saved {}{}", file_name(&path), extra))
    }

    fn export_selection(&mut self, path: PathBuf) -> Result<String> {
        let (s, e) = self.selection.ok_or_else(|| anyhow!("no selection"))?;
        audio::save_with(&self.audio.slice(s, e), &path, self.save_bits(), &self.export_meta(e - s), self.config.editor.mp3_kbps)?;
        Ok(format!("Exported selection to {}", path.display()))
    }

    /// Pieces are the regions between consecutive slice points (plus file
    /// start/end). `pattern` may contain `{n}` for the 1-based piece number.
    fn export_pieces(&mut self, pattern: &str) -> Result<String> {
        let n = self.audio.frames();
        let mut bounds = vec![0usize];
        bounds.extend(self.slices.iter().copied().filter(|&s| s > 0 && s < n));
        bounds.push(n);
        bounds.dedup();
        let pieces: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).filter(|(s, e)| e > s).collect();
        if self.slices.is_empty() {
            bail!("no slice points: press m at each cut point first");
        }
        let pattern = if pattern.contains("{n}") {
            pattern.to_string()
        } else {
            let p = Path::new(pattern);
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("piece");
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("wav");
            let parent = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
            parent.join(format!("{stem}_{{n}}.{ext}")).to_string_lossy().into_owned()
        };
        let mut last = PathBuf::new();
        for (i, (s, e)) in pieces.iter().enumerate() {
            let name = pattern.replace("{n}", &format!("{:02}", i + 1));
            last = PathBuf::from(&name);
            audio::save_with(&self.audio.slice(*s, *e), &last, self.save_bits(), &self.export_meta(e - s), self.config.editor.mp3_kbps)?;
        }
        Ok(format!("Exported {} pieces, last: {}", pieces.len(), last.display()))
    }

    fn default_export_pattern(&self) -> String {
        match &self.path {
            Some(p) => {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("piece");
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("wav");
                let parent = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                parent.join(format!("{stem}_{{n}}.{ext}")).to_string_lossy().into_owned()
            }
            None => "piece_{n}.wav".to_string(),
        }
    }

    // ----- edit plumbing --------------------------------------------------

    fn rebuild(&mut self) {
        self.version += 1;
        self.peaks = Peaks::build(&self.audio);
        self.player.set_audio(&self.audio);
        self.paused = false;
        self.wave_cache = None;
        self.clamp_marks();
    }

    fn clamp_marks(&mut self) {
        let n = self.audio.frames();
        self.cursor = self.cursor.min(n);
        self.selection = self.selection.map(|(s, e)| (s.min(n), e.min(n))).filter(|(s, e)| e > s);
        self.loop_range = self.loop_range.map(|(s, e)| (s.min(n), e.min(n))).filter(|(s, e)| e > s);
        self.slices.retain(|&s| s > 0 && s < n);
        self.slices.sort_unstable();
        self.slices.dedup();
        self.bar_start = self.bar_start.map(|p| p.min(n));
        self.bar_end = self.bar_end.filter(|&e| Some(e) > self.bar_start).map(|p| p.min(n));
    }

    fn snapshot(&self) -> Snap {
        Snap {
            audio: self.audio.clone(),
            cursor: self.cursor,
            selection: self.selection,
            loop_range: self.loop_range,
            slices: self.slices.clone(),
            grid: self.grid,
            bar_marks: (self.bar_start, self.bar_end),
        }
    }

    fn restore(&mut self, s: Snap) {
        self.audio = s.audio;
        self.cursor = s.cursor;
        self.selection = s.selection;
        self.loop_range = s.loop_range;
        self.slices = s.slices;
        self.grid = s.grid;
        self.bar_start = s.bar_marks.0;
        self.bar_end = s.bar_marks.1;
        self.dirty = true;
        self.rebuild();
    }

    fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > self.max_undo() {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn after_edit(&mut self) {
        self.dirty = true;
        self.rebuild();
    }

    fn undo(&mut self) {
        match self.undo.pop() {
            Some(s) => {
                self.redo.push(self.snapshot());
                self.restore(s);
                self.info("Undo");
            }
            None => self.info("Nothing to undo"),
        }
    }

    fn redo(&mut self) {
        match self.redo.pop() {
            Some(s) => {
                self.undo.push(self.snapshot());
                self.restore(s);
                self.info("Redo");
            }
            None => self.info("Nothing to redo"),
        }
    }

    fn shift_marks_delete(&mut self, s: usize, e: usize) {
        let len = e - s;
        self.slices.retain(|&p| !(s..e).contains(&p));
        for p in &mut self.slices {
            if *p >= e {
                *p -= len;
            }
        }
        let adj = |p: usize| if p < s { p } else if p < e { s } else { p - len };
        self.loop_range = self.loop_range.map(|(a, b)| (adj(a), adj(b))).filter(|(a, b)| b > a);
        if let Some(g) = &mut self.grid {
            g.anchor = adj(g.anchor.max(0) as usize) as i64;
        }
        self.bar_start = self.bar_start.map(adj);
        self.bar_end = self.bar_end.map(adj);
    }

    fn shift_marks_insert(&mut self, at: usize, len: usize) {
        for p in &mut self.slices {
            if *p >= at {
                *p += len;
            }
        }
        let adj = |p: usize| if p >= at { p + len } else { p };
        self.loop_range = self.loop_range.map(|(a, b)| (adj(a), adj(b)));
        if let Some(g) = &mut self.grid {
            g.anchor = adj(g.anchor.max(0) as usize) as i64;
        }
        self.bar_start = self.bar_start.map(adj);
        self.bar_end = self.bar_end.map(adj);
    }

    // ----- editing commands ----------------------------------------------

    fn copy(&mut self) {
        match self.selection {
            Some((s, e)) => {
                self.clipboard = Some(self.audio.slice(s, e));
                self.info(format!("Copied {}", fmt_time(e - s, self.audio.sample_rate)));
            }
            None => self.error("Nothing selected"),
        }
    }

    fn delete_selection(&mut self, verb: &str) {
        let Some((s, e)) = self.selection else {
            self.error("Nothing selected");
            return;
        };
        self.push_undo();
        self.audio.delete(s, e);
        self.shift_marks_delete(s, e);
        self.cursor = s;
        self.selection = None;
        self.after_edit();
        self.info(format!("{verb} {}", fmt_time(e - s, self.audio.sample_rate)));
    }

    fn cut(&mut self) {
        if let Some((s, e)) = self.selection {
            self.clipboard = Some(self.audio.slice(s, e));
            self.delete_selection("Cut");
        } else {
            self.error("Nothing selected");
        }
    }

    fn paste(&mut self) {
        let Some(clip) = self.clipboard.clone() else {
            self.error("Clipboard empty");
            return;
        };
        self.push_undo();
        let at = if let Some((s, e)) = self.selection {
            self.audio.delete(s, e);
            self.shift_marks_delete(s, e);
            s
        } else {
            self.cursor
        };
        let at = at.min(self.audio.frames());
        self.audio.insert(at, &clip);
        self.shift_marks_insert(at, clip.frames());
        self.cursor = at;
        self.selection = Some((at, at + clip.frames()));
        self.after_edit();
        self.info(format!("Pasted {}", fmt_time(clip.frames(), self.audio.sample_rate)));
    }

    fn trim(&mut self) {
        let Some((s, e)) = self.selection else {
            self.error("Nothing selected");
            return;
        };
        self.push_undo();
        let n = self.audio.frames();
        self.audio.delete(e, n);
        self.shift_marks_delete(e, n);
        self.audio.delete(0, s);
        self.shift_marks_delete(0, s);
        self.cursor = 0;
        self.selection = None;
        self.needs_fit = true;
        self.after_edit();
        self.info("Trimmed to selection");
    }

    /// Frame range an effect acts on.
    fn fx_range(&self, info: &fx::FxInfo) -> (usize, usize) {
        let n = self.audio.frames();
        if info.whole_file {
            (0, n)
        } else if let Some(sel) = self.selection {
            sel
        } else if info.insert {
            (self.cursor, self.cursor)
        } else {
            (0, n)
        }
    }

    /// Shift slice points and the loop after a range `[s, e)` became `[s, new_end)`.
    fn shift_marks_resized(&mut self, s: usize, e: usize, new_end: usize) {
        if new_end == e {
            return;
        }
        let adj = |p: usize| -> usize {
            if p < s {
                p
            } else if p >= e {
                p - e + new_end
            } else {
                p.min(new_end)
            }
        };
        self.slices = self.slices.iter().map(|&p| adj(p)).collect();
        self.loop_range = self.loop_range.map(|(a, b)| (adj(a), adj(b)));
        if let Some(g) = &mut self.grid {
            g.anchor = adj(g.anchor.max(0) as usize) as i64;
        }
        self.bar_start = self.bar_start.map(adj);
        self.bar_end = self.bar_end.map(adj);
        self.clamp_marks();
    }

    pub fn run_fx(&mut self, kind: FxKind, params: &[f64]) {
        let info = fx::info(kind);
        let generator = info.insert;
        if self.audio.is_empty() && !generator {
            self.error("No audio loaded");
            return;
        }
        let (s, e) = self.fx_range(info);
        let prm: Vec<f64> = info
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| params.get(i).copied().unwrap_or(p.default).clamp(p.min, p.max))
            .collect();
        self.push_undo();
        match fx::apply(kind, &mut self.audio, s, e, &prm) {
            Ok(out) => {
                let where_ = if info.whole_file {
                    "file"
                } else if self.selection.is_some() {
                    "selection"
                } else if info.insert {
                    "at cursor"
                } else {
                    "whole file"
                };
                if info.whole_file {
                    self.selection = None;
                } else if self.selection.is_some() || info.insert {
                    self.shift_marks_resized(s, e, out.new_end);
                    self.selection = Some((s, out.new_end)).filter(|(a, b)| b > a);
                }
                self.sel_anchor = None;
                let shown: Vec<String> = info.params.iter().zip(&prm).map(|(p, v)| format!("{}={}", p.name.split(' ').next().unwrap_or(""), trim_num(*v))).collect();
                let args = if shown.is_empty() { String::new() } else { format!(" [{}]", shown.join(", ")) };
                self.after_edit();
                self.info(format!("{} → {}{}{}", info.name, where_, args, out.note));
            }
            Err(err) => {
                self.undo.pop();
                self.error(format!("{}: {err:#}", info.name));
            }
        }
    }

    fn fx_prompt(&mut self, kind: FxKind) {
        let info = fx::info(kind);
        let defaults: Vec<String> = info
            .params
            .iter()
            .map(|p| {
                let d = if kind == FxKind::ChangeSpeed { self.speed as f64 } else if kind == FxKind::ResampleFile { self.audio.sample_rate as f64 } else { p.default };
                trim_num(d)
            })
            .collect();
        let names: Vec<&str> = info.params.iter().map(|p| p.name).collect();
        let label = format!("{} — {}", info.name, names.join(", "));
        self.open_prompt(PromptKind::FxParams(kind), &label, defaults.join(", "));
    }

    // ----- marks ----------------------------------------------------------

    fn toggle_slice(&mut self) {
        let n = self.audio.frames();
        if self.cursor == 0 || self.cursor >= n {
            self.error("Slice points must be inside the file");
            return;
        }
        let tol = self.fpc.max(1.0) as usize;
        if let Some(i) = self.slices.iter().position(|&p| p.abs_diff(self.cursor) <= tol) {
            self.slices.remove(i);
            self.info("Removed slice point");
        } else {
            self.slices.push(self.cursor);
            self.slices.sort_unstable();
            self.info(format!("Slice point {} at {} ({} pieces)", self.slices.len(), fmt_time(self.cursor, self.audio.sample_rate), self.slices.len() + 1));
        }
    }

    fn jump_slice(&mut self, forward: bool) {
        let target = if forward {
            self.slices.iter().copied().find(|&p| p > self.cursor).or(Some(self.audio.frames()))
        } else {
            self.slices.iter().rev().copied().find(|&p| p < self.cursor).or(Some(0))
        };
        if let Some(t) = target {
            self.cursor = t;
            self.ensure_cursor_visible();
        }
    }

    fn toggle_loop(&mut self) {
        match (self.selection, self.loop_range) {
            (Some(sel), Some(lp)) if sel == lp => {
                self.loop_range = None;
                self.info("Loop cleared");
            }
            (Some(sel), _) => {
                self.loop_range = Some(sel);
                self.info(format!(
                    "Loop {} – {}",
                    fmt_time(sel.0, self.audio.sample_rate),
                    fmt_time(sel.1, self.audio.sample_rate)
                ));
            }
            (None, Some(_)) => {
                self.loop_range = None;
                self.info("Loop cleared");
            }
            (None, None) => self.error("Select a region first, then press l"),
        }
        self.sync_player_range();
    }

    // ----- beat grid ------------------------------------------------------

    fn detect_beats(&mut self) {
        let bpb = self.grid.map(|g| g.beats_per_bar).unwrap_or(self.config.editor.beats_per_bar.clamp(1, 32));
        let hint = self.bar_start.map(|start| BarHint { start, end: self.bar_end });
        match beats::detect(&self.audio, bpb, hint) {
            Ok(d) => {
                let rate = self.audio.sample_rate;
                self.grid = Some(d.grid);
                self.info(format!(
                    "Analysed: {:.2} BPM, {} beats, grid fit ±{:.0} ms, downbeat at {}{}. s = snap, C = conform for DAW import",
                    d.grid.bpm,
                    d.n_beats,
                    d.fit_ms,
                    fmt_time(d.grid.anchor.max(0) as usize, rate),
                    d.note
                ));
            }
            Err(e) => self.error(format!("Beat analysis failed: {e:#}")),
        }
    }

    fn mark_bar_start(&mut self) {
        let c = self.cursor;
        self.bar_start = Some(c);
        if self.bar_end.map_or(false, |e| e <= c) {
            self.bar_end = None;
        }
        self.apply_bar_marks();
    }

    fn mark_bar_end(&mut self) {
        let c = self.cursor;
        match self.bar_start {
            Some(st) if c > st => {
                self.bar_end = Some(c);
                self.apply_bar_marks();
            }
            Some(_) => self.error("Bar end must be after the bar start"),
            None => self.error("Mark the bar start first (b at the downbeat), or g to use the selection"),
        }
    }

    fn mark_bar_from_selection(&mut self) {
        match self.selection.or(self.loop_range) {
            Some((st, en)) if en > st => {
                self.bar_start = Some(st);
                self.bar_end = Some(en);
                self.apply_bar_marks();
            }
            _ => self.error("Select one bar (or set a loop over it) first"),
        }
    }

    /// Marked bar start = grid downbeat; a marked bar end also sets the tempo.
    fn apply_bar_marks(&mut self) {
        let rate = self.audio.sample_rate;
        match (self.bar_start, self.bar_end) {
            (Some(st), Some(en)) => {
                let bpb = self.grid.map(|g| g.beats_per_bar).unwrap_or(self.config.editor.beats_per_bar.clamp(1, 32));
                let bpm = 60.0 * bpb as f64 * rate as f64 / (en - st) as f64;
                let bpm = ((bpm * 100.0).round() / 100.0).clamp(20.0, 400.0);
                self.grid = Some(Grid { bpm, anchor: st as i64, beats_per_bar: bpb });
                self.info(format!(
                    "Bar marked {}–{} → {:.2} BPM, downbeat at bar start. A refines against the audio, C conforms",
                    fmt_time(st, rate),
                    fmt_time(en, rate),
                    bpm
                ));
            }
            (Some(st), None) => match self.grid.as_mut() {
                Some(g) => {
                    g.anchor = st as i64;
                    self.info(format!("Bar start marked at {}: grid downbeat moved there. B marks the bar end, A re-analyses", fmt_time(st, rate)));
                }
                None => self.info(format!("Bar start marked at {}. B at the bar end sets the tempo, or A analyses from here", fmt_time(st, rate))),
            },
            _ => {}
        }
    }

    fn grid_mut(&mut self) -> Option<&mut Grid> {
        if self.grid.is_none() {
            self.error("No beat grid: mark a bar (b/B or g) or press A to analyse");
        }
        self.grid.as_mut()
    }

    fn grid_msg(&mut self) {
        if let Some(g) = self.grid {
            let rate = self.audio.sample_rate;
            self.info(format!(
                "Grid {:.2} BPM, {}/4, downbeat at {}, {} bars",
                g.bpm,
                g.beats_per_bar,
                fmt_time(g.anchor.max(0) as usize, rate),
                g.bars_in(0, self.audio.frames(), rate, 1).len()
            ));
        }
    }

    fn set_bpm(&mut self, bpm: f64) {
        let bpm = bpm.clamp(20.0, 400.0);
        let rate = self.audio.sample_rate;
        match self.grid.as_mut() {
            Some(g) => g.bpm = bpm,
            None => self.grid = Some(Grid { bpm, anchor: self.cursor as i64, beats_per_bar: self.config.editor.beats_per_bar.clamp(1, 32) }),
        }
        let _ = rate;
        self.grid_msg();
    }

    fn grid_shift_ms(&mut self, ms: f64) {
        let rate = self.audio.sample_rate as f64;
        let n = self.audio.frames() as i64;
        if let Some(g) = self.grid_mut() {
            g.anchor = (g.anchor + (ms * rate / 1000.0).round() as i64).clamp(0, n);
            self.grid_msg();
        }
    }

    /// Pad or trim the start so the downbeat lands exactly `lead_bars` bars in,
    /// then pad the end to a whole bar (or beat). Makes the file DAW-ready:
    /// drop it at bar 1 and the music's first downbeat is at bar lead+1.
    fn conform_bars(&mut self, lead_bars: i64, end_to_bar: bool) {
        let Some(g) = self.grid else {
            self.error("No beat grid: mark a bar (b/B or g) or press A to analyse");
            return;
        };
        let rate = self.audio.sample_rate;
        let fpb = g.fpb(rate);
        let bar = g.bar_frames(rate);
        self.push_undo();
        let desired = (lead_bars.max(0) as f64 * bar).round() as i64;
        let delta = desired - g.anchor;
        if delta > 0 {
            let mut sil = Audio::new(rate, self.audio.n_channels());
            for c in &mut sil.channels {
                c.resize(delta as usize, 0.0);
            }
            self.audio.insert(0, &sil);
            self.shift_marks_insert(0, delta as usize);
        } else if delta < 0 {
            self.audio.delete(0, (-delta) as usize);
            self.shift_marks_delete(0, (-delta) as usize);
        }
        if let Some(g) = &mut self.grid {
            g.anchor = desired;
        }
        let n = self.audio.frames();
        let unit = if end_to_bar { bar } else { fpb };
        let used = (n as f64 - desired as f64) / unit;
        let whole = (used - 0.02).ceil().max(1.0);
        let target = (desired as f64 + whole * unit).round() as usize;
        let mut padded = 0usize;
        let mut trimmed = 0usize;
        if target > n {
            padded = target - n;
            let mut sil = Audio::new(rate, self.audio.n_channels());
            for c in &mut sil.channels {
                c.resize(padded, 0.0);
            }
            self.audio.insert(n, &sil);
        } else if target < n {
            trimmed = n - target;
            self.audio.delete(target, n);
            self.shift_marks_delete(target, n);
        }
        self.cursor = 0;
        self.selection = None;
        self.needs_fit = true;
        self.after_edit();
        let total_bars = (self.audio.frames() as f64 / bar).round();
        let lead_note = if delta > 0 {
            format!("padded {} ms of lead-in", (delta as f64 / rate as f64 * 1000.0).round())
        } else if delta < 0 {
            format!("trimmed {} ms of lead-in", (-delta as f64 / rate as f64 * 1000.0).round())
        } else {
            "lead-in already exact".into()
        };
        let end_note = if padded > 0 {
            format!("padded {} ms at the end", (padded as f64 / rate as f64 * 1000.0).round())
        } else if trimmed > 0 {
            format!("trimmed {} ms at the end", (trimmed as f64 / rate as f64 * 1000.0).round())
        } else {
            "end already exact".into()
        };
        self.info(format!(
            "Conformed to {} bars at {:.2} BPM: {} bars lead-in, first downbeat at bar {}; {}, {}",
            total_bars,
            g.bpm,
            lead_bars.max(0),
            lead_bars.max(0) + 1,
            lead_note,
            end_note
        ));
    }

    fn select_bars(&mut self, bars: f64) {
        let Some(g) = self.grid else {
            self.error("No beat grid: mark a bar (b/B or g) or press A to analyse");
            return;
        };
        let rate = self.audio.sample_rate;
        let n = self.audio.frames();
        let start = g.snap(self.cursor, rate, n);
        let end = ((start as f64) + bars.max(0.0) * g.bar_frames(rate)).round() as usize;
        self.selection = Some((start, end.min(n))).filter(|(s, e)| e > s);
        self.cursor = start;
        self.sel_anchor = None;
        self.info(format!("Selected {} bars from {}", trim_num(bars), fmt_time(start, rate)));
    }

    fn slice_at_grid(&mut self, every_bars: Option<i64>) {
        let Some(g) = self.grid else {
            self.error("No beat grid: mark a bar (b/B or g) or press A to analyse");
            return;
        };
        let rate = self.audio.sample_rate;
        let n = self.audio.frames();
        let pts: Vec<usize> = match every_bars {
            Some(every) => g.bars_in(0, n, rate, every).into_iter().map(|(_, f)| f.round() as usize).collect(),
            None => g.beats_in(0, n, rate).into_iter().map(|(_, f)| f.round() as usize).collect(),
        };
        self.slices = pts.into_iter().filter(|&p| p > 0 && p < n).collect();
        self.slices.dedup();
        self.info(format!("{} slice points → {} pieces; press e to export them", self.slices.len(), self.slices.len() + 1));
    }

    fn export_markers(&mut self, path: PathBuf) -> Result<String> {
        let g = self.grid.ok_or_else(|| anyhow!("no beat grid"))?;
        let rate = self.audio.sample_rate;
        let n = self.audio.frames();
        let mut out = String::new();
        for (k, f) in g.beats_in(0, n, rate) {
            let t = f / rate as f64;
            let label = if g.is_downbeat(k) {
                format!("Bar {}", g.bar_number(k, rate))
            } else {
                format!("{}.{}", g.bar_number(k, rate), k.rem_euclid(g.beats_per_bar as i64) + 1)
            };
            out += &format!("{t:.6}\t{t:.6}\t{label}\n");
        }
        std::fs::write(&path, out)?;
        Ok(format!("Wrote {} beat labels to {} (Audacity: File → Import → Labels)", g.beats_in(0, n, rate).len(), path.display()))
    }

    fn num_prompt(&mut self, cmd: NumCmd) {
        let g = self.grid;
        let rate = self.audio.sample_rate;
        let (label, default) = match cmd {
            NumCmd::Bpm => ("Grid BPM", trim_num(g.map(|g| g.bpm).unwrap_or(120.0))),
            NumCmd::BeatsPerBar => ("Beats per bar", trim_num(g.map(|g| g.beats_per_bar as f64).unwrap_or(self.config.editor.beats_per_bar as f64))),
            NumCmd::Offset => ("Shift grid by ms (negative = earlier)", "0".to_string()),
            NumCmd::Conform => (
                "Conform — lead-in bars, end to (1 = whole bar, 0 = whole beat)",
                format!("{}, 1", g.map(|g| g.lead_in_bars(rate)).unwrap_or(0)),
            ),
            NumCmd::SelectBars => ("Select bars from cursor — count", "4".to_string()),
            NumCmd::SliceBars => ("Slice every N bars", "1".to_string()),
        };
        if g.is_none() && !matches!(cmd, NumCmd::Bpm | NumCmd::BeatsPerBar) {
            self.error("No beat grid: mark a bar (b/B or g) or press A to analyse");
            return;
        }
        self.open_prompt(PromptKind::Numbers(cmd), label, default);
    }

    fn run_numbers(&mut self, cmd: NumCmd, v: &[f64]) {
        let a = v.first().copied();
        match cmd {
            NumCmd::Bpm => self.set_bpm(a.unwrap_or(120.0)),
            NumCmd::BeatsPerBar => {
                let bpb = a.unwrap_or(4.0).clamp(1.0, 32.0) as u32;
                match self.grid.as_mut() {
                    Some(g) => g.beats_per_bar = bpb,
                    None => self.grid = Some(Grid { bpm: 120.0, anchor: 0, beats_per_bar: bpb }),
                }
                self.grid_msg();
            }
            NumCmd::Offset => self.grid_shift_ms(a.unwrap_or(0.0)),
            NumCmd::Conform => self.conform_bars(a.unwrap_or(0.0).round() as i64, v.get(1).copied().unwrap_or(1.0) >= 0.5),
            NumCmd::SelectBars => self.select_bars(a.unwrap_or(4.0)),
            NumCmd::SliceBars => self.slice_at_grid(Some(a.unwrap_or(1.0).max(1.0) as i64)),
        }
    }

    // ----- transport ------------------------------------------------------

    /// What Space would play right now: (start, end, loop).
    /// The selection wins; it loops only when it is the loop region. With no
    /// selection the loop region plays looped, otherwise play runs from the
    /// cursor to the end.
    fn play_plan(&self) -> (usize, usize, Option<(usize, usize)>) {
        let n = self.audio.frames();
        match (self.selection, self.loop_range) {
            (Some(sel), Some(lp)) if sel == lp => (sel.0, sel.1, Some(lp)),
            (Some((s, e)), _) => (s, e, None),
            (None, Some((ls, le))) => {
                let start = if self.cursor >= ls && self.cursor < le { self.cursor } else { ls };
                (start, le, Some((ls, le)))
            }
            (None, None) => (self.cursor.min(n.saturating_sub(1)), n, None),
        }
    }

    /// Push the current plan's end/loop to a running or paused player, so
    /// changing the loop takes effect immediately.
    fn sync_player_range(&mut self) {
        if self.transport_active() {
            let (_, end, lp) = self.play_plan();
            self.player.set_range(end, lp);
        }
    }

    fn toggle_play(&mut self) {
        if !self.player.available() {
            self.error("No audio output device");
            return;
        }
        if self.player.is_playing() {
            self.player.pause();
            self.paused = true;
            return;
        }
        if self.audio.is_empty() {
            self.error("No audio loaded");
            return;
        }
        let (start, end, lp) = self.play_plan();
        if self.paused {
            self.paused = false;
            let pos = self.player.position();
            if pos >= start && pos < end {
                // Same region as before: carry on, with any loop changes applied.
                self.player.set_range(end, lp);
                self.player.resume();
                return;
            }
            // Region changed while paused: start the new one from its beginning.
        }
        self.player.play(start, end, lp);
    }

    fn stop(&mut self) {
        self.player.stop();
        self.paused = false;
    }

    pub fn set_speed(&mut self, pct: i64) {
        let pct = pct.clamp(SPEED_MIN as i64, SPEED_MAX as i64) as u32;
        if pct != self.speed {
            self.speed = pct;
            self.player.set_speed_pct(pct);
        }
        self.info(format!("Speed {}%", self.speed));
    }

    fn nudge_speed(&mut self, delta: i64) {
        self.set_speed(self.speed as i64 + delta);
    }

    pub fn transport_active(&self) -> bool {
        self.player.is_playing() || self.paused
    }

    /// Short label for the title bar: what the transport is playing.
    pub fn playing_what(&self) -> &'static str {
        let (_, _, lp) = self.play_plan();
        if lp.is_some() {
            "loop"
        } else if self.selection.is_some() {
            "selection"
        } else {
            "from cursor"
        }
    }

    pub fn playhead(&self) -> Option<usize> {
        if self.transport_active() {
            Some(self.player.position())
        } else {
            None
        }
    }

    pub fn tick(&mut self) {
        // Nothing periodic for now; playback end is detected via is_playing().
    }

    // ----- view -----------------------------------------------------------

    pub fn width(&self) -> usize {
        self.wave_area.width.max(1) as usize
    }

    /// Called by the UI each frame with the waveform rectangle.
    pub fn layout_wave(&mut self, area: Rect) {
        let resized = area.width != self.wave_area.width;
        self.wave_area = area;
        if self.needs_fit || (resized && self.fpc >= self.fit_fpc()) {
            self.zoom_fit();
            self.needs_fit = false;
        }
        self.clamp_view();
    }

    fn fit_fpc(&self) -> f64 {
        (self.audio.frames() as f64 / self.width() as f64).max(1.0)
    }

    fn zoom_fit(&mut self) {
        self.fpc = self.fit_fpc();
        self.view_start = 0;
    }

    fn clamp_view(&mut self) {
        let span = (self.fpc * self.width() as f64) as usize;
        let max_start = self.audio.frames().saturating_sub(span);
        self.view_start = self.view_start.min(max_start);
    }

    pub fn col_to_frame(&self, col: usize) -> usize {
        (self.view_start as f64 + col as f64 * self.fpc) as usize
    }

    pub fn frame_to_col(&self, frame: usize) -> Option<usize> {
        if frame < self.view_start {
            return None;
        }
        let c = ((frame - self.view_start) as f64 / self.fpc) as usize;
        (c < self.width()).then_some(c)
    }

    fn zoom_at(&mut self, col: usize, factor: f64) {
        let anchor = self.col_to_frame(col);
        self.fpc = (self.fpc * factor).clamp(1.0, self.fit_fpc());
        let off = (col as f64 * self.fpc) as usize;
        self.view_start = anchor.saturating_sub(off);
        self.clamp_view();
    }

    fn zoom_cursor(&mut self, factor: f64) {
        let col = self.frame_to_col(self.cursor).unwrap_or(self.width() / 2);
        self.zoom_at(col, factor);
        self.ensure_cursor_visible();
    }

    fn zoom_selection(&mut self) {
        let Some((s, e)) = self.selection else {
            self.error("Nothing selected");
            return;
        };
        self.fpc = ((e - s) as f64 / self.width() as f64).clamp(1.0, self.fit_fpc());
        self.view_start = s;
        self.clamp_view();
    }

    fn scroll_cols(&mut self, cols: i64) {
        let delta = (cols as f64 * self.fpc) as i64;
        let vs = (self.view_start as i64 + delta).max(0) as usize;
        self.view_start = vs;
        self.clamp_view();
    }

    fn ensure_cursor_visible(&mut self) {
        let span = (self.fpc * self.width() as f64) as usize;
        if self.cursor < self.view_start {
            self.view_start = self.cursor.saturating_sub(span / 5);
        } else if self.cursor >= self.view_start + span {
            self.view_start = (self.cursor + span / 5).saturating_sub(span);
        }
        self.clamp_view();
    }

    fn move_cursor(&mut self, delta: i64, extend: bool) {
        let n = self.audio.frames() as i64;
        let target = (self.cursor as i64 + delta).clamp(0, n) as usize;
        self.set_cursor(target, extend);
    }

    fn set_cursor(&mut self, target: usize, extend: bool) {
        if extend {
            let anchor = match (self.sel_anchor, self.selection) {
                (Some(a), Some(_)) => a,
                (_, Some((s, e))) => if self.cursor == s { e } else { s },
                _ => self.cursor,
            };
            self.sel_anchor = Some(anchor);
            self.selection = Some((anchor.min(target), anchor.max(target))).filter(|(s, e)| e > s);
        } else {
            self.sel_anchor = None;
        }
        self.cursor = target;
        self.ensure_cursor_visible();
    }

    // ----- waveform cache -------------------------------------------------

    pub fn wave_columns(&mut self) -> &Vec<Vec<(f32, f32)>> {
        let key = WaveKey {
            view_start: self.view_start,
            fpc_bits: self.fpc.to_bits(),
            width: self.wave_area.width,
            version: self.version,
        };
        let stale = self.wave_cache.as_ref().is_none_or(|(k, _)| *k != key);
        if stale {
            let w = self.width();
            let n = self.audio.frames();
            let cols: Vec<Vec<(f32, f32)>> = (0..self.audio.n_channels())
                .map(|ch| {
                    (0..w)
                        .map(|c| {
                            let s = self.col_to_frame(c);
                            let e = self.col_to_frame(c + 1);
                            if s >= n {
                                (f32::NAN, f32::NAN)
                            } else {
                                audio::column_minmax(&self.audio, &self.peaks, ch, s, e)
                            }
                        })
                        .collect()
                })
                .collect();
            self.wave_cache = Some((key, cols));
        }
        &self.wave_cache.as_ref().unwrap().1
    }

    // ----- prompts & confirmations ---------------------------------------

    fn open_prompt(&mut self, kind: PromptKind, label: &str, input: String) {
        self.mode = Mode::Prompt(Prompt {
            kind,
            label: label.to_string(),
            input,
        });
    }

    fn submit_prompt(&mut self, kind: PromptKind, input: String) {
        let input = input.trim().to_string();
        if input.is_empty() {
            self.info("Cancelled");
            return;
        }
        let path = expand_tilde(&input);
        match kind {
            PromptKind::Open => {
                if self.dirty {
                    self.mode = Mode::Confirm {
                        question: "Discard unsaved changes and open another file?".into(),
                        action: ConfirmAction::Open(path),
                    };
                } else {
                    let r = self.load_file(&path);
                    self.report(r);
                }
            }
            PromptKind::SaveAs => {
                let r = self.save_as(path);
                self.report(r);
            }
            PromptKind::ExportSelection => {
                let r = self.export_selection(path);
                self.report(r);
            }
            PromptKind::ExportPieces => {
                let r = self.export_pieces(&path.to_string_lossy());
                self.report(r);
            }
            PromptKind::FxParams(_) | PromptKind::Numbers(_) => {
                let mut vals = Vec::new();
                for tok in input.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()) {
                    match tok.parse::<f64>() {
                        Ok(v) => vals.push(v),
                        Err(_) => {
                            self.error(format!("not a number: {tok}"));
                            return;
                        }
                    }
                }
                match kind {
                    PromptKind::FxParams(k) => self.run_fx(k, &vals),
                    PromptKind::Numbers(c) => self.run_numbers(c, &vals),
                    _ => unreachable!(),
                }
            }
            PromptKind::ExportMarkers => {
                let r = self.export_markers(path);
                self.report(r);
            }
        }
    }

    fn open_palette(&mut self) {
        self.mode = Mode::Palette(Palette { input: String::new(), selected: 0, matches: commands::search("") });
    }

    /// Execute a command. `from_palette` makes effects with parameters ask for them.
    pub fn run_cmd(&mut self, cmd: Cmd, from_palette: bool) {
        let page = (self.width() / 2) as i64;
        match cmd {
            Cmd::Open => {
                let dir = self
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(|d| {
                        let s = d.to_string_lossy().into_owned();
                        if s.is_empty() { s } else { s + "/" }
                    })
                    .unwrap_or_default();
                self.open_prompt(PromptKind::Open, "Open file", dir);
            }
            Cmd::Save => self.save(),
            Cmd::SaveAs => {
                let cur = self.path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                self.open_prompt(PromptKind::SaveAs, "Save as (.wav or .mp3)", cur);
            }
            Cmd::ExportPieces => {
                if self.slices.is_empty() {
                    self.error("No slice points: press m at each cut point, then e to export pieces");
                } else {
                    let def = self.default_export_pattern();
                    self.open_prompt(PromptKind::ExportPieces, "Export pieces to ({n} = piece number)", def);
                }
            }
            Cmd::ExportSelection => {
                if self.selection.is_none() {
                    self.error("Select a region first");
                } else {
                    let def = match &self.path {
                        Some(p) => {
                            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("selection");
                            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("wav");
                            let parent = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                            parent.join(format!("{stem}_sel.{ext}")).to_string_lossy().into_owned()
                        }
                        None => "selection.wav".into(),
                    };
                    self.open_prompt(PromptKind::ExportSelection, "Export selection to", def);
                }
            }
            Cmd::Quit => self.request_quit(),
            Cmd::Help => self.mode = Mode::Help,
            Cmd::Palette => self.open_palette(),
            Cmd::PlayPause => self.toggle_play(),
            Cmd::Stop => self.stop(),
            Cmd::CursorToPlayhead => {
                if let Some(p) = self.playhead() {
                    self.set_cursor(p.min(self.audio.frames()), false);
                }
            }
            Cmd::SelectAll => {
                let n = self.audio.frames();
                self.selection = (n > 0).then_some((0, n));
                self.sel_anchor = None;
            }
            Cmd::ClearSelection => {
                self.selection = None;
                self.sel_anchor = None;
            }
            Cmd::SelStartHere => {
                let n = self.audio.frames();
                let c = self.cursor;
                let e = self.selection.map(|(_, e)| e).filter(|&e| e > c).unwrap_or(n);
                self.selection = Some((c, e)).filter(|(s, e)| e > s);
                self.sel_anchor = None;
            }
            Cmd::SelEndHere => {
                let c = self.cursor;
                let s = self.selection.map(|(s, _)| s).filter(|&s| s < c).unwrap_or(0);
                self.selection = Some((s, c)).filter(|(s, e)| e > s);
                self.sel_anchor = None;
            }
            Cmd::GoStart => self.set_cursor(0, false),
            Cmd::GoEnd => self.set_cursor(self.audio.frames(), false),
            Cmd::LoopToggle => self.toggle_loop(),
            Cmd::LoopClear => {
                self.loop_range = None;
                self.info("Loop cleared");
                self.sync_player_range();
            }
            Cmd::SliceToggle => self.toggle_slice(),
            Cmd::SlicesClear => {
                self.slices.clear();
                self.info("All slice points cleared");
            }
            Cmd::SliceNext => self.jump_slice(true),
            Cmd::SlicePrev => self.jump_slice(false),
            Cmd::ZoomIn => self.zoom_cursor(0.5),
            Cmd::ZoomOut => self.zoom_cursor(2.0),
            Cmd::ZoomFit => self.zoom_fit(),
            Cmd::ZoomSel => self.zoom_selection(),
            Cmd::ScrollLeft => self.scroll_cols(-page),
            Cmd::ScrollRight => self.scroll_cols(page),
            Cmd::Cut => self.cut(),
            Cmd::Copy => self.copy(),
            Cmd::Paste => self.paste(),
            Cmd::Delete => self.delete_selection("Deleted"),
            Cmd::Trim => self.trim(),
            Cmd::Undo => self.undo(),
            Cmd::Redo => self.redo(),
            Cmd::SpeedUp => self.nudge_speed(1),
            Cmd::SpeedDown => self.nudge_speed(-1),
            Cmd::SpeedReset => self.set_speed(100),
            Cmd::KeepAliveToggle => {
                let mode = self.player.keep_alive().next();
                self.player.set_keep_alive(mode);
                self.info(format!("Output keep-alive {}", mode.label()));
            }
            Cmd::DetectBeats => self.detect_beats(),
            Cmd::GridSetBpm => self.num_prompt(NumCmd::Bpm),
            Cmd::GridBarStart => self.mark_bar_start(),
            Cmd::GridBarEnd => self.mark_bar_end(),
            Cmd::GridBarFromSelection => self.mark_bar_from_selection(),
            Cmd::GridNudgeEarlier => self.grid_shift_ms(-1.0),
            Cmd::GridNudgeLater => self.grid_shift_ms(1.0),
            Cmd::GridOffset => self.num_prompt(NumCmd::Offset),
            Cmd::GridBpmUp | Cmd::GridBpmDown | Cmd::GridDouble | Cmd::GridHalve => {
                if let Some(g) = self.grid_mut() {
                    g.bpm = match cmd {
                        Cmd::GridBpmUp => g.bpm + 0.1,
                        Cmd::GridBpmDown => g.bpm - 0.1,
                        Cmd::GridDouble => g.bpm * 2.0,
                        _ => g.bpm / 2.0,
                    }
                    .clamp(20.0, 400.0);
                    g.bpm = (g.bpm * 100.0).round() / 100.0;
                    self.grid_msg();
                }
            }
            Cmd::GridBeatsPerBar => self.num_prompt(NumCmd::BeatsPerBar),
            Cmd::GridClear => {
                self.grid = None;
                self.bar_start = None;
                self.bar_end = None;
                self.snap = false;
                self.info("Beat grid and bar marks cleared");
            }
            Cmd::SnapToggle => {
                if self.grid.is_none() {
                    self.error("No beat grid to snap to: mark a bar (b/B or g) or press A to analyse");
                } else {
                    self.snap = !self.snap;
                    self.info(if self.snap { "Snap on: ← → step beats, Ctrl ← → step bars, clicks snap" } else { "Snap off" });
                }
            }
            Cmd::ConformBars => self.num_prompt(NumCmd::Conform),
            Cmd::SelectBars => self.num_prompt(NumCmd::SelectBars),
            Cmd::SliceAtBars => self.num_prompt(NumCmd::SliceBars),
            Cmd::SliceAtBeats => self.slice_at_grid(None),
            Cmd::ExportMarkers => {
                if self.grid.is_none() {
                    self.error("No beat grid: mark a bar (b/B or g) or press A to analyse");
                } else {
                    let def = match &self.path {
                        Some(p) => {
                            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("markers");
                            let parent = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                            parent.join(format!("{stem}_markers.txt")).to_string_lossy().into_owned()
                        }
                        None => "markers.txt".into(),
                    };
                    self.open_prompt(PromptKind::ExportMarkers, "Export markers to (.txt, Audacity label format)", def);
                }
            }
            Cmd::Fx(kind) => {
                if from_palette && !fx::info(kind).params.is_empty() {
                    self.fx_prompt(kind);
                } else if kind == FxKind::Normalize {
                    let db = self.config.editor.normalize_db;
                    self.run_fx(kind, &[db]);
                } else {
                    self.run_fx(kind, &[]);
                }
            }
            Cmd::ReloadConfig => self.reload_config(),
            Cmd::ShowConfigPath => {
                let p = self.config_path.display().to_string();
                self.info(format!("Settings file: {p} — edit it, then run Reload settings"));
            }
        }
    }

    fn request_quit(&mut self) {
        if self.dirty {
            self.mode = Mode::Confirm {
                question: "Unsaved changes. Quit anyway?".into(),
                action: ConfirmAction::Quit,
            };
        } else {
            self.should_quit = true;
        }
    }

    fn run_confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Quit => self.should_quit = true,
            ConfirmAction::Open(p) => {
                let r = self.load_file(&p);
                self.report(r);
            }
        }
    }

    // ----- input ----------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        match mode {
            Mode::Help => {}
            Mode::Confirm { question, action } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => self.run_confirm(action),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                    self.info("Cancelled");
                }
                _ => self.mode = Mode::Confirm { question, action },
            },
            Mode::Prompt(mut p) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => self.info("Cancelled"),
                    KeyCode::Enter => self.submit_prompt(p.kind, p.input),
                    KeyCode::Backspace => {
                        p.input.pop();
                        self.mode = Mode::Prompt(p);
                    }
                    KeyCode::Char('u') if ctrl => {
                        p.input.clear();
                        self.mode = Mode::Prompt(p);
                    }
                    KeyCode::Char('w') if ctrl => {
                        let trimmed = p.input.trim_end_matches(|c: char| !c.is_whitespace() && c != '/');
                        let trimmed = trimmed.trim_end_matches(|c: char| c.is_whitespace() || c == '/');
                        p.input.truncate(trimmed.len());
                        self.mode = Mode::Prompt(p);
                    }
                    KeyCode::Char(c) if !ctrl => {
                        p.input.push(c);
                        self.mode = Mode::Prompt(p);
                    }
                    _ => self.mode = Mode::Prompt(p),
                }
            }
            Mode::Palette(mut pal) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let mut refilter = false;
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Enter => {
                        if let Some(&i) = pal.matches.get(pal.selected) {
                            let cmd = commands::all()[i].cmd;
                            self.run_cmd(cmd, true);
                        }
                        return;
                    }
                    KeyCode::Up => pal.selected = pal.selected.saturating_sub(1),
                    KeyCode::Char('p') | KeyCode::Char('k') if ctrl => pal.selected = pal.selected.saturating_sub(1),
                    KeyCode::Down => pal.selected += 1,
                    KeyCode::Char('n') | KeyCode::Char('j') if ctrl => pal.selected += 1,
                    KeyCode::PageUp => pal.selected = pal.selected.saturating_sub(10),
                    KeyCode::PageDown => pal.selected += 10,
                    KeyCode::Home => pal.selected = 0,
                    KeyCode::End => pal.selected = usize::MAX,
                    KeyCode::Backspace => {
                        pal.input.pop();
                        refilter = true;
                    }
                    KeyCode::Char('u') if ctrl => {
                        pal.input.clear();
                        refilter = true;
                    }
                    KeyCode::Char(ch) if !ctrl => {
                        pal.input.push(ch);
                        refilter = true;
                    }
                    _ => {}
                }
                if refilter {
                    pal.matches = commands::search(&pal.input);
                    pal.selected = 0;
                }
                pal.selected = pal.selected.min(pal.matches.len().saturating_sub(1));
                self.mode = Mode::Palette(pal);
            }
            Mode::Normal => self.on_normal_key(key),
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let col_frames = self.fpc.max(1.0) as i64;
        let step = if ctrl || alt { col_frames * 10 } else { col_frames };
        let big = shift || ctrl || alt;

        // Keys that need modifier/context handling the command table can't express.
        let snap_grid = if self.snap { self.grid } else { None };
        match key.code {
            KeyCode::Left | KeyCode::Right if snap_grid.is_some() => {
                let g = snap_grid.unwrap();
                let dir = if key.code == KeyCode::Left { -1 } else { 1 };
                let t = g.step(self.cursor, dir, ctrl || alt, self.audio.sample_rate, self.audio.frames());
                return self.set_cursor(t, shift);
            }
            KeyCode::Left => return self.move_cursor(-step, shift),
            KeyCode::Right => return self.move_cursor(step, shift),
            KeyCode::Home => return self.set_cursor(0, shift),
            KeyCode::End => return self.set_cursor(self.audio.frames(), shift),
            KeyCode::Up => return self.nudge_speed(if big { 10 } else { 1 }),
            KeyCode::Down => return self.nudge_speed(if big { -10 } else { -1 }),
            KeyCode::Esc => {
                return if self.transport_active() { self.stop() } else { self.run_cmd(Cmd::ClearSelection, false) };
            }
            _ => {}
        }

        let Some(&cmd) = self.keymap.get(&KeyChord::from_event(&key)) else { return };
        self.run_cmd(cmd, false);
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let area = self.wave_area;
        let in_x = m.column >= area.x && m.column < area.x + area.width;
        let in_y = m.row + 1 >= area.y && m.row <= area.y + area.height; // allow marker/timeline rows
        let col = m.column.saturating_sub(area.x).min(area.width.saturating_sub(1)) as usize;
        let sl = self.slider_area;
        let on_slider = sl.width > 1 && m.row == sl.y && m.column >= sl.x && m.column < sl.x + sl.width;
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if on_slider => {
                self.slider_drag = true;
                self.slider_to(m.column);
            }
            MouseEventKind::Drag(MouseButton::Left) if self.slider_drag => self.slider_to(m.column),
            MouseEventKind::Up(MouseButton::Left) if self.slider_drag => self.slider_drag = false,
            MouseEventKind::ScrollUp if on_slider => self.nudge_speed(1),
            MouseEventKind::ScrollDown if on_slider => self.nudge_speed(-1),
            MouseEventKind::Down(MouseButton::Left) if in_x && in_y => {
                let mut f = self.col_to_frame(col).min(self.audio.frames());
                if let (true, Some(g)) = (self.snap, self.grid) {
                    f = g.snap(f, self.audio.sample_rate, self.audio.frames());
                }
                self.set_cursor(f, false);
                self.sel_anchor = Some(f);
                self.mouse_drag = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.mouse_drag => {
                let mut f = self.col_to_frame(col).min(self.audio.frames());
                if let (true, Some(g)) = (self.snap, self.grid) {
                    f = g.snap(f, self.audio.sample_rate, self.audio.frames());
                }
                self.set_cursor(f, true);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.mouse_drag = false;
            }
            MouseEventKind::ScrollUp if in_x => self.zoom_at(col, 0.5),
            MouseEventKind::ScrollDown if in_x => self.zoom_at(col, 2.0),
            MouseEventKind::ScrollLeft => self.scroll_cols(-4),
            MouseEventKind::ScrollRight => self.scroll_cols(4),
            _ => {}
        }
    }
}

impl App {
    /// Map a terminal column on the slider track to a speed percentage.
    fn slider_to(&mut self, column: u16) {
        let sl = self.slider_area;
        if sl.width < 2 {
            return;
        }
        let x = column.saturating_sub(sl.x).min(sl.width - 1) as f64;
        let frac = x / (sl.width - 1) as f64;
        let pct = SPEED_MIN as f64 + frac * (SPEED_MAX - SPEED_MIN) as f64;
        self.set_speed(pct.round() as i64);
    }
}

/// Just the file name, for short status messages.
pub fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Format a number for prompts: no trailing zeros.
pub fn trim_num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

pub fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(s)
}
