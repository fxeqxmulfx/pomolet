# Pomolet

A desktop Pomodoro timer built with Rust and Iced.

## Run

Install Rust, then run the app from the project directory:

```bash
cargo run --release
```

The first run downloads dependencies and builds the app. A graphical desktop session is required.

## Features

- Focus, short break, and long break timers with adjustable lengths.
- Daily focus count and settings saved across launches.
- Optional focus noise and completion sounds.
- Desktop notifications on Linux.
- Light and dark appearance that follows the system setting.

## Test

```bash
cargo test
```

## License

The application code is licensed under the [MIT License](LICENSE). The bundled
Inter fonts use the [SIL Open Font License 1.1](assets/fonts/LICENSE.txt).
Include the font license when distributing a build that contains the fonts.
