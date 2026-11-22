//! Fake Tasmota devices on a LOCAL test broker, for trying the apps without real hardware.
//!
//!   mosquitto -p 1883                                  # a throwaway local broker
//!   cargo run -p tasmota-derust-core --example simulator [-- 127.0.0.1:1883]
//!
//! It answers the read queries (Status 0, Module, SSId1/2) and the commands the apps send
//! (POWER<n> TOGGLE, AP, UPGRADE), so every screen can be exercised. Special cases:
//!   "ESP32 plug"     upgrades like a real ESP32: restarts into safeboot, then into 16.0.0
//!   "Unplugged plug" its retained LWT says Online, but it never answers (stale LWT)
//!   "Old switch"     ESP8266 on 6.7.1 with ~1 MB flash: follows OtaUrl, refuses .gz before
//!                    8.2, goes through <name>-minimal when the image does not fit
//!
//!   HTTP-only devices on 127.0.0.1:18081-18083 (no MQTT until they get its settings), and a
//!                    non-Tasmota web server on 18084 that answers 401 like a Shelly: for
//!                    the network search, use the range 127.0.0.1:18081-18083; the one on
//!                    18083 has the web password "secret"
//!
//!   "Fan (HQ)", "Spot (HQ)": FullTopic headquarters/meeting-room-3/%prefix%/%topic%/ and no
//!                    tasmota/discovery (like SetOption19 1): found with the topic pattern
//!                    headquarters/+/%prefix%/%topic%/ (Settings > MQTT topics)
//!
//! Safety: it refuses any broker that is not on this machine (127.0.0.1 / ::1 / localhost),
//! so it can never publish fake states or answers on a real home broker.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use serde_json::{Value, json};

#[derive(Clone)]
struct Fake {
    /// Talks MQTT (false: HTTP-only until it gets broker settings).
    mqtt: bool,
    /// Its web server on 127.0.0.1 (HTTP-only devices).
    http_port: Option<u16>,
    web_password: Option<&'static str>,
    /// Custom FullTopic `headquarters/<room>/%prefix%/%topic%/` (no tasmota/discovery).
    room: Option<&'static str>,
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
    ota_url: String,
    /// Build tag in the version string, e.g. "sonoff", "lite", "release-tasmota".
    build: String,
    /// Size of the running image (ESP8266 flash model: about 1000 KB for code).
    program_kb: i64,
    /// Light channels (0: no light, 2 CCT, 5 RGBCCT) and its state.
    light: u8,
    dimmer: i64,
    hsb: (i64, i64),
    ct: i64,
    white: i64,
    /// Like real Tasmota, after (re)connecting it publishes its LWT before it subscribes to
    /// its command topic: commands arriving before this instant are lost.
    deaf_until: Option<Instant>,
    /// `StatusLOG.SetOption` registers.
    set_options: Vec<String>,
    /// SSId1 / SSId2 ("" = not set).
    ssids: [String; 2],
}

const FLASH_KB: i64 = 1000;

#[allow(clippy::type_complexity)]
/// What a firmware image does on the simulated old switch: (version, build, program KB,
/// download KB). Sizes are those of the official files.
fn image(url: &str) -> Option<(&'static str, &'static str, i64, i64)> {
    let table: &[(&str, (&str, &str, i64, i64))] = &[
        (
            "release-5.14.0/sonoff-classic.bin",
            ("5.14.0", "classic", 489, 489),
        ),
        (
            "release-5.14.0/sonoff-minimal.bin",
            ("5.14.0", "minimal", 428, 428),
        ),
        ("release-6.7.1/sonoff.bin", ("6.7.1", "sonoff", 564, 564)),
        (
            "release-6.7.1/sonoff-minimal.bin",
            ("6.7.1", "minimal", 368, 368),
        ),
        (
            "release-7.2.0/tasmota-lite.bin",
            ("7.2.0", "lite", 463, 464),
        ),
        (
            "release-7.2.0/tasmota-minimal.bin",
            ("7.2.0", "minimal", 373, 374),
        ),
        (
            "release-8.5.1/tasmota-lite.bin",
            ("8.5.1", "lite", 473, 474),
        ),
        (
            "release-8.5.1/tasmota-lite.bin.gz",
            ("8.5.1", "lite", 473, 338),
        ),
        (
            "release-8.5.1/tasmota-minimal.bin",
            ("8.5.1", "minimal", 351, 352),
        ),
        (
            "release-9.1.0/tasmota-lite.bin.gz",
            ("9.1.0", "lite", 473, 339),
        ),
        (
            "release-16.0.0/tasmota.bin.gz",
            ("16.0.0", "tasmota", 651, 459),
        ),
        ("release/tasmota.bin.gz", ("16.0.0", "tasmota", 651, 459)),
        (
            "release/tasmota-minimal.bin.gz",
            ("16.0.0", "minimal", 380, 258),
        ),
    ];
    table
        .iter()
        .find(|(k, _)| url.ends_with(k))
        .map(|(_, v)| *v)
}

