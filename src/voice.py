"""Embedded, optional offline recognizer. Audio stays in memory on this machine."""
import json
import queue
import sys
import time


def main():
    try:
        import sounddevice as sd
        from vosk import KaldiRecognizer, Model, SetLogLevel
    except ImportError:
        raise RuntimeError("Install vosk and sounddevice in your voice Python environment; see README.md.") from None

    SetLogLevel(-1)
    model = Model(sys.argv[1])
    recognizer = KaldiRecognizer(model, 16000)
    chunks = queue.Queue(maxsize=64)

    def callback(data, frames, timing, status):
        try:
            chunks.put_nowait(bytes(data))
        except queue.Full:
            pass

    # No recording starts until the user explicitly requests it in the app.
    with sd.RawInputStream(samplerate=16000, blocksize=4000, dtype="int16",
                           channels=1, callback=callback):
        print(json.dumps({"status": "listening"}), flush=True)
        deadline = time.monotonic() + 4
        audio = bytearray()
        while time.monotonic() < deadline:
            try:
                audio.extend(chunks.get(timeout=0.25))
            except queue.Empty:
                pass
    recognizer.AcceptWaveform(bytes(audio))
    print(json.dumps({"text": json.loads(recognizer.FinalResult()).get("text", "")}), flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
