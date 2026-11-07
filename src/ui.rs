//! Rendering: waveform lanes, marker strip, timeline, status, prompts, help.

use crate::app::{App, Mode, SPEED_MAX, SPEED_MIN};
use crate::audio::fmt_time;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;

const WAVE_FG: Color = Color::Rgb(90, 200, 130);
const WAVE_DIM: Color = Color::Rgb(60, 70, 65);
const SEL_BG: Color = Color::Rgb(40, 65, 120);
const LOOP_BG: Color = Color::Rgb(85, 40, 95);
const SEL_LOOP_BG: Color = Color::Rgb(95, 70, 150);
const CURSOR_FG: Color = Color::White;
const PLAYHEAD_FG: Color = Color::Yellow;
const SLICE_FG: Color = Color::Rgb(255, 170, 60);
const TITLE_BG: Color = Color::Rgb(35, 35, 45);
const BAR_FG: Color = Color::Rgb(120, 160, 230);
const BEAT_FG: Color = Color::Rgb(70, 90, 130);
const ANCHOR_FG: Color = Color::Rgb(255, 220, 90);

/// Which grid lines fit on screen: (draw beats?, draw every n-th bar).
fn grid_density(app: &App) -> (bool, i64) {
    let Some(g) = app.grid else { return (false, 1) };
    let rate = app.audio.sample_rate;
    let beat_cols = g.fpb(rate) / app.fpc;
    let bar_cols = g.bar_frames(rate) / app.fpc;
    let mut every = 1i64;
    while (every as f64) * bar_cols < 6.0 && every < 1 << 20 {
        every *= 2;
    }
    (beat_cols >= 3.0, every)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let rows = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // markers
        Constraint::Length(1), // bars / beats
        Constraint::Min(4),    // waveform
        Constraint::Length(1), // timeline
        Constraint::Length(1), // speed slider
        Constraint::Length(1), // status
        Constraint::Length(1), // message / prompt
        Constraint::Length(1), // key hints
    ])
    .split(area);

    app.layout_wave(rows[3]);
    let buf = f.buffer_mut();
    draw_title(app, rows[0], buf);
    draw_markers(app, rows[1], buf);
    draw_bars(app, rows[2], buf);
    draw_wave(app, rows[3], buf);
    draw_timeline(app, rows[4], buf);
    draw_speed(app, rows[5], buf);
    draw_status(app, rows[6], buf);
    draw_message(app, rows[7], buf);
    draw_keys(rows[8], buf);

    match &app.mode {
        Mode::Help => draw_help(area, buf),
        Mode::Confirm { question, .. } => draw_confirm(question, area, buf),
        Mode::Palette(p) => draw_palette(app, p, area, buf),
        _ => {}
    }
}