const SSIDS: [&str; 2] = ["Home-Main", "Home-Extender"];

#[allow(clippy::type_complexity)]
fn devices() -> Vec<Fake> {
    devices_mqtt_template()
        .into_iter()
        .chain(http_devices())
        .chain(room_devices())
        .collect()
}

/// Devices on a custom FullTopic, without tasmota/discovery (8.5.1, like SetOption19 1).
fn room_devices() -> Vec<Fake> {
    let base = devices_mqtt_template();
    let make = |from: &str, topic: &'static str, name: &'static str, mac: &str| {
        let mut f = base.iter().find(|f| f.topic == from).cloned().expect("template");
        f.mqtt = false;
        f.room = Some("meeting-room-3");
        f.topic = topic;
        f.name = name;
        f.mac = mac.into();
        f.version = "8.5.1".into();
        f.build = "tasmota".into();
        f.online = true;
        f.stale_lwt = false;
        f
    };
    vec![
        make("cucina", "hq_fan", "Fan (HQ)", "AA:BB:CC:00:00:91"),
        make("lampada", "hq_spot", "Spot (HQ)", "AA:BB:CC:00:00:92"),
    ]
}

fn room_topic(f: &Fake, prefix: &str, rest: &str) -> String {
    format!("headquarters/{}/{prefix}/{}/{rest}", f.room.unwrap_or("room"), f.topic)
}

#[allow(clippy::type_complexity)]
fn devices_mqtt_template() -> Vec<Fake> {
    let list: &[(
        &str,
        &str,
        &str,
        usize,
        i64,
        i64,
        &str,
        bool,
        &str,
        u64,
        bool,
    )] = &[
        (
            "cucina",
            "Kitchen light",
            "Sonoff Basic",
            1,
            96,
            1,
            "16.0.0",
            true,
            "Power On",
            1_123_456,
            false,
        ),
        (
            "salotto",
            "Living room",
            "Sonoff Dual R2",
            2,
            78,
            1,
            "16.0.0",
            true,
            "Software/System restart",
            86_400 * 3,
            false,
        ),
        (
            "camera",
            "Bedroom lamp",
            "Sonoff Mini",
            1,
            64,
            2,
            "15.2.0",
            true,
            "Software/System restart",
            5_400,
            false,
        ),
        (
            "lavatrice",
            "Washing machine",
            "Gosund SP111",
            1,
            88,
            1,
            "16.0.0",
            true,
            "Power On",
            86_400 * 24,
            true,
        ),
        (
            "garage",
            "Garage door",
            "Generic",
            1,
            41,
            2,
            "16.0.0",
            true,
            "External System",
            3_600 * 7,
            false,
        ),
        (
            "giardino",
            "Garden lights",
            "Sonoff 4CH",
            4,
            55,
            2,
            "8.5.1",
            true,
            "Hardware Watchdog",
            600,
            false,
        ),
        (
            "termostato",
            "Thermostat",
            "ESP32-DevKit",
            1,
            100,
            1,
            "16.0.0",
            true,
            "Power On",
            86_400 * 41,
            true,
        ),
        (
            "vecchia_presa",
            "Old plug",
            "Sonoff S20",
            1,
            0,
            1,
            "6.7.1",
            false,
            "Power On",
            0,
            false,
        ),
        (
            "presa_esp32",
            "ESP32 plug",
            "NOUS A8T",
            1,
            70,
            1,
            "13.3.0",
            true,
            "Power On",
            7_200,
            false,
        ),
        (
            "staccata",
            "Unplugged plug",
            "NOUS A8T",
            1,
            0,
            1,
            "13.3.0",
            false,
            "Power On",
            0,
            false,
        ),
        (
            "vecchio_interruttore",
            "Old switch",
            "Sonoff Basic",
            1,
            66,
            1,
            "6.7.1",
            true,
            "Power On",
            86_400 * 900,
            false,
        ),
        (
            "lampada",
            "Desk lamp",
            "Generic",
            1,
            82,
            1,
            "16.0.0",
            true,
            "Power On",
            3_600 * 30,
            false,
        ),
        (
            "corridoio",
            "Hall light",
            "Generic",
            1,
            71,
            2,
            "16.0.0",
            true,
            "Power On",
            3_600 * 50,
            false,
        ),
    ];
    list.iter()
        .enumerate()
        .map(
            |(i, &(topic, name, module, relays, rssi, ap, version, online, reason, up, sensor))| {
                Fake {
                    mqtt: true,
                    http_port: None,
                    web_password: None,
                    room: None,
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
                    ota_url: if topic == "vecchio_interruttore" {
                        "http://thehackbox.org/tasmota/release/sonoff.bin".into()
                    } else if topic == "presa_esp32" || topic == "staccata" {
                        "http://ota.tasmota.com/tasmota32/release/tasmota32.bin".into()
                    } else {
                        "http://ota.tasmota.com/tasmota/release/tasmota.bin.gz".into()
                    },
                    build: if topic == "vecchio_interruttore" {
                        "sonoff".into()
                    } else {
                        "release-tasmota".into()
                    },
                    program_kb: if topic == "vecchio_interruttore" {
                        564
                    } else {
                        651
                    },
                    light: match topic {
                        "lampada" => 5,
                        "corridoio" => 2,
                        _ => 0,
                    },
                    dimmer: 70,
                    hsb: (30, 80),
                    ct: 300,
                    white: 0,
                    deaf_until: None,
                    // One device with a single Wi-Fi network, to add the second one.
                    ssids: if topic == "cucina" {
                        [SSIDS[0].to_string(), String::new()]
                    } else {
                        [SSIDS[0].to_string(), SSIDS[1].to_string()]
                    },
                    // 6.7.1 reports four registers, current firmware six.
                    set_options: if topic == "vecchio_interruttore" {
                        [
                            "00008009",
                            "2805C80001000600003C5A0A000000000000",
                            "00000000",
                            "00000000",
                        ]
                        .map(String::from)
                        .to_vec()
                    } else {
                        [
                            "00008009",
                            "2805C80001000600003C5A0A190000000000",
                            "00000080",
                            "00006000",
                            "00006000",
                            "00000001",
                        ]
                        .map(String::from)
                        .to_vec()
                    },
                }
            },
        )
        .collect()
}

