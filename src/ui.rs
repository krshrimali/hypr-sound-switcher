use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Hit, Overlay};
use crate::audio::Action;

const BG: Color = Color::Rgb(11, 16, 24);
const PANEL: Color = Color::Rgb(17, 27, 40);
const SELECTED: Color = Color::Rgb(24, 47, 56);
const BORDER: Color = Color::Rgb(48, 66, 85);
const TEXT: Color = Color::Rgb(235, 242, 250);
const MUTED: Color = Color::Rgb(155, 174, 195);
const ACCENT: Color = Color::Rgb(114, 230, 198);
const LILAC: Color = Color::Rgb(190, 173, 255);
const ERROR: Color = Color::Rgb(255, 173, 152);

fn style(color: Color) -> Style {
    Style::default().fg(color)
}
fn bold(color: Color) -> Style {
    style(color).add_modifier(Modifier::BOLD)
}
fn panel(title: impl Into<Line<'static>>, selected: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style(if selected { ACCENT } else { BORDER }))
        .style(Style::default().bg(if selected { SELECTED } else { PANEL }))
        .title(title)
}

fn fit(text: &str, width: u16) -> String {
    let width = usize::from(width);
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for c in text.chars() {
        let size = c.width().unwrap_or(0);
        if used + size > width - 1 {
            break;
        }
        result.push(c);
        used += size;
    }
    result.push('…');
    result
}

fn input_tail(text: &str, width: u16) -> String {
    if text.width() <= width as usize {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut tail = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let size = c.width().unwrap_or(0);
        if used + size > width as usize {
            break;
        }
        tail.push(c);
        used += size;
    }
    format!("…{}", tail.into_iter().rev().collect::<String>())
}

fn row(frame: &mut Frame, area: Rect, left: Line, right: Line, right_width: u16) {
    let [left_area, right_area] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(right_width.min(area.width)),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(left), left_area);
    frame.render_widget(
        Paragraph::new(right).alignment(Alignment::Right),
        right_area,
    );
}

fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(x),
        area.y.saturating_add(y),
        area.width.saturating_sub(x * 2),
        area.height.saturating_sub(y * 2),
    )
}

pub fn render(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(BG).fg(TEXT)),
        screen,
    );
    app.hits.clear();
    if screen.width < 24 || screen.height < 7 {
        frame.render_widget(
            Paragraph::new("Sound Switcher\nEnlarge to 24 × 7\nq to close").style(style(TEXT)),
            screen,
        );
        return;
    }
    let margin = if screen.width >= 60 { 3 } else { 1 };
    let width = screen.width.saturating_sub(margin * 2).min(120);
    let height = screen
        .height
        .saturating_sub(if screen.height > 24 { 2 } else { 0 })
        .min(40);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    let roomy = area.height >= 28;
    let [header, active, gap, body, footer] = Layout::vertical([
        Constraint::Length(if roomy {
            3
        } else if area.height >= 18 {
            2
        } else {
            1
        }),
        Constraint::Length(if roomy { 4 } else { 0 }),
        Constraint::Length(if roomy { 1 } else { 0 }),
        Constraint::Min(1),
        Constraint::Length(if area.height >= 12 { 3 } else { 2 }),
    ])
    .areas(area);
    let brand = Line::from(vec![
        Span::styled("◖ ", bold(ACCENT)),
        Span::styled("sound", bold(TEXT)),
        Span::styled(" / switcher", style(MUTED)),
    ]);
    let status = if app.demo {
        "DEMO"
    } else if app.error.is_some() {
        "RECONNECTING"
    } else if !app.loaded {
        "CONNECTING"
    } else {
        "LIVE AUDIO"
    };
    let right_width = if header.width >= 48 { 20 } else { 0 };
    row(
        frame,
        Rect {
            height: 1,
            ..header
        },
        brand,
        Line::from(Span::styled(
            format!("● {status}"),
            style(if app.error.is_some() { ERROR } else { ACCENT }),
        )),
        right_width,
    );
    if roomy {
        frame.render_widget(
            Paragraph::new("Good sound. Right where you want it.").style(style(MUTED)),
            Rect::new(
                header.x + 2,
                header.y + 1,
                header.width.saturating_sub(2),
                1,
            ),
        );
        active_output(frame, active, app);
    }
    let _ = gap;
    let wide = body.width >= 96 && body.height >= 14;
    if wide {
        let [devices, _, detail] = Layout::horizontal([
            Constraint::Min(35),
            Constraint::Length(2),
            Constraint::Length(36),
        ])
        .areas(body);
        device_list(frame, devices, app);
        details(frame, detail, app);
    } else {
        let [devices, controls] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(if body.height >= 10 {
                5
            } else if body.height >= 6 {
                3
            } else {
                0
            }),
        ])
        .areas(body);
        device_list(frame, devices, app);
        compact_controls(frame, controls, app);
    }
    footer_bar(frame, footer, app);
    if let Some(overlay) = app.overlay {
        overlay_panel(frame, area, app, overlay);
    }
}

