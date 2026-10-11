#!/usr/bin/env python3
"""Capture the running demo UI with Kitty's native PNG screenshot command.

Requires Kitty with `kitten @ screenshot` support (0.49+) and a working graphics
session. Kitty stays hidden; no desktop capture or compositor configuration is
needed. No system audio settings or microphone input are used.
"""
import argparse
from contextlib import contextmanager
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def remote(socket, *args, check=True):
    return subprocess.run(
        ["kitty", "@", "--to", socket, "--use-password=never", *args],
        check=check, capture_output=True, text=True, timeout=15,
    )


@contextmanager
def terminal(binary, columns, rows):
    with tempfile.TemporaryDirectory(prefix="sound-switcher-screenshot-") as directory:
        socket = f"unix:{directory}/kitty.sock"
        overrides = {
            "allow_remote_control": "socket-only",
            "remember_window_size": "no",
            "initial_window_width": f"{columns}c",
            "initial_window_height": f"{rows}c",
            "font_family": "DejaVu Sans Mono",
            "font_size": "13",
            "background": "#0b1018",
            "foreground": "#ebf2fa",
            "window_padding_width": "12",
            "background_opacity": "1",
            "hide_window_decorations": "yes",
            "confirm_os_window_close": "0",
            "enable_audio_bell": "no",
            "shell_integration": "disabled",
        }
        command = ["kitty", "--config", "NONE", "--start-as=hidden", "--listen-on", socket]
        for key, value in overrides.items():
            command.extend(["--override", f"{key}={value}"])
        command.extend([str(binary), "--demo", "--no-mouse"])
        environment = os.environ.copy()
        # Captures document the app's color theme even when a CI shell disables
        # ANSI colors. This is scoped to this dedicated terminal process.
        environment.pop("NO_COLOR", None)
        environment["COLORTERM"] = "truecolor"
        with (Path(directory) / "kitty.log").open("w+") as log:
            process = subprocess.Popen(command, cwd=ROOT, env=environment, stdout=log, stderr=log)
            try:
                wait_for(socket, process, "Studio headphones")
                yield socket, process
            except Exception:
                if process.poll() is not None:
                    log.seek(0)
                    detail = log.read().strip()
                    if detail:
                        print(detail[-2000:])
                raise
            finally:
                if process.poll() is None:
                    try:
                        remote(socket, "send-key", "ctrl+c", check=False)
                        process.wait(timeout=3)
                    except (subprocess.SubprocessError, OSError):
                        process.terminate()
                        try:
                            process.wait(timeout=3)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()


def wait_for(socket, process, text):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("The screenshot terminal exited before the UI was ready.")
        result = remote(socket, "get-text", check=False)
        if result.returncode == 0 and text in result.stdout:
            return
        time.sleep(0.05)
    raise RuntimeError(f"Timed out waiting for the terminal to display {text!r}.")


def capture(binary, directory, name, columns, rows, text=None):
    with terminal(binary, columns, rows) as (socket, process):
        if text is not None:
            remote(socket, "send-text", text)
            wait_for(socket, process, text[1:])
        path = directory / name
        remote(socket, "screenshot", str(path))
        if not path.read_bytes().startswith(b"\x89PNG\r\n\x1a\n"):
            raise RuntimeError(f"Kitty did not produce a PNG screenshot: {path}")
        print(f"Captured {columns} × {rows}: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=ROOT / "docs")
    parser.add_argument("--binary", type=Path, help="Use this binary instead of building the release.")
    args = parser.parse_args()
    if not shutil.which("kitty"):
        parser.error("Kitty 0.49+ is required to capture native terminal screenshots.")
    support = subprocess.run(["kitty", "@", "screenshot", "--help"], capture_output=True, timeout=10)
    if support.returncode:
        parser.error("This Kitty version does not support the screenshot command. Upgrade Kitty.")
    if args.binary is None:
        subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    binary = (args.binary or ROOT / "target/release/sound_switcher").resolve()
    if not binary.is_file():
        parser.error(f"Sound Switcher binary not found: {binary}")
    directory = args.output_dir.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    capture(binary, directory, "preview.png", 120, 36)
    capture(binary, directory, "compact.png", 40, 18)
    capture(binary, directory, "commands.png", 80, 24, ":switch to living room speakers")


if __name__ == "__main__":
    main()