fn draw_palette(app: &App, p: &crate::app::Palette, area: Rect, buf: &mut Buffer) {
    // current bindings per command, from the (possibly user-modified) keymap
    let mut bound: std::collections::HashMap<crate::commands::Cmd, Vec<String>> = std::collections::HashMap::new();
    for (chord, cmd) in &app.keymap {
        bound.entry(*cmd).or_default().push(crate::commands::format_key(chord));
    }
    for v in bound.values_mut() {
        v.sort_by_key(|k| (k.len(), k.clone()));
    }
    let w = 84.min(area.width.saturating_sub(2)).max(20);
    let rows = p.matches.len().clamp(1, 18) as u16;
    let h = (rows + 4).min(area.height);
    let r = Rect::new(area.x + (area.width - w) / 2, area.y + 1.min(area.height.saturating_sub(h)), w, h);
    Clear.render(r, buf);
    let block = Block::bordered()
        .title(" command palette  (type to filter · ↑↓ · Enter · Esc) ")
        .style(Style::default().bg(Color::Rgb(25, 25, 35)).fg(Color::White));
    let inner = block.inner(r);
    block.render(r, buf);
    let line = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(p.input.clone()),
        Span::styled("█", Style::default().fg(Color::Cyan)),
    ]);
    Paragraph::new(line).render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
    let list_h = inner.height.saturating_sub(2) as usize;
    if list_h == 0 {
        return;
    }
    if p.matches.is_empty() {
        Paragraph::new(Span::styled("  no matching commands", Style::default().fg(Color::DarkGray)))
            .render(Rect::new(inner.x, inner.y + 2, inner.width, 1), buf);
        return;
    }
    let first = p.selected.saturating_sub(list_h.saturating_sub(1)).min(p.matches.len().saturating_sub(list_h));
    let cmds = crate::commands::all();
    for (row, &ci) in p.matches.iter().enumerate().skip(first).take(list_h) {
        let c = &cmds[ci];
        let y = inner.y + 2 + (row - first) as u16;
        let selected = row == p.selected;
        let base = if selected {
            Style::default().bg(Color::Rgb(60, 80, 140)).fg(Color::White)
        } else {
            Style::default().fg(Color::White)
        };
        fill_row(Rect::new(inner.x, y, inner.width, 1), buf, base);
        let marker = if selected { "▶ " } else { "  " };
        let is_fx = matches!(c.cmd, crate::commands::Cmd::Fx(_));
        let cat = format!("{:<11}", c.category);
        let line = Line::from(vec![
            Span::styled(marker, base.fg(Color::Cyan)),
            Span::styled(cat, base.fg(if is_fx { Color::Rgb(255, 170, 60) } else { Color::DarkGray })),
            Span::styled(c.name, base.add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() })),
        ]);
        Paragraph::new(line).render(Rect::new(inner.x, y, inner.width, 1), buf);
        if let Some(keys) = bound.get(&c.cmd) {
            let k = format!("{} ", keys.join("  "));
            let kw = k.chars().count() as u16;
            if kw + 20 < inner.width {
                Paragraph::new(Span::styled(k, base.fg(Color::Cyan)))
                    .render(Rect::new(inner.x + inner.width - kw, y, kw, 1), buf);
            }
        }
    }
    let count = format!(" {}/{} ", p.matches.len(), cmds.len());
    let cw = count.len() as u16;
    if cw < r.width {
        Paragraph::new(Span::styled(count, Style::default().fg(Color::DarkGray).bg(Color::Rgb(25, 25, 35))))
            .render(Rect::new(r.x + r.width - cw - 1, r.y + r.height - 1, cw, 1), buf);
    }
}

fn fill_row(area: Rect, buf: &mut Buffer, style: Style) {
    for x in area.x..area.x + area.width {
        buf[(x, area.y)].set_char(' ').set_style(style);
    }
}

