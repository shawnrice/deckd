# deckd

A lightweight daemon for the Elgato Stream Deck+, written in Rust. One binary replaces three vendor apps (Elgato Stream Deck, Neewer, Logitech G HUB) with a fast, config-driven daemon that runs as a launchd service.

## Hardware

- **Elgato Stream Deck+** — 8 buttons, 4 encoders, LCD touchstrip
- **Neewer lights** — key lights (BLE), desk lights (USB serial), GL1 PRO (UDP broadcast)
- **Logitech webcam** — PTZ control via UVC

## Build & run

```sh
cargo build --release
./target/release/deckd
```

## Subcommands

```
deckd                    Start the daemon
deckd install            Install as launchd service (auto-start)
deckd uninstall          Remove launchd service
deckd notify <message>   Show notification on LCD
deckd timer [toggle|start_25|start_5|start_10|stop]
deckd sound <name>       Play a sound from assets/sounds/
deckd sounds             List available sounds
deckd auth google        Authorize Google Calendar access
deckd reload             Reload config without restarting
deckd help               Show this help
```

## Config

Config lives at `~/.config/deckd/config.toml` (or `$DECKD_CONFIG`).

Auto-reloads on file change — no restart needed.

### Button

```toml
[buttons.b1]
label = "Terminal"
icon_name = "terminal"
bg_color = "#1a1a2e"
fg_color = "#e94560"

[buttons.b1.on_press]
type = "shell"
command = "open -a iTerm"
```

### Encoder

```toml
[encoders.e1]
label = "Vol"

[encoders.e1.on_turn_cw]
type = "audio"
command = "volume_up"

[encoders.e1.on_turn_ccw]
type = "audio"
command = "volume_down"

[encoders.e1.on_press]
type = "audio"
command = "mic_toggle"
```

### Multi-action

```toml
[buttons.b5]
label = "Deploy"

[buttons.b5.on_press]
type = "multi"

[[buttons.b5.on_press.actions]]
type = "shell"
command = "cd ~/project && make deploy"

[[buttons.b5.on_press.actions]]
type = "sound"
name = "success"
```

### Light preset

```toml
[buttons.b3]
label = "Meeting"
icon_name = "video"

[buttons.b3.on_press]
type = "light_preset"
brightness = 70
temp_k = 5000
group = "keylights"
```

### Pages

```toml
default_page = "main"

[pages.main.buttons.b1]
label = "Home"

[pages.tools.buttons.b1]
label = "Tools"

[buttons.b8.on_press]
type = "page"
page = "tools"
```

### Audio devices

Preferred device order for cycling:

```toml
[[output_devices]]
uid = "BuiltInSpeakerDevice"
name = "Speakers"

[[output_devices]]
uid = "HyperX-1234-5678"
name = "HyperX"
```

### GitHub dashboard

```toml
github_repo = "owner/repo"
```

Shows open PRs, review requests, mergeable PRs, and CI status on the LCD. Requires `gh` CLI authenticated.

### Google Calendar

```sh
deckd auth google
```

Shows next meeting countdown on the LCD.

## Features

- **Auto-meeting mode** — detects Zoom/Google Meet, switches page and light presets automatically
- **Audio device cycling** — rotary encoder cycles output/input devices, excludes virtual devices (ZoomAudioDevice)
- **Pomodoro timer** — start from button or CLI (`deckd timer start_25`)
- **Soundboard** — drop `.wav`/`.mp3` files in `assets/sounds/`, trigger from buttons
- **Notifications** — `deckd notify "deploy done"` shows a toast on the LCD strip
- **Hot reload** — config auto-reloads on save, or `deckd reload` / `SIGHUP`

## The pet

A tamagotchi lives on your LCD strip. It gains XP when you ship PRs and complete reviews, gets hungry when reviews pile up, and evolves through species (Cat -> Dog -> Penguin -> Ghost) as it levels up.

```toml
pet_name = "deckchi"
pet_pages = ["pet"]
```

## Environment

| Variable | Effect |
|---|---|
| `DECKD_CONFIG` | Override config file path |
| `DECKD_LIGHTS` | Set to `0` to skip light discovery |
| `RUST_LOG` | Log level (`info`, `debug`, etc.) |
