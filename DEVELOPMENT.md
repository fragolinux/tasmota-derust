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
| `make dist` | Windows executables + APK, then `dist/latest/` with fixed names (`tasmota-derust.exe`, `tasmota-derust-web.exe`, `tasmota-derust.apk`), refreshed by every local Windows or APK build too: copy the folder to another PC and it overwrites the old ones |
| `make package` | Trivy scan, then release files of `dist/` in `dist/release/` (reports in `dist/security/`) |
| `make release` | checks, version bump, build + scan, signed commit and tag, push (see [Releases](#releases-and-ci)) |
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

# Trivy scan, release files and checksums
bash scripts/package.sh dist dist/release dist/security 0.1.0
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
make package     # = bash scripts/package.sh dist dist/release dist/security <version>
```

`make package` scans before it packs: the content of each release file (program and
licenses) is staged and scanned with `trivy rootfs`, and nothing is packed if the scan fails.
Thanks to cargo-auditable, Trivy reads the exact crates inside each binary (and inside the
APK's native library); the APK's Java/Kotlin side is checked through the Gradle lockfile.

Release files: macOS desktop app `.dmg` (drag the app to *Applications*), macOS web server and
Windows `.zip`, Linux `.tar.gz` (the executable bit is kept), Android the `.apk` alone. Every
archive and disk image contains the program, `LICENSE-MIT`, `LICENSE-APACHE`,
`TRIVY-REPORT.txt` / `TRIVY-REPORT.json` (scan of exactly those files) and `SHA256SUMS`
(every file inside; `shasum -a 256 -c SHA256SUMS`). `dist/release/SHA256SUMS` covers the
release files themselves.

`dist/security/` gets, per release file, `<name>.trivy.txt` (readable table),
`<name>.trivy.json` and `<name>.cdx.json` (CycloneDX SBOM), plus `SUMMARY.md`. The scan fails
when it finds a HIGH or CRITICAL vulnerability that already has a fixed version, or a binary
without dependency data (built without cargo-auditable); `SCAN_NO_FAIL=1 make package` only
reports. The `.dmg` is made with `hdiutil`, so the macOS desktop app is packed on a Mac only.

# Development

## Trying it without real devices

`crates/core/examples/simulator.rs` plays a few fake Tasmota devices on a **local** broker
(among them an ESP32 that upgrades through safeboot, an unplugged plug with a stale
"Online" LWT, and an ESP8266 on 6.7.1 with about 1 MB of flash that follows the whole
upgrade path, minimal images included). It refuses any broker that is not on this computer.
It answers the read queries and the
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
  and the APK (Windows with MSVC on a Windows runner, static C runtime: antivirus heuristics
  flag MinGW-built Rust executables much more often; `make windows` keeps MinGW for local
  tries), smoke-tests the Linux web server, scans what goes into each release file
  with Trivy, packs it (report and checksums inside), and only then publishes the GitHub
  release. A failing scan stops it before anything is published. The scan summary goes into
  the release notes; all reports and SBOMs are attached as `security-reports.zip`.

One-time setup, for signed APKs:

1. `make release-key` creates `~/.android/tasmota-derust-release.jks` (you choose the
   password). Keep a backup of the file and the password: without them, new versions of the
   app no longer install over the old ones.
2. In the GitHub repository, *Settings > Secrets and variables > Actions*, add
   `ANDROID_KEYSTORE_BASE64` (output of `base64 -i ~/.android/tasmota-derust-release.jks`)
   and `ANDROID_KEYSTORE_PASSWORD`.

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
`feat: ...`, `fix: ...`, `perf:`, `docs:`, `chore:`, `ci:`, `build:`, `refactor:`, `test:`,
`style:`, `revert:` (an optional scope: `fix(android): ...`). The release version comes from
them.

To release, on the Mac:

```
make release                    # next version from the commits since the last tag
make release VERSION=1.0.0      # a given version: majors, pre-releases (1.0.0-rc1), overrides
make release DRY_RUN=1          # everything up to the commit, then the bump is undone
```

Without `VERSION`, since the last tag pushed to origin: any `feat` makes a minor release
(0.1.3 -> 0.2.0), otherwise any `fix`/`perf` a patch (0.1.3 -> 0.1.4); only `docs`, `chore`,
`ci` and the like make no release. A breaking change (`feat!:`, `BREAKING CHANGE:`), a commit
that is not a Conventional Commit, or a pre-release as last tag stop it: then give `VERSION`.
Majors are never automatic.

`make release` never stops halfway to ask for a password: it checks both signing keys first.
One-time setup on the Mac:

- APK key password into the login Keychain (it asks for the password twice, nothing is
  shown or saved elsewhere); `make apk` and `make release` read it from there:

  ```
  security add-generic-password -s tasmota-derust-keystore -a tasmota-derust -w
  ```

  To change it later, add `-U` to the same command. `DERUST_KEYSTORE_PASSWORD`, when set,
  still wins.
- GPG passphrase: the next time pinentry-mac asks for it, tick *Save in Keychain*; from
  then on commits and tags are signed without asking.

`make release` stops at the first failure: on `main`, clean tree, not behind `origin/main`;
`cargo test`; Trivy source scan (as `security.yml`); `actionlint`; official Gradle wrapper;
version bump in `Cargo.toml` (`[workspace.package]`, the Android app reads it too) and
`Cargo.lock`; `make` (every target); `make package` (Trivy scan of the release files). If any of
these fails the bump is put back. Then a signed commit `chore(release): vX.Y.Z`, a signed tag,
a summary, a confirmation (`RELEASE_CONFIRM=yes` skips it) and
`git push --atomic origin main vX.Y.Z`. The Release workflow does the rest.

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
