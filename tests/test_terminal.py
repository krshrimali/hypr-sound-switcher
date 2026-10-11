"""Real PTY tests with fake audio/voice backends; never change system audio.

Run: cargo build && python3 -m unittest discover -s tests -v
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/sound_switcher"


class Terminal(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sound-switcher-test-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.state_path = self.directory / "state.json"
        self.log_path = self.directory / "commands.jsonl"
        sinks = [
            {"index": i, "name": name, "description": label, "state": "RUNNING",
             "sample_specification": "s16le 2ch 48000Hz", "mute": False,
             "volume": {"left": {"value_percent": "50%"}}, "ports": []}
            for i, name, label in [(1, "usb.headphones", "Studio headphones"),
                                   (2, "bluez.speakers", "Living room speakers"),
                                   (3, "alsa.speakers", "Built-in speakers")]
        ]
        self.state_path.write_text(json.dumps({"sinks": sinks, "default": "usb.headphones"}))
        fake = self.directory / "pactl"
        shutil.copyfile(ROOT / "tests/fixtures/pactl.py", fake)
        fake.chmod(0o755)
        voice = self.directory / "voice-python"
        voice.write_text('#!/usr/bin/env python3\nimport json,os,time\nfrom pathlib import Path\nPath(os.environ["FAKE_VOICE_PID"]).write_text(str(os.getpid()))\nprint(json.dumps({"status":"listening"}),flush=True)\ntime.sleep(float(os.environ.get("FAKE_VOICE_DELAY","0.1")))\nprint(json.dumps({"text":os.environ.get("FAKE_VOICE_TEXT","volume forty five")}))\n')
        voice.chmod(0o755)
        self.env = {
            **os.environ, "TERM": "xterm-256color", "PATH": f"{self.directory}:{os.environ['PATH']}",
            "SOUND_SWITCHER_TEST_STATE": str(self.state_path), "SOUND_SWITCHER_TEST_LOG": str(self.log_path),
            "SOUND_SWITCHER_VOICE_MODEL": str(self.directory), "SOUND_SWITCHER_VOICE_PYTHON": str(voice),
            "FAKE_VOICE_PID": str(self.directory / "voice.pid"),
        }
        self.process = None
        self.output = bytearray()

    def state(self):
        try:
            return json.loads(self.state_path.read_text())
        except json.JSONDecodeError:
            return self.state()

    def update(self, **values):
        state = self.state()
        state.update(values)
        self.state_path.write_text(json.dumps(state))

    def start(self, *args, ready=True):
        self.master, self.slave = pty.openpty()
        self.addCleanup(os.close, self.master)
        self.addCleanup(os.close, self.slave)
        self.original_mode = termios.tcgetattr(self.slave)
        self.resize(100, 30)
        self.process = subprocess.Popen([str(BINARY), *args], stdin=self.slave, stdout=self.slave,
                                        stderr=self.slave, env=self.env, start_new_session=True)
        self.addCleanup(self.stop)
        if ready:
            self.until(lambda: b"Studio headphones" in self.output)

    def resize(self, width, height):
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        if self.process is not None:
            self.process.send_signal(signal.SIGWINCH)

    def pump(self):
        if select.select([self.master], [], [], 0.03)[0]:
            try:
                self.output.extend(os.read(self.master, 65536))
            except OSError:
                pass

    def until(self, check, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump()
            if check():
                return
            if self.process.poll() is not None:
                break
        self.fail(f"Timed out. Terminal output: {self.output[-1500:]!r}")

    def send(self, text):
        os.write(self.master, text.encode())

    def stop(self):
        if self.process is not None and self.process.poll() is None:
            self.process.kill()
            self.process.wait()

    def close_cleanly(self):
        self.send("q")
        self.until(lambda: self.process.poll() is not None)
        self.assertEqual(self.process.returncode, 0)
        self.assertEqual(termios.tcgetattr(self.slave), self.original_mode)

    def test_switch_moves_all_streams_even_if_one_disappears(self):
        self.update(fail_moves=["101"])
        self.start()
        self.send("j\r")
        self.until(lambda: self.state()["default"] == "bluez.speakers")
        self.until(lambda: b"Some playing apps" in self.output)
        log = [json.loads(line) for line in self.log_path.read_text().splitlines()]
        self.assertIn(["move-sink-input", "102", "bluez.speakers"], log)
        self.close_cleanly()

    def test_rapid_volume_and_mute_preserve_both_actions(self):
        self.start()
        self.send("++++m++++")
        self.until(lambda: self.state()["sinks"][0]["volume"]["left"]["value_percent"] == "90%"
                   and self.state()["sinks"][0]["mute"])
        self.close_cleanly()

    def test_search_command_palette_and_resizing(self):
        self.start()
        self.send("/living\r\r")
        self.until(lambda: self.state()["default"] == "bluez.speakers")
        self.send(":volume 35\r")
        self.until(lambda: self.state()["sinks"][1]["volume"]["left"]["value_percent"] == "35%")
        for w, h in [(24, 7), (40, 15), (140, 50)]:
            self.resize(w, h)
            for _ in range(5):
                self.pump()
        self.close_cleanly()

    def test_voice_transcript_controls_active_output(self):
        self.start()
        self.send("jv")  # Highlight a different device; voice still targets the active one.
        self.until(lambda: self.state()["sinks"][0]["volume"]["left"]["value_percent"] == "45%")
        self.assertEqual(self.state()["sinks"][1]["volume"]["left"]["value_percent"], "50%")
        self.close_cleanly()

    def test_mouse_select_connect_and_drag_volume(self):
        self.start()
        # SGR mouse coordinates for the compact controls at 100 × 30 cells.
        self.send("\x1b[<0;8;19M\x1b[<0;8;19m")
        self.send("\x1b[<0;10;25M\x1b[<0;10;25m")
        self.until(lambda: self.state()["default"] == "bluez.speakers")
        self.send("\x1b[<0;6;24M\x1b[<32;95;24M\x1b[<0;95;24m")
        self.until(lambda: self.state()["sinks"][1]["volume"]["left"]["value_percent"] == "100%")
        self.close_cleanly()

    def test_cancelled_voice_never_executes(self):
        self.env["FAKE_VOICE_DELAY"] = "0.5"
        self.start()
        self.send("v")
        self.until(lambda: (self.directory / "voice.pid").exists())
        self.send("v")
        deadline = time.monotonic() + 0.65
        while time.monotonic() < deadline:
            self.pump()
        self.assertEqual(self.state()["sinks"][0]["volume"]["left"]["value_percent"], "50%")
        self.close_cleanly()

    def test_closing_app_reaps_active_recognizer(self):
        self.env["FAKE_VOICE_DELAY"] = "10"
        self.start()
        self.send("v")
        pid_file = self.directory / "voice.pid"
        self.until(pid_file.exists)
        pid = int(pid_file.read_text())
        self.close_cleanly()
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_failed_switch_keeps_app_open_and_default_unchanged(self):
        self.update(fail_switch=True)
        self.start("--once")
        self.send("j\r")
        self.until(lambda: b"Device disconnected" in self.output)
        self.assertIsNone(self.process.poll())
        self.assertEqual(self.state()["default"], "usb.headphones")
        self.close_cleanly()

    def test_once_mode_exits_after_success(self):
        self.start("--once", "--no-mouse")
        self.send("j\r")
        self.until(lambda: self.process.poll() is not None)
        self.assertEqual(self.process.returncode, 0)
        self.assertEqual(self.state()["default"], "bluez.speakers")
        self.assertEqual(termios.tcgetattr(self.slave), self.original_mode)

    def test_offline_server_can_recover(self):
        self.update(offline=True)
        self.start(ready=False)
        self.until(lambda: b"Connection refused" in self.output)
        self.update(offline=False)
        self.send("r")
        self.until(lambda: b"Studio headphones" in self.output)
        self.close_cleanly()

    def test_demo_never_calls_audio_or_voice_backend(self):
        self.start("--demo")
        self.send("j\r+m:vOLUME 20\rv")
        self.close_cleanly()
        self.assertFalse(self.log_path.exists())


if __name__ == "__main__":
    unittest.main()
