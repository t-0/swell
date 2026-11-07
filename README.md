# swell

A terminal audio editor for WAV and MP3 (also reads FLAC and AIFF).

```
cargo build --release
./target/release/swell song.wav
```

Inside the editor press `?` for the full key reference.

## What it does

- **Load / save / save as** by extension: `.wav` (16/24/32-bit, matching the source) or `.mp3` (LAME, 192 kbps).
- **Waveform view** with per-channel lanes, zoom (`+` `-` `0` `z`, mouse wheel) and scrolling.
- **Select** with Shift+arrows, `[` `]`, `a`, or mouse drag.
- **Loop**: `l` turns the selection into a loop region; Space plays it round and round.
- **Play / pause / stop**: Space / Space / Enter. Plays the selection (looped when it is the loop region), else the loop, else from the cursor.
- **Speed slider**: 50–200% in 1% steps with `↑` `↓` (Shift for ±10%, `1` resets), or click, drag and wheel on the track. Tape-style varispeed, so pitch follows speed.
- **Edit**: cut, copy, paste, delete, trim to selection, with undo/redo.
- **Command palette** (`:` or Ctrl+P): fuzzy-search every command and effect, VS Code style. Shortcuts are shown beside each entry. Effects with parameters prompt for them with defaults prefilled, comma separated.
- **45 effects** (selection, or whole file when nothing is selected; undoable):
  - amplitude: gain, normalize, fade in/out with curve, invert polarity, silence, remove DC
  - dynamics: compressor, limiter, noise gate
  - filters / EQ: low-pass, high-pass, band-pass, notch, low shelf, high shelf, peaking, telephone
  - delay / reverb: delay, ping-pong delay, Freeverb-style reverb (tails ring on past the selection)
  - modulation: chorus, flanger, phaser, vibrato, tremolo, auto-pan, ring modulator
  - distortion: bit crusher, overdrive, hard clip
  - time / pitch: reverse, change speed, time stretch (keeps pitch, WSOLA), pitch shift (keeps length), repeat, insert silence, trim silence, resample file
  - stereo: swap channels, mix to mono, stereo width, pan
  - generate: tone, white noise
  - quick keys: fade in `f`, fade out `F`, normalize `n`, reverse `r`
- **Slices / pieces**: `m` drops a slice point at the cursor (Tab jumps between them). `e` exports every piece between slice points as its own file, numbered with `{n}`, e.g. `song_{n}.wav`. `E` exports just the selection.

## Beat grid and DAW export

Files load plainly. The beat grid is something you switch on:

1. **Mark a bar** you can hear: put the cursor on a downbeat and press `b`, then on the next downbeat and press `B`. Or select one bar (or set a loop over it) and press `g`. A full bar sets the tempo and the grid appears at once, with your bar start as the downbeat.
2. **Analyse** with `A`. Beat tracking (spectral-flux onsets, autocorrelation tempo, dynamic-programming beat tracking, constant-tempo fit) runs over the whole file. If you marked a bar it is the prior: tempo is searched within ±8% of your bar and refined from the tracked beats (a mark more than 6% off is kept as yours and reported), and the downbeat stays exactly where you put it. With only a bar start marked the tempo is free but the downbeat is yours. With no marks it finds both, picking the downbeat from kick-band energy.
3. **Check and adjust**: `(` `)` trim BPM by 0.1, `{` `}` shift the grid by 1 ms, `b` moves the downbeat. The palette has set BPM, beats per bar, double/halve, shift by ms, clear.
4. **Snap** (`s`): `← →` step beats, Ctrl `← →` step bars, Shift extends, mouse clicks snap. "Select N bars from cursor" is in the palette.
5. **Conform** (`C`) for the DAW: pads or trims the lead-in to exactly N bars and pads the end to a whole bar, so you drop the file at bar 1 and your marked downbeat sits at bar N+1. Loops, slices and bar marks move with the audio.

