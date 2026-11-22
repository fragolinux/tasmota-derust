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
explicitly allow commands. The web server is read-only unless you deliberately start the
read-write one. Firmware upgrades are always started by you, after a confirmation that says
what will happen to each device; when several devices are upgraded together, each one runs on
its own, so a problem on one never touches the others.

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

### 0.6.0: a friendlier face

- A proper **app icon** everywhere (a house with a power symbol), including Android's adaptive
  and themed icons, instead of the generic default.
- An **About** box on every platform, opened from the version number (now clearly styled as
  something to tap): what the app is, what changed, where the source is.
- **Light, dark or system theme** (in 0.6.0 it sat in the About box on desktop and web; it
  has since moved to Settings everywhere).

### 0.7.0 and 0.7.1: tidying up

- **Remove offline devices** from the list, one or all at once, together with the messages the
  broker keeps for them; a device that comes back shows up again by itself.
- Version checks repeat every hour, so a long-running app or server notices new releases.
- About opens on the latest release (download for your system and what changed), and the
  project got a **website** with screenshots and downloads.

### 0.8.0: a safer web server, and ESP32 upgrades

Prompted by a security report
([issue #1](https://github.com/fragolinux/tasmota-derust/issues/1)): the standalone web
server started writable, listened on the whole network without a login, and any web page
opened on that network could have sent it commands through a visitor's browser.

- **Two web programs**: `tasmota-derust-web` is always read-only, `tasmota-derust-web-rw`
  always sends commands. Which one you run is the choice.
- The desktop app's **Start web server** asks every time: read-only or read-write. It no
  longer follows the app's Read-only checkbox.
- Requests from other web sites are refused, and so are public domain names pointed at the
  server (DNS rebinding).
- **Optional login**, shared by the desktop app's server and both programs; only a salted hash
  of the password is stored.
- **Optional HTTPS** on port 8443, with a self-signed certificate made once and shared; its
  fingerprint is shown so you can check it.
- Read-write without a login still works, but the app, the console and a red banner on the page
  spell out the risk, and that plain HTTP sends everything in clear text.
- **Settings** now hold everything you set: theme, broker, web server. The web page got its
  own Settings button.
- Every choice, its reason and its limits: [SECURITY.md](../SECURITY.md#the-web-server-choices-and-their-limits).
- The **16.0.0 stop now covers ESP32 devices too** (from 9.1 up), with the 16.0.0 image of
  their own build or chip. The settings steps Tasmota dropped are shared by every chip, and one
  of them (9.5.0.2) fixes the ESP32-C3 settings layout: our own report to the Tasmota
  developers listed it, but the app only stopped ESP8266 devices.

### 0.8.1: spot the board you just plugged in

A friend testing boards on the bench was looking at the wrong one for a while: the list grew
by one when he plugged in a new board, but nothing said which. Now a device that comes online
while the app is watching gets a **★ next to its name and an amber row for 5 minutes**, on the
desktop, the web page and Android, in light and dark themes. It counts only a live "Online"
from the device (plugged in, restarted, reconnected), not the old status the broker replays
when the app starts.

Real devices soon showed that Tasmota also says "Online" after a mere Wi-Fi or broker
reconnect, so a fan that had been running for days got a star. From 0.8.3 the star goes by
the device's **uptime**: only a device that started in the last 5 minutes is marked.

### 0.8.2 and 0.8.3: small corrections

- About offers a new version only when one is really newer; the release workflow publishes a
  version only when every build (macOS, Windows, Linux, Android) succeeded.
- The star of a just-plugged-in device goes by its uptime (see above), and devices that the
  broker still calls "Online" but that don't answer stay hidden until they do.

### 1.0.0: a console for each device

The tool the original TDM had and this one was missing: a **console per device**. It shows that
device's MQTT messages, sent and received, with the time (telemetry can be hidden), and a
command line that works like Tasmota's own console, with the device's own history under the
arrows and five editable **favourite commands** shared by all devices (the default ones only
read settings). Commands that change the Wi-Fi or MQTT connection, or reset the settings,
ask first, since a wrong value cuts the device off. Passwords are hidden in the log and never
kept in the history; read-only mode shows the messages but sends nothing. Devices can also be
**restarted** from their tools, after a confirmation.

### 1.1.0: a new icon

The app icon became the author's own house drawing: dark lines, an orange inside, on a blue
tile, the same on the desktop, the web page, the website and Android (themed icon included).

### 1.2.0: dashboard, group upgrades, your own firmware, devices without MQTT

- A **Dashboard** opens at every start: one card per online device, big relay buttons, the
  light's colour and the main values. Choosing the values on the cards comes later.
- **Tick devices and upgrade them together**: each one on its own, at the same time; old
  firmware follows the upgrade path step after step by itself, and a failure stops only that
  device.
- **Your own firmware**: a custom OTA server (`http://server/{build}.bin.gz`, where `{build}`
  becomes the device's build) or a highest version on the official server (e.g. stay on 11.x),
  for every device or per device.
- **Find devices on the network** over HTTP, also those without MQTT. The range is filled in
  from the network the app runs on (address and netmask). Only devices that answer as Tasmota
  are added (Shelly, routers and other web servers are left out); they join the list marked
  HTTP and work over HTTP, and **Enable MQTT** sends them the broker settings in one go.
- Offline devices are hidden at every start; an offline device has a remove button in place of
  its gear, and a device removed in one app disappears from the others at once.
- The web server opens the page in the browser when it runs on a computer with a screen. On
  Windows the desktop app uses the system's own fonts, sharper than its built-in one. The
  desktop views became tabs on a row of their own.

### 1.3.0: devices on custom MQTT topics

A user's devices lived on topics like `headquarters/meeting-room-3/%prefix%/%topic%/` and did
not publish `tasmota/discovery` (`SetOption19 1`, or firmware older than 9.x): the app could
not see them. Settings now take **FullTopic patterns** (`headquarters/+/%prefix%/%topic%/`,
`+` for any one level, `%hostname%` or `%id%` in place of `%topic%`) and **single devices**
with their own FullTopic. The app listens to the LWT they imply, learns each device's real
FullTopic from it, and the devices then work like any other (relays, console, upgrades).

## What each version does today

All three share the same Rust core, so device handling, discovery, upgrade logic, SetOption
decoding and safety rules are identical everywhere.

### Desktop app (Windows, macOS, Linux)

- Opens on the **Dashboard**: a card per online device with big relay buttons, the light's
  colour (a click opens colour and brightness) and the main values.
- A dense, sortable table of every device in five more views (Home, Health, Firmware, Wifi,
  MQTT), as tabs on their own row, with customizable and reorderable columns per view, search,
  and an offline filter (offline devices hidden at every start). Click a device name to open
  its own web page.
- Relay buttons and light controls (colour wheel and sliders) directly from the list.
- Before each name, always in the same place: a tick box, the signal, the gear (or, for an
  offline device, a button that removes it) and the console. The gear opens the device's
  tools: **SetOptions** editor, **Wi-Fi networks**, **backup / restore** (backups go to
  Downloads; restore from that list or by dropping a file on the window), **firmware source**
  and **restart**.
- **Tick devices** to upgrade them together, or to connect the HTTP-only ones to MQTT.
- **Find devices** on the network over HTTP; Tasmota devices without MQTT join the list marked
  HTTP.
- **Firmware** view: compares each device with the latest release or development build,
  offers the guided **upgrade path** for old firmware, the release/development switch and
  progress for the device being upgraded.
- A **console** for each device (the `>_` button): its messages, a command line with history
  and five favourite commands.
- **MQTT message log** with a filter, for troubleshooting.
- A built-in **web server** you can start from the desktop app, so phones on the same network
  get the web version without a separate install; it asks whether to start it read-only or
  read-write.
- Read-only checkbox (ticked at every start), Settings (theme, broker, custom MQTT topics,
  firmware server and highest version, web server HTTPS and login), About window, notice and
  download link when a newer version is out.
- Single native executable; macOS as a regular `.app` in a `.dmg`, Windows as a plain `.exe`.

### Web version (any browser, desktop or phone)

- A standalone server for macOS, Windows and Linux in two programs, `tasmota-derust-web`
  (read-only) and `tasmota-derust-web-rw` (commands allowed), or the one built into the desktop
  app. Nothing to install on the devices that open it.
- It opens the page in the default browser when it runs on a computer with a screen, and prints
  the address (Ctrl-click, or Cmd-click on macOS); `--no-browser` for servers.
- The same Dashboard first, then the same table, views, column chooser, search, tick boxes,
  group upgrade, Find devices and Enable MQTT as the desktop app.
- On a phone: a layout of its own, with dashboard cards and compact rows, a bottom navigation
  bar for the views, a Select button for ticking devices, and a bottom sheet with everything
  about a device and its actions (relays, lights, console, SetOptions, Wi-Fi networks, backup /
  restore, restart, firmware source, upgrade).
- Backups download through the browser and restores upload from it.
- A **console** for each device, as on the desktop (full screen on phones).
- A Settings button (a gear on phones): light / dark / system theme remembered per browser,
  firmware server and highest version, and what the server uses (broker, custom MQTT topics,
  read-only, login, HTTPS). Release notes with the download
  for the visitor's own system.
- Optional login (stored only as a hash) and optional HTTPS on port 8443 with a self-signed
  certificate, both shared with the desktop app's server. Requests from other web sites and
  rebound domain names are refused. A red banner warns when commands are allowed without a
  login.

### Android app (Android 13+)

- A native app (Kotlin and Jetpack Compose on top of the same Rust core), with Material You
  colours from the wallpaper and a light / dark / system theme setting.
- Opens on the **Dashboard**, a grid of cards with big relay buttons and light colours; then
  the device list with five more views, search, sorting, offline filter (hidden at every
  start) and pull to refresh.
- Tap a device for its sheet: relays, colour and brightness for lights, Wi-Fi and firmware
  details, the guided upgrade, SetOptions (full screen), Wi-Fi networks, backup / restore
  through Android's own save and open dialogs, restart and firmware source.
- In the sheet, a square **CLI** button beside the name opens the device's **console**: its
  messages, a command line with history arrows and the favourite commands.
- Icons in the toolbar to **find devices** on the network and to **tick devices** for a group
  upgrade or Enable MQTT.
- Settings: broker, custom MQTT topics, firmware server and highest version, theme.
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
- **Security**: the web server's protections, what is stored where (and what is in plain
  text), and what is not covered are written down in [SECURITY.md](../SECURITY.md).
