//! Fake Tasmota devices on a LOCAL test broker, for trying the apps without real hardware.
//!
//!   mosquitto -p 1883                                  # a throwaway local broker
//!   cargo run -p tasmota-derust-core --example simulator [-- 127.0.0.1:1883]
//!
//! It answers the read queries (Status 0, Module, SSId1/2) and the commands the apps send
//! (POWER<n> TOGGLE, AP, UPGRADE), so every screen can be exercised. Special cases:
//!   "ESP32 plug"     upgrades like a real ESP32: restarts into safeboot, then into 16.0.0
//!   "Unplugged plug" its retained LWT says Online, but it never answers (stale LWT)
//!
//! Safety: it refuses any broker that is not on this machine (127.0.0.1 / ::1 / localhost),
//! so it can never publish fake states or answers on a real home broker.

use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use serde_json::{Value, json};

struct Fake {
    topic: &'static str,
    name: &'static str,
    module: &'static str,
    relays: usize,
    power: Vec<bool>,
    rssi: i64,
    ap: i64,
    version: String,
    online: bool,
    boot_count: i64,
    restart_reason: &'static str,
    started: Instant,
    uptime_base: u64,
    ip: String,
    mac: String,
    sensor: bool,
    esp32: bool,
    /// Running the ESP32 safeboot image (in the middle of an upgrade).
    safeboot: bool,
    /// Retained LWT "Online" although the device is gone.
    stale_lwt: bool,
}

const SSIDS: [&str; 2] = ["Home-Main", "Home-Extender"];

#[allow(clippy::type_complexity)]
fn devices() -> Vec<Fake> {
    let list: &[(&str, &str, &str, usize, i64, i64, &str, bool, &str, u64, bool)] = &[
        ("cucina", "Kitchen light", "Sonoff Basic", 1, 96, 1, "16.0.0", true, "Power On", 1_123_456, false),
        ("salotto", "Living room", "Sonoff Dual R2", 2, 78, 1, "16.0.0", true, "Software/System restart", 86_400 * 3, false),
        ("camera", "Bedroom lamp", "Sonoff Mini", 1, 64, 2, "15.2.0", true, "Software/System restart", 5_400, false),
        ("lavatrice", "Washing machine", "Gosund SP111", 1, 88, 1, "16.0.0", true, "Power On", 86_400 * 24, true),
        ("garage", "Garage door", "Generic", 1, 41, 2, "16.0.0", true, "External System", 3_600 * 7, false),
        ("giardino", "Garden lights", "Sonoff 4CH", 4, 55, 2, "8.5.1", true, "Hardware Watchdog", 600, false),
        ("termostato", "Thermostat", "ESP32-DevKit", 1, 100, 1, "16.0.0", true, "Power On", 86_400 * 41, true),
        ("vecchia_presa", "Old plug", "Sonoff S20", 1, 0, 1, "6.7.1", false, "Power On", 0, false),
        ("presa_esp32", "ESP32 plug", "NOUS A8T", 1, 70, 1, "13.3.0", true, "Power On", 7_200, false),
        ("staccata", "Unplugged plug", "NOUS A8T", 1, 0, 1, "13.3.0", false, "Power On", 0, false),
    ];
    list.iter()
        .enumerate()
        .map(|(i, &(topic, name, module, relays, rssi, ap, version, online, reason, up, sensor))| Fake {
            topic,
            name,
            module,
            relays,
            power: (0..relays).map(|k| (i + k) % 3 == 0).collect(),
            rssi,
            ap,
            version: version.to_string(),
            online,
            boot_count: 3 + i as i64 * 17,
            restart_reason: reason,
            started: Instant::now(),
            uptime_base: up,
            ip: format!("10.99.0.{}", 20 + i),
            mac: format!("AA:BB:CC:00:00:{:02X}", 0x10 + i),
            sensor,
            esp32: topic == "presa_esp32" || topic == "staccata",
            safeboot: false,
            stale_lwt: topic == "staccata",
        })
        .collect()
}

