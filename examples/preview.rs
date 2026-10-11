//! Render the real UI to SVG without opening a terminal or touching audio.
//! cargo run --example preview -- /tmp/sound-switcher-previews
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
};
use sound_switcher::{app::App, ui, voice::Settings};
use std::fmt::Write;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

fn color(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "#ebf2fa".into(),
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/sound-switcher-previews"));
    std::fs::create_dir_all(&directory)?;
    for (name, w, h) in [("wide", 120, 36), ("standard", 80, 24), ("compact", 40, 18)] {
        let mut app = App::new(
            true,
            false,
            Settings {
                model: None,
                python: "python3".into(),
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(w, h))?;
        terminal.draw(|frame| ui::render(frame, &mut app))?;
        let buffer = terminal.backend().buffer();
        let mut svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}"><rect width="100%" height="100%" fill="#0b1018"/><g font-family="DejaVu Sans Mono, monospace" font-size="16.6">"##,
            w * 10 + 40,
            h * 21 + 40,
            w * 10 + 40,
            h * 21 + 40
        );
        for y in 0..h {
            let mut x = 0;
            while x < w {
                let cell = &buffer[(x, y)];
                let width = cell.symbol().width().max(1) as u16;
                let px = x * 10 + 20;
                let py = y * 21 + 20;
                write!(
                    svg,
                    r#"<rect x="{px}" y="{py}" width="{}" height="21" fill="{}"/>"#,
                    width * 10,
                    color(cell.bg)
                )?;
                if cell.symbol() != " " {
                    write!(
                        svg,
                        r#"<text x="{px}" y="{}" fill="{}" font-weight="{}">{}</text>"#,
                        py + 17,
                        color(cell.fg),
                        if cell.modifier.contains(Modifier::BOLD) {
                            "bold"
                        } else {
                            "normal"
                        },
                        xml(cell.symbol())
                    )?;
                }
                x += width;
            }
        }
        svg.push_str("</g></svg>");
        let path = directory.join(format!("{name}.svg"));
        std::fs::write(&path, svg)?;
        println!("{}", path.display());
    }
    Ok(())
}
