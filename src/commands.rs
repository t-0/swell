//! Command registry shared by the key map and the command palette.

use crate::fx::{self, FxKind};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Cmd {
    Open,
    Save,
    SaveAs,
    ExportPieces,
    ExportSelection,
    Quit,
    Help,
    Palette,
    PlayPause,
    Stop,
    CursorToPlayhead,
    SelectAll,
    ClearSelection,
    SelStartHere,
    SelEndHere,
    GoStart,
    GoEnd,
    LoopToggle,
    LoopClear,
    SliceToggle,
    SlicesClear,
    SliceNext,
    SlicePrev,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ZoomSel,
    ScrollLeft,
    ScrollRight,
    Cut,
    Copy,
    Paste,
    Delete,
    Trim,
    Undo,
    Redo,
    SpeedUp,
    SpeedDown,
    SpeedReset,
    KeepAliveToggle,
    ReloadConfig,
    ShowConfigPath,
    DetectBeats,
    GridSetBpm,
    GridBarStart,
    GridBarEnd,
    GridBarFromSelection,
    GridNudgeEarlier,
    GridNudgeLater,
    GridOffset,
    GridBpmUp,
    GridBpmDown,
    GridDouble,
    GridHalve,
    GridBeatsPerBar,
    GridClear,
    SnapToggle,
    ConformBars,
    SelectBars,
    SliceAtBars,
    SliceAtBeats,
    ExportMarkers,
    Fx(FxKind),
}

pub struct CommandInfo {
    pub cmd: Cmd,
    pub name: &'static str,
    pub category: &'static str,
    /// Stable id used in the settings file.
    pub id: &'static str,
}

const fn c(cmd: Cmd, id: &'static str, name: &'static str, category: &'static str) -> CommandInfo {
    CommandInfo { cmd, name, category, id }
}