fn draw_title(app: &App, area: Rect, buf: &mut Buffer) {
    let style = Style::default().bg(TITLE_BG).fg(Color::White);
    fill_row(area, buf, style);
    let transport = if app.player.is_playing() {
        format!("▶ playing {} ", app.playing_what())
    } else if app.paused {
        "❚❚ paused ".to_string()
    } else {
        String::new()
    };
    let dirty = if app.dirty { " [modified]" } else { "" };
    let name = app
        .path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(no file)".into());
    let fixed = 7 + transport.chars().count() + dirty.len() + 1;
    let room = (area.width as usize).saturating_sub(fixed);
    let name = left_truncate(&name, room);
    let line = Line::from(vec![
        Span::styled(" swell ", style.add_modifier(Modifier::BOLD).fg(Color::Cyan)),
        Span::styled(transport, style.fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(name, style),
        Span::styled(dirty, style.fg(Color::Yellow)),
    ]);
    Paragraph::new(line).style(style).render(area, buf);
}

/// Keep the tail of `s` so the file name stays visible when a path is long.
fn left_truncate(s: &str, room: usize) -> String {
    let n = s.chars().count();
    if n <= room {
        return s.to_string();
    }
    if room < 2 {
        return String::new();
    }
    let tail: String = s.chars().skip(n - (room - 1)).collect();
    format!("…{tail}")
}

fn draw_markers(app: &App, area: Rect, buf: &mut Buffer) {
    fill_row(area, buf, Style::default());
    let w = app.width();
    if app.audio.is_empty() {
        return;
    }
    if let Some((ls, le)) = app.loop_range {
        let c0 = app.frame_to_col(ls).unwrap_or(0);
        let c1 = app.frame_to_col(le).unwrap_or(w - 1);
        if le > app.view_start && ls < app.col_to_frame(w) {
            for c in c0..=c1.min(w - 1) {
                buf[(area.x + c as u16, area.y)]
                    .set_char('─')
                    .set_fg(Color::Magenta);
            }
            if let Some(c) = app.frame_to_col(ls) {
                buf[(area.x + c as u16, area.y)].set_char('◆').set_fg(Color::Magenta);
            }
            if let Some(c) = app.frame_to_col(le) {
                buf[(area.x + c as u16, area.y)].set_char('◆').set_fg(Color::Magenta);
            }
        }
    }
    for &s in &app.slices {
        if let Some(c) = app.frame_to_col(s) {
            buf[(area.x + c as u16, area.y)]
                .set_char('┃')
                .set_style(Style::default().fg(SLICE_FG).add_modifier(Modifier::BOLD));
        }
    }
    for (mark, ch) in [(app.bar_start, '⟦'), (app.bar_end, '⟧')] {
        if let Some(c) = mark.and_then(|f| app.frame_to_col(f)) {
            buf[(area.x + c as u16, area.y)]
                .set_char(ch)
                .set_style(Style::default().fg(ANCHOR_FG).add_modifier(Modifier::BOLD));
        }
    }
    if let Some(p) = app.playhead() {
        if let Some(c) = app.frame_to_col(p) {
            buf[(area.x + c as u16, area.y)]
                .set_char('▶')
                .set_style(Style::default().fg(PLAYHEAD_FG).add_modifier(Modifier::BOLD));
        }
    }
    if let Some(c) = app.frame_to_col(app.cursor) {
        buf[(area.x + c as u16, area.y)]
            .set_char('▼')
            .set_style(Style::default().fg(CURSOR_FG).add_modifier(Modifier::BOLD));
    }
}

fn draw_bars(app: &App, area: Rect, buf: &mut Buffer) {
    fill_row(area, buf, Style::default());
    let Some(g) = app.grid else {
        if !app.audio.is_empty() {
            let hint = match app.bar_start {
                Some(_) => " bar start marked — B at the bar end sets the tempo, or A analyses",
                None => " no beat grid — b/B mark a bar at the cursor, g marks the selection, A analyses",
            };
            Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))).render(area, buf);
        }
        return;
    };
    let rate = app.audio.sample_rate;
    let w = app.width();
    let (beats, every) = grid_density(app);
    let vs = app.view_start;
    let ve = app.col_to_frame(w);
    if beats {
        for (k, f) in g.beats_in(vs, ve, rate) {
            if g.is_downbeat(k) {
                continue;
            }
            if let Some(c) = app.frame_to_col(f.round() as usize) {
                buf[(area.x + c as u16, area.y)].set_char('·').set_fg(BEAT_FG);
            }
        }
    }
    let mut next_free = 0usize;
    for (k, f) in g.bars_in(vs, ve, rate, every) {
        let Some(c) = app.frame_to_col(f.round() as usize) else { continue };
        let is_anchor = k == 0;
        let style = if is_anchor {
            Style::default().fg(ANCHOR_FG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(BAR_FG)
        };
        buf[(area.x + c as u16, area.y)].set_char(if is_anchor { '◆' } else { '┃' }).set_style(style);
        let label = g.bar_number(k, rate).to_string();
        if c >= next_free || c == 0 {
            for (i, ch) in label.chars().enumerate() {
                let x = c + 1 + i;
                if x < w {
                    buf[(area.x + x as u16, area.y)].set_char(ch).set_style(style);
                }
            }
            next_free = c + label.len() + 2;
        }
    }
    // right-aligned readout: tempo, meter, cursor position in bars.beats, snap state
    let idx = g.beat_index(app.cursor as f64, rate);
    let k = idx.floor() as i64;
    let pos = format!("{}.{}", g.bar_number(k, rate), k.rem_euclid(g.beats_per_bar as i64) + 1);
    let readout = format!(" {:.2} BPM {}/4 @{} ", g.bpm, g.beats_per_bar, pos);
    let tag = if app.snap { " snap " } else { "" };
    let total = (readout.chars().count() + tag.chars().count()) as u16;
    if total < area.width {
        let mut x = area.x + area.width - total;
        for ch in readout.chars() {
            buf[(x, area.y)].set_char(ch).set_style(Style::default().fg(BAR_FG).bg(TITLE_BG));
            x += 1;
        }
        for ch in tag.chars() {
            buf[(x, area.y)].set_char(ch).set_style(Style::default().fg(Color::Black).bg(ANCHOR_FG));
            x += 1;
        }
    }
}