/// Devices with MQTT off, reached over HTTP only (made from the "Kitchen light" and "Desk
/// lamp" fakes).
fn http_devices() -> Vec<Fake> {
    let base = devices_mqtt_template();
    let make = |from: &str, topic: &'static str, name: &'static str, port: u16, mac: &str, pw| {
        let mut f = base.iter().find(|f| f.topic == from).cloned().expect("template");
        f.mqtt = false;
        f.http_port = Some(port);
        f.web_password = pw;
        f.topic = topic;
        f.name = name;
        f.mac = mac.into();
        f.ip = format!("10.99.0.{}", 100 + port % 100);
        f.online = true;
        f.stale_lwt = false;
        f
    };
    vec![
        make("cucina", "http_presa", "Porch socket (HTTP)", 18081, "AA:BB:CC:00:00:81", None),
        make("lampada", "http_lampada", "Studio lamp (HTTP)", 18082, "AA:BB:CC:00:00:82", None),
        make("cucina", "http_protetto", "Locked plug (HTTP)", 18083, "AA:BB:CC:00:00:83", Some("secret")),
    ]
}

impl Fake {
    fn uptime(&self) -> u64 {
        self.uptime_base + self.started.elapsed().as_secs()
    }

    fn tasmota_duration(s: u64) -> String {
        format!(
            "{}T{:02}:{:02}:{:02}",
            s / 86400,
            s % 86400 / 3600,
            s % 3600 / 60,
            s % 60
        )
    }

    fn power_key(&self, i: usize) -> String {
        if self.relays == 1 {
            "POWER".into()
        } else {
            format!("POWER{}", i + 1)
        }
    }

