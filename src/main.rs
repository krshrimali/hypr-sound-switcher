use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
};
use sound_switcher::{app, ui, voice};
use std::error::Error;
use std::io::{self, IsTerminal};
use std::time::Duration;

const HELP: &str = "Sound Switcher — put your audio in the right place.

Usage: sound_switcher [--once] [--demo] [--no-mouse]

  --once       Close after connecting an output (quick-picker mode)
  --demo       Explore with sample devices; never changes system audio
  --no-mouse   Leave mouse and text selection to your terminal
  -h, --help   Show this help
  --version    Show the version

Inside: ↑/↓ select · Enter connect · ←/→ volume · m mute · / search
        v voice · : command · ? shortcuts · q close

Optional offline voice: set SOUND_SWITCHER_VOICE_MODEL to a Vosk model
folder and SOUND_SWITCHER_VOICE_PYTHON to Python with vosk + sounddevice.
See README.md for setup. The microphone opens only when you press v.";

fn main() -> Result<(), Box<dyn Error>> {
    let mut demo = false;
    let mut once = false;
    let mut mouse = true;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--demo" => demo = true,
            "--once" => once = true,
            "--no-mouse" => mouse = false,
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "--version" => {
                println!("sound_switcher {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => return Err(format!("Unknown option: {arg}. Use --help for usage.").into()),
        }
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("Open Sound Switcher inside a terminal. Use --help for usage.".into());
    }
    let mut app = app::App::new(demo, once, voice::Settings::from_env());
    // Ratatui restores raw mode and the alternate screen, including on panic.
    // The guard also restores mouse capture on every exit path.
    ratatui::run(|terminal| -> io::Result<()> {
        let _mouse = MouseGuard::new(mouse)?;
        loop {
            app.poll();
            if app.quit {
                return Ok(());
            }
            terminal.draw(|frame| ui::render(frame, &mut app))?;
            if !event::poll(Duration::from_millis(80))? {
                continue;
            }
            // Drain a small burst before repainting. Fast typing and resize events
            // should not each trigger an expensive full-screen redraw.
            for _ in 0..64 {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                    Event::Mouse(mouse) => app.mouse(mouse),
                    _ => {}
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    })?;
    Ok(())
}

struct MouseGuard(bool);
impl MouseGuard {
    fn new(enabled: bool) -> io::Result<Self> {
        if enabled {
            execute!(io::stdout(), EnableMouseCapture)?;
        }
        Ok(Self(enabled))
    }
}
impl Drop for MouseGuard {
    fn drop(&mut self) {
        if self.0 {
            let _ = execute!(io::stdout(), DisableMouseCapture);
        }
    }
}
