use std::collections::HashMap;
use std::error::Error;
use std::io::{self, BufRead};
use std::process::{Command, Output};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

#[derive(Debug, PartialEq, Eq)]
struct Sink {
    id: u32,
    name: String,
    description: String,
    driver: String,
    sample_format: String,
    channels: u16,
    rate: u32,
    state: String,
}

fn parse_sinks_short<R: BufRead>(reader: R) -> io::Result<Vec<Sink>> {
    let mut sinks = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let parts: Vec<&str> = line.split_whitespace().collect();

        // A short sink entry is: id, name, driver, format, channels, rate, state.
        if parts.len() < 7 {
            continue;
        }

        let (Ok(id), Some(channels), Some(rate)) = (
            parts[0].parse(),
            parts[4]
                .strip_suffix("ch")
                .and_then(|value| value.parse().ok()),
            parts[5]
                .strip_suffix("Hz")
                .and_then(|value| value.parse().ok()),
        ) else {
            continue;
        };

        let name = parts[1].to_owned();
        sinks.push(Sink {
            id,
            description: name.clone(),
            name,
            driver: parts[2].to_owned(),
            sample_format: parts[3].to_owned(),
            channels,
            rate,
            state: parts[6..].join(" "),
        });
    }

    Ok(sinks)
}

fn parse_sink_descriptions<R: BufRead>(reader: R) -> io::Result<HashMap<String, String>> {
    let mut descriptions = HashMap::new();
    let mut name = None;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Name: ") {
            name = Some(value.to_owned());
        } else if let (Some(value), Some(sink_name)) =
            (trimmed.strip_prefix("Description: "), name.take())
        {
            descriptions.insert(sink_name, value.to_owned());
        }
    }

    Ok(descriptions)
}

fn checked_output(command: &mut Command, action: &str) -> Result<Output, Box<dyn Error>> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("{action}: {}", stderr.trim()).into())
}

fn fetch_sinks() -> Result<Vec<Sink>, Box<dyn Error>> {
    let output = checked_output(
        Command::new("pactl").args(["list", "sinks", "short"]),
        "pactl could not list audio sinks",
    )?;
    let mut sinks = parse_sinks_short(output.stdout.as_slice())?;

    if sinks.is_empty() {
        return Err("no audio sinks were found".into());
    }
    // The short format has the reliable machine name but not the friendly device
    // description. Failure to fetch descriptions should not make the picker unusable.
    if let Ok(output) = checked_output(
        Command::new("pactl")
            .env("LC_ALL", "C")
            .args(["list", "sinks"]),
        "pactl could not get sink descriptions",
    ) {
        let descriptions = parse_sink_descriptions(output.stdout.as_slice())?;
        for sink in &mut sinks {
            if let Some(description) = descriptions.get(&sink.name) {
                sink.description.clone_from(description);
            }
        }
    }

    Ok(sinks)
}

fn default_sink() -> Result<String, Box<dyn Error>> {
    let output = checked_output(
        Command::new("pactl").arg("get-default-sink"),
        "pactl could not get the default sink",
    )?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn set_default_sink(name: &str) -> Result<(), Box<dyn Error>> {
    checked_output(
        Command::new("pactl").args(["set-default-sink", name]),
        "pactl could not set the default sink",
    )?;

    // Changing the default only affects new streams on PulseAudio. Move existing
    // playback streams as well so the switch is audible immediately.
    let output = checked_output(
        Command::new("pactl").args(["list", "sink-inputs", "short"]),
        "pactl could not list active playback streams",
    )?;
    for line in String::from_utf8(output.stdout)?.lines() {
        let Some(id) = line.split_whitespace().next() else {
            continue;
        };
        checked_output(
            Command::new("pactl").args(["move-sink-input", id, name]),
            "pactl could not move an active playback stream",
        )?;
    }

    Ok(())
}

fn run_picker(sinks: &[Sink], current: &str) -> io::Result<Option<usize>> {
    let initial = sinks
        .iter()
        .position(|sink| sink.name == current)
        .unwrap_or(0);
    let mut state = ListState::default().with_selected(Some(initial));

    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| {
                let [title_area, list_area, help_area] = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Min(3),
                    Constraint::Length(2),
                ])
                .areas(frame.area());

                frame.render_widget(
                    Paragraph::new("Select an audio output").style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    title_area,
                );

                let items = sinks.iter().map(|sink| {
                    let active = sink.name == current;
                    let marker = if active { " [default]" } else { "" };
                    ListItem::new(vec![
                        Line::from(vec![Span::styled(
                            format!("{}{}", sink.description, marker),
                            Style::default().add_modifier(Modifier::BOLD),
                        )]),
                        Line::from(format!(
                            "  #{} | {} | {} | {}ch | {} Hz | {}",
                            sink.id,
                            sink.driver,
                            sink.sample_format,
                            sink.channels,
                            sink.rate,
                            sink.state
                        ))
                        .style(Style::default().fg(Color::DarkGray)),
                    ])
                });

                let list = List::new(items)
                    .block(Block::default().borders(Borders::ALL).title(" Sinks "))
                    .highlight_symbol("> ")
                    .highlight_style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    );
                frame.render_stateful_widget(list, list_area, &mut state);

                frame.render_widget(
                    Paragraph::new("Up/Down or j/k: move   Enter: select   Esc/q: cancel")
                        .style(Style::default().fg(Color::DarkGray)),
                    help_area,
                );
            })?;

            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                KeyCode::Home => state.select_first(),
                KeyCode::End => state.select_last(),
                KeyCode::Enter => return Ok(state.selected()),
                KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                _ => {}
            }
        }
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let sinks = fetch_sinks()?;
    let current = default_sink()?;

    if let Some(selected) = run_picker(&sinks, &current)? {
        set_default_sink(&sinks[selected].name)?;
        println!("Default audio output set to {}", sinks[selected].name);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pactl_short_output() {
        let input = b"53\talsa_output.hdmi\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED\n\
                      57\talsa_output.usb-headset\tPipeWire\ts16le 2ch 44100Hz\tRUNNING\n";

        let sinks = parse_sinks_short(input.as_slice()).unwrap();

        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[0].id, 53);
        assert_eq!(sinks[0].channels, 2);
        assert_eq!(sinks[0].rate, 48_000);
        assert_eq!(sinks[1].name, "alsa_output.usb-headset");
        assert_eq!(sinks[1].state, "RUNNING");
    }

    #[test]
    fn ignores_malformed_lines() {
        let input = b"bad line\nnot-a-number sink PipeWire s32le 2ch 48000Hz RUNNING\n";
        assert!(parse_sinks_short(input.as_slice()).unwrap().is_empty());
    }

    #[test]
    fn parses_friendly_sink_descriptions() {
        let input = b"Sink #53\n\tName: alsa_output.hdmi\n\tDescription: Monitor HDMI Audio\n";
        let descriptions = parse_sink_descriptions(input.as_slice()).unwrap();

        assert_eq!(descriptions["alsa_output.hdmi"], "Monitor HDMI Audio");
    }
}
