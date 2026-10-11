use serde_json::Value;
use std::process::Command;
use std::time::Duration;

use crate::command::capture;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sink {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub port: String,
    pub sample_spec: String,
    pub state: String,
    pub volume: u16,
    pub muted: bool,
}

impl Sink {
    /// Prefer the monitor/model name over PulseAudio's verbose profile suffix.
    pub fn label(&self) -> &str {
        if let Some((_, model)) = self.description.rsplit_once('[')
            && let Some(model) = model.strip_suffix(']')
            && !model.is_empty()
        {
            return model;
        }
        [
            " Analog Stereo",
            " Digital Stereo (HDMI)",
            " Digital Stereo (IEC958)",
        ]
        .iter()
        .find_map(|suffix| self.description.strip_suffix(suffix))
        .unwrap_or(&self.description)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub sinks: Vec<Sink>,
    pub default: String,
    pub streams: usize,
}

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    Switch(String),
    Volume(String, u16),
    Mute(String, bool),
}

pub struct Reply {
    pub snapshot: Result<Snapshot, String>,
    pub notice: Option<String>,
    pub warning: bool,
}

fn text(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

pub fn parse_sinks(bytes: &[u8]) -> Result<Vec<Sink>, String> {
    let values: Vec<Value> =
        serde_json::from_slice(bytes).map_err(|e| format!("Could not read audio devices: {e}"))?;
    values
        .into_iter()
        .map(|value| {
            let name = text(&value, "name");
            if name.is_empty() {
                return Err("An audio device has no name".to_owned());
            }
            let description = text(&value, "description");
            let description = if description.is_empty() {
                name.clone()
            } else {
                description
            };
            let port = value["ports"]
                .as_array()
                .and_then(|ports| ports.iter().find(|p| p["name"] == value["active_port"]));
            let port_name = port.map(|p| text(p, "description")).unwrap_or_default();
            let hint = format!(
                "{name} {description} {} {}",
                text(&value["properties"], "device.form_factor"),
                port_name
            )
            .to_lowercase();
            let kind = if hint.contains("bluez") || hint.contains("bluetooth") {
                "Bluetooth"
            } else if hint.contains("hdmi") || hint.contains("displayport") {
                "HDMI"
            } else if hint.contains("headphone") || hint.contains("headset") {
                "Headphones"
            } else if hint.contains("usb") {
                "USB audio"
            } else if hint.contains("analog") || hint.contains("speaker") {
                "Speakers"
            } else {
                "Virtual audio"
            }
            .to_owned();
            let volumes: Vec<f64> = value["volume"]
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
                .filter_map(|v| {
                    v["value"]
                        .as_f64()
                        .map(|n| n / 65536.0 * 100.0)
                        .or_else(|| {
                            v["value_percent"]
                                .as_str()?
                                .trim_end_matches('%')
                                .parse()
                                .ok()
                        })
                })
                .collect();
            let volume = if volumes.is_empty() {
                0
            } else {
                (volumes.iter().sum::<f64>() / volumes.len() as f64)
                    .round()
                    .clamp(0.0, u16::MAX as f64) as u16
            };
            Ok(Sink {
                id: value["index"]
                    .as_u64()
                    .and_then(|n| n.try_into().ok())
                    .unwrap_or_default(),
                name,
                description,
                kind,
                port: port_name,
                volume,
                sample_spec: text(&value, "sample_specification"),
                state: text(&value, "state"),
                muted: value["mute"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

fn pactl(args: &[&str]) -> Result<Vec<u8>, String> {
    capture(
        Command::new("pactl").env("LC_ALL", "C").args(args),
        "Audio server",
        Duration::from_secs(3),
        None,
        None,
    )
    .map(|out| out.stdout)
}

pub fn fetch() -> Result<Snapshot, String> {
    let sinks = parse_sinks(&pactl(&["--format=json", "list", "sinks"])?)?;
    let default = if sinks.is_empty() {
        String::new()
    } else {
        String::from_utf8_lossy(&pactl(&["get-default-sink"])?)
            .trim()
            .to_owned()
    };
    let streams = pactl(&["list", "sink-inputs", "short"])
        .map(|bytes| {
            String::from_utf8_lossy(&bytes)
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .unwrap_or(0);
    Ok(Snapshot {
        sinks,
        default,
        streams,
    })
}

pub fn execute(action: Action) -> Reply {
    let mut notice = None;
    let mut warning = false;
    let change = match action {
        Action::Refresh => Ok(()),
        Action::Volume(name, volume) => {
            pactl(&["set-sink-volume", &name, &format!("{}%", volume.min(100))]).map(|_| ())
        }
        Action::Mute(name, muted) => {
            pactl(&["set-sink-mute", &name, if muted { "1" } else { "0" }]).map(|_| ())
        }
        Action::Switch(name) => pactl(&["set-default-sink", &name]).map(|_| {
            // A disappearing stream should not stop the remaining streams moving.
            let failed = match pactl(&["list", "sink-inputs", "short"]) {
                Ok(bytes) => String::from_utf8_lossy(&bytes)
                    .lines()
                    .filter_map(|line| line.split_whitespace().next())
                    .filter(|id| pactl(&["move-sink-input", id, &name]).is_err())
                    .count(),
                Err(_) => 1,
            };
            if failed > 0 {
                warning = true;
                notice = Some(
                    "Output changed. Some playing apps could not be moved; restart their playback."
                        .to_owned(),
                );
            } else {
                notice = Some("Output connected. Playing apps moved with you.".to_owned());
            }
        }),
    };
    Reply {
        snapshot: change.and_then(|_| fetch()),
        notice,
        warning,
    }
}

pub fn demo() -> Snapshot {
    Snapshot {
        default: "usb.headphones".into(),
        streams: 2,
        sinks: vec![
            Sink {
                id: 1,
                name: "usb.headphones".into(),
                description: "Studio headphones".into(),
                kind: "Headphones".into(),
                port: "USB-C".into(),
                sample_spec: "s24le 2ch 48000Hz".into(),
                state: "RUNNING".into(),
                volume: 64,
                muted: false,
            },
            Sink {
                id: 2,
                name: "bluez.speakers".into(),
                description: "Living room speakers".into(),
                kind: "Bluetooth".into(),
                port: "High fidelity playback".into(),
                sample_spec: "s16le 2ch 44100Hz".into(),
                state: "IDLE".into(),
                volume: 42,
                muted: false,
            },
            Sink {
                id: 3,
                name: "hdmi.monitor".into(),
                description: "Studio display".into(),
                kind: "HDMI".into(),
                port: "HDMI / DisplayPort".into(),
                sample_spec: "s32le 2ch 48000Hz".into(),
                state: "SUSPENDED".into(),
                volume: 30,
                muted: false,
            },
            Sink {
                id: 4,
                name: "alsa.speakers".into(),
                description: "Built-in speakers".into(),
                kind: "Speakers".into(),
                port: "Analog output".into(),
                sample_spec: "s16le 2ch 48000Hz".into(),
                state: "SUSPENDED".into(),
                volume: 50,
                muted: false,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unicode_channel_average_and_active_port() {
        let sinks = parse_sinks(br#"[{"index":42,"name":"alsa.usb","description":"Studio","mute":true,"sample_specification":"s24le 2ch 48000Hz","volume":{"left":{"value":32768},"right":{"value":65536}},"active_port":"headphones","ports":[{"name":"headphones","description":"Headphones"}]}]"#).unwrap();
        assert_eq!(sinks[0].volume, 75);
        assert!(sinks[0].muted);
        assert_eq!(sinks[0].kind, "Headphones");
        assert_eq!(sinks[0].port, "Headphones");
    }

    #[test]
    fn handles_empty_and_rejects_invalid_server_data() {
        assert!(parse_sinks(b"[]").unwrap().is_empty());
        assert!(parse_sinks(b"not json").is_err());
        assert!(parse_sinks(br#"[{"index":1}]"#).is_err());
    }
}
