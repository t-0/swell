//! Settings file: `~/.swell.toml` (or `$SWELL_CONFIG`). Created with
//! commented defaults when missing; a broken file is reported and ignored.

use crate::commands::{self, Cmd, KeyChord};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    /// "off", "dither" or "tone"
    pub keep_alive: String,
    pub dither_db: f64,
    pub tone_db: f64,
    pub tone_hz: f64,
}

impl Default for Output {
    fn default() -> Self {
        Output { keep_alive: "tone".into(), dither_db: -96.0, tone_db: -60.0, tone_hz: 15.0 }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Editor {
    pub beats_per_bar: u32,
    pub normalize_db: f64,
    pub speed: u32,
    pub snap: bool,
    pub undo_levels: usize,
    pub mp3_kbps: u32,
    /// "source", "16", "24" or "32"
    pub wav_bits: String,
    pub mouse: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            beats_per_bar: 4,
            normalize_db: 0.0,
            speed: 100,
            snap: false,
            undo_levels: 32,
            mp3_kbps: 192,
            wav_bits: "source".into(),
            mouse: true,
        }
    }
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub output: Output,
    pub editor: Editor,
    /// key chord → command id, e.g. `"ctrl+s" = "save"`; `"none"` unbinds.
    pub keys: HashMap<String, String>,
}

pub struct Loaded {
    pub config: Config,
    pub path: PathBuf,
    pub created: bool,
    /// False when the file could not be parsed (config holds defaults).
    pub parsed: bool,
    pub warnings: Vec<String>,
}

pub fn path() -> PathBuf {
    if let Ok(p) = std::env::var("SWELL_CONFIG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".swell.toml")
}

/// Read the settings file, creating it with defaults if it does not exist.
pub fn load() -> Loaded {
    let path = path();
    let mut warnings = Vec::new();
    let mut created = false;
    if !path.exists() {
        match std::fs::write(&path, default_file_text()) {
            Ok(()) => created = true,
            Err(e) => warnings.push(format!("could not create {}: {e}", path.display())),
        }
        return Loaded { config: Config::default(), path, created, parsed: true, warnings };
    }
    let (config, parsed) = match read(&path) {
        Ok(c) => (c, true),
        Err(e) => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            warnings.push(format!("{name} ignored, {e:#}"));
            (Config::default(), false)
        }
    };
    for w in validate(&config) {
        warnings.push(w);
    }
    Loaded { config, path, created, parsed, warnings }
}

fn read(path: &std::path::Path) -> Result<Config> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let cfg: Config = toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}", short_toml_error(&e.to_string())))?;
    Ok(cfg)
}

/// Collapse toml's multi-line diagnostic into "line N: reason".
fn short_toml_error(full: &str) -> String {
    let lines: Vec<&str> = full.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    let loc = lines
        .first()
        .and_then(|l| l.strip_prefix("TOML parse error at "))
        .map(|l| l.split(',').next().unwrap_or(l).to_string())
        .unwrap_or_default();
    let reason = lines.iter().rev().find(|l| !l.starts_with('|') && !l.starts_with('^') && !l.contains("| ")).copied().unwrap_or("parse error");
    let reason: String = reason.chars().take(90).collect();
    if loc.is_empty() {
        reason
    } else {
        format!("{loc}: {reason}")
    }
}

fn validate(c: &Config) -> Vec<String> {
    let mut w = Vec::new();
    if !matches!(c.output.keep_alive.as_str(), "off" | "dither" | "tone") {
        w.push(format!("output.keep_alive = {:?} is not off/dither/tone", c.output.keep_alive));
    }
    if !matches!(c.editor.wav_bits.as_str(), "source" | "16" | "24" | "32") {
        w.push(format!("editor.wav_bits = {:?} is not source/16/24/32", c.editor.wav_bits));
    }
    if !(50..=200).contains(&c.editor.speed) {
        w.push("editor.speed must be 50..200".into());
    }
    for (k, v) in &c.keys {
        if commands::parse_key(k).is_none() {
            w.push(format!("keys: cannot parse key {k:?}"));
        }
        if v != "none" && commands::by_id(v).is_none() {
            w.push(format!("keys: unknown command {v:?} for {k:?}"));
        }
    }
    w
}

