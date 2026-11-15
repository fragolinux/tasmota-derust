# Tasmota Derust: distribution builds.
#
#   make               everything below (macOS only builds on a Mac)
#   make mac           macOS (Apple Silicon): desktop app + web server
#   make windows       Windows x64: desktop app + web server
#   make linux         Linux x64 (glibc 2.31+): desktop app + web server
#   make apk           Android APK
#   make mac-desktop | mac-web | win-desktop | win-web | linux-desktop | linux-web
#   make package       Trivy scan + release files of whatever is in dist/ -> dist/release/,
#                      reports -> dist/security/
#   make release       checks, version bump from the Conventional Commits since the last tag,
#                      build + scan, signed commit and tag, push (VERSION=x.y.z to force it,
#                      DRY_RUN=1 to stop before committing; see scripts/release.sh)
#   make sizes         list what is in dist/
#   make release-key   create the APK signing key (once; see DEVELOPMENT.md)
#   make lock-android  refresh android/app/gradle.lockfile after changing Android dependencies
#   make check-tools   what is missing on this computer
#   make clean         remove dist/ (cargo's target/ is left alone)
#
# Builds use the size-optimized `dist` Cargo profile. With cargo-auditable installed, every
# binary embeds its dependency list, so `make package` (Trivy) can check the actual artifacts.

CARGO        ?= cargo
DIST         := dist
WIN_TARGET   := x86_64-pc-windows-gnu
LINUX_TARGET := x86_64-unknown-linux-gnu
# Oldest glibc the Linux binaries run on (Ubuntu 20.04, Debian 11).
LINUX_GLIBC  := 2.31
APP_NAME     := Tasmota Derust
VERSION      := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

# Homebrew (mingw-w64, zig) and cargo-installed tools on the PATH.
export PATH := /opt/homebrew/bin:$(HOME)/.cargo/bin:$(PATH)

# `cargo auditable build` when available: same binary plus its dependency list.
AUDITABLE    := $(if $(shell PATH="$(PATH)" command -v cargo-auditable),auditable,)
CARGO_BUILD  := $(CARGO) $(AUDITABLE) build --locked --profile dist