    fn wifi(&self) -> Value {
        json!({
            "AP": self.ap, "SSId": self.ssids[(self.ap - 1) as usize], "BSSId": "11:22:33:44:55:66",
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
        if let (Value::Object(m), Value::Object(l)) = (&mut v, self.light_json()) {
            m.extend(l);
        }
        v
    }

    /// Light fields of STATE / RESULT, roughly as Tasmota computes them.
    fn light_json(&self) -> Value {
        if self.light == 0 {
            return json!({});
        }
        let d = if self.power[0] { self.dimmer } else { 0 };
        let scale = |x: i64| (x * d / 100).clamp(0, 255);
        let t = (self.ct - 153) * 255 / (500 - 153);
        let (cw, ww) = (scale(255 - t), scale(t));
        let pct = |x: i64| x * 100 / 255;
        if self.light == 2 {
            return json!({ "Dimmer": self.dimmer, "Color": format!("{cw:02X}{ww:02X}"), "CT": self.ct,
                           "Channel": [pct(cw), pct(ww)] });
        }
        // RGBCCT: either the colour or the white channels, like Tasmota's default.
        let white = self.white > 0;
        let (r, g, b) = if white {
            (0, 0, 0)
        } else {
            let (r, g, b) = tasmota_derust_core::light::hsv_to_rgb(
                self.hsb.0 as f32,
                self.hsb.1 as f32 / 100.0,
                1.0,
            );
            (scale(r as i64), scale(g as i64), scale(b as i64))
        };
        let (cw, ww) = if white { (cw, ww) } else { (0, 0) };
        json!({
            "Dimmer": self.dimmer,
            "Color": format!("{r:02X}{g:02X}{b:02X}{cw:02X}{ww:02X}"),
            "HSBColor": format!("{},{},{}", self.hsb.0, self.hsb.1, self.dimmer),
            "White": self.white,
            "CT": self.ct,
            "Channel": [pct(r), pct(g), pct(b), pct(cw), pct(ww)],
        })
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
                           "OtaUrl": self.ota_url,
                           "RestartReason": self.restart_reason, "Uptime": Self::tasmota_duration(self.uptime()),
                           "BootCount": self.boot_count, "Sleep": 50 },
            "StatusFWR": { "Version": self.full_version(),
                           "Core": if self.esp32 { "3_1_3" } else { "2_7_8" },
                           "SDK": if self.esp32 { "5.3.3.250501" } else { "2.2.2-dev(38a443e)" },
                           "Hardware": if self.esp32 { "ESP32-D0WD-V3 v3.1" } else { "ESP8266EX" } },
            "StatusMEM": if self.safeboot { json!({ "ProgramSize": 772, "Free": 59, "Heap": 120 }) }
                         else if self.esp32 { json!({ "ProgramSize": 1889, "Free": 990, "Heap": 140 }) }
                         else { json!({ "ProgramSize": self.program_kb, "Free": FLASH_KB - self.program_kb, "Heap": 25 }) },
            "StatusNET": { "Hostname": format!("{}-1234", self.topic), "IPAddress": self.ip, "Gateway": "10.99.0.1", "Mac": self.mac },
            "StatusMQT": { "MqttHost": "127.0.0.1", "MqttClient": format!("DVES_{}", self.topic), "MqttCount": 1 },
            "StatusSTS": self.state(),
            "StatusLOG": self.status_log(),
        })
    }

    fn status_log(&self) -> Value {
        json!({ "SerialLog": 2, "WebLog": 2, "MqttLog": 0, "SysLog": 0, "LogHost": "", "LogPort": 514,
                "SSId": self.ssids, "TelePeriod": 15, "Resolution": "558180C0",
                "SetOption": self.set_options })
    }

    fn full_version(&self) -> String {
        let build = if self.safeboot {
            "safeboot"
        } else if self.esp32 {
            "tasmota32"
        } else {
            &self.build
        };
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
    let (host, port) = addr
        .rsplit_once(':')
        .map(|(h, p)| (h, p.parse().unwrap_or(1883)))
        .unwrap_or((addr, 1883));
    let host = host.trim_matches(['[', ']']).to_string();
    let ok = (host.as_str(), port)
        .to_socket_addrs()
        .map(|mut it| it.all(|a| a.ip().is_loopback()))
        .unwrap_or(false);
    if !ok {
        eprintln!(
            "Refusing {addr}: the simulator only talks to a broker on this machine (127.0.0.1 / ::1)."
        );
        std::process::exit(2);
    }
    (host, port)
}