Saving a WAV writes an **ACID chunk** (tempo, beat count, meter) that Ableton Live, FL Studio, Reaper, Cubase and Logic read for tempo matching and warping, plus **cue markers** labelled `Bar n` (and `Slice n`) that Reaper, Pro Tools, Audition and others show as markers. Both are read back on reopen; exported pieces carry the tempo. "Slice at every N bars" fills slice points from the grid so `e` exports bar-exact loops. "Export markers" writes an Audacity label file with every beat.

Bar numbering follows DAW convention with the file at bar 1: a partial lead-in before the first downbeat is bar 1 and the first full bar is bar 2 until you conform.

## Keys

| Key | Action |
| --- | --- |
| `o` | open file |
| `w` / `Ctrl+S` | save |
| `W` | save as |
| `e` / `E` | export pieces / export selection |
| `Space` / `Enter` | play-pause / stop |
| `↑` `↓` / `1` | playback speed ±1% (Shift ±10%) / reset to 100% |
| `p` | move cursor to playhead |
| `← →` (`Shift` extends, `Ctrl` ×10) | move cursor / select |
| `Home` `End` `[` `]` `a` `Esc` | go to start/end, set selection start/end, select all, clear |
| `l` / `L` | loop = selection (toggle) / clear loop |
| `m` / `M` / `Tab` | add-remove slice point / clear all / jump to next |
| `x` `c` `v` `Del` `t` | cut, copy, paste, delete, trim |
| `u` / `U` | undo / redo |
| `f` `F` `n` `r` | fade in, fade out, normalize, reverse |
| `+` `-` `0` `z` | zoom in, out, fit, to selection |
| `PgUp` `PgDn` | scroll |
| `:` / `Ctrl+P` | command palette |
| `b` / `B` / `g` | mark bar start / bar end at cursor / bar = selection |
| `A` | beat analysis (seeded by the marked bar) |
| `s` / `C` | snap to grid / conform to bars |
| `( )` `{ }` | BPM ∓0.1 / shift grid ∓1 ms |
| `?` / `q` | help / quit |

Mouse: click sets the cursor, drag selects, wheel zooms around the pointer.

## Settings: `~/.swell.toml`

Created with commented defaults on first run (override the location with `$SWELL_CONFIG`). Uncomment a line to change it; delete the file to regenerate it. "Reload settings" in the palette applies edits live.

```toml
[output]
keep_alive = "tone"     # "off" | "dither" | "tone": signal while idle so the device never mutes
dither_db = -96.0       # dither level, dBFS peak
tone_db = -60.0         # 15 Hz tone level for "tone"
tone_hz = 15.0

[editor]
beats_per_bar = 4
normalize_db = 0.0      # target for the quick n key
speed = 100             # playback speed at start, 50..200
snap = false
undo_levels = 32
mp3_kbps = 192
wav_bits = "source"     # or "16" | "24" | "32"
mouse = true

[keys]                  # key = "command id"; "none" unbinds. All ids are listed in the file.
"ctrl+q" = "quit"
"q" = "none"
"F" = "fx.reverb"
```

Keys are a letter or symbol (upper case means Shift), or `space`, `enter`, `esc`, `tab`, `backtab`, `backspace`, `del`, `home`, `end`, `pgup`, `pgdn`, `f1`..`f12`, with `ctrl+`, `alt+`, `shift+` prefixes. The arrows, Esc and the mouse are fixed. The palette always shows the live bindings. A file with a typo is reported and ignored rather than overwritten.

## Layout

```
src/audio.rs   buffer type, decode (symphonia), WAV writer with ACID/cue chunks, LAME encode, peak cache
src/beats.rs   beat detection (FFT, onset strength, tempo, DP tracking) and the grid model
src/fx.rs      all effects (biquads, dynamics, delays, reverb, modulation, WSOLA stretch, generators)
src/commands.rs command registry, ids, default keys, key parsing, fuzzy matcher
src/config.rs  ~/.swell.toml loading, validation, default file text
src/player.rs  cpal output stream with resampling, pause and loop
src/app.rs     editor state, undo, commands, key and mouse handling
src/ui.rs      ratatui rendering
src/main.rs    terminal setup and event loop
```

## Versions

### v1

- initial release
