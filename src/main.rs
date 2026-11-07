//! swell — a terminal WAV/MP3 audio editor.

mod app;
mod audio;
mod beats;
mod commands;
mod config;
mod fx;
mod player;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use std::path::PathBuf;
use std::time::Duration;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: swell [FILE.wav|FILE.mp3]\n\nA terminal audio editor. Press ? inside for the key reference.\nSettings and key bindings: ~/.swell.toml (or $SWELL_CONFIG), created on first run.");
        return Ok(());
    }
    let path = args.first().map(PathBuf::from);

    let mut app = App::new(path);

    let mouse = app.config.editor.mouse;
    let mut terminal = ratatui::init();
    if mouse {
        execute!(std::io::stdout(), EnableMouseCapture)?;
    }
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        ratatui::restore();
        hook(info);
    }));

    let result = run(&mut terminal, &mut app);

    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        let timeout = if app.transport_active() {
            Duration::from_millis(33)
        } else {
            Duration::from_millis(250)
        };
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => app.on_key(k),
                Event::Mouse(m) => app.on_mouse(m),
                _ => {}
            }
            // drain any burst of queued events before redrawing
            while event::poll(Duration::ZERO)? {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => app.on_key(k),
                    Event::Mouse(m) => app.on_mouse(m),
                    _ => {}
                }
            }
        }
        app.tick();
        if app.should_quit {
            return Ok(());
        }
    }
}