/// Default bindings overlaid with the user's `[keys]` table.
pub fn keymap(c: &Config) -> HashMap<KeyChord, Cmd> {
    let mut map: HashMap<KeyChord, Cmd> = commands::DEFAULT_KEYS
        .iter()
        .filter_map(|(k, cmd)| commands::parse_key(k).map(|chord| (chord, *cmd)))
        .collect();
    for (k, v) in &c.keys {
        let Some(chord) = commands::parse_key(k) else { continue };
        if v == "none" {
            map.remove(&chord);
        } else if let Some(cmd) = commands::by_id(v) {
            map.insert(chord, cmd);
        }
    }
    map
}

pub fn default_file_text() -> String {
    let mut s = String::from(
        r#"# swell settings. Lines starting with # are the defaults; uncomment to change.
# Delete this file to regenerate it. Reload inside swell with the palette
# command "Reload settings" (or restart).

[output]
# Signal sent while nothing is playing, so the output device never mutes and
# fades in on the first sound: "off" (digital silence), "dither", or "tone"
# (dither plus a sub-audible tone, strongest).
# keep_alive = "tone"
# Dither level, dBFS peak. -96 is below 16-bit resolution.
# dither_db = -96.0
# Tone level and frequency for keep_alive = "tone". 15 Hz at -60 dBFS is inaudible.
# tone_db = -60.0
# tone_hz = 15.0

[editor]
# Beats per bar for new grids and beat analysis.
# beats_per_bar = 4
# Peak target for the quick normalize key (n), dBFS.
# normalize_db = 0.0
# Playback speed at start, percent (50..200).
# speed = 100
# Start with snap-to-grid on (needs a grid).
# snap = false
# undo_levels = 32
# MP3 bitrate for .mp3 saves: 8..320 (nearest standard rate is used).
# mp3_kbps = 192
# WAV bit depth: "source" keeps the loaded file's, or "16", "24", "32" (float).
# wav_bits = "source"
# Mouse: click sets cursor, drag selects, wheel zooms.
# mouse = true

[keys]
# key = "command id". Keys: a letter or symbol (upper case = shift), or
# space, enter, esc, tab, backtab, backspace, del, home, end, pgup, pgdn,
# f1..f12, with ctrl+ / alt+ / shift+ prefixes, e.g. "ctrl+s" = "save".
# Bind to "none" to remove a default. Arrows, Esc and mouse are fixed.
#
# Defaults:
"#,
    );
    for (k, cmd) in commands::DEFAULT_KEYS {
        s += &format!("# {:<12} = \"{}\"\n", format!("\"{k}\""), commands::id_of(*cmd));
    }
    s += "#\n# All command ids:\n";
    for c in commands::all() {
        s += &format!("#   {:<28} {}\n", commands::id_of(c.cmd), c.name);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_and_matches_defaults() {
        let text = default_file_text();
        let cfg: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg.editor.mp3_kbps, 192);
        assert_eq!(cfg.output.keep_alive, "tone");
        assert!(cfg.keys.is_empty());
        // uncommenting every default binding must still parse and validate
        let active: String = text.lines().map(|l| if l.starts_with("# \"") { l[2..].to_string() } else { l.to_string() }).collect::<Vec<_>>().join("\n");
        let cfg: Config = toml::from_str(&active).unwrap();
        assert!(!cfg.keys.is_empty());
        assert!(validate(&cfg).is_empty(), "{:?}", validate(&cfg));
    }

    #[test]
    fn overrides_and_unbinds() {
        let cfg: Config = toml::from_str(
            r#"
[output]
keep_alive = "dither"
dither_db = -80
[editor]
speed = 90
[keys]
"q" = "none"
"ctrl+q" = "quit"
"F" = "fx.reverb"
"#,
        )
        .unwrap();
        assert!(validate(&cfg).is_empty(), "{:?}", validate(&cfg));
        let map = keymap(&cfg);
        assert!(!map.contains_key(&commands::parse_key("q").unwrap()));
        assert_eq!(map[&commands::parse_key("ctrl+q").unwrap()], Cmd::Quit);
        assert_eq!(map[&commands::parse_key("F").unwrap()], Cmd::Fx(crate::fx::FxKind::Reverb));
        assert_eq!(map[&commands::parse_key("ctrl+s").unwrap()], Cmd::Save);
        let bad: Config = toml::from_str("[keys]\n\"ctrl+\" = \"nope\"\n[output]\nkeep_alive = \"loud\"").unwrap();
        assert_eq!(validate(&bad).len(), 3);
        assert!(toml::from_str::<Config>("[editor]\nbogus = 1").is_err());
        let e = toml::from_str::<Config>("[editor]\nbogus = 1").unwrap_err().to_string();
        let short = short_toml_error(&e);
        assert!(short.starts_with("line 2: unknown field `bogus`"), "{short}");
    }
}
