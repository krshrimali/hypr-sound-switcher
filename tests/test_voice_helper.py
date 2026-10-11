"""Exercise the embedded Python recognizer with synthetic input, without a mic."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch


class VoiceHelper(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).resolve().parents[1] / "src/voice.py"
        spec = importlib.util.spec_from_file_location("voice_helper", path)
        self.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.helper)

    def test_records_pcm_and_closes_stream_before_recognition(self):
        captured = []
        stream_closed = []
        audio = b"\x00\x01" * 100

        class Stream:
            def __init__(self, **kwargs):
                self.kwargs = kwargs
                captured.append(kwargs)

            def __enter__(self):
                self.kwargs["callback"](audio, 100, None, None)
                return self

            def __exit__(self, *args):
                stream_closed.append(True)

        class Recognizer:
            def AcceptWaveform(self, data):
                self.assert_closed = bool(stream_closed)
                captured.append(bytes(data))
                if not self.assert_closed:
                    raise AssertionError("Microphone was left open during recognition")

            def FinalResult(self):
                return json.dumps({"text": "volume fifty"})

        vosk = SimpleNamespace(Model=lambda path: object(), KaldiRecognizer=lambda model, rate: Recognizer(),
                               SetLogLevel=lambda level: None)
        stdout = io.StringIO()
        with patch.dict(sys.modules, {"sounddevice": SimpleNamespace(RawInputStream=Stream), "vosk": vosk}), \
                patch.object(sys, "argv", ["voice.py", "/fake/model"]), \
                patch.object(self.helper.time, "monotonic", side_effect=[0, 0, 5]), \
                contextlib.redirect_stdout(stdout):
            self.helper.main()
        events = [json.loads(line) for line in stdout.getvalue().splitlines()]
        self.assertEqual(events, [{"status": "listening"}, {"text": "volume fifty"}])
        self.assertEqual(captured[0]["samplerate"], 16000)
        self.assertEqual(captured[0]["channels"], 1)
        self.assertEqual(captured[0]["dtype"], "int16")
        self.assertEqual(captured[1], audio)

    def test_missing_dependencies_produce_setup_guidance(self):
        with patch.dict(sys.modules, {"vosk": None, "sounddevice": None}):
            with self.assertRaisesRegex(RuntimeError, "Install vosk and sounddevice"):
                self.helper.main()