const BASE: &[CommandInfo] = &[
    c(Cmd::Open, "open", "Open file", "file"),
    c(Cmd::Save, "save", "Save", "file"),
    c(Cmd::SaveAs, "save_as", "Save as", "file"),
    c(Cmd::ExportSelection, "export_selection", "Export selection", "file"),
    c(Cmd::ExportPieces, "export_pieces", "Export pieces between slice points", "file"),
    c(Cmd::Quit, "quit", "Quit", "file"),
    c(Cmd::ShowConfigPath, "show_config_path", "Settings: show the config file path (~/.swell.toml)", "file"),
    c(Cmd::ReloadConfig, "reload_config", "Reload settings from ~/.swell.toml", "file"),
    c(Cmd::Help, "help", "Help: key reference", "view"),
    c(Cmd::Palette, "palette", "Command palette", "view"),
    c(Cmd::PlayPause, "play_pause", "Play / pause", "transport"),
    c(Cmd::Stop, "stop", "Stop", "transport"),
    c(Cmd::CursorToPlayhead, "cursor_to_playhead", "Move cursor to playhead", "transport"),
    c(Cmd::SpeedUp, "speed_up", "Speed +1%", "transport"),
    c(Cmd::SpeedDown, "speed_down", "Speed -1%", "transport"),
    c(Cmd::SpeedReset, "speed_reset", "Speed 100%", "transport"),
    c(Cmd::KeepAliveToggle, "keep_alive_toggle", "Output keep-alive while idle: cycle off / dither / tone (stops device mute & fade-in)", "transport"),
    c(Cmd::SelectAll, "select_all", "Select all", "select"),
    c(Cmd::ClearSelection, "clear_selection", "Clear selection", "select"),
    c(Cmd::SelStartHere, "sel_start_here", "Selection start = cursor", "select"),
    c(Cmd::SelEndHere, "sel_end_here", "Selection end = cursor", "select"),
    c(Cmd::GoStart, "go_start", "Go to start", "select"),
    c(Cmd::GoEnd, "go_end", "Go to end", "select"),
    c(Cmd::LoopToggle, "loop_toggle", "Loop = selection (toggle)", "loop"),
    c(Cmd::LoopClear, "loop_clear", "Clear loop", "loop"),
    c(Cmd::SliceToggle, "slice_toggle", "Add / remove slice point at cursor", "slice"),
    c(Cmd::SlicesClear, "slices_clear", "Clear all slice points", "slice"),
    c(Cmd::SliceNext, "slice_next", "Next slice point", "slice"),
    c(Cmd::SlicePrev, "slice_prev", "Previous slice point", "slice"),
    c(Cmd::ZoomIn, "zoom_in", "Zoom in", "view"),
    c(Cmd::ZoomOut, "zoom_out", "Zoom out", "view"),
    c(Cmd::ZoomFit, "zoom_fit", "Zoom to fit", "view"),
    c(Cmd::ZoomSel, "zoom_sel", "Zoom to selection", "view"),
    c(Cmd::ScrollLeft, "scroll_left", "Scroll left", "view"),
    c(Cmd::ScrollRight, "scroll_right", "Scroll right", "view"),
    c(Cmd::GridBarStart, "grid_bar_start", "Mark bar start = cursor (downbeat)", "beats"),
    c(Cmd::GridBarEnd, "grid_bar_end", "Mark bar end = cursor (sets tempo from the bar)", "beats"),
    c(Cmd::GridBarFromSelection, "grid_bar_from_selection", "Mark bar = selection / loop (sets tempo)", "beats"),
    c(Cmd::DetectBeats, "detect_beats", "Beat analysis: detect BPM & beats, seeded by the marked bar", "beats"),
    c(Cmd::GridSetBpm, "grid_set_bpm", "Grid: set BPM…", "beats"),
    c(Cmd::GridNudgeEarlier, "grid_nudge_earlier", "Grid: nudge earlier 1 ms", "beats"),
    c(Cmd::GridNudgeLater, "grid_nudge_later", "Grid: nudge later 1 ms", "beats"),
    c(Cmd::GridOffset, "grid_offset", "Grid: shift by ms…", "beats"),
    c(Cmd::GridBpmDown, "grid_bpm_down", "Grid: BPM -0.1", "beats"),
    c(Cmd::GridBpmUp, "grid_bpm_up", "Grid: BPM +0.1", "beats"),
    c(Cmd::GridDouble, "grid_double", "Grid: double BPM", "beats"),
    c(Cmd::GridHalve, "grid_halve", "Grid: halve BPM", "beats"),
    c(Cmd::GridBeatsPerBar, "grid_beats_per_bar", "Grid: beats per bar…", "beats"),
    c(Cmd::GridClear, "grid_clear", "Grid: clear", "beats"),
    c(Cmd::SnapToggle, "snap_toggle", "Snap cursor/selection to beats (toggle)", "beats"),
    c(Cmd::ConformBars, "conform_bars", "Conform to bars: lead-in N bars, pad end to whole bar…", "beats"),
    c(Cmd::SelectBars, "select_bars", "Select N bars from cursor…", "beats"),
    c(Cmd::SliceAtBars, "slice_at_bars", "Slice at every N bars…", "beats"),
    c(Cmd::SliceAtBeats, "slice_at_beats", "Slice at every beat", "beats"),
    c(Cmd::ExportMarkers, "export_markers", "Export bar/beat markers (Audacity labels .txt)…", "beats"),
    c(Cmd::Cut, "cut", "Cut", "edit"),
    c(Cmd::Copy, "copy", "Copy", "edit"),
    c(Cmd::Paste, "paste", "Paste", "edit"),
    c(Cmd::Delete, "delete", "Delete selection", "edit"),
    c(Cmd::Trim, "trim", "Trim to selection", "edit"),
    c(Cmd::Undo, "undo", "Undo", "edit"),
    c(Cmd::Redo, "redo", "Redo", "edit"),
];

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

pub fn all() -> &'static [CommandInfo] {
    static ALL: OnceLock<Vec<CommandInfo>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v: Vec<CommandInfo> = BASE.iter().map(|b| c(b.cmd, b.id, b.name, b.category)).collect();
        for i in fx::ALL {
            let id: &'static str = Box::leak(format!("fx.{}", snake(&format!("{:?}", i.kind))).into_boxed_str());
            v.push(c(Cmd::Fx(i.kind), id, i.name, i.category));
        }
        v
    })
}

pub fn id_of(cmd: Cmd) -> &'static str {
    all().iter().find(|c| c.cmd == cmd).map(|c| c.id).unwrap_or("?")
}

pub fn by_id(id: &str) -> Option<Cmd> {
    all().iter().find(|c| c.id == id).map(|c| c.cmd)
}