fn draw_wave(app: &mut App, area: Rect, buf: &mut Buffer) {
    for y in area.y..area.y + area.height {
        fill_row(Rect::new(area.x, y, area.width, 1), buf, Style::default());
    }
    if app.audio.is_empty() {
        let msg = "no audio — press o to open a .wav / .mp3, ? for help";
        let x = area.x + area.width.saturating_sub(msg.len() as u16) / 2;
        let y = area.y + area.height / 2;
        Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)).render(Rect::new(x, y, msg.len() as u16, 1), buf);
        return;
    }
    let w = app.width();
    let nch = app.audio.n_channels();
    let selection = app.selection;
    let loop_range = app.loop_range;
    let cursor_col = app.frame_to_col(app.cursor);
    let play_col = app.playhead().and_then(|p| app.frame_to_col(p));
    let slice_cols: Vec<usize> = app.slices.iter().filter_map(|&s| app.frame_to_col(s)).collect();
    let bar_cols: Vec<usize> = match app.grid {
        Some(g) => {
            let (_, every) = grid_density(app);
            g.bars_in(app.view_start, app.col_to_frame(w), app.audio.sample_rate, every)
                .into_iter()
                .filter_map(|(_, f)| app.frame_to_col(f.round() as usize))
                .collect()
        }
        None => Vec::new(),
    };
    let col_starts: Vec<usize> = (0..=w).map(|c| app.col_to_frame(c)).collect();
    let cols = app.wave_columns().clone();

    let lane_h = (area.height as usize / nch).max(1);
    for ch in 0..nch {
        let y0 = area.y as usize + ch * lane_h;
        if y0 >= (area.y + area.height) as usize {
            break;
        }
        let h = if ch == nch - 1 {
            (area.y + area.height) as usize - y0
        } else {
            lane_h
        };
        let half_rows = (h * 2) as f32;
        for c in 0..w {
            let (mn, mx) = cols[ch][c];
            let fs = col_starts[c];
            let fe = col_starts[c + 1].max(fs + 1);
            let in_sel = selection.is_some_and(|(s, e)| fe > s && fs < e);
            let in_loop = loop_range.is_some_and(|(s, e)| fe > s && fs < e);
            let bg = match (in_sel, in_loop) {
                (true, true) => Some(SEL_LOOP_BG),
                (true, false) => Some(SEL_BG),
                (false, true) => Some(LOOP_BG),
                _ => None,
            };
            let is_cursor = cursor_col == Some(c);
            let is_play = play_col == Some(c);
            let is_slice = slice_cols.contains(&c);
            let is_bar = bar_cols.contains(&c);
            let fg = if is_play {
                PLAYHEAD_FG
            } else if is_cursor {
                CURSOR_FG
            } else {
                WAVE_FG
            };
            let x = area.x + c as u16;
            let valid = !mn.is_nan();
            // half-row indices covered by [mn, mx]
            let top = if valid { (((1.0 - mx) / 2.0) * half_rows).floor().clamp(0.0, half_rows - 1.0) as usize } else { 0 };
            let bot = if valid { (((1.0 - mn) / 2.0) * half_rows).floor().clamp(0.0, half_rows - 1.0) as usize } else { 0 };
            let (top, bot) = (top.min(bot), top.max(bot));
            for r in 0..h {
                let up = valid && (2 * r) >= top && (2 * r) <= bot;
                let lo = valid && (2 * r + 1) >= top && (2 * r + 1) <= bot;
                let y = (y0 + r) as u16;
                let cell = &mut buf[(x, y)];
                let ch_sym = match (up, lo) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => {
                        if is_cursor || is_play {
                            '│'
                        } else if is_slice {
                            '┆'
                        } else if is_bar {
                            '┊'
                        } else if valid && r == h / 2 {
                            '─'
                        } else {
                            ' '
                        }
                    }
                };
                let mut style = Style::default();
                if let Some(b) = bg {
                    style = style.bg(b);
                }
                let fgc = if up || lo {
                    fg
                } else if is_cursor {
                    CURSOR_FG
                } else if is_play {
                    PLAYHEAD_FG
                } else if is_slice {
                    SLICE_FG
                } else if is_bar {
                    BEAT_FG
                } else {
                    WAVE_DIM
                };
                cell.set_char(ch_sym).set_style(style.fg(fgc));
            }
        }
        // channel label in the lane's top-left corner
        let label = format!("{}", ch + 1);
        if h >= 2 {
            buf[(area.x, y0 as u16)].set_char(label.chars().next().unwrap()).set_fg(Color::DarkGray);
        }
    }
}

