use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::audio::{self, Action, Reply, Sink, Snapshot};
use crate::voice::{self, Intent, Session, Settings};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Help,
    Voice,
    Command,
    Status,
}

#[derive(Clone, Copy)]
pub enum Hit {
    Device(usize),
    Connect,
    Mute,
    Volume(u16),
    Voice,
    Search,
    Help,
    Command,
    Close,
    Status,
}

pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct App {
    pub snapshot: Snapshot,
    pub selected: usize,
    pub query: String,
    pub searching: bool,
    pub input: String,
    pub overlay: Option<Overlay>,
    pub overlay_scroll: u16,
    pub notice: Option<Notice>,
    pub error: Option<String>,
    pub loaded: bool,
    pub pending: Option<Action>,
    pub queued: VecDeque<Action>,
    pub hits: Vec<(Rect, Hit)>,
    pub list_offset: usize,
    pub visible_count: usize,
    pub voice: Option<Session>,
    pub listening: bool,
    pub settings: Settings,
    pub demo: bool,
    pub once: bool,
    pub quit: bool,
    pub quit_requested: bool,
    refreshed: Instant,
    requests: Option<Sender<Action>>,
    replies: Receiver<Reply>,
}

impl App {
    pub fn new(demo: bool, once: bool, settings: Settings) -> Self {
        let (tx, rx) = mpsc::channel();
        let (reply_tx, replies) = mpsc::channel();
        if !demo {
            std::thread::spawn(move || {
                for action in rx {
                    if reply_tx.send(audio::execute(action)).is_err() {
                        break;
                    }
                }
            });
        }
        let mut app = Self {
            snapshot: if demo {
                audio::demo()
            } else {
                Snapshot::default()
            },
            selected: 0,
            query: String::new(),
            searching: false,
            input: String::new(),
            overlay: None,
            overlay_scroll: 0,
            notice: None,
            error: None,
            loaded: demo,
            pending: None,
            queued: VecDeque::new(),
            hits: Vec::new(),
            list_offset: 0,
            visible_count: 1,
            voice: None,
            listening: false,
            settings,
            demo,
            once,
            quit: false,
            quit_requested: false,
            refreshed: Instant::now(),
            requests: (!demo).then_some(tx),
            replies,
        };
        if !demo {
            app.request(Action::Refresh);
        }
        app
    }

