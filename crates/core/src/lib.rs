//! Shared core of Tasmota Derust: device model, MQTT connection, settings.
//! Used by the desktop app, the web server and the Android library.

pub mod backup;
pub mod columns;
pub mod config;
pub mod device;
pub mod github;
pub mod light;
pub mod mqtt;
pub mod ota;
pub mod session;
pub mod setoption;
pub mod wifi;

use std::sync::Arc;

/// Callback used by background threads to say "something changed".
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// What the app is, in a few lines: About boxes of the desktop app, the web page and Android.
pub const ABOUT: &str = "Tasmota Derust manages the Tasmota devices (plugs, switches, relays, \
lights, sensors) on your MQTT broker. It finds them by itself and shows them in one list: \
power, Wi-Fi signal, firmware, MQTT settings and health. From there you can switch relays, \
control lights, edit SetOptions, change Wi-Fi networks, back up and restore configurations \
and upgrade the firmware, also from very old versions, one safe step at a time.\n\n\
It always starts read-only: nothing is sent to the devices until you allow commands.\n\n\
Desktop app (Windows, macOS, Linux), web server and Android app. \
Free and open source (MIT or Apache-2.0).";

/// Source code, issues and downloads.
pub const REPO_URL: &str = "https://github.com/fragolinux/tasmota-derust";

/// Project website (GitHub Pages, from `docs/`): what it is, screenshots, downloads.
pub const SITE_URL: &str = "https://fragolinux.github.io/tasmota-derust/";