fn draw_timeline(app: &App, area: Rect, buf: &mut Buffer) {
    let style = Style::default().fg(Color::DarkGray);
    for x in area.x..area.x + area.width {
        buf[(x, area.y)].set_char('─').set_style(style);
    }
    if app.audio.is_empty() {
        return;
    }
    let rate = app.audio.sample_rate as f64;
    let spc = app.fpc / rate; // seconds per column
    const STEPS: [f64; 18] = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0];
    let step = STEPS.iter().copied().find(|s| s / spc >= 14.0).unwrap_or(600.0);
    let t0 = app.view_start as f64 / rate;
    let t1 = t0 + app.width() as f64 * spc;
    let mut t = (t0 / step).ceil() * step;
    let w = area.width as usize;
    while t <= t1 {
        let col = ((t - t0) / spc).round() as usize;
        if col >= w {
            break;
        }
        let frame = (t * rate).round() as usize;
        let label = if step < 1.0 {
            fmt_time(frame, app.audio.sample_rate)
        } else {
            let s = fmt_time(frame, app.audio.sample_rate);
            s[..s.len() - 4].to_string()
        };
        let x = area.x + col as u16;
        buf[(x, area.y)].set_char('┴').set_fg(Color::Gray);
        let room = (step / spc) as usize;
        if label.len() + 1 < room && col + label.len() + 1 < w {
            for (i, ch) in label.chars().enumerate() {
                buf[(x + 1 + i as u16, area.y)].set_char(ch).set_fg(Color::Gray);
            }
        }
        t += step;
    }
    let z = format!(" {} ", zoom_label(app.fpc, app.audio.sample_rate));
    if (z.len() as u16) < area.width {
        let x0 = area.x + area.width - z.len() as u16;
        for (i, ch) in z.chars().enumerate() {
            buf[(x0 + i as u16, area.y)].set_char(ch).set_fg(Color::DarkGray);
        }
    }
}

fn draw_speed(app: &mut App, area: Rect, buf: &mut Buffer) {
    fill_row(area, buf, Style::default());
    let label = format!(" speed {:>3}% ", app.speed);
    let left = format!("{SPEED_MIN}% ");
    let right = format!(" {SPEED_MAX}%");
    let hint = "   ↑↓ ±1%  ⇧↑↓ ±10%  1 reset  (pitch follows speed)";
    let fixed = (label.len() + left.len() + right.len()) as u16;
    let hint_w = if area.width > fixed + 60 { hint.chars().count() as u16 } else { 0 };
    let track_w = area.width.saturating_sub(fixed + hint_w).clamp(0, 61);
    let mut x = area.x;
    let put = |buf: &mut Buffer, x: &mut u16, text: &str, style: Style| {
        for ch in text.chars() {
            if *x < area.x + area.width {
                buf[(*x, area.y)].set_char(ch).set_style(style);
                *x += 1;
            }
        }
    };
    let accent = if app.speed == 100 { Color::Gray } else { Color::Yellow };
    put(buf, &mut x, &label, Style::default().fg(accent).add_modifier(Modifier::BOLD));
    put(buf, &mut x, &left, Style::default().fg(Color::DarkGray));
    let track = Rect::new(x, area.y, track_w, 1);
    app.slider_area = track;
    if track_w >= 2 {
        let frac = (app.speed - SPEED_MIN) as f64 / (SPEED_MAX - SPEED_MIN) as f64;
        let knob = (frac * (track_w - 1) as f64).round() as u16;
        let mid = (((100 - SPEED_MIN) as f64 / (SPEED_MAX - SPEED_MIN) as f64) * (track_w - 1) as f64).round() as u16;
        for i in 0..track_w {
            let (ch, color) = if i == knob {
                ('●', accent)
            } else if i == mid {
                ('┼', Color::DarkGray)
            } else if i < knob {
                ('━', accent)
            } else {
                ('─', Color::DarkGray)
            };
            buf[(x + i, area.y)].set_char(ch).set_fg(color);
        }
        x += track_w;
    }
    put(buf, &mut x, &right, Style::default().fg(Color::DarkGray));
    if hint_w > 0 {
        put(buf, &mut x, hint, Style::default().fg(Color::DarkGray));
    }
}