fn main() {
    let arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:1883".into());
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
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(15));
                for f in fakes.lock().unwrap().iter().filter(|f| f.online && f.mqtt) {
                    publish(
                        &client,
                        format!("tele/{}/STATE", f.topic),
                        f.state().to_string(),
                        false,
                    );
                    if let Some(s) = f.sensor_msg() {
                        publish(
                            &client,
                            format!("tele/{}/SENSOR", f.topic),
                            s.to_string(),
                            false,
                        );
                    }
                }
            }
        });
    }

    // Not a Tasmota: a web server with a login, answering 401 to everything (like a Shelly).
    thread::spawn(|| {
        let Ok(l) = TcpListener::bind(("127.0.0.1", 18084)) else { return };
        for conn in l.incoming().flatten() {
            let mut r = BufReader::new(&conn);
            let mut line = String::new();
            while r.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let mut w = &conn;
            let _ = write!(w, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 16\r\nConnection: close\r\n\r\n401 Unauthorized");
        }
    });
    for f in fakes.lock().unwrap().iter() {
        if let Some(p) = f.http_port {
            let (client, fakes, topic) = (client.clone(), fakes.clone(), f.topic);
            thread::spawn(move || http_server(p, client, fakes, topic));
        }
    }
    println!(
        "Simulating {} devices on {host}:{port}",
        fakes.lock().unwrap().len()
    );
    for ev in connection.iter() {
        match ev {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                let _ = client.try_subscribe("cmnd/+/+", QoS::AtMostOnce);
                let _ = client.try_subscribe("headquarters/+/cmnd/+/+", QoS::AtMostOnce);
                for f in fakes.lock().unwrap().iter().filter(|f| f.room.is_some()) {
                    publish(&client, room_topic(f, "tele", "LWT"), "Online".into(), true);
                    publish(&client, room_topic(f, "tele", "STATE"), f.state().to_string(), false);
                }
                for f in fakes.lock().unwrap().iter().filter(|f| f.mqtt) {
                    publish(
                        &client,
                        format!("tasmota/discovery/{}/config", f.mac.replace(':', "")),
                        f.discovery().to_string(),
                        true,
                    );
                    let lwt = if f.online || f.stale_lwt {
                        "Online"
                    } else {
                        "Offline"
                    };
                    publish(&client, format!("tele/{}/LWT", f.topic), lwt.into(), true);
                    if f.online {
                        publish(
                            &client,
                            format!("tele/{}/STATE", f.topic),
                            f.state().to_string(),
                            false,
                        );
                    }
                }
            }
            Ok(Event::Incoming(Packet::Publish(p))) => {
                if p.topic.starts_with("headquarters/") {
                    let payload = String::from_utf8_lossy(&p.payload).trim().to_string();
                    println!("<- {} {}", p.topic, payload);
                    room_command(&client, &fakes, &p.topic, &payload);
                    continue;
                }
                let parts: Vec<&str> = p.topic.split('/').collect();
                if parts.len() != 3 {
                    continue;
                }
                let (topic, cmd) = (parts[1], parts[2].to_ascii_uppercase());
                let payload = String::from_utf8_lossy(&p.payload).trim().to_string();
                println!(
                    "<- {} {}",
                    p.topic,
                    tasmota_derust_core::wifi::mask_passwords(&payload)
                );
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
    let Some(f) = all
        .iter_mut()
        .find(|f| f.topic == topic && f.online && f.mqtt && f.deaf_until.is_none_or(|t| Instant::now() >= t))
    else {
        return;
    };
    let stat = |s: &str| format!("stat/{topic}/{s}");

    match cmd {
        "STATUS" if payload == "0" => send(stat("STATUS0"), f.status0().to_string()),
        "STATUS" if payload == "3" => send(
            stat("STATUS3"),
            json!({ "StatusLOG": f.status_log() }).to_string(),
        ),
        c if c.starts_with("SETOPTION") => {
            use tasmota_derust_core::setoption;
            let Ok(n) = c[9..].parse::<u16>() else { return };
            if n > setoption::MAX_OPTION {
                return;
            }
            if !payload.is_empty() {
                let v = match payload.to_ascii_uppercase().as_str() {
                    "ON" => 1,
                    "OFF" => 0,
                    p => match p.parse::<u32>() {
                        Ok(v) => v,
                        Err(_) => return,
                    },
                };
                setoption::apply(&mut f.set_options, n, v);
            }
            let Some(&(_, v)) = setoption::decode(&f.set_options)
                .iter()
                .find(|(k, _)| *k == n)
            else {
                return;
            };
            let key = format!("SetOption{n}");
            let r = if setoption::is_numeric(n) {
                json!({ key: v })
            } else {
                json!({ key: if v == 1 { "ON" } else { "OFF" } })
            };
            send(stat("RESULT"), r.to_string());
        }
        "MODULE" if payload.is_empty() => send(
            stat("RESULT"),
            json!({ "Module": { "1": f.module } }).to_string(),
        ),
        "SSID1" if payload.is_empty() => {
            send(stat("RESULT"), json!({ "SSId1": f.ssids[0] }).to_string())
        }
        "SSID2" if payload.is_empty() => {
            send(stat("RESULT"), json!({ "SSId2": f.ssids[1] }).to_string())
        }
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
            let mut r = json!({ key.clone(): val });
            if let (Value::Object(m), Value::Object(l)) = (&mut r, f.light_json()) {
                m.extend(l);
            }
            send(stat("RESULT"), r.to_string());
            send(stat(&key), val.into());
        }
        "DIMMER" | "HSBCOLOR" | "CT" | "WHITE" if f.light > 0 && !payload.is_empty() => {
            let n = |s: &str| s.trim().parse::<i64>().ok();
            match cmd {
                "DIMMER" => {
                    if let Some(v) = n(payload) {
                        f.dimmer = v.clamp(0, 100);
                    }
                }
                "HSBCOLOR" if f.light >= 3 => {
                    let p: Vec<i64> = payload.split(',').filter_map(n).collect();
                    if p.len() == 3 {
                        f.hsb = (p[0].rem_euclid(360), p[1].clamp(0, 100));
                        f.dimmer = p[2].clamp(0, 100);
                        f.white = 0;
                    }
                }
                "CT" => {
                    if let Some(v) = n(payload) {
                        f.ct = v.clamp(153, 500);
                        if f.light == 5 && f.white == 0 {
                            f.white = f.dimmer.max(1);
                        }
                    }
                }
                "WHITE" if f.light >= 4 => {
                    if let Some(v) = n(payload) {
                        f.white = v.clamp(0, 100);
                    }
                }
                _ => return,
            }
            // Any light command switches the light on, like Tasmota.
            f.power[0] = true;
            let mut r = json!({ "POWER": "ON" });
            if let (Value::Object(m), Value::Object(l)) = (&mut r, f.light_json()) {
                m.extend(l);
            }
            send(stat("RESULT"), r.to_string());
        }
        "AP" if payload == "1" || payload == "2" => {
            f.ap = payload.parse().unwrap();
            restart(client.clone(), fakes.clone(), topic.to_string(), None);
        }
        // Only what the app sends: "SSId<n> <name>; Password<n> <pw>" (one restart).
        "BACKLOG" => {
            let mut changed = false;
            for part in payload.split(';') {
                let (cmd, val) = part.trim().split_once(' ').unwrap_or((part.trim(), ""));
                let cmd = cmd.to_ascii_uppercase();
                if let Some(n) = cmd
                    .strip_prefix("SSID")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    if (1..=2).contains(&n) && !val.is_empty() {
                        f.ssids[n - 1] = val.trim().to_string();
                        changed = true;
                    }
                } else if cmd.starts_with("PASSWORD") {
                    changed = true; // never stored, never printed
                }
            }
            if changed {
                restart(client.clone(), fakes.clone(), topic.to_string(), None);
            }
        }
        "OTAURL" if !payload.is_empty() => {
            f.ota_url = payload.to_string();
            send(stat("RESULT"), json!({ "OtaUrl": payload }).to_string());
        }
        "UPGRADE" if payload == "1" && f.topic == "vecchio_interruttore" => {
            let Some((version, build, program, download)) = image(&f.ota_url) else {
                send(
                    stat("UPGRADE"),
                    json!({ "Upgrade": "Failed: file not found" }).to_string(),
                );
                return;
            };
            let now: Vec<u32> = f
                .version
                .split('.')
                .map(|x| x.parse().unwrap_or(0))
                .collect();
            if f.ota_url.ends_with(".gz") && now < vec![8, 2] {
                // Old firmware can't unpack .gz images.
                send(
                    stat("UPGRADE"),
                    json!({ "Upgrade": "Failed: unexpected file" }).to_string(),
                );
                return;
            }
            send(
                stat("UPGRADE"),
                json!({ "Upgrade": format!("Version {version} from {}", f.ota_url) }).to_string(),
            );
            let free = FLASH_KB - f.program_kb;
            let minimal = if download > free {
                // Tasmota's own rule: <name up to the last '-'>-minimal.<ext> in the same folder.
                let (dir, file) = f.ota_url.rsplit_once('/').unwrap();
                let ext = &file[file.find('.').unwrap()..];
                let stem = &file[..file.len() - ext.len()];
                let name = stem.rsplit_once('-').map(|(n, _)| n).unwrap_or(stem);
                match image(&format!("{dir}/{name}-minimal{ext}")) {
                    Some(m) if m.3 <= free => Some(m),
                    _ => {
                        send(
                            stat("UPGRADE"),
                            json!({ "Upgrade": "Failed: not enough space" }).to_string(),
                        );
                        return;
                    }
                }
            } else {
                None
            };
            let (c, fk, t) = (client.clone(), fakes.clone(), topic.to_string());
            let old = now.first().is_some_and(|m| *m < 7);
            thread::spawn(move || {
                if old {
                    // Tasmota 6.x drops and restores MQTT while it downloads, before it
                    // restarts: back online for a while, still on the old version.
                    reconnect_now(&c, &fk, &t);
                }
                if let Some((mv, mb, mp, _)) = minimal {
                    restart_now(&c, &fk, &t, |f| {
                        f.version = mv.into();
                        f.build = mb.into();
                        f.program_kb = mp;
                    });
                    thread::sleep(Duration::from_secs(6));
                }
                restart_now(&c, &fk, &t, |f| {
                    f.version = version.into();
                    f.build = build.into();
                    f.program_kb = program;
                });
            });
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
            restart(
                client.clone(),
                fakes.clone(),
                topic.to_string(),
                Some("16.0.0"),
            );
        }
        _ => {}
    }
}