fn active_output(frame: &mut Frame, area: Rect, app: &App) {
    let block = panel(
        Line::from(Span::styled("  NOW CONNECTED  ", bold(ACCENT))),
        false,
    );
    let inner = inset(block.inner(area), 1, 0);
    frame.render_widget(block, area);
    let current = app
        .snapshot
        .sinks
        .iter()
        .find(|s| s.name == app.snapshot.default);
    let (name, state) = match current {
        Some(s) => (
            s.label(),
            if s.muted {
                "MUTED"
            } else if s.state == "RUNNING" {
                "PLAYING"
            } else {
                "READY"
            },
        ),
        None => (
            if app.loaded {
                "No output connected"
            } else {
                "Finding your audio devices…"
            },
            "",
        ),
    };
    let side = if inner.width > 52 { 20 } else { 0 };
    row(
        frame,
        Rect { height: 1, ..inner },
        Line::from(Span::styled(
            fit(name, inner.width.saturating_sub(side)),
            bold(TEXT),
        )),
        Line::from(Span::styled(state, style(ACCENT))),
        side,
    );
    if inner.height > 1 {
        let caption = current
            .map(|s| {
                format!(
                    "{}  ·  {}% volume  ·  {} playing app{}",
                    s.kind,
                    s.volume,
                    app.snapshot.streams,
                    if app.snapshot.streams == 1 { "" } else { "s" }
                )
            })
            .unwrap_or_else(|| "Connect an output below. Your playing apps will follow.".into());
        frame.render_widget(
            Paragraph::new(fit(&caption, inner.width)).style(style(MUTED)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }
}

fn device_list(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.is_empty() {
        return;
    }
    let search_height = if area.height >= 8 { 3 } else { 1 };
    let [heading, search, cards] = Layout::vertical([
        Constraint::Length(u16::from(area.height >= 6)),
        Constraint::Length(search_height),
        Constraint::Min(1),
    ])
    .areas(area);
    let filtered = app.filtered();
    if !heading.is_empty() {
        row(
            frame,
            heading,
            Line::from(Span::styled(" OUTPUTS", bold(TEXT))),
            Line::from(Span::styled(
                format!("{} / {} ", filtered.len(), app.snapshot.sinks.len()),
                style(MUTED),
            )),
            10,
        );
    }
    let search_text = if app.query.is_empty() && !app.searching {
        " /  Find an output…".to_owned()
    } else {
        format!(" /  {}{}", app.query, if app.searching { "▏" } else { "" })
    };
    let search_block = if search_height == 3 {
        panel("", app.searching)
    } else {
        Block::default().style(Style::default().bg(PANEL))
    };
    let inner = search_block.inner(search);
    let suffix = if app.searching {
        input_tail(&search_text, inner.width)
    } else {
        fit(&search_text, inner.width)
    };
    frame.render_widget(
        Paragraph::new(suffix)
            .block(search_block)
            .style(style(if app.searching { TEXT } else { MUTED })),
        search,
    );
    app.hits.push((search, Hit::Search));
    if app.searching {
        let cursor_x = inner.x
            + (format!(" /  {}", app.query).width() as u16).min(inner.width.saturating_sub(1));
        frame.set_cursor_position((cursor_x, inner.y));
    }
    if filtered.is_empty() {
        let message = if !app.loaded {
            "Finding your outputs…\nThis only takes a moment."
        } else if app.error.is_some() {
            "Audio server unavailable\nPress r to retry. Check that PipeWire\nor PulseAudio is running."
        } else if app.snapshot.sinks.is_empty() {
            "Your next sound starts here.\nConnect headphones or speakers.\nOutputs refresh automatically."
        } else {
            "No matching outputs\nPress / to edit or Esc to clear."
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(style(MUTED))
                .wrap(Wrap { trim: false })
                .block(Block::default().padding(ratatui::widgets::Padding::new(1, 1, 1, 0))),
            cards,
        );
        return;
    }
    let card_height = if cards.height >= 8 {
        4
    } else if cards.height >= 6 {
        3
    } else {
        1
    };
    let count = usize::from((cards.height / card_height).max(1));
    app.visible_count = count;
    let position = filtered
        .iter()
        .position(|i| *i == app.selected)
        .unwrap_or(0);
    if position < app.list_offset {
        app.list_offset = position;
    }
    if position >= app.list_offset + count {
        app.list_offset = position + 1 - count;
    }
    app.list_offset = app.list_offset.min(filtered.len().saturating_sub(count));
    for (row_index, index) in filtered
        .iter()
        .skip(app.list_offset)
        .take(count)
        .enumerate()
    {
        let sink = &app.snapshot.sinks[*index];
        let selected = *index == app.selected;
        let connected = sink.name == app.snapshot.default;
        let card = Rect::new(
            cards.x,
            cards.y + row_index as u16 * card_height,
            cards.width,
            card_height.min(cards.height),
        );
        let block = if card_height > 1 {
            panel("", selected)
        } else {
            Block::default().style(Style::default().bg(if selected { SELECTED } else { PANEL }))
        };
        let inner = inset(block.inner(card), u16::from(card.width > 30), 0);
        frame.render_widget(block, card);
        let badge = if connected {
            if inner.width >= 40 {
                "● CONNECTED"
            } else {
                "● ON"
            }
        } else if selected && inner.width >= 40 {
            "ENTER ↵"
        } else {
            ""
        };
        let badge_width = if badge.is_empty() {
            0
        } else {
            badge.width() as u16 + 1
        };
        let number = app.list_offset + row_index + 1;
        let label = format!(
            "{} {}  {}",
            if selected { "›" } else { " " },
            number,
            sink.label()
        );
        row(
            frame,
            Rect { height: 1, ..inner },
            Line::from(Span::styled(
                fit(&label, inner.width.saturating_sub(badge_width)),
                if selected { bold(TEXT) } else { style(TEXT) },
            )),
            Line::from(Span::styled(
                badge,
                style(if connected { ACCENT } else { MUTED }),
            )),
            badge_width,
        );
        if inner.height > 1 {
            let state = if sink.muted {
                "Muted"
            } else if sink.state == "RUNNING" {
                "Playing"
            } else {
                "Ready"
            };
            let meta = format!(
                "     {}  ·  {}  ·  {}%",
                sink.kind,
                state,
                app.shown_volume(sink)
            );
            frame.render_widget(
                Paragraph::new(fit(&meta, inner.width)).style(style(MUTED)),
                Rect::new(inner.x, inner.y + 1, inner.width, 1),
            );
        }
        app.hits.push((card, Hit::Device(*index)));
    }
    let hidden_below = filtered.len().saturating_sub(app.list_offset + count);
    if hidden_below > 0 && cards.height % card_height > 0 {
        frame.render_widget(
            Paragraph::new(format!(" ↓ {hidden_below} more · scroll or ↓")).style(style(MUTED)),
            Rect::new(cards.x, cards.bottom() - 1, cards.width, 1),
        );
    }
}

fn details(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel(
        Line::from(Span::styled("  MAKE IT YOURS  ", bold(LILAC))),
        false,
    );
    let inner = inset(block.inner(area), 2, 1);
    frame.render_widget(block, area);
    let Some(sink) = app.selected_sink().cloned() else {
        frame.render_widget(
            Paragraph::new("Select an output to see\nits controls here.").style(style(MUTED)),
            inner,
        );
        return;
    };
    let [name, meta, _, volume_label, volume, _, buttons, _, voice] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(sink.description.clone())
            .style(bold(TEXT))
            .wrap(Wrap { trim: true }),
        name,
    );
    let spec = sink.sample_spec.replace("le", "").replace("Hz", " Hz");
    frame.render_widget(
        Paragraph::new(format!("{} · {}\n{}", sink.kind, sink.port, spec))
            .style(style(MUTED))
            .wrap(Wrap { trim: true }),
        meta,
    );
    let muted = app.shown_mute(&sink);
    row(
        frame,
        volume_label,
        Line::from(Span::styled("Volume", bold(TEXT))),
        Line::from(Span::styled(
            if muted {
                "MUTED".into()
            } else {
                format!("{}%", app.shown_volume(&sink))
            },
            style(if muted { ERROR } else { ACCENT }),
        )),
        8,
    );
    volume_bar(frame, volume, app, &sink);
    let button_parts = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(10),
    ])
    .split(buttons);
    connect_button(
        frame,
        button_parts[0],
        app,
        sink.name == app.snapshot.default,
    );
    button(
        frame,
        button_parts[2],
        if muted { "Unmute" } else { "Mute" },
        false,
    );
    app.hits.push((button_parts[2], Hit::Mute));
    if voice.height >= 3 {
        let message = if app.voice.is_some() {
            if app.listening {
                "● Listening for 4 seconds…\nSpeak a command. v cancels."
            } else {
                "Preparing your microphone…\nv cancels."
            }
        } else {
            "A little more hands-free.\nv  Voice command\n:  Type a command"
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(style(LILAC))
                .wrap(Wrap { trim: false }),
            voice,
        );
        app.hits.push((voice, Hit::Voice));
    }
}

