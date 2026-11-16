# Tasmota Derust: where it started, how it grew, what it does

## Where it started

[Tasmota](https://tasmota.github.io/) is the open firmware that runs on millions of cheap
ESP8266/ESP32 smart plugs, switches, relays, bulbs and sensors. Anyone with more than a few of
those devices needs one place to see them all, and for years that place was **TDM, the Tasmota
Device Manager** by jziolkowski: a Python/Qt desktop program that listened to the MQTT broker,
found every Tasmota device on it and showed them in a table. TDM is no longer maintained, and
it was getting harder to install and run on current systems.

**Tasmota Derust** started as a modern replacement for it: an independent rewrite in Rust
(no TDM code reused, not affiliated with the Tasmota project), with one shared core and three
front ends:

- a native **desktop app** for Windows, macOS and Linux;
- a **web server** that serves the same features to any browser, laid out for phones too;
- an **Android app**.

The name says it: TDM, "de-rusted" and rebuilt in Rust. It is free and open source
(MIT or Apache-2.0), at https://github.com/fragolinux/tasmota-derust.

One principle shaped it from day one: **it must never surprise you on your real devices.**
Every front end starts in **read-only mode** at every launch. In that mode it only sends
harmless status queries; relays, Wi-Fi changes, settings and upgrades are refused until you
explicitly allow commands. Firmware upgrades always go one device at a time, never in bulk.

## How it grew, release by release

### 0.1.0: the foundation

- MQTT connection and automatic discovery: native `tasmota/discovery` messages, plus LWT-based
  discovery for older firmware that doesn't publish them.
- One device list with five views: **Home** (relays, live telemetry such as power and
  temperature, uptime, Wi-Fi signal), **Health** (uptime, restarts, boot count, signal),
  **Firmware** (version, core, SDK, free flash, OtaUrl), **Wifi** (IP, SSID, access point,
  signal) and **MQTT** (topics).
- One-click relay buttons on each row, switching a device between its two Wi-Fi networks,
  single-device firmware upgrades with progress tracking.
- Per-view column chooser, saved between runs; offline devices shown or hidden, and removed
  (with the retained messages the broker keeps for them); a message log for troubleshooting.
- New Tasmota and Tasmota Derust versions are checked every hour, so a running app shows them.
- Read-only mode, on at every start, across desktop, web and Android.
- A local **device simulator**, so every screen can be tried without touching real devices.
- Reproducible, size-optimized, auditable builds for every platform, security-scanned (Trivy)
  on every push and again on the finished release archives, with checksums for every file.

### 0.1.1: honest online status

A retained "Online" message on the broker is no longer taken at face value: a device counts as
online only after it answers. Devices that stop answering show as "No answer" and refuse
commands. ESP32 upgrades through safeboot are followed to the real firmware.

### 0.2.0: the upgrade path for old firmware

The feature that sets it apart. Tasmota can't jump from an old version straight to the latest:
it requires an official path (5.14 → 6.7.1 → 7.2.0 → 8.5.1 → 9.1 → latest), with the right
image at each step. Tasmota Derust now walks a device along it, one confirmed step at a time:
it sets the official image for each step (old devices often still point to dead download
URLs), uses `.bin` before 8.2 and `.gz` after, checks free flash against the image size before
each step, follows the restart and verifies the new version, and continues from wherever the
device is if a step fails.

Also new: **release / development channels** (one click points a device at official release
builds or at development builds), "Up to date" badges, and a notice when a newer Tasmota
Derust is published.

### 0.3.0: lights and release notes

- **Colour control for lights**: dimmable, CCT, RGB, RGBW and RGBCCT lights are recognised
  from their channels; a colour wheel plus brightness, white-temperature and white sliders,
  each shown only when the light supports it.
- Clicking the app's version shows that release's notes, fetched from GitHub.

### Shipped with 0.4.0: proven on a real device

The upgrade path was run end to end on a real ESP8266 (a Wemos D1 mini) from 6.7.1 to 16.0.0,
which exposed real-world quirks the simulator didn't have: Tasmota announcing itself online
before it listens for commands, old firmware splitting its status into many messages, 6.x
dropping MQTT mid-download without restarting, 7+ restarting without an offline message. All
handled. The whole run (four steps, under three minutes) is documented with a screenshot of
every state and the full log.

### 0.4.0: the SetOption editor

- Every device's **SetOptions** decoded from its status (old firmware formats included),
  grouped by category, searchable, with an "only changed" filter, on/off switches and number
  fields, and a plain-language explanation of each option and its default.
- Safety built in: options that change how apps read the device, or can make it misbehave,
  ask first and say why; turning MQTT or Wi-Fi off is refused outright, since it would cut the
  device off.
- Defaults checked against the firmware source where the official documentation turned out to
  be wrong. The findings (wrong defaults, reversed descriptions, and upgrade steps the
  development branch drops) are written up for the Tasmota developers in
  [Tasmota SetOption defaults: firmware vs documentation](tasmota-setoption-report.md).
- The upgrade path gained a stop at **16.0.0** for devices older than 10.0, because newer
  releases dropped the settings migrations those devices still need.

### 0.5.0: Wi-Fi networks, backup and restore

- **Wi-Fi networks**: see each device's two configured networks and which one is in use; set
  the one *not* in use (add a second network, or change the spare). The network in use can't
  be edited, so a typo can't strand the device. Passwords go only to the device, never stored
  or logged.
- **Backup / restore** of a device's whole configuration (Wi-Fi, MQTT, GPIOs, rules,
  SetOptions…) as Tasmota's own `.dmp` file, the same as its web page produces; "Back up first"
  is offered before risky SetOption changes.