/// Goes offline for a few seconds, then comes back like a restarted Tasmota.
fn restart(
    client: Client,
    fakes: Arc<Mutex<Vec<Fake>>>,
    topic: String,
    new_version: Option<&'static str>,
) {
    thread::spawn(move || {
        restart_now(&client, &fakes, &topic, |f| {
            if let Some(v) = new_version {
                f.version = v.into();
            }
        })
    });
}

/// Restart on the calling thread: offline, `change` applied, online again.
/// MQTT dropped and restored without a restart (same version, uptime goes on).
fn reconnect_now(client: &Client, fakes: &Mutex<Vec<Fake>>, topic: &str) {
    let pub_ = |t: String, p: String| {
        let _ = client.try_publish(t, QoS::AtMostOnce, true, p.into_bytes());
    };
    thread::sleep(Duration::from_secs(1));
    pub_(format!("tele/{topic}/LWT"), "Offline".into());
    thread::sleep(Duration::from_secs(3));
    if let Some(f) = fakes.lock().unwrap().iter_mut().find(|f| f.topic == topic) {
        f.deaf_until = Some(Instant::now() + Duration::from_millis(1500));
    }
    pub_(format!("tele/{topic}/LWT"), "Online".into());
    thread::sleep(Duration::from_secs(8));
}

