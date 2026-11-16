//! `tasmota-derust-web-rw`: the web interface with commands allowed (relays, Wi-Fi,
//! upgrades…). Set a web login in the desktop app's Settings (or DERUST_WEB_USER and
//! DERUST_WEB_PASSWORD) before running it on a shared network.
//! `tasmota-derust-web` is the same server, read-only.
//!
//! Usage: tasmota-derust-web-rw [--listen 0.0.0.0:8080]

fn main() {
    tasmota_derust_web::standalone_main(false);
}