fn volume_bar(frame: &mut Frame, area: Rect, app: &mut App, sink: &crate::audio::Sink) {
    if area.is_empty() {
        return;
    }
    let n = app.shown_volume(sink).min(100);
    let filled = (u32::from(area.width) * u32::from(n)).div_ceil(100) as usize;
    let line = Line::from(vec![
        Span::styled(
            "━".repeat(filled),
            style(if app.shown_mute(sink) { MUTED } else { ACCENT }),
        ),
        Span::styled(
            "─".repeat(usize::from(area.width).saturating_sub(filled)),
            style(BORDER),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
    for x in 0..area.width {
        let value = if area.width <= 1 {
            0
        } else {
            (u32::from(x) * 100 / u32::from(area.width - 1)) as u16
        };
        app.hits.push((
            Rect::new(area.x + x, area.y, 1, area.height),
            Hit::Volume(value),
        ));
    }
}

fn button(frame: &mut Frame, area: Rect, label: &str, primary: bool) {
    let block = Block::default()
        .borders(if area.height >= 3 {
            Borders::ALL
        } else {
            Borders::NONE
        })
        .border_type(BorderType::Rounded)
        .border_style(style(if primary { ACCENT } else { BORDER }))
        .style(
            Style::default()
                .bg(if primary { ACCENT } else { PANEL })
                .fg(if primary { BG } else { TEXT }),
        );
    frame.render_widget(
        Paragraph::new(label)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::BOLD))
            .block(block),
        area,
    );
}

fn connect_button(frame: &mut Frame, area: Rect, app: &mut App, connected: bool) {
    let switching = matches!(&app.pending, Some(Action::Switch(_)))
        || app
            .queued
            .iter()
            .any(|action| matches!(action, Action::Switch(_)));
    button(
        frame,
        area,
        if switching {
            "Connecting…"
        } else if connected {
            "Connected ✓"
        } else {
            "Connect ↵"
        },
        !connected,
    );
    app.hits.push((area, Hit::Connect));
}

fn compact_controls(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.is_empty() {
        return;
    }
    let Some(sink) = app.selected_sink().cloned() else {
        return;
    };
    let inner = if area.height >= 5 {
        let block = panel("", false);
        let inner = inset(block.inner(area), 1, 0);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };
    let [label, bar, buttons] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let muted = app.shown_mute(&sink);
    let side_width = 8;
    row(
        frame,
        label,
        Line::from(Span::styled(
            fit(
                &format!("Volume · {}", sink.label()),
                label.width.saturating_sub(side_width),
            ),
            style(MUTED),
        )),
        Line::from(Span::styled(
            if muted {
                "MUTED".into()
            } else {
                format!("{}%", app.shown_volume(&sink))
            },
            bold(if muted { ERROR } else { ACCENT }),
        )),
        side_width,
    );
    volume_bar(frame, bar, app, &sink);
    let parts = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(8),
        Constraint::Length(1),
        Constraint::Length(9),
    ])
    .split(buttons);
    connect_button(frame, parts[0], app, sink.name == app.snapshot.default);
    button(
        frame,
        parts[2],
        if muted { "Unmute m" } else { "Mute m" },
        false,
    );
    app.hits.push((parts[2], Hit::Mute));
    button(frame, parts[4], "Voice v", false);
    app.hits.push((parts[4], Hit::Voice));
}