- Release notes now include the right download for the system you're on (Windows zip, macOS
  dmg, Linux archive, Android APK).

### Next release: a friendlier face

- A proper **app icon** everywhere (a house with a power symbol), including Android's adaptive
  and themed icons, instead of the generic default.
- An **About** box on every platform, opened from the version number (now clearly styled as
  something to tap): what the app is, what changed, where the source is.
- **Light, dark or system theme**, chosen in Settings on Android and in the About box on the
  desktop and web.

## What each version does today

All three share the same Rust core, so device handling, discovery, upgrade logic, SetOption
decoding and safety rules are identical everywhere.

### Desktop app (Windows, macOS, Linux)

- A dense, sortable table of every device, with the five views, customizable and reorderable
  columns per view, search, and an offline filter. Click a device name to open its own web page.
- Relay buttons and light controls (colour wheel and sliders) directly from the list.
- A gear next to each device opens its tools: **SetOptions** editor, **Wi-Fi networks**,
  **backup / restore** (backups go to Downloads; restore from that list or by dropping a file
  on the window).
- **Firmware** view: compares each device with the latest release or development build,
  offers the guided **upgrade path** for old firmware, the release/development switch and
  progress for the device being upgraded.
- **MQTT message log** with a filter, for troubleshooting.
- A built-in **web server** you can start from the desktop app, so phones on the same network
  get the web version without a separate install.
- Read-only checkbox (ticked at every start), broker settings, About window with theme choice,
  notice and download link when a newer version is out.
- Single native executable; macOS as a regular `.app` in a `.dmg`, Windows as a plain `.exe`.

### Web version (any browser, desktop or phone)

- A standalone server (`tasmota-derust-web`) for macOS, Windows and Linux, or the one built
  into the desktop app. Nothing to install on the devices that open it.
- On a computer: the same table, views, column chooser and search as the desktop app.
- On a phone: a layout of its own, with compact rows, a bottom navigation bar for the five
  views, and a bottom sheet with everything about a device and its actions (relays, lights,
  SetOptions, Wi-Fi networks, backup / restore, upgrade).
- Backups download through the browser and restores upload from it.
- Light / dark / system theme remembered per browser; release notes with the download for the
  visitor's own system.
- The server can be started read-only, so it can be shared with others safely.

### Android app (Android 13+)

- A native app (Kotlin and Jetpack Compose on top of the same Rust core), with Material You
  colours from the wallpaper and a light / dark / system theme setting.
- The device list with the five views, search, sorting, offline filter and pull to refresh.
- Tap a device for its sheet: relays, colour and brightness for lights, Wi-Fi and firmware
  details, the guided upgrade, SetOptions (full screen), Wi-Fi networks, and backup / restore
  through Android's own save and open dialogs.
- A lock icon in the top bar for read-only mode, which turns back on at every launch.
- Version and available update next to the title, with the release notes and a direct APK
  download.
- Broker settings stored on the phone only and excluded from backups.

## Under the hood

- **Rust** everywhere it counts: the core (MQTT, device model, upgrade logic, SetOptions,
  backup), the desktop UI (egui) and the web server (axum). Android calls the same core through
  UniFFI.
- **Privacy**: it talks only to your MQTT broker and your devices, plus GitHub for version
  numbers and release notes, the Tasmota OTA server for firmware sizes, and the Tasmota docs
  for SetOption descriptions. Nothing about you or your devices is sent anywhere.
- **Releases**: built and published by CI from signed tags, each archive with its licenses,
  vulnerability report and checksums.
