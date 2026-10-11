#!/usr/bin/env python3
"""An isolated audio server for terminal integration tests."""
import json
import os
from pathlib import Path
import sys
import time

path = Path(os.environ["SOUND_SWITCHER_TEST_STATE"])
state = json.loads(path.read_text())
args = sys.argv[1:]
with Path(os.environ["SOUND_SWITCHER_TEST_LOG"]).open("a") as log:
    log.write(json.dumps(args) + "\n")
if state.get("offline"):
    print("Connection refused", file=sys.stderr)
    sys.exit(1)
if args == ["--format=json", "list", "sinks"]:
    print(json.dumps(state["sinks"]))
elif args == ["get-default-sink"]:
    print(state["default"])
elif args == ["list", "sink-inputs", "short"]:
    print("101\t1\t0\tPipeWire\n102\t1\t0\tPipeWire")
elif args[0] == "set-default-sink":
    if state.get("fail_switch"):
        print("Device disconnected", file=sys.stderr)
        sys.exit(1)
    state["default"] = args[1]
elif args[0] == "move-sink-input":
    if args[1] in state.get("fail_moves", []):
        print("Stream disappeared", file=sys.stderr)
        sys.exit(1)
elif args[0] == "set-sink-volume":
    time.sleep(0.08)
    sink = next(s for s in state["sinks"] if s["name"] == args[1])
    value = int(args[2].rstrip("%"))
    sink["volume"] = {"left": {"value_percent": f"{value}%"}}
elif args[0] == "set-sink-mute":
    sink = next(s for s in state["sinks"] if s["name"] == args[1])
    sink["mute"] = args[2] == "1"
else:
    print(f"Unexpected pactl command: {args}", file=sys.stderr)
    sys.exit(1)
if args[0].startswith("set-"):
    path.write_text(json.dumps(state))
