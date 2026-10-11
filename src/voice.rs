use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    Arc,
    atomic::AtomicBool,
    mpsc::{self, Receiver},
};
use std::time::Duration;

use crate::command::capture;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Switch(String),
    Volume(u16),
    Adjust(i16),
    Mute(bool),
    Next,
    Previous,
    Cancel,
}

pub fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn number(words: &str) -> Option<u16> {
    if let Ok(n) = words.parse() {
        return Some(n);
    }
    let units = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    if let Some(n) = units.iter().position(|word| *word == words) {
        return Some(n as u16);
    }
    if words == "one hundred" || words == "a hundred" || words == "hundred" {
        return Some(100);
    }
    let tens = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    let parts: Vec<_> = words.split_whitespace().collect();
    let first = *parts.first()?;
    let base = (tens.iter().position(|word| *word == first)? as u16 + 2) * 10;
    match parts.as_slice() {
        [_] => Some(base),
        [_, unit] => units[1..10]
            .iter()
            .position(|word| word == unit)
            .map(|n| base + n as u16 + 1),
        _ => None,
    }
}

pub fn parse(text: &str) -> Result<Intent, String> {
    let clean = normalize(text);
    let clean = clean.strip_prefix("please ").unwrap_or(&clean);
    let clean = clean.strip_suffix(" please").unwrap_or(clean);
    match clean {
        "mute" | "mute audio" => return Ok(Intent::Mute(true)),
        "unmute" | "unmute audio" => return Ok(Intent::Mute(false)),
        "volume up" | "louder" => return Ok(Intent::Adjust(5)),
        "volume down" | "quieter" => return Ok(Intent::Adjust(-5)),
        "next" | "next output" | "next device" => return Ok(Intent::Next),
        "previous" | "previous output" | "previous device" => return Ok(Intent::Previous),
        "cancel" | "stop" => return Ok(Intent::Cancel),
        _ => {}
    }
    if let Some(value) = clean
        .strip_prefix("set volume to ")
        .or_else(|| clean.strip_prefix("volume "))
    {
        let value = value.strip_suffix(" percent").unwrap_or(value);
        return match number(value) {
            Some(n) if n <= 100 => Ok(Intent::Volume(n)),
            _ => Err("Choose a volume from 0 to 100 percent.".into()),
        };
    }
    if let Some(device) = clean
        .strip_prefix("switch to ")
        .or_else(|| clean.strip_prefix("connect to "))
        .or_else(|| clean.strip_prefix("use "))
        .filter(|device| !device.is_empty())
    {
        return Ok(Intent::Switch(device.to_owned()));
    }
    Err("Try \"switch to headphones\", \"volume 50\", or \"mute\".".into())
}

#[derive(Clone)]
pub struct Settings {
    pub model: Option<PathBuf>,
    pub python: String,
}

impl Settings {
    pub fn from_env() -> Self {
        Self {
            model: std::env::var_os("SOUND_SWITCHER_VOICE_MODEL").map(PathBuf::from),
            python: std::env::var("SOUND_SWITCHER_VOICE_PYTHON")
                .unwrap_or_else(|_| "python3".into()),
        }
    }

    pub fn ready(&self) -> bool {
        self.model.as_ref().is_some_and(|path| path.is_dir())
    }
}

pub struct Session {
    pub lines: Receiver<String>,
    pub result: Receiver<Result<String, String>>,
    pub cancelled: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

pub fn listen(settings: Settings) -> Session {
    let (line_tx, lines) = mpsc::channel();
    let (result_tx, result) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let worker = std::thread::spawn(move || {
        let outcome = capture(
            Command::new(settings.python)
                .args(["-c", include_str!("voice.py")])
                .arg(settings.model.unwrap_or_default()),
            "Voice control",
            Duration::from_secs(30),
            Some(flag),
            Some(line_tx),
        )
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .find_map(|value| value["text"].as_str().map(str::to_owned))
                .ok_or_else(|| "The recognizer did not return a command.".into())
        });
        let _ = result_tx.send(outcome);
    });
    Session {
        lines,
        result,
        cancelled,
        worker: Some(worker),
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Reap the recognizer before the app can exit, so cancellation cannot
        // leave a microphone-owning Python process behind.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn understands_spoken_numbers_and_deliberate_actions() {
        assert_eq!(
            parse("Please set volume to sixty-five percent"),
            Ok(Intent::Volume(65))
        );
        assert_eq!(parse("volume 0%"), Ok(Intent::Volume(0)));
        assert_eq!(parse("volume one hundred"), Ok(Intent::Volume(100)));
        assert_eq!(parse("unmute"), Ok(Intent::Mute(false)));
        assert_eq!(
            parse("switch to USB headphones"),
            Ok(Intent::Switch("usb headphones".into()))
        );
        assert_eq!(parse("quieter"), Ok(Intent::Adjust(-5)));
    }

    #[test]
    fn refuses_ambiguous_or_out_of_range_instructions() {
        for input in [
            "do not mute",
            "volume 200",
            "volume twenty ten",
            "switch to",
            "",
            "play something",
        ] {
            assert!(parse(input).is_err(), "{input}");
        }
    }
}