/// Default key bindings. Arrows, Esc and the mouse are handled directly.
pub const DEFAULT_KEYS: &[(&str, Cmd)] = &[
    ("q", Cmd::Quit),
    ("ctrl+c", Cmd::Quit),
    ("?", Cmd::Help),
    ("f1", Cmd::Help),
    (":", Cmd::Palette),
    ("ctrl+p", Cmd::Palette),
    ("ctrl+k", Cmd::Palette),
    ("o", Cmd::Open),
    ("w", Cmd::Save),
    ("ctrl+s", Cmd::Save),
    ("W", Cmd::SaveAs),
    ("e", Cmd::ExportPieces),
    ("E", Cmd::ExportSelection),
    ("space", Cmd::PlayPause),
    ("enter", Cmd::Stop),
    ("p", Cmd::CursorToPlayhead),
    ("1", Cmd::SpeedReset),
    ("[", Cmd::SelStartHere),
    ("]", Cmd::SelEndHere),
    ("a", Cmd::SelectAll),
    ("tab", Cmd::SliceNext),
    ("backtab", Cmd::SlicePrev),
    ("pgup", Cmd::ScrollLeft),
    ("<", Cmd::ScrollLeft),
    (",", Cmd::ScrollLeft),
    ("pgdn", Cmd::ScrollRight),
    (">", Cmd::ScrollRight),
    (".", Cmd::ScrollRight),
    ("l", Cmd::LoopToggle),
    ("L", Cmd::LoopClear),
    ("m", Cmd::SliceToggle),
    ("M", Cmd::SlicesClear),
    ("+", Cmd::ZoomIn),
    ("=", Cmd::ZoomIn),
    ("-", Cmd::ZoomOut),
    ("_", Cmd::ZoomOut),
    ("0", Cmd::ZoomFit),
    ("z", Cmd::ZoomSel),
    ("x", Cmd::Cut),
    ("c", Cmd::Copy),
    ("v", Cmd::Paste),
    ("del", Cmd::Delete),
    ("backspace", Cmd::Delete),
    ("t", Cmd::Trim),
    ("u", Cmd::Undo),
    ("ctrl+z", Cmd::Undo),
    ("U", Cmd::Redo),
    ("ctrl+y", Cmd::Redo),
    ("ctrl+r", Cmd::Redo),
    ("b", Cmd::GridBarStart),
    ("B", Cmd::GridBarEnd),
    ("g", Cmd::GridBarFromSelection),
    ("A", Cmd::DetectBeats),
    ("s", Cmd::SnapToggle),
    ("C", Cmd::ConformBars),
    ("{", Cmd::GridNudgeEarlier),
    ("}", Cmd::GridNudgeLater),
    ("(", Cmd::GridBpmDown),
    (")", Cmd::GridBpmUp),
    ("f", Cmd::Fx(FxKind::FadeIn)),
    ("F", Cmd::Fx(FxKind::FadeOut)),
    ("n", Cmd::Fx(FxKind::Normalize)),
    ("r", Cmd::Fx(FxKind::Reverse)),
];

/// A key with modifiers. For character keys Shift is implied by the case of
/// the character and dropped from `mods`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyChord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl KeyChord {
    pub fn from_event(k: &KeyEvent) -> KeyChord {
        let mods = match k.code {
            KeyCode::Char(_) => k.modifiers - KeyModifiers::SHIFT,
            _ => k.modifiers,
        };
        KeyChord { code: k.code, mods }
    }
}

/// Parse "ctrl+s", "W", "space", "shift+tab", "f1"…
pub fn parse_key(spec: &str) -> Option<KeyChord> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = spec.split('+').collect();
    // the plus key itself is written "+" or "ctrl++"
    if spec == "+" || spec.ends_with("++") {
        parts.pop();
        parts.pop();
        parts.push("+");
    }
    let key = parts.pop()?;
    let mut mods = KeyModifiers::empty();
    for m in parts {
        match m.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "c" => mods |= KeyModifiers::CONTROL,
            "alt" | "opt" | "option" | "meta" => mods |= KeyModifiers::ALT,
            "shift" | "s" => mods |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }
    let lower = key.to_ascii_lowercase();
    let code = match lower.as_str() {
        "space" | "spc" => KeyCode::Char(' '),
        "enter" | "return" | "ret" | "cr" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" | "bs" => KeyCode::Backspace,
        "del" | "delete" => KeyCode::Delete,
        "ins" | "insert" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pgup" | "pageup" => KeyCode::PageUp,
        "pgdn" | "pagedown" => KeyCode::PageDown,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        _ => {
            if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n)) {
                KeyCode::F(n)
            } else {
                let mut chars = key.chars();
                let ch = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                let ch = if mods.contains(KeyModifiers::SHIFT) && ch.is_alphabetic() { ch.to_ascii_uppercase() } else { ch };
                KeyCode::Char(ch)
            }
        }
    };
    if code == KeyCode::Tab && mods.contains(KeyModifiers::SHIFT) {
        return Some(KeyChord { code: KeyCode::BackTab, mods: KeyModifiers::empty() });
    }
    if matches!(code, KeyCode::Char(_)) {
        mods -= KeyModifiers::SHIFT;
    }
    Some(KeyChord { code, mods })
}