impl Fake {
    fn uptime(&self) -> u64 {
        self.uptime_base + self.started.elapsed().as_secs()
    }

    fn tasmota_duration(s: u64) -> String {
        format!("{}T{:02}:{:02}:{:02}", s / 86400, s % 86400 / 3600, s % 3600 / 60, s % 60)
    }

    fn power_key(&self, i: usize) -> String {
        if self.relays == 1 { "POWER".into() } else { format!("POWER{}", i + 1) }
    }

    fn wifi(&self) -> Value {
        json!({
            "AP": self.ap, "SSId": SSIDS[(self.ap - 1) as usize], "BSSId": "11:22:33:44:55:66",
            "Channel": if self.ap == 1 { 6 } else { 11 }, "RSSI": self.rssi, "LinkCount": 1, "Downtime": "0T00:00:04"
        })
    }

    fn state(&self) -> Value {
        let mut v = json!({
            "Time": "2026-11-14T23:00:00", "Uptime": Self::tasmota_duration(self.uptime()),
            "UptimeSec": self.uptime(), "LoadAvg": 19, "Sleep": 50, "MqttCount": 1, "Wifi": self.wifi()
        });
        for i in 0..self.relays {
            v[self.power_key(i)] = json!(if self.power[i] { "ON" } else { "OFF" });
        }
        v
    }

    fn sensor_msg(&self) -> Option<Value> {
        self.sensor.then(|| {
            json!({ "Time": "2026-11-14T23:00:00",
                    "ENERGY": { "Power": if self.power[0] { 412 } else { 0 }, "Voltage": 231, "Current": 1.78, "Total": 812.4 },
                    "DS18B20": { "Temperature": 21.4 }, "TempUnit": "C" })
        })
    }

    fn status0(&self) -> Value {
        json!({
            "Status": { "Module": 1, "DeviceName": self.name, "FriendlyName": [self.name], "Topic": self.topic },
            "StatusPRM": { "Baudrate": 115200, "GroupTopic": "tasmotas",
                           "OtaUrl": if self.esp32 { "http://ota.tasmota.com/tasmota32/release/tasmota32.bin" } else { "http://ota.tasmota.com/tasmota/release/tasmota.bin.gz" },
                           "RestartReason": self.restart_reason, "Uptime": Self::tasmota_duration(self.uptime()),
                           "BootCount": self.boot_count, "Sleep": 50 },
            "StatusFWR": { "Version": self.full_version(),
                           "Core": if self.esp32 { "3_1_3" } else { "2_7_8" },
                           "SDK": if self.esp32 { "5.3.3.250501" } else { "2.2.2-dev(38a443e)" },
                           "Hardware": if self.esp32 { "ESP32-D0WD-V3 v3.1" } else { "ESP8266EX" } },
            "StatusMEM": if self.safeboot { json!({ "ProgramSize": 772, "Free": 59, "Heap": 120 }) }
                         else if self.esp32 { json!({ "ProgramSize": 1889, "Free": 990, "Heap": 140 }) }
                         else { json!({ "ProgramSize": 651, "Free": 352, "Heap": 25 }) },
            "StatusNET": { "Hostname": format!("{}-1234", self.topic), "IPAddress": self.ip, "Gateway": "10.99.0.1", "Mac": self.mac },
            "StatusMQT": { "MqttHost": "127.0.0.1", "MqttClient": format!("DVES_{}", self.topic), "MqttCount": 1 },
            "StatusSTS": self.state(),
        })
    }

    fn full_version(&self) -> String {
        let build = if self.safeboot { "safeboot" } else if self.esp32 { "tasmota32" } else { "release-tasmota" };
        format!("{}({build})", self.version)
    }

