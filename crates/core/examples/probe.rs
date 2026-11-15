//! Connects to the broker from the config file for a few seconds and prints what the
//! device parser understood. Passive: it only subscribes and never publishes anything.
//! Usage: cargo run --example probe [seconds]

use tasmota_derust_core::device;

use std::time::{Duration, Instant};

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let path = directories::ProjectDirs::from("", "", "tasmota-derust")
        .unwrap()
        .config_dir()
        .join("config.json");
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let b = &cfg["broker"];
    let mut opts = MqttOptions::new(
        "tasmota-derust-probe",
        b["host"].as_str().unwrap(),
        b["port"].as_u64().unwrap_or(1883) as u16,
    );
    opts.set_max_packet_size(512 * 1024, 64 * 1024);
    if let Some(u) = b["username"].as_str().filter(|u| !u.is_empty()) {
        opts.set_credentials(u, b["password"].as_str().unwrap_or(""));
    }
    let (client, mut conn) = Client::new(opts, 256);
    let mut store = device::Store::default();
    let start = Instant::now();
    let mut msgs = 0;
    while start.elapsed() < Duration::from_secs(secs) {
        match conn.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(Event::Incoming(Packet::ConnAck(_)))) => {
                for t in [
                    "tasmota/discovery/+/config",
                    "+/+/LWT",
                    "tele/+/+",
                    "stat/+/+",
                ] {
                    client.subscribe(t, QoS::AtMostOnce).unwrap();
                }
            }
            Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                msgs += 1;
                let payload = String::from_utf8_lossy(&p.payload).into_owned();
                // Passive probe: whatever the app would send is discarded, never published.
                let _ = store.handle_message(&p.topic, &payload, p.retain);
            }
            Ok(Err(e)) => {
                eprintln!("connection error: {e}");
                std::thread::sleep(Duration::from_secs(1));
            }
            _ => {}
        }
    }
    println!(
        "{msgs} messages, {} devices, SSIDs {:?}\n",
        store.devices.len(),
        store.ssids
    );
    for d in store.devices.values() {
        println!(
            "{:<28} {:<8} relays={:?} ap={:?} rssi={:?} ver={:?} module={:?} ip={:?} up={:?} tele={:?}",
            d.display_name(),
            match d.online {
                Some(true) => "online",
                Some(false) => "OFFLINE",
                None => "?",
            },
            d.power,
            d.ap,
            d.rssi,
            d.version.as_deref().map(device::short_version),
            d.module,
            d.ip,
            d.current_uptime().map(device::format_duration),
            d.telemetry(),
        );
    }
}