    pub fn filtered(&self) -> Vec<usize> {
        let query = voice::normalize(&self.query);
        let words: Vec<_> = query.split_whitespace().collect();
        self.snapshot
            .sinks
            .iter()
            .enumerate()
            .filter(|(_, sink)| {
                let haystack = voice::normalize(&format!(
                    "{} {} {} {}",
                    sink.description, sink.kind, sink.port, sink.name
                ));
                words.iter().all(|word| haystack.contains(word))
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected_sink(&self) -> Option<&Sink> {
        self.snapshot
            .sinks
            .get(self.selected)
            .filter(|_| self.filtered().contains(&self.selected))
    }

    pub fn effective_default(&self) -> &str {
        self.queued
            .iter()
            .rev()
            .chain(self.pending.iter())
            .find_map(|action| match action {
                Action::Switch(name) => Some(name.as_str()),
                _ => None,
            })
            .unwrap_or(&self.snapshot.default)
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    pub fn request(&mut self, action: Action) {
        if self.demo {
            match action {
                Action::Refresh => self.notify("Demo devices refreshed.", false),
                Action::Switch(name) => {
                    self.snapshot.default = name;
                    self.notify(
                        "Demo output connected. Your system audio is unchanged.",
                        false,
                    );
                    if self.once {
                        self.quit = true;
                    }
                }
                Action::Volume(name, volume) => {
                    if let Some(sink) = self.snapshot.sinks.iter_mut().find(|s| s.name == name) {
                        sink.volume = volume;
                    }
                }
                Action::Mute(name, muted) => {
                    if let Some(sink) = self.snapshot.sinks.iter_mut().find(|s| s.name == name) {
                        sink.muted = muted;
                    }
                }
            }
            return;
        }
        if self.pending.is_some() {
            if !matches!(action, Action::Refresh) {
                // Coalesce rapid slider/key changes instead of filling a command queue.
                if let Some(last) = self
                    .queued
                    .back_mut()
                    .filter(|last| match (&**last, &action) {
                        (Action::Volume(a, _), Action::Volume(b, _))
                        | (Action::Mute(a, _), Action::Mute(b, _)) => a == b,
                        (Action::Switch(_), Action::Switch(_)) => true,
                        _ => false,
                    })
                {
                    *last = action;
                } else if self.queued.len() < 16 {
                    self.queued.push_back(action);
                } else {
                    self.notify("Finishing your audio changes. Please wait a moment.", false);
                }
            }
            return;
        }
        if self
            .requests
            .as_ref()
            .is_some_and(|tx| tx.send(action.clone()).is_ok())
        {
            self.pending = Some(action);
        } else {
            self.notify("The audio worker stopped. Restart the switcher.", true);
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: Snapshot) {
        let name = self
            .snapshot
            .sinks
            .get(self.selected)
            .map(|sink| sink.name.clone());
        self.selected = name
            .and_then(|name| snapshot.sinks.iter().position(|s| s.name == name))
            .or_else(|| {
                snapshot
                    .sinks
                    .iter()
                    .position(|s| s.name == snapshot.default)
            })
            .unwrap_or(0);
        self.snapshot = snapshot;
        self.reconcile_selection();
    }

    pub fn reconcile_selection(&mut self) {
        let visible = self.filtered();
        if !visible.contains(&self.selected) {
            self.selected = visible.first().copied().unwrap_or(0);
        }
    }

    pub fn poll(&mut self) {
        while let Ok(reply) = self.replies.try_recv() {
            let switched = matches!(self.pending, Some(Action::Switch(_)));
            self.pending = None;
            self.loaded = true;
            self.refreshed = Instant::now();
            match reply.snapshot {
                Ok(snapshot) => {
                    self.error = None;
                    self.apply_snapshot(snapshot);
                    if let Some(message) = reply.notice {
                        self.notify(message, reply.warning);
                    }
                    if self.once && switched && !reply.warning {
                        self.quit_requested = true;
                    }
                }
                Err(error) => {
                    if switched {
                        self.queued.clear();
                    }
                    self.error = Some(error.clone());
                    self.notify(error, true);
                }
            }
            if let Some(action) = self.queued.pop_front() {
                self.request(action);
            }
        }
        let mut heard = None;
        if let Some(session) = &self.voice {
            while let Ok(line) = session.lines.try_recv() {
                if line.contains("\"listening\"") {
                    self.listening = true;
                }
            }
            if let Ok(result) = session.result.try_recv() {
                heard = Some(result);
            }
        }
        if let Some(result) = heard {
            self.voice = None;
            self.listening = false;
            match result {
                Ok(text) if text.trim().is_empty() => {
                    self.notify("No speech heard. Press v and try again.", true)
                }
                Ok(text) => self.execute_text(&text),
                Err(error) => self.notify(error, true),
            }
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|n| n.at.elapsed() > Duration::from_secs(if n.error { 12 } else { 5 }))
        {
            self.notice = None;
        }
        if !self.demo
            && !self.quit_requested
            && self.pending.is_none()
            && self.refreshed.elapsed() > Duration::from_secs(3)
        {
            self.request(Action::Refresh);
        }
        if self.quit_requested
            && (self.pending.is_none() || matches!(self.pending, Some(Action::Refresh)))
            && self.queued.is_empty()
        {
            self.quit = true;
        }
    }

    pub fn navigate(&mut self, amount: isize) {
        let visible = self.filtered();
        if visible.is_empty() {
            return;
        }
        let pos = visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0);
        self.selected = visible[pos.saturating_add_signed(amount).min(visible.len() - 1)];
    }

    pub fn connect(&mut self) {
        if let Some(sink) = self.selected_sink() {
            if sink.name == self.effective_default() {
                if self.once {
                    self.quit_requested = true;
                } else {
                    self.notify("This output is already connected.", false);
                }
            } else {
                self.request(Action::Switch(sink.name.clone()));
            }
        }
    }

    pub fn shown_volume(&self, sink: &Sink) -> u16 {
        self.queued
            .iter()
            .rev()
            .chain(self.pending.iter())
            .find_map(|action| match action {
                Action::Volume(name, value) if *name == sink.name => Some(*value),
                _ => None,
            })
            .unwrap_or(sink.volume)
    }

    pub fn shown_mute(&self, sink: &Sink) -> bool {
        self.queued
            .iter()
            .rev()
            .chain(self.pending.iter())
            .find_map(|action| match action {
                Action::Mute(name, value) if *name == sink.name => Some(*value),
                _ => None,
            })
            .unwrap_or(sink.muted)
    }

    pub fn adjust_volume(&mut self, amount: i16) {
        if let Some(sink) = self.selected_sink() {
            let volume =
                (i32::from(self.shown_volume(sink)) + i32::from(amount)).clamp(0, 100) as u16;
            self.request(Action::Volume(sink.name.clone(), volume));
        }
    }

    pub fn toggle_mute(&mut self) {
        if let Some(sink) = self.selected_sink() {
            self.request(Action::Mute(sink.name.clone(), !self.shown_mute(sink)));
        }
    }

    pub fn start_voice(&mut self) {
        if self.voice.is_some() {
            self.voice = None;
            self.listening = false;
            self.notify("Voice capture cancelled.", false);
        } else if self.demo {
            self.notify(
                "Voice recording is off in demo mode. Press : to try a command.",
                false,
            );
        } else if !self.settings.ready() {
            self.overlay = Some(Overlay::Voice);
            self.overlay_scroll = 0;
        } else {
            self.voice = Some(voice::listen(self.settings.clone()));
            self.listening = false;
        }
    }

    pub fn execute_text(&mut self, text: &str) {
        let intent = match voice::parse(text) {
            Ok(intent) => intent,
            Err(error) => {
                self.notify(error, true);
                return;
            }
        };
        if matches!(intent, Intent::Cancel) {
            self.notify("Command cancelled.", false);
            return;
        }
        match intent {
            Intent::Switch(query) => {
                let matches: Vec<_> = self
                    .snapshot
                    .sinks
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        let haystack = voice::normalize(&format!(
                            "{} {} {} {}",
                            s.description, s.kind, s.port, s.name
                        ));
                        query.split_whitespace().all(|word| haystack.contains(word))
                    })
                    .map(|(i, _)| i)
                    .collect();
                if matches.len() == 1 {
                    self.query.clear();
                    self.selected = matches[0];
                    self.connect();
                } else {
                    self.query = query;
                    self.reconcile_selection();
                    self.notify(
                        if matches.is_empty() {
                            "No matching output. Press / to edit your search."
                        } else {
                            "Several outputs match. Select one and press Enter."
                        },
                        true,
                    );
                }
            }
            Intent::Next | Intent::Previous => {
                if self.snapshot.sinks.is_empty() {
                    self.notify("No audio outputs are available.", true);
                    return;
                }
                let current = self
                    .snapshot
                    .sinks
                    .iter()
                    .position(|s| s.name == self.effective_default())
                    .unwrap_or(0);
                let len = self.snapshot.sinks.len();
                self.selected = if intent == Intent::Next {
                    (current + 1) % len
                } else {
                    (current + len - 1) % len
                };
                self.query.clear();
                self.connect();
            }
            Intent::Volume(_) | Intent::Adjust(_) | Intent::Mute(_) => {
                let Some(sink) = self
                    .snapshot
                    .sinks
                    .iter()
                    .find(|s| s.name == self.effective_default())
                else {
                    self.notify("No active output. Connect a device first.", true);
                    return;
                };
                let name = sink.name.clone();
                let action = match intent {
                    Intent::Volume(n) => Action::Volume(name, n),
                    Intent::Adjust(n) => Action::Volume(
                        name,
                        (i32::from(self.shown_volume(sink)) + i32::from(n)).clamp(0, 100) as u16,
                    ),
                    Intent::Mute(muted) => Action::Mute(name, muted),
                    _ => unreachable!(),
                };
                self.request(action);
                self.notify(format!("Command: {text}"), false);
            }
            Intent::Cancel => {}
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'd'))
        {
            self.voice = None;
            self.quit_requested = true;
            return;
        }
        if self.overlay.is_some() {
            if self.overlay == Some(Overlay::Command) {
                match key.code {
                    KeyCode::Esc => self.overlay = None,
                    KeyCode::Enter => {
                        let text = std::mem::take(&mut self.input);
                        self.overlay = None;
                        self.execute_text(&text);
                    }
                    KeyCode::Backspace => {
                        self.input.pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.input.push(c);
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q' | '?' | 'v') => self.overlay = None,
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.overlay_scroll = self.overlay_scroll.saturating_add(1)
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.overlay_scroll = self.overlay_scroll.saturating_sub(1)
                    }
                    KeyCode::PageDown => {
                        self.overlay_scroll = self.overlay_scroll.saturating_add(5)
                    }
                    KeyCode::PageUp => self.overlay_scroll = self.overlay_scroll.saturating_sub(5),
                    _ => {}
                }
            }
            return;
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.query.clear();
                    self.searching = false;
                    self.reconcile_selection();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.reconcile_selection();
                }
                KeyCode::Up => self.navigate(-1),
                KeyCode::Down => self.navigate(1),
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.clear();
                    self.reconcile_selection();
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.query.push(c);
                    self.reconcile_selection();
                }
                _ => {}
            }
            return;
        }
        // Leave terminal zoom shortcuts (Ctrl/Alt +/-) to the emulator.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return;
        }
        match key.code {
            KeyCode::Char('q') => {
                self.voice = None;
                self.quit_requested = true;
            }
            KeyCode::Esc => {
                if self.voice.is_some() {
                    self.voice = None;
                    self.listening = false;
                    self.notify("Voice capture cancelled.", false);
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.reconcile_selection();
                } else {
                    self.quit_requested = true;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.navigate(-1),
            KeyCode::Down | KeyCode::Char('j') => self.navigate(1),
            KeyCode::PageUp => self.navigate(-(self.visible_count as isize)),
            KeyCode::PageDown => self.navigate(self.visible_count as isize),
            KeyCode::Home => self.navigate(isize::MIN),
            KeyCode::End => self.navigate(isize::MAX),
            KeyCode::Enter => self.connect(),
            KeyCode::Left | KeyCode::Char('-' | 'h') => self.adjust_volume(-5),
            KeyCode::Right | KeyCode::Char('+' | '=' | 'l') => self.adjust_volume(5),
            KeyCode::Char('m' | ' ') => self.toggle_mute(),
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char(':') => {
                self.overlay = Some(Overlay::Command);
                self.input.clear();
            }
            KeyCode::Char('r') => {
                self.request(Action::Refresh);
                self.notify("Refreshing audio outputs…", false);
            }
            KeyCode::Char('v') => self.start_voice(),
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.overlay = Some(Overlay::Help);
                self.overlay_scroll = 0;
            }
            KeyCode::Char('e') => {
                self.overlay = Some(Overlay::Status);
                self.overlay_scroll = 0;
            }
            KeyCode::Char(c @ '1'..='9') => {
                if let Some(index) = self.filtered().get(c as usize - '1' as usize) {
                    self.selected = *index;
                }
            }
            _ => {}
        }
    }

    pub fn mouse(&mut self, event: MouseEvent) {
        if self.overlay.is_some() {
            match event.kind {
                MouseEventKind::ScrollDown => {
                    self.overlay_scroll = self.overlay_scroll.saturating_add(1)
                }
                MouseEventKind::ScrollUp => {
                    self.overlay_scroll = self.overlay_scroll.saturating_sub(1)
                }
                _ => {}
            }
            return;
        }
        match event.kind {
            MouseEventKind::ScrollUp => self.navigate(-1),
            MouseEventKind::ScrollDown => self.navigate(1),
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                let hit = self
                    .hits
                    .iter()
                    .find(|(rect, _)| rect.contains((event.column, event.row).into()))
                    .map(|(_, hit)| *hit);
                if matches!(event.kind, MouseEventKind::Drag(_))
                    && !matches!(hit, Some(Hit::Volume(_)))
                {
                    return;
                }
                match hit {
                    Some(Hit::Device(index)) => self.selected = index,
                    Some(Hit::Connect) => self.connect(),
                    Some(Hit::Mute) => self.toggle_mute(),
                    Some(Hit::Volume(n)) => {
                        if let Some(sink) = self.selected_sink() {
                            self.request(Action::Volume(sink.name.clone(), n));
                        }
                    }
                    Some(Hit::Voice) => self.start_voice(),
                    Some(Hit::Search) => self.searching = true,
                    Some(Hit::Help) => {
                        self.overlay = Some(Overlay::Help);
                        self.overlay_scroll = 0;
                    }
                    Some(Hit::Command) => {
                        self.overlay = Some(Overlay::Command);
                        self.input.clear();
                    }
                    Some(Hit::Close) => {
                        self.voice = None;
                        self.quit_requested = true;
                    }
                    Some(Hit::Status) => {
                        self.overlay = Some(Overlay::Status);
                        self.overlay_scroll = 0;
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> App {
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
    fn selection_survives_reorder_and_device_removal() {
        let mut app = demo();
        app.selected = 2;
        let mut snapshot = app.snapshot.clone();
        snapshot.sinks.reverse();
        app.apply_snapshot(snapshot);
        assert_eq!(app.selected_sink().unwrap().name, "hdmi.monitor");
        let mut snapshot = app.snapshot.clone();
        snapshot.sinks.retain(|s| s.name != "hdmi.monitor");
        app.apply_snapshot(snapshot);
        assert_eq!(app.selected_sink().unwrap().name, "usb.headphones");
    }

    #[test]
    fn searches_multiple_words_and_handles_no_matches() {
        let mut app = demo();
        app.query = "living bluetooth".into();
        app.reconcile_selection();
        assert_eq!(app.filtered().len(), 1);
        assert_eq!(app.selected_sink().unwrap().name, "bluez.speakers");
        app.query = "missing".into();
        app.reconcile_selection();
        app.navigate(1);
        app.connect();
        assert!(app.selected_sink().is_none());
    }

    #[test]
    fn voice_volume_targets_active_output_and_caps_adjustments() {
        let mut app = demo();
        app.selected = 2;
        app.execute_text("volume 95");
        app.execute_text("louder");
        app.execute_text("louder");
        assert_eq!(app.snapshot.sinks[0].volume, 100);
        assert_eq!(app.snapshot.sinks[2].volume, 30);
        app.adjust_volume(-50);
        assert_eq!(app.snapshot.sinks[2].volume, 0);
    }

    #[test]
    fn ambiguous_voice_switch_never_changes_output() {
        let mut app = demo();
        let current = app.snapshot.default.clone();
        app.execute_text("switch to speakers");
        assert_eq!(app.snapshot.default, current);
        assert_eq!(app.filtered().len(), 2);
    }

    #[test]
    fn terminal_zoom_does_not_change_volume() {
        let mut app = demo();
        app.key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::CONTROL));
        assert_eq!(app.snapshot.sinks[0].volume, 64);
    }

    #[test]
    fn command_after_connect_targets_the_requested_output() {
        let mut app = demo();
        app.pending = Some(Action::Switch("hdmi.monitor".into()));
        app.execute_text("volume 20");
        assert_eq!(app.snapshot.sinks[2].volume, 20);
        assert_eq!(app.snapshot.sinks[0].volume, 64);
    }

    #[test]
    fn rapid_controls_coalesce_volume_without_discarding_mute() {
        let mut app = demo();
        app.demo = false;
        app.pending = Some(Action::Refresh);
        app.adjust_volume(5);
        app.adjust_volume(5);
        app.toggle_mute();
        app.adjust_volume(5);
        assert_eq!(app.queued.len(), 3);
        let sink = &app.snapshot.sinks[0];
        assert_eq!(app.shown_volume(sink), 79);
        assert!(app.shown_mute(sink));
    }
}
