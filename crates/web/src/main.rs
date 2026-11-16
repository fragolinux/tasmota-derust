//! `tasmota-derust-web`: the web interface, read-only. Only read queries (Status, Module,
//! SSId) are sent to devices; every command that changes something is refused.
//! `tasmota-derust-web-rw` is the same server with commands allowed.
//!
//! Usage: tasmota-derust-web [--listen 0.0.0.0:8080]
//! Broker and web login come from the desktop app's config file; DERUST_MQTT_HOST,
//! DERUST_MQTT_PORT, DERUST_MQTT_USER, DERUST_MQTT_PASSWORD, DERUST_WEB_USER,
//! DERUST_WEB_PASSWORD and DERUST_LISTEN override them.

fn main() {
    tasmota_derust_web::standalone_main(true);
}