fn restart_now(
    client: &Client,
    fakes: &Mutex<Vec<Fake>>,
    topic: &str,
    change: impl FnOnce(&mut Fake),
) {
    let pub_ = |t: String, p: String, r: bool| {
        let _ = client.try_publish(t, QoS::AtMostOnce, r, p.into_bytes());
    };
    thread::sleep(Duration::from_secs(2));
    pub_(format!("tele/{topic}/LWT"), "Offline".into(), true);
    thread::sleep(Duration::from_secs(5));
    let mut all = fakes.lock().unwrap();
    let Some(f) = all.iter_mut().find(|f| f.topic == topic) else {
        return;
    };
    change(f);
    f.started = Instant::now();
    f.uptime_base = 0;
    f.boot_count += 1;
    f.restart_reason = "Software/System restart";
    f.deaf_until = Some(Instant::now() + Duration::from_millis(1500));
    pub_(format!("tele/{topic}/LWT"), "Online".into(), true);
    let info1 = json!({ "Info1": { "Module": f.module, "Version": f.full_version(), "FallbackTopic": format!("cmnd/DVES_{topic}_fb/"), "GroupTopic": "tasmotas" } });
    pub_(format!("tele/{topic}/INFO1"), info1.to_string(), false);
    pub_(format!("tele/{topic}/STATE"), f.state().to_string(), false);
}

