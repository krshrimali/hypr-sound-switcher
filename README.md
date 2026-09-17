# Sound Switcher

A small Ratatui picker for changing the default PipeWire/PulseAudio output with
`pactl`. It also moves currently playing audio to the selected output.

## Build and install

The program needs `pactl` and a terminal emulator.

```sh
cargo build --release
install -Dm755 target/release/sound_switcher ~/.local/bin/sound_switcher
```

## Hyprland keybind

Launch the picker inside your terminal. For example, with Kitty:

```ini
bind = $mainMod, F8, exec, kitty --class sound-switcher -e ~/.local/bin/sound_switcher
```

Or with Foot:

```ini
bind = $mainMod, F8, exec, foot --app-id=sound-switcher ~/.local/bin/sound_switcher
```

Use the arrow keys or `j`/`k` to move, Enter to select, and Escape or `q` to
cancel. The currently configured default sink is selected when the UI opens.