fn draw_status(app: &App, area: Rect, buf: &mut Buffer) {
    let style = Style::default().bg(TITLE_BG).fg(Color::Gray);
    fill_row(area, buf, style);
    let a = &app.audio;
    let rate = a.sample_rate;
    let mut spans = vec![
        Span::styled(
            format!(" {}Hz {}ch {}bit ", rate, a.n_channels(), app.bits),
            style,
        ),
        Span::styled(format!("│ len {} ", fmt_time(a.frames(), rate)), style.fg(Color::White)),
        Span::styled(format!("│ cur {} ", fmt_time(app.cursor, rate)), style.fg(Color::White)),
    ];
    if let Some((s, e)) = app.selection {
        spans.push(Span::styled(
            format!("│ sel {}–{} ({}) ", fmt_time(s, rate), fmt_time(e, rate), fmt_time(e - s, rate)),
            style.fg(Color::LightBlue),
        ));
    }
    if let Some((s, e)) = app.loop_range {
        spans.push(Span::styled(
            format!("│ loop {}–{} ", fmt_time(s, rate), fmt_time(e, rate)),
            style.fg(Color::Magenta),
        ));
    }
    if !app.slices.is_empty() {
        spans.push(Span::styled(
            format!("│ {} slices/{} pieces ", app.slices.len(), app.slices.len() + 1),
            style.fg(SLICE_FG),
        ));
    }
    if let Some(p) = app.playhead() {
        spans.push(Span::styled(format!("│ ▶ {} ", fmt_time(p, rate)), style.fg(PLAYHEAD_FG)));
    }
    Paragraph::new(Line::from(spans)).style(style).render(area, buf);
}

fn zoom_label(fpc: f64, rate: u32) -> String {
    let secs = fpc / rate as f64;
    if fpc < 1.5 {
        "1 frame/col".into()
    } else if secs < 0.001 {
        format!("{:.0} frames/col", fpc)
    } else if secs < 1.0 {
        format!("{:.1} ms/col", secs * 1000.0)
    } else {
        format!("{:.2} s/col", secs)
    }
}