ANDROID_HOME ?= $(HOME)/Library/Android/sdk
JAVA_HOME    ?= /Applications/Android Studio.app/Contents/jbr/Contents/Home
export JAVA_HOME
BUILD_TOOLS   = $(lastword $(sort $(wildcard $(ANDROID_HOME)/build-tools/*)))

# APK signing: the release key when it exists, otherwise Android Studio's debug key.
# Release key password: DERUST_KEYSTORE_PASSWORD, else the macOS Keychain item
# $(KEYCHAIN_SERVICE) (see DEVELOPMENT.md), else apksigner asks for it.
RELEASE_KEYSTORE ?= $(HOME)/.android/tasmota-derust-release.jks
RELEASE_KEY_ALIAS ?= tasmota-derust
KEYCHAIN_SERVICE ?= tasmota-derust-keystore
ifneq ($(wildcard $(RELEASE_KEYSTORE)),)
  KEYSTORE  := $(RELEASE_KEYSTORE)
  KEY_ALIAS := $(RELEASE_KEY_ALIAS)
  # Shell: the password goes into DERUST_KEYSTORE_PASSWORD, never onto a command line.
  GET_PASS  := DERUST_KEYSTORE_PASSWORD="$${DERUST_KEYSTORE_PASSWORD:-$$(security find-generic-password -s $(KEYCHAIN_SERVICE) -w 2>/dev/null || true)}"; export DERUST_KEYSTORE_PASSWORD;
  SIGN_PASS := $$([ -n "$$DERUST_KEYSTORE_PASSWORD" ] && echo --ks-pass env:DERUST_KEYSTORE_PASSWORD --key-pass env:DERUST_KEYSTORE_PASSWORD)
else
  KEYSTORE  := $(HOME)/.android/debug.keystore
  KEY_ALIAS := androiddebugkey
  GET_PASS  :=
  SIGN_PASS := --ks-pass pass:android --key-pass pass:android
endif

MAC_DIR   := $(DIST)/mac
WIN_DIR   := $(DIST)/windows
LINUX_DIR := $(DIST)/linux
APK_DIR   := $(DIST)/android
MAC_APP   := $(MAC_DIR)/$(APP_NAME).app

.PHONY: all mac windows linux desktop web mac-desktop mac-web win-desktop win-web \
        linux-desktop linux-web apk package release sizes clean check-tools release-key lock-android

all: mac windows linux apk
	@$(MAKE) --no-print-directory sizes

mac: mac-desktop mac-web
windows: win-desktop win-web
linux: linux-desktop linux-web
desktop: mac-desktop win-desktop linux-desktop
web: mac-web win-web linux-web

# ------------------------------------------------------------------------- macOS

mac-desktop:
	$(CARGO_BUILD) -p tasmota-derust
	@rm -rf "$(MAC_APP)"
	@mkdir -p "$(MAC_APP)/Contents/MacOS"
	cp target/dist/tasmota-derust "$(MAC_APP)/Contents/MacOS/tasmota-derust"
	@sed 's/@VERSION@/$(VERSION)/g' packaging/macos/Info.plist > "$(MAC_APP)/Contents/Info.plist"
	codesign --force --sign - "$(MAC_APP)"

mac-web:
	$(CARGO_BUILD) -p tasmota-derust-web
	@mkdir -p $(MAC_DIR)
	cp target/dist/tasmota-derust-web $(MAC_DIR)/

# ----------------------------------------------------------------------- Windows

win-desktop:
	$(CARGO_BUILD) --target $(WIN_TARGET) -p tasmota-derust
	@mkdir -p $(WIN_DIR)
	cp target/$(WIN_TARGET)/dist/tasmota-derust.exe $(WIN_DIR)/

win-web:
	$(CARGO_BUILD) --target $(WIN_TARGET) -p tasmota-derust-web
	@mkdir -p $(WIN_DIR)
	cp target/$(WIN_TARGET)/dist/tasmota-derust-web.exe $(WIN_DIR)/

# ------------------------------------------------------------------------- Linux
# Cross-compiled with cargo-zigbuild (zig as the linker), also on Linux hosts, so the
# glibc version stays pinned.

linux-desktop:
	$(CARGO) $(AUDITABLE) zigbuild --locked --profile dist --target $(LINUX_TARGET).$(LINUX_GLIBC) -p tasmota-derust
	@mkdir -p $(LINUX_DIR)
	cp target/$(LINUX_TARGET)/dist/tasmota-derust $(LINUX_DIR)/

linux-web:
	$(CARGO) $(AUDITABLE) zigbuild --locked --profile dist --target $(LINUX_TARGET).$(LINUX_GLIBC) -p tasmota-derust-web
	@mkdir -p $(LINUX_DIR)
	cp target/$(LINUX_TARGET)/dist/tasmota-derust-web $(LINUX_DIR)/

# ----------------------------------------------------------------------- Android

apk:
	cd android && ANDROID_HOME="$(ANDROID_HOME)" sh ./gradlew --quiet assembleRelease
	@mkdir -p $(APK_DIR)
	"$(BUILD_TOOLS)/zipalign" -f -p 4 \
		android/app/build/outputs/apk/release/app-release-unsigned.apk $(APK_DIR)/aligned.apk
	$(GET_PASS) "$(JAVA_HOME)/bin/java" --enable-native-access=ALL-UNNAMED -jar "$(BUILD_TOOLS)/lib/apksigner.jar" \
		sign --ks "$(KEYSTORE)" --ks-key-alias $(KEY_ALIAS) $(SIGN_PASS) \
		--out $(APK_DIR)/tasmota-derust.apk $(APK_DIR)/aligned.apk
	@rm -f $(APK_DIR)/aligned.apk $(APK_DIR)/*.idsig
	@echo "Signed with $(KEYSTORE)"

release-key:
	@test ! -e "$(RELEASE_KEYSTORE)" || { echo "$(RELEASE_KEYSTORE) already exists."; exit 1; }
	@mkdir -p "$(dir $(RELEASE_KEYSTORE))"
	"$(JAVA_HOME)/bin/keytool" -genkeypair -keystore "$(RELEASE_KEYSTORE)" -alias $(RELEASE_KEY_ALIAS) \
		-keyalg RSA -keysize 4096 -validity 10000 -dname "CN=Tasmota Derust"
	@echo
	@echo "Key created: $(RELEASE_KEYSTORE). Back it up: losing it means users must uninstall to update."
	@echo "For GitHub releases add these repository secrets (Settings > Secrets and variables > Actions):"
	@echo "  ANDROID_KEYSTORE_BASE64    base64 -i \"$(RELEASE_KEYSTORE)\" | pbcopy"
	@echo "  ANDROID_KEYSTORE_PASSWORD  the password you just chose"

lock-android:
	cd android && ANDROID_HOME="$(ANDROID_HOME)" sh ./gradlew --quiet :app:dependencies \
		--configuration releaseRuntimeClasspath --write-locks > /dev/null
	@echo "Updated android/app/gradle.lockfile"

# ---------------------------------------------------------------------- release

package:
	bash scripts/package.sh $(DIST) $(DIST)/release $(DIST)/security $(VERSION)

# VERSION is passed on only when given on the command line (make release VERSION=0.2.0).
release:
	bash scripts/release.sh $(if $(filter command line,$(origin VERSION)),$(VERSION))

# ---------------------------------------------------------------------- helpers

sizes:
	@test -d $(DIST) || { echo "Nothing built yet."; exit 0; }; \
	cd $(DIST) && find mac windows linux android -type f \( -name 'tasmota-derust*' -o -name '*.apk' \) \
		-print0 2>/dev/null | sort -z | xargs -0 du -h

clean:
	rm -rf $(DIST)

check-tools:
	@ok() { if eval "$$2" >/dev/null 2>&1; then echo "ok      $$1"; else echo "MISSING $$1  ->  $$3"; fi; }; \
	ok cargo "command -v $(CARGO)" "install rustup"; \
	ok cargo-auditable "command -v cargo-auditable" "cargo install --locked cargo-auditable"; \
	ok "$(WIN_TARGET) target" "rustup target list --installed | grep -q $(WIN_TARGET)" "rustup target add $(WIN_TARGET)"; \
	ok mingw-w64 "command -v x86_64-w64-mingw32-gcc" "brew install mingw-w64"; \
	ok "$(LINUX_TARGET) target" "rustup target list --installed | grep -q $(LINUX_TARGET)" "rustup target add $(LINUX_TARGET)"; \
	ok zig "command -v zig" "brew install zig"; \
	ok cargo-zigbuild "command -v cargo-zigbuild" "cargo install --locked cargo-zigbuild"; \
	for t in aarch64-linux-android armv7-linux-androideabi x86_64-linux-android; do \
		ok "$$t target" "rustup target list --installed | grep -q $$t" "rustup target add $$t"; done; \
	ok cargo-ndk "command -v cargo-ndk" "cargo install --locked cargo-ndk"; \
	ok "Android NDK" "test -d '$(ANDROID_HOME)/ndk'" "Android Studio > SDK Manager"; \
	ok "Android build-tools" "test -n '$(BUILD_TOOLS)'" "Android Studio > SDK Manager"; \
	ok "APK signing key" "test -f '$(KEYSTORE)'" "make release-key"; \
	ok trivy "command -v trivy" "brew install trivy"
