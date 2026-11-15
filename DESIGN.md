# Design notes

## Goals

- Native desktop app (Windows and macOS; Linux from the same code), web server, Android app.
- See every Tasmota device at a glance, switch relays, keep firmware up to date.
- Safe by default: anything that restarts or reflashes a device asks for confirmation and
  acts on **one device at a time**. No mass upgrades.
- Distributable: no hard-coded broker or credentials; settings live in the user's config dir.
- Support old firmware too: the parser accepts missing/renamed fields; discovery also works
  from LWT messages for devices without native discovery.

## Stack

| Concern  | Choice |
|----------|--------|
| GUI      | `eframe` / `egui` + `egui_extras` tables (immediate mode, cross-platform, wasm possible) |
| MQTT     | `rumqttc` sync client on a background thread (TLS disabled for now) |
| JSON     | `serde_json` (`Value`, tolerant parsing) |
| HTTP     | `ureq` (latest Tasmota release from the GitHub API) |
| Settings | JSON file in `directories::ProjectDirs` config dir (Android: app preferences) |
| Web      | `axum` + one HTML page |
| Android  | Kotlin + Jetpack Compose UI, Rust core through UniFFI (JNA), built with `cargo-ndk` |

## Modules

`crates/core` (library `tasmota_derust_core`), shared by every front end:

- `mqtt.rs` – connection thread, subscriptions, events through a channel.
- `device.rs` – device model, topic handling, parsing of Tasmota messages, commands to send.
- `session.rs` – a running connection plus device store for front ends that poll (web, Android):
  `snapshot()`, `refresh()` (read queries only) and the commands `toggle_power`, `switch_ap`,
  `upgrade`, all behind one read-only guard.
- `columns.rs` – views and columns (titles, defaults).
- `config.rs` – persisted settings.
- `github.rs` – latest release lookup.
- `examples/probe.rs` (passive listener), `examples/simulator.rs` (fake devices, local broker only).

Front ends:

- `crates/desktop` – desktop app (egui, OpenGL via glow); drives the device store itself. Its
  "Start web server" button runs `tasmota_derust_web::WebServer` on a background thread with
  its own `Session` (a second MQTT connection). One Read-only checkbox (ticked at every start)
  guards the app's own commands and, through `Session::set_read_only`, the web server's.
- `crates/web` – web server (axum), a library (`router`, `WebServer`, `browser_url`) plus the
  standalone binary. JSON API: `GET /api/state`, `POST /api/refresh`,
  `POST /api/devices/{topic}/power/{n}`, `…/ap/{n}`, `…/upgrade`. The page is
  `crates/web/index.html` (plain HTML/JS, embedded in the binary), polling every 2 s; below
  760 px it switches to a phone layout. `--read-only` refuses every state-changing command.
- `crates/mobile` – UniFFI wrapper of `Session` for Kotlin. `android/` – the Android app:
  `MainViewModel` owns the session and polls it once a second; Gradle task `rustCore` runs
  cargo-ndk for arm64-v8a, armeabi-v7a and x86_64 and generates the bindings into `build/`.

## Discovery and topics

Subscriptions: `tasmota/discovery/+/config`, `+/+/LWT`, `tele/+/+`, `stat/+/+`, plus per-device
subscriptions for devices whose FullTopic or prefixes are not the default `%prefix%/%topic%/`.

When a device comes online the app sends `Status 0`, `Module`, and (until known) `SSId1`/`SSId2`.

## Roadmap

- tdmgr's advanced editors: rules, timers, switches, buttons, power settings.
- Console per device with command history.
- MQTT over TLS, multiple brokers/sites in one window.
- Forget/remove stale devices (clear retained discovery/LWT).
- Web: login/password, live push (SSE) instead of polling, Home Assistant add-on / Docker image.
- Installers, macOS notarization, Windows code signing (applied to SignPath Foundation).