fn draw_message(app: &App, area: Rect, buf: &mut Buffer) {
    fill_row(area, buf, Style::default());
    match &app.mode {
        Mode::Prompt(p) => {
            let line = Line::from(vec![
                Span::styled(format!(" {}: ", p.label), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(p.input.clone()),
                Span::styled("█", Style::default().fg(Color::Cyan)),
                Span::styled("   (Enter confirm · Esc cancel)", Style::default().fg(Color::DarkGray)),
            ]);
            Paragraph::new(line).render(area, buf);
        }
        _ => {
            if let Some(m) = &app.msg {
                let style = if m.is_error {
                    Style::default().fg(Color::LightRed)
                } else {
                    Style::default().fg(Color::LightGreen)
                };
                Paragraph::new(Line::from(Span::styled(format!(" {}", m.text), style))).render(area, buf);
            }
        }
    }
}

fn draw_keys(area: Rect, buf: &mut Buffer) {
    let style = Style::default().bg(TITLE_BG).fg(Color::Gray);
    fill_row(area, buf, style);
    let keys = [
        ("␣", "play"),
        ("⏎", "stop"),
        ("←→", "move"),
        ("⇧←→", "select"),
        ("l", "loop"),
        ("m", "slice"),
        ("x/c/v", "cut/copy/paste"),
        ("f/F", "fade"),
        ("n", "norm"),
        ("r", "rev"),
        ("b/B", "bar"),
        ("A", "analyse"),
        ("s", "snap"),
        (":", "palette"),
        ("↑↓", "speed"),
        ("e", "export pieces"),
        ("w", "save"),
        ("u", "undo"),
        ("?", "help"),
    ];
    let mut spans = vec![Span::styled(" ", style)];
    for (k, d) in keys {
        spans.push(Span::styled(k, style.fg(Color::Cyan).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {d}  "), style));
    }
    Paragraph::new(Line::from(spans)).style(style).render(area, buf);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn draw_confirm(question: &str, area: Rect, buf: &mut Buffer) {
    let w = (question.len() as u16 + 6).max(30);
    let r = centered(area, w, 5);
    Clear.render(r, buf);
    let block = Block::bordered()
        .title(" confirm ")
        .style(Style::default().bg(Color::Rgb(40, 30, 30)).fg(Color::White));
    let inner = block.inner(r);
    block.render(r, buf);
    Paragraph::new(vec![
        Line::from(question.to_string()),
        Line::from(""),
        Line::from(Span::styled("y = yes   n / Esc = no", Style::default().fg(Color::Gray))),
    ])
    .wrap(Wrap { trim: true })
    .render(inner, buf);
}

fn draw_help(area: Rect, buf: &mut Buffer) {
    const HELP: &[(&str, &str)] = &[
        ("COMMAND PALETTE", ""),
        (":  /  Ctrl+P", "open the palette: fuzzy-search every command and all 45 effects"),
        ("FILES", ""),
        ("o", "open a .wav / .mp3 / .flac / .aiff file"),
        ("w  /  Ctrl+S", "save (same file; format by extension)"),
        ("W", "save as (.wav or .mp3)"),
        ("E", "export selection to a new file"),
        ("e", "export pieces between slice points, {n} = piece number"),
        ("q  /  Ctrl+C", "quit"),
        ("TRANSPORT", ""),
        ("Space", "play / pause: selection (looped if it is the loop), else loop, else from cursor"),
        ("Enter", "stop"),
        ("p", "move cursor to playhead"),
        ("↑ / ↓", "playback speed ±1%  (Shift: ±10%),  1 = back to 100%"),
        ("mouse on slider", "click / drag / wheel to set speed (50–200%, pitch follows)"),
        ("NAVIGATE & SELECT", ""),
        ("← →", "move cursor one column   (Ctrl/Alt: ×10)"),
        ("Shift+← →", "extend selection"),
        ("Home / End", "start / end of file   (Shift: extend selection)"),
        ("[  /  ]", "set selection start / end at cursor"),
        ("a", "select all"),
        ("Esc", "stop playback, else clear selection"),
        ("mouse", "click = cursor, drag = select, wheel = zoom"),
        ("+ / -  /  0  /  z", "zoom in / out  /  fit  /  zoom to selection"),
        ("PgUp / PgDn  , .", "scroll"),
        ("LOOP & SLICES", ""),
        ("l  /  L", "loop = selection (toggle; applies while playing)  /  clear loop"),
        ("m  /  M", "add or remove slice point at cursor  /  clear all"),
        ("Tab / Shift+Tab", "jump to next / previous slice point"),
        ("BEAT GRID  (saved as ACID tempo + bar cue markers in WAV)", ""),
        ("b  /  B", "mark bar start / bar end at the cursor; a full bar sets the tempo"),
        ("g", "mark bar = selection or loop"),
        ("A", "beat analysis: tracks beats, seeded by the marked bar (tempo prior + downbeat)"),
        ("( )  /  { }", "BPM ∓0.1  /  shift grid 1 ms earlier / later"),
        ("s", "snap: ← → step beats, Ctrl ← → bars, Shift extends, clicks snap"),
        ("C", "conform: pad/trim lead-in to N whole bars, pad end to a whole bar"),
        ("palette", "set BPM, beats per bar, double/halve, select N bars, slice at bars,"),
        ("", "export markers as Audacity labels, clear grid"),
        ("EDIT", ""),
        ("x  c  v", "cut / copy / paste (paste replaces selection)"),
        ("Del / Backspace", "delete selection"),
        ("t", "trim to selection"),
        ("u  /  U", "undo / redo  (also Ctrl+Z / Ctrl+Y)"),
        ("EFFECTS  (apply to selection, or whole file if nothing selected)", ""),
        ("f  /  F  /  n  /  r", "fade in / fade out / normalize / reverse with defaults"),
        ("via palette", "gain, compressor, limiter, gate, filters & EQ, delay, reverb,"),
        ("", "chorus, flanger, phaser, vibrato, ring mod, bit crush, overdrive,"),
        ("", "time stretch, pitch shift, speed, repeat, silence, stereo tools, generators"),
    ];
    let h = (HELP.len() as u16 + 4).min(area.height);
    let r = centered(area, 78, h);
    Clear.render(r, buf);
    let block = Block::bordered()
        .title(" swell — keys  (any key to close) ")
        .style(Style::default().bg(Color::Rgb(25, 25, 35)).fg(Color::White));
    let inner = block.inner(r);
    block.render(r, buf);
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(k, d)| {
            if d.is_empty() && !k.is_empty() {
                Line::from(Span::styled(*k, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            } else {
                Line::from(vec![
                    Span::styled(format!("  {k:<18}"), Style::default().fg(Color::Cyan)),
                    Span::raw(*d),
                ])
            }
        })
        .collect();
    Paragraph::new(lines).render(inner, buf);
}