fn footer_bar(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.is_empty() {
        return;
    }
    let status = if app.quit_requested {
        ("Finishing your audio change…".to_owned(), MUTED)
    } else if app.voice.is_some() {
        (
            if app.listening {
                "● Listening… say a command. v or Esc cancels."
            } else {
                "Preparing voice control… v or Esc cancels."
            }
            .to_owned(),
            LILAC,
        )
    } else if let Some(notice) = &app.notice {
        (
            notice.text.clone(),
            if notice.error { ERROR } else { ACCENT },
        )
    } else if !app.loaded {
        ("Connecting to your audio server…".into(), MUTED)
    } else if let Some(error) = &app.error {
        (format!("Audio needs attention: {error}"), ERROR)
    } else if app.searching {
        (
            "Type to filter · Enter keeps results · Esc clears · Ctrl+U resets".into(),
            ACCENT,
        )
    } else {
        (
            if area.width >= 48 {
                "↑ ↓ select   ↵ connect   ← → volume   m mute"
            } else {
                "↑↓ select  ↵ connect  ←→ vol  m mute"
            }
            .into(),
            MUTED,
        )
    };
    let status_text = if status.0.width() > area.width as usize && area.width > 16 {
        format!(
            "{} · e details",
            fit(&status.0, area.width.saturating_sub(12))
        )
    } else {
        status.0
    };
    frame.render_widget(
        Paragraph::new(status_text).style(style(status.1)),
        Rect { height: 1, ..area },
    );
    app.hits.push((Rect { height: 1, ..area }, Hit::Status));
    if area.height >= 2 {
        let labels = if area.width >= 62 {
            [
                "/ search    ",
                "v voice    ",
                ": command    ",
                "? help    ",
                "q close",
            ]
        } else if area.width >= 38 {
            ["/ find ", "v voice ", ": cmd ", "? help ", "q close"]
        } else {
            ["/ find ", "v mic ", ": cmd ", "? ", "q"]
        };
        let hits = [Hit::Search, Hit::Voice, Hit::Command, Hit::Help, Hit::Close];
        let mut x = area.x;
        for (label, hit) in labels.into_iter().zip(hits) {
            let width = (label.width() as u16).min(area.right().saturating_sub(x));
            let target = Rect::new(x, area.bottom() - 1, width, 1);
            frame.render_widget(Paragraph::new(label).style(style(MUTED)), target);
            app.hits.push((target, hit));
            x += width;
        }
    }
}