    fn discovery(&self) -> Value {
        json!({
            "ip": self.ip, "dn": self.name, "fn": [self.name], "hn": format!("{}-1234", self.topic),
            "mac": self.mac.replace(':', ""), "md": self.module, "sw": self.version, "t": self.topic,
            "ft": "%prefix%/%topic%/", "tp": ["cmnd", "stat", "tele"],
            "rl": (0..8).map(|i| if i < self.relays { 1 } else { 0 }).collect::<Vec<_>>(),
        })
    }
}

fn require_local(addr: &str) -> (String, u16) {
    let (host, port) = addr.rsplit_once(':').map(|(h, p)| (h, p.parse().unwrap_or(1883))).unwrap_or((addr, 1883));
    let host = host.trim_matches(['[', ']']).to_string();
    let ok = (host.as_str(), port)
        .to_socket_addrs()
        .map(|mut it| it.all(|a| a.ip().is_loopback()))
        .unwrap_or(false);
    if !ok {
        eprintln!("Refusing {addr}: the simulator only talks to a broker on this machine (127.0.0.1 / ::1).");
        std::process::exit(2);
    }
    (host, port)
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:1883".into());
    let (host, port) = require_local(&arg);

    let mut opts = MqttOptions::new("tasmota-derust-simulator", host.clone(), port);
    opts.set_keep_alive(Duration::from_secs(30));
    let (client, mut connection) = Client::new(opts, 256);
    let fakes = Arc::new(Mutex::new(devices()));

    let publish = |c: &Client, t: String, p: String, retain: bool| {
        let _ = c.try_publish(t, QoS::AtMostOnce, retain, p.into_bytes());
    };

    // Periodic telemetry, like Tasmota's TelePeriod (shortened to 15 s).
    {
        let client = client.clone();
        let fakes = fakes.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(15));
            for f in fakes.lock().unwrap().iter().filter(|f| f.online) {
                publish(&client, format!("tele/{}/STATE", f.topic), f.state().to_string(), false);
                if let Some(s) = f.sensor_msg() {
                    publish(&client, format!("tele/{}/SENSOR", f.topic), s.to_string(), false);
                }
            }
        });
    }

    println!("Simulating {} devices on {host}:{port}", fakes.lock().unwrap().len());
    for ev in connection.iter() {
        match ev {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                let _ = client.try_subscribe("cmnd/+/+", QoS::AtMostOnce);
                for f in fakes.lock().unwrap().iter() {
                    publish(&client, format!("tasmota/discovery/{}/config", f.mac.replace(':', "")), f.discovery().to_string(), true);
                    let lwt = if f.online || f.stale_lwt { "Online" } else { "Offline" };
                    publish(&client, format!("tele/{}/LWT", f.topic), lwt.into(), true);
                    if f.online {
                        publish(&client, format!("tele/{}/STATE", f.topic), f.state().to_string(), false);
                    }
                }
            }
            Ok(Event::Incoming(Packet::Publish(p))) => {
                let parts: Vec<&str> = p.topic.split('/').collect();
                if parts.len() != 3 {
                    continue;
                }
                let (topic, cmd) = (parts[1], parts[2].to_ascii_uppercase());
                let payload = String::from_utf8_lossy(&p.payload).trim().to_string();
                println!("<- {} {}", p.topic, payload);
                handle(&client, &fakes, topic, &cmd, &payload);
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("Broker: {e}");
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn handle(client: &Client, fakes: &Arc<Mutex<Vec<Fake>>>, topic: &str, cmd: &str, payload: &str) {
    let send = |t: String, p: String| {
        let _ = client.try_publish(t, QoS::AtMostOnce, false, p.into_bytes());
    };
    let mut all = fakes.lock().unwrap();
    let Some(f) = all.iter_mut().find(|f| f.topic == topic && f.online) else { return };
    let stat = |s: &str| format!("stat/{topic}/{s}");

    match cmd {
        "STATUS" if payload == "0" => send(stat("STATUS0"), f.status0().to_string()),
        "MODULE" if payload.is_empty() => send(stat("RESULT"), json!({ "Module": { "1": f.module } }).to_string()),
        "SSID1" if payload.is_empty() => send(stat("RESULT"), json!({ "SSId1": SSIDS[0] }).to_string()),
        "SSID2" if payload.is_empty() => send(stat("RESULT"), json!({ "SSId2": SSIDS[1] }).to_string()),
        c if c.starts_with("POWER") => {
            let n = c[5..].parse::<usize>().unwrap_or(1);
            if n == 0 || n > f.relays {
                return;
            }
            let i = n - 1;
            f.power[i] = match payload.to_ascii_uppercase().as_str() {
                "ON" | "1" => true,
                "OFF" | "0" => false,
                "TOGGLE" | "2" => !f.power[i],
                _ => f.power[i],
            };
            let key = f.power_key(i);
            let val = if f.power[i] { "ON" } else { "OFF" };
            send(stat("RESULT"), json!({ key.clone(): val }).to_string());
            send(stat(&key), val.into());
        }
        "AP" if payload == "1" || payload == "2" => {
            f.ap = payload.parse().unwrap();
            restart(client.clone(), fakes.clone(), topic.to_string(), None);
        }
        "UPGRADE" if payload == "1" && f.esp32 => {
            send(stat("UPGRADE"), json!({ "Upgrade": "Version 16.0.0 from http://ota.tasmota.com/tasmota32/release/tasmota32.bin" }).to_string());
            // ESP32: first into the safeboot partition, which downloads and writes the new
            // image, then into the new firmware.
            let (c, fk, t) = (client.clone(), fakes.clone(), topic.to_string());
            thread::spawn(move || {
                restart_now(&c, &fk, &t, |f| f.safeboot = true);
                thread::sleep(Duration::from_secs(10));
                restart_now(&c, &fk, &t, |f| {
                    f.safeboot = false;
                    f.version = "16.0.0".into();
                });
            });
        }
        "UPGRADE" if payload == "1" => {
            send(stat("UPGRADE"), json!({ "Upgrade": "Version 16.0.0 from http://ota.tasmota.com/tasmota/release/tasmota.bin.gz" }).to_string());
            restart(client.clone(), fakes.clone(), topic.to_string(), Some("16.0.0"));
        }
        _ => {}
    }
}

/// Goes offline for a few seconds, then comes back like a restarted Tasmota.
fn restart(client: Client, fakes: Arc<Mutex<Vec<Fake>>>, topic: String, new_version: Option<&'static str>) {
    thread::spawn(move || {
        restart_now(&client, &fakes, &topic, |f| {
            if let Some(v) = new_version {
                f.version = v.into();
            }
        })
    });
}

/// Restart on the calling thread: offline, `change` applied, online again.
fn restart_now(client: &Client, fakes: &Mutex<Vec<Fake>>, topic: &str, change: impl FnOnce(&mut Fake)) {
    let pub_ = |t: String, p: String, r: bool| {
        let _ = client.try_publish(t, QoS::AtMostOnce, r, p.into_bytes());
    };
    thread::sleep(Duration::from_secs(2));
    pub_(format!("tele/{topic}/LWT"), "Offline".into(), true);
    thread::sleep(Duration::from_secs(5));
    let mut all = fakes.lock().unwrap();
    let Some(f) = all.iter_mut().find(|f| f.topic == topic) else { return };
    change(f);
    f.started = Instant::now();
    f.uptime_base = 0;
    f.boot_count += 1;
    f.restart_reason = "Software/System restart";
    pub_(format!("tele/{topic}/LWT"), "Online".into(), true);
    let info1 = json!({ "Info1": { "Module": f.module, "Version": f.full_version(), "FallbackTopic": format!("cmnd/DVES_{topic}_fb/"), "GroupTopic": "tasmotas" } });
    pub_(format!("tele/{topic}/INFO1"), info1.to_string(), false);
    pub_(format!("tele/{topic}/STATE"), f.state().to_string(), false);
}