/// Human-readable form of a chord for the palette.
pub fn format_key(k: &KeyChord) -> String {
    let mut s = String::new();
    if k.mods.contains(KeyModifiers::CONTROL) {
        s += "Ctrl+";
    }
    if k.mods.contains(KeyModifiers::ALT) {
        s += "Alt+";
    }
    if k.mods.contains(KeyModifiers::SHIFT) {
        s += "Shift+";
    }
    s += &match k.code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "Shift+Tab".into(),
        KeyCode::Backspace => "Bksp".into(),
        KeyCode::Delete => "Del".into(),
        KeyCode::Insert => "Ins".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        KeyCode::Left => "←".into(),
        KeyCode::Right => "→".into(),
        KeyCode::Up => "↑".into(),
        KeyCode::Down => "↓".into(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    };
    s
}

/// Fuzzy subsequence match of `query` against `text`. Higher is better; None = no match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.chars().flat_map(|c| c.to_lowercase()).collect();
    let mut score = 0i32;
    let mut qi = 0usize;
    let mut last_match: Option<usize> = None;
    for (ti, &tc) in t.iter().enumerate() {
        if qi < q.len() && tc == q[qi] {
            let word_start = ti == 0 || !t[ti - 1].is_alphanumeric();
            score += 1;
            if word_start {
                score += 6;
            }
            if let Some(lm) = last_match {
                if lm + 1 == ti {
                    score += 4;
                }
            }
            if ti == 0 {
                score += 4;
            }
            last_match = Some(ti);
            qi += 1;
        }
    }
    if qi == q.len() {
        Some(score - (last_match.unwrap_or(0) as i32) / 4)
    } else {
        None
    }
}

/// Indices into `all()` matching `query`, best first.
pub fn search(query: &str) -> Vec<usize> {
    let cmds = all();
    let mut scored: Vec<(i32, usize)> = cmds
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let hay = format!("{} {}", c.category, c.name);
            fuzzy_score(query, &hay).map(|s| (s, i))
        })
        .collect();
    if !query.trim().is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_parsing() {
        let k = parse_key("ctrl+s").unwrap();
        assert_eq!(k, KeyChord { code: KeyCode::Char('s'), mods: KeyModifiers::CONTROL });
        assert_eq!(parse_key("W").unwrap(), KeyChord { code: KeyCode::Char('W'), mods: KeyModifiers::empty() });
        assert_eq!(parse_key("shift+w").unwrap(), parse_key("W").unwrap());
        assert_eq!(parse_key("shift+tab").unwrap().code, KeyCode::BackTab);
        assert_eq!(parse_key("F1").unwrap().code, KeyCode::F(1));
        assert_eq!(parse_key("+").unwrap().code, KeyCode::Char('+'));
        assert_eq!(parse_key("ctrl++").unwrap(), KeyChord { code: KeyCode::Char('+'), mods: KeyModifiers::CONTROL });
        assert!(parse_key("ctrl+").is_none());
        assert!(parse_key("hyper+x").is_none());
        assert!(parse_key("ab").is_none());
        let ev = KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT);
        assert_eq!(KeyChord::from_event(&ev), parse_key("W").unwrap());
        for (k, _) in DEFAULT_KEYS {
            assert!(parse_key(k).is_some(), "default key {k}");
        }
        assert_eq!(id_of(Cmd::Fx(FxKind::LowPass)), "fx.low_pass");
        assert_eq!(by_id("save_as"), Some(Cmd::SaveAs));
        assert_eq!(format_key(&parse_key("ctrl+s").unwrap()), "Ctrl+s");
    }

    #[test]
    fn fuzzy_prefers_word_starts() {
        let r = search("rev");
        let names: Vec<&str> = r.iter().take(2).map(|&i| all()[i].name).collect();
        assert!(names.contains(&"Reverb") && names.contains(&"Reverse"), "{names:?}");
        assert!(search("zzzz").is_empty());
        assert_eq!(search("").len(), all().len());
        let top = all()[search("fade in")[0]].name;
        assert_eq!(top, "Fade in");
    }
}