// ------------------------------------------------------------------ HTTP-only devices

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A tiny Tasmota web server: `/cm?cmnd=…` for Status, Power and the MQTT settings.
fn http_server(port: u16, client: Client, fakes: Arc<Mutex<Vec<Fake>>>, topic: &'static str) {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else {
        eprintln!("HTTP {port}: port busy");
        return;
    };
    println!("HTTP-only device {topic} on http://127.0.0.1:{port}/");
    for conn in listener.incoming().flatten() {
        let mut reader = BufReader::new(&conn);
        let mut first = String::new();
        if reader.read_line(&mut first).is_err() {
            continue;
        }
        // Skip the headers.
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
            line.clear();
        }
        let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
        let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
        let param = |k: &str| {
            query
                .split('&')
                .find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(url_decode))
        };
        let (code, body) = http_answer(&client, &fakes, topic, param("cmnd"), param("password"));
        let mut w = &conn;
        let _ = write!(
            w,
            "HTTP/1.1 {code}\r\nServer: Tasmota/16.0.0 (ESP8266EX)\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

fn http_answer(
    client: &Client,
    fakes: &Arc<Mutex<Vec<Fake>>>,
    topic: &'static str,
    cmnd: Option<String>,
    password: Option<String>,
) -> (&'static str, String) {
    let mut all = fakes.lock().unwrap();
    let Some(f) = all.iter_mut().find(|f| f.topic == topic && f.online) else {
        return ("503 Service Unavailable", "{}".into());
    };
    if let Some(pw) = f.web_password
        && password.as_deref() != Some(pw)
    {
        return (
            "401 Unauthorized",
            json!({"WARNING": "Need user=<username>&password=<password>"}).to_string(),
        );
    }
    let Some(cmnd) = cmnd else {
        return ("200 OK", "{}".into());
    };
    println!("<- http {topic} {}", tasmota_derust_core::wifi::mask_passwords(&cmnd));
    let (cmd, payload) = cmnd.split_once(' ').unwrap_or((&cmnd, ""));
    let (cmd, payload) = (cmd.to_ascii_uppercase(), payload.trim().to_string());
    let body = match cmd.as_str() {
        "STATUS" => f.status0(),
        c if c.starts_with("POWER") => {
            let i = c[5..].parse::<usize>().unwrap_or(1).saturating_sub(1);
            if i >= f.relays {
                return ("200 OK", json!({"Command": "Unknown"}).to_string());
            }
            f.power[i] = match payload.to_ascii_uppercase().as_str() {
                "ON" | "1" => true,
                "OFF" | "0" => false,
                "TOGGLE" | "2" => !f.power[i],
                _ => f.power[i],
            };
            json!({ f.power_key(i): if f.power[i] { "ON" } else { "OFF" } })
        }
        "BACKLOG" if payload.to_ascii_uppercase().contains("MQTTHOST") => {
            // Like Tasmota: the new broker settings take effect after a restart; then it
            // shows up on MQTT.
            let (client, fakes) = (client.clone(), fakes.clone());
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(3));
                let mut all = fakes.lock().unwrap();
                let Some(f) = all.iter_mut().find(|f| f.topic == topic) else { return };
                f.mqtt = true;
                f.started = Instant::now();
                f.uptime_base = 0;
                f.boot_count += 1;
                f.restart_reason = "Software/System restart";
                let pub_ = |t: String, p: String, r: bool| {
                    let _ = client.try_publish(t, QoS::AtMostOnce, r, p.into_bytes());
                };
                pub_(format!("tasmota/discovery/{}/config", f.mac.replace(':', "")), f.discovery().to_string(), true);
                pub_(format!("tele/{topic}/LWT"), "Online".into(), true);
                pub_(format!("tele/{topic}/STATE"), f.state().to_string(), false);
                println!("{topic}: MQTT on");
            });
            json!({"SetOption3": "ON", "MqttHost": "set", "Restart": "Restarting"})
        }
        _ => json!({"Command": "Unknown"}),
    };
    ("200 OK", body.to_string())
}

/// Commands to the custom-FullTopic devices: Status, Power, Module, SSId.
fn room_command(client: &Client, fakes: &Arc<Mutex<Vec<Fake>>>, topic: &str, payload: &str) {
    let parts: Vec<&str> = topic.split('/').collect();
    let [_, room, "cmnd", t, cmd] = parts[..] else { return };
    let mut all = fakes.lock().unwrap();
    let Some(f) = all.iter_mut().find(|f| f.room == Some(room) && f.topic == t) else { return };
    let send = |rest: &str, body: String| {
        let _ = client.try_publish(room_topic(f, "stat", rest), QoS::AtMostOnce, false, body.into_bytes());
    };
    let cmd = cmd.to_ascii_uppercase();
    match cmd.as_str() {
        "STATUS" => send("STATUS0", f.status0().to_string()),
        "MODULE" => send("RESULT", json!({"Module": {"1": f.module}}).to_string()),
        c if c.starts_with("SSID") => {
            let i = if c.ends_with('2') { 1 } else { 0 };
            send("RESULT", json!({ format!("SSId{}", i + 1): f.ssids[i] }).to_string())
        }
        c if c.starts_with("POWER") => {
            let i = c[5..].parse::<usize>().unwrap_or(1).saturating_sub(1);
            if i >= f.relays {
                return;
            }
            f.power[i] = match payload.to_ascii_uppercase().as_str() {
                "ON" | "1" => true,
                "OFF" | "0" => false,
                "TOGGLE" | "2" => !f.power[i],
                _ => f.power[i],
            };
            let key = f.power_key(i);
            let on = if f.power[i] { "ON" } else { "OFF" };
            let _ = client.try_publish(room_topic(f, "stat", "RESULT"), QoS::AtMostOnce, false, json!({ key.clone(): on }).to_string().into_bytes());
            let _ = client.try_publish(room_topic(f, "stat", &key), QoS::AtMostOnce, false, on.as_bytes().to_vec());
        }
        _ => {}
    }
}

