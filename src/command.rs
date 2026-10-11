use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Output, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};
use std::time::{Duration, Instant};

/// Drain both pipes concurrently: a large device list must never block pactl.
pub fn capture(
    command: &mut Command,
    action: &str,
    timeout: Duration,
    cancelled: Option<Arc<AtomicBool>>,
    lines: Option<Sender<String>>,
) -> Result<Output, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{action}: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut reader = BufReader::new(stdout.take(4 * 1024 * 1024));
        loop {
            let mut line = Vec::new();
            if reader.read_until(b'\n', &mut line).unwrap_or(0) == 0 {
                break;
            }
            if let Some(tx) = &lines {
                let _ = tx.send(String::from_utf8_lossy(&line).trim().to_owned());
            }
            bytes.extend(line);
        }
        bytes
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.take(64 * 1024).read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => break Err(format!("{action}: {error}")),
        }
        if cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            break Err("Voice capture cancelled".to_owned());
        }
        if started.elapsed() >= timeout {
            break Err(format!("{action} timed out"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if status.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    let status = status?;
    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr);
        return Err(format!(
            "{action}: {}",
            if detail.trim().is_empty() {
                status.to_string()
            } else {
                detail.trim().to_owned()
            }
        ));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