fn overlay_panel(frame: &mut Frame, area: Rect, app: &mut App, overlay: Overlay) {
    let (title, lines): (&str, Vec<Line>) = match overlay {
        Overlay::Help => (
            " YOUR SHORTCUTS ",
            vec![
                Line::from(Span::styled("Everything, a keystroke away.", bold(TEXT))),
                Line::from(""),
                Line::from("↑ ↓ / j k     Select an output"),
                Line::from("Enter         Connect + move playing apps"),
                Line::from("← → / - +     Selected output volume ±5%"),
                Line::from("m / Space     Mute the selected output"),
                Line::from("/             Search devices, types, ports"),
                Line::from(":             Type a voice-style command"),
                Line::from("v             Listen once / cancel capture"),
                Line::from("1–9           Focus an output"),
                Line::from("Home / End    First / last output"),
                Line::from("PgUp / PgDn   Move a page at a time"),
                Line::from("r             Refresh devices"),
                Line::from("e             Read the full status message"),
                Line::from("q / Ctrl+C    Close the switcher"),
                Line::from(""),
                Line::from(Span::styled(
                    "Click a card, button, or volume bar.",
                    style(ACCENT),
                )),
                Line::from("Scroll to browse. Drag to set volume."),
                Line::from("Use your terminal's zoom shortcuts."),
                Line::from(""),
                Line::from(Span::styled(
                    "Voice commands control the active output.",
                    style(LILAC),
                )),
            ],
        ),
        Overlay::Voice => (
            " VOICE, ON YOUR TERMS ",
            vec![
                Line::from(Span::styled("Offline. Only when you ask.", bold(LILAC))),
                Line::from(""),
                Line::from("Press v to listen for four seconds."),
                Line::from("Audio stays on your machine; no account needed."),
                Line::from(""),
                Line::from(Span::styled("One-time setup", bold(TEXT))),
                Line::from("1. Install Python vosk + sounddevice in a venv."),
                Line::from("2. Download and extract a Vosk English model."),
                Line::from("3. Set SOUND_SWITCHER_VOICE_MODEL to its folder."),
                Line::from("4. Set SOUND_SWITCHER_VOICE_PYTHON to venv Python."),
                Line::from("See README.md for the exact setup commands."),
                Line::from(""),
                Line::from(Span::styled("Say something like…", bold(TEXT))),
                Line::from("“Switch to headphones” · “Next output”"),
                Line::from("“Volume fifty percent” · “Mute” · “Unmute”"),
                Line::from(""),
                Line::from(Span::styled(
                    "Try the same commands with : right now.",
                    style(ACCENT),
                )),
            ],
        ),
        Overlay::Status => (
            " AUDIO STATUS ",
            vec![
                Line::from(Span::styled("Here's what's happening.", bold(TEXT))),
                Line::from(""),
                Line::from(
                    app.notice
                        .as_ref()
                        .map(|n| n.text.clone())
                        .or_else(|| app.error.clone())
                        .unwrap_or_else(|| {
                            "Your audio is ready. Select an output and press Enter to connect."
                                .into()
                        }),
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "Press r to retry an audio connection.",
                    style(MUTED),
                )),
            ],
        ),
        Overlay::Command => (
            " COMMAND ",
            vec![
                Line::from(Span::styled("Tell your audio where to go.", bold(TEXT))),
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "> {}▏",
                        input_tail(&app.input, area.width.min(62).saturating_sub(7))
                    ),
                    style(ACCENT),
                )),
                Line::from(""),
                Line::from("switch to headphones  ·  next output"),
                Line::from("volume 50  ·  louder  ·  mute  ·  unmute"),
                Line::from(""),
                Line::from(Span::styled(
                    "Commands control the active output.",
                    style(MUTED),
                )),
            ],
        ),
    };
    let width = area.width.min(62);
    let height = area
        .height
        .min(if overlay == Overlay::Command { 13 } else { 26 });
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let block = panel(Line::from(Span::styled(title, bold(LILAC))), true);
    let inner = inset(block.inner(popup), 1, 0);
    frame.render_widget(block, popup);
    let [content, hint] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let paragraph = Paragraph::new(lines)
        .style(style(TEXT))
        .wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(content.width)
        .saturating_sub(content.height as usize) as u16;
    app.overlay_scroll = app.overlay_scroll.min(max_scroll);
    frame.render_widget(paragraph.scroll((app.overlay_scroll, 0)), content);
    let text = if overlay == Overlay::Command {
        "Enter execute · Esc close"
    } else if max_scroll > 0 {
        "↑ ↓ scroll · Esc close"
    } else {
        "Esc close"
    };
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Right)
            .style(style(MUTED)),
        hint,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::Settings;
    use ratatui::{Terminal, backend::TestBackend};

    fn app() -> App {
        App::new(
            true,
            false,
            Settings {
                model: None,
                python: "python3".into(),
            },
        )
    }

    #[test]
    fn layouts_survive_zoom_tiny_windows_and_overlays() {
        for (w, h) in [
            (1, 1),
            (23, 6),
            (24, 7),
            (32, 10),
            (40, 15),
            (60, 20),
            (80, 24),
            (100, 30),
            (140, 50),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut app = app();
            for overlay in [
                None,
                Some(Overlay::Help),
                Some(Overlay::Voice),
                Some(Overlay::Command),
                Some(Overlay::Status),
            ] {
                app.overlay = overlay;
                terminal.draw(|f| render(f, &mut app)).unwrap();
                for (rect, _) in &app.hits {
                    assert!(rect.right() <= w && rect.bottom() <= h, "{w}x{h}: {rect:?}");
                }
            }
        }
    }

    #[test]
    fn selected_last_device_remains_visible_after_resize() {
        let mut app = app();
        app.selected = 3;
        for (w, h) in [(120, 36), (40, 15), (24, 7), (100, 28)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| render(f, &mut app)).unwrap();
            assert!(
                app.hits
                    .iter()
                    .any(|(_, hit)| matches!(hit, Hit::Device(3))),
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn truncation_respects_wide_and_combining_characters() {
        for width in 0..20 {
            assert!(fit("音声 ヘッドフォン e\u{301}", width).width() <= width as usize);
        }
    }
}
