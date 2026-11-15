# Tasmota Derust: development

How to build every version from source, try it without real devices, run the security
checks and publish releases. For what the apps do, see the [README](README.md).

- [Building from source](#building-from-source): [tools](#what-you-need),
  [setup per system](#setting-up-the-tools), [with make](#building-with-make),
  [without make](#building-without-make), [security scan](#security-scan-with-trivy)
- [Development](#development): [simulator](#trying-it-without-real-devices), [other tools](#other-tools)
- [Releases and CI](#releases-and-ci) · [Project layout](#project-layout)

# Building from source

## What you need

| Tool | Needed for | macOS | Debian / Ubuntu | Windows |
|---|---|---|---|---|
| [Rust](https://rustup.rs) (rustup, stable) | everything | rustup.rs script | rustup.rs script | rustup-init.exe from rustup.rs |
| C toolchain | everything | `xcode-select --install` | `sudo apt install build-essential` | Visual Studio Build Tools, "Desktop development with C++" |
| GNU make, git, zip | the Makefile | included (Xcode tools) | `sudo apt install make git zip` | use the [commands without make](#building-without-make) |
| mingw-w64 | Windows builds from macOS/Linux | `brew install mingw-w64` | `sudo apt install gcc-mingw-w64-x86-64` | not needed |
| zig + cargo-zigbuild | Linux builds (any host) | `brew install zig` | [ziglang.org](https://ziglang.org/download/) | — |
| cargo-auditable | dependency list inside the binaries (for scanning) | `cargo install --locked cargo-auditable` | same | same |
| Android Studio (SDK, NDK 30.0.16248370, JDK) | the APK | [developer.android.com/studio](https://developer.android.com/studio) | same | same |
| cargo-ndk + Android Rust targets | the APK | see below | same | same |
| Trivy | security scan | `brew install trivy` | [trivy.dev install](https://trivy.dev/latest/getting-started/installation/) | same |
| mosquitto | the device simulator | `brew install mosquitto` | `sudo apt install mosquitto` | mosquitto.org |

Only Rust and a C toolchain are needed for a plain `cargo run`. The rest depends on what you
build. macOS builds can only be made on a Mac; everything else can be built from macOS or Linux.

## Setting up the tools

**macOS** (the reference setup: it builds every target):

```
xcode-select --install
brew install mingw-w64 zig trivy mosquitto
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && source ~/.cargo/env
rustup target add x86_64-pc-windows-gnu x86_64-unknown-linux-gnu \
    aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install --locked cargo-auditable cargo-zigbuild cargo-ndk
```

Then Android Studio: install it, open *Settings > Languages & Frameworks > Android SDK*, and in
*SDK Tools* tick **NDK (Side by side)** version 30.0.16248370 and **Android SDK Build-Tools**.
Open the `android/` folder once so Gradle downloads what it needs. The Makefile expects the SDK
in `~/Library/Android/sdk` and Android Studio's JDK; set `ANDROID_HOME` / `JAVA_HOME` if yours
are elsewhere.

**Debian / Ubuntu**: same Rust commands; `sudo apt install build-essential make git zip
gcc-mingw-w64-x86-64 mosquitto`, zig from ziglang.org on the `PATH`, Trivy from its install
page. For the APK, Android Studio for Linux (or the command-line tools) with the same NDK,
and `ANDROID_HOME` / `JAVA_HOME` exported.

**Windows**: rustup and Visual Studio Build Tools are enough for the desktop app and the web
server (`cargo build --profile dist -p tasmota-derust`, see below). For the APK, Android
Studio plus the Android Rust targets and cargo-ndk.

`make check-tools` lists what is installed and what is missing, with the command to fix it.

## Building with make

Every artifact lands in `dist/`, size-optimized:

| Command | Output |
|---|---|
| `make mac-desktop` | `dist/mac/Tasmota Derust.app` (Apple Silicon) |
| `make mac-web` | `dist/mac/tasmota-derust-web` |
| `make win-desktop` | `dist/windows/tasmota-derust.exe` (x64) |
| `make win-web` | `dist/windows/tasmota-derust-web.exe` |
| `make linux-desktop` | `dist/linux/tasmota-derust` (x64, glibc 2.31+) |
| `make linux-web` | `dist/linux/tasmota-derust-web` |
| `make apk` | `dist/android/tasmota-derust.apk` (arm64 + armv7) |

Groups: `make mac`, `make windows`, `make linux`, `make desktop`, `make web`. `make` alone
builds everything and prints the sizes. Then:

| Command | What it does |
|---|---|
| `make package` | release archives of `dist/` in `dist/release/`, with `SHA256SUMS` |
| `make scan` | Trivy scan of those archives, reports in `dist/security/` |
| `make sizes` | sizes of what is in `dist/` |
| `make release-key` | creates the APK signing key (once, see [Releases](#releases-and-ci)) |
| `make lock-android` | refreshes `android/app/gradle.lockfile` after changing Android dependencies |
| `make check-tools` | what is missing |
| `make clean` | removes `dist/` (Cargo's `target/` stays, so builds stay fast) |

The APK is signed with `~/.android/tasmota-derust-release.jks` when it exists (password from
`DERUST_KEYSTORE_PASSWORD`, or asked), otherwise with Android Studio's debug key. APKs signed
with different keys don't install over each other: changing key means uninstalling once.

Why the files are small: the `dist` Cargo profile uses `opt-level = "s"`, fat LTO, one codegen
unit, `panic = "abort"` and full strip; the desktop app draws with OpenGL (glow) instead of
wgpu; the Android library keeps unwinding (UniFFI turns panics into exceptions) and release
APKs leave out the emulator-only x86_64 code.

## Building without make

What each `make` target runs, for doing it by hand (or on Windows). Drop `auditable` if you
didn't install cargo-auditable.

```
# Desktop app and web server for the computer you are on (macOS, Windows or Linux)
cargo auditable build --locked --profile dist -p tasmota-derust -p tasmota-derust-web
#   -> target/dist/tasmota-derust[.exe], target/dist/tasmota-derust-web[.exe]

# macOS app bundle
mkdir -p "dist/mac/Tasmota Derust.app/Contents/MacOS"
cp target/dist/tasmota-derust "dist/mac/Tasmota Derust.app/Contents/MacOS/"
sed 's/@VERSION@/0.1.0/g' packaging/macos/Info.plist > "dist/mac/Tasmota Derust.app/Contents/Info.plist"
codesign --force --sign - "dist/mac/Tasmota Derust.app"

# Windows from macOS/Linux (mingw-w64)
cargo auditable build --locked --profile dist --target x86_64-pc-windows-gnu \
    -p tasmota-derust -p tasmota-derust-web
#   -> target/x86_64-pc-windows-gnu/dist/*.exe

# Linux from any system (zig), for glibc 2.31 and newer
cargo auditable zigbuild --locked --profile dist --target x86_64-unknown-linux-gnu.2.31 \
    -p tasmota-derust -p tasmota-derust-web
#   -> target/x86_64-unknown-linux-gnu/dist/

# Android: Gradle runs cargo-ndk and UniFFI by itself (task rustCore)
cd android && ./gradlew assembleRelease
#   -> android/app/build/outputs/apk/release/app-release-unsigned.apk, then align and sign:
zipalign -f -p 4 app-release-unsigned.apk aligned.apk
apksigner sign --ks ~/.android/tasmota-derust-release.jks --ks-key-alias tasmota-derust \
    --out tasmota-derust.apk aligned.apk
# (zipalign and apksigner are in $ANDROID_HOME/build-tools/<version>/)

# Archives and checksums
bash scripts/package.sh dist dist/release 0.1.0
```

For day-to-day work: `cargo run --release` (desktop app), `cargo run --release -p
tasmota-derust-web -- --read-only`, `cd android && ./gradlew installDebug` (APK on a connected
phone or emulator), or *Run* in Android Studio.

## Security scan with Trivy

Two checks, the same ones the GitHub workflows run.

**Dependencies of the source tree** (fast, nothing to build): `Cargo.lock` covers every Rust
crate, `android/app/gradle.lockfile` the APK's Java/Kotlin libraries; `secret` looks for
committed passwords and keys.

```
trivy fs --scanners vuln,secret --skip-dirs target --skip-dirs dist \
    --skip-dirs android/app/build --skip-dirs android/build --skip-dirs android/.gradle .
```

**The files you ship** (after `make` and `make package`):

```
make scan        # = bash scripts/scan-artifacts.sh dist/release dist/security
```

Each archive is unpacked and scanned with `trivy rootfs`. Thanks to cargo-auditable, Trivy
reads the exact crates inside each binary (and inside the APK's native library); the APK's
Java/Kotlin side is checked through the Gradle lockfile. For every archive you get
`<name>.trivy.txt` (readable table), `<name>.trivy.json` and `<name>.cdx.json` (CycloneDX
SBOM), plus `SUMMARY.md`. The scan fails when it finds a HIGH or CRITICAL vulnerability that
already has a fixed version, or a binary without dependency data (built without
cargo-auditable); `SCAN_NO_FAIL=1 make scan` only reports.

By hand, for a single file:

```
mkdir /tmp/scan && tar -xzf dist/release/tasmota-derust-web-0.1.0-linux-x64.tar.gz -C /tmp/scan
trivy rootfs --scanners vuln /tmp/scan
```

# Development

## Trying it without real devices

`crates/core/examples/simulator.rs` plays a few fake Tasmota devices on a **local** broker
(it refuses any broker that is not on this computer). It answers the read queries and the
commands (relays, AP switch, upgrade), so every screen can be tried safely:

```
mosquitto -p 1884
cargo run -p tasmota-derust-core --example simulator -- 127.0.0.1:1884
```

Then point the app at `127.0.0.1`, port `1884`. From the Android emulator the computer is
`10.0.2.2`; from a phone on USB, `adb reverse tcp:1884 tcp:1884` and use `127.0.0.1`.

## Other tools

- README screenshots: `cargo run --release -p tasmota-derust --features screenshots` with
  `DERUST_SCREENSHOTS=<dir>` walks through the desktop screens, saves a PNG of each and
  quits. Run it with the simulator and a separate config (e.g. `HOME=/tmp/demo-home` on
  macOS/Linux, with a config pointing at `127.0.0.1:1884`), so it never touches a real broker.
  The feature is development-only and never part of `make` builds.

- `cargo test --workspace` runs the tests (the web one starts the server on a free port).
- `cargo run -p tasmota-derust-core --example probe [seconds]` listens to the configured broker
  and prints what was understood. It is passive and never publishes.

# Releases and CI

GitHub Actions (`.github/workflows/`):

- **Security** (`security.yml`), on every push and pull request and weekly: the Trivy source
  scan above. It fails on committed secrets and on HIGH/CRITICAL vulnerabilities that have a
  fix. Nothing is built.
- **Release** (`release.yml`), only when a version tag is pushed: builds macOS, Linux, Windows
  and the APK, smoke-tests the Linux web server, packs everything, scans **the finished
  archives** with Trivy, and only then publishes the GitHub release. A failing scan stops it
  before anything is published. The scan summary goes into the release notes; the full
  reports and SBOMs are attached as `security-reports.zip`.

One-time setup, for signed APKs:

1. `make release-key` creates `~/.android/tasmota-derust-release.jks` (you choose the
   password). Keep a backup of the file and the password: without them, new versions of the
   app no longer install over the old ones.
2. In the GitHub repository, *Settings > Secrets and variables > Actions*, add
   `ANDROID_KEYSTORE_BASE64` (output of `base64 -i ~/.android/tasmota-derust-release.jks`)
   and `ANDROID_KEYSTORE_PASSWORD`.

To release: set the version in `Cargo.toml` (`[workspace.package]`; the Android app takes its
version from there too), commit, then tag with the same version and push the tag:

```
git tag v0.2.0
git push origin v0.2.0
```

The workflow checks that the tag matches `Cargo.toml`. Tags with a suffix (`v0.2.0-rc1`)
become pre-releases. *Actions > Release > Run workflow* runs it as a dry run: everything is
built and scanned (with a throwaway APK key if the secrets are not set yet), nothing is
published. Dependabot (`.github/dependabot.yml`) opens weekly update PRs for the Rust crates
and the pinned GitHub Actions. Security reports: see [SECURITY.md](SECURITY.md).

# Project layout

| Path | What |
|---|---|
| `crates/core` | Device model, MQTT, settings, `Session` (store + commands with the read-only guard), simulator and probe examples |
| `crates/desktop` | Desktop app (egui), binary `tasmota-derust` |
| `crates/web` | Web server (axum): library used by the desktop app, binary `tasmota-derust-web`, page `index.html` |
| `crates/mobile` | Android library (UniFFI) |
| `crates/uniffi-bindgen` | Build tool that generates the Kotlin bindings |
| `android/` | Android app (Kotlin, Jetpack Compose) |
| `packaging/` | macOS `Info.plist` |
| `scripts/` | Packaging and security scan, used by the Makefile and the workflows |

See [DESIGN.md](DESIGN.md) for the architecture and the roadmap.
