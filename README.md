# Pomolet

A minimal desktop Pomodoro timer built with Rust and Iced.

## License

The application code is licensed under the [MIT License](LICENSE). The bundled
Inter fonts remain under the [SIL Open Font License 1.1](assets/fonts/LICENSE.txt).
Include the font license when distributing a build that contains the fonts.

## Run

```bash
cargo run --release
```

Rust 1.80+ and a graphical desktop environment are required. Cargo downloads dependencies and builds the app on the first run.

### Wayland

Run this from a terminal in an active Wayland session:

```bash
./run-wayland.sh
```

The script starts an optimized build and unsets `DISPLAY` so Iced connects to Wayland directly. To run an existing binary: `env -u DISPLAY ./target/release/pomolet`.

## Features

- Focus, short break, and long break modes. Every fourth completed focus session leads to a long break.
- Start, pause, reset, and skip controls.
- Session lengths adjustable in five-minute steps; settings and the daily focus count are saved across launches.
- Brown, pink, white, blue, or violet noise while a focus timer runs, with an Off option.
- Selecting a sound in Settings previews noise for three seconds or a completion cue for one second; selecting focus noise during a running session changes the continuous sound directly.
- Focus noise fades in and out over 250 ms; changing its type crossfades on the same audio output.
- A generated gong, chime, bell, woodblock, or pulse when a phase finishes naturally, with an Off option. Breaks have no background sound.
- Desktop notifications on Linux when a focus session or break finishes naturally.
- A responsive layout that centers the timer in large windows and scrolls in small windows.
- An iOS 18-inspired interface with bundled Inter fonts.
- Light and dark appearance follow the system setting automatically, including live changes through the XDG Settings portal on Linux.

## Architecture

```text
Iced button --Command--> CoreHandle --> core thread --> Timer
    Iced <--Snapshot-- subscription <-- broadcast to subscribers
    Audio <--Snapshot-- subscription <-- broadcast to subscribers
    Notifications <--Snapshot-- subscription <-- broadcast to subscribers
```

`src/timer.rs` contains the countdown and phase rules. `src/core.rs` receives commands, runs the timer, saves settings, and broadcasts immutable snapshots. `src/main.rs` sends commands and renders snapshots. `src/audio.rs` subscribes independently and plays sound through the default audio output. `src/notification.rs` subscribes independently and sends completion notices through the desktop notification service. Additional consumers can call `CoreHandle::subscribe()`.

The UI receives a new snapshot only when the displayed second changes. Settings and statistics are stored in the user's configuration directory at `pomolet/state.json`. Existing data from `pomodoro/state.json` or `animecat-pomodoro/state.json` is read during migration.

## Test

```bash
cargo test
```
