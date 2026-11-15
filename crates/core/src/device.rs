//! Device model and parsing of Tasmota MQTT messages.
//!
//! Parsing is deliberately tolerant: fields may be missing or named differently on
//! older firmware, and every value is optional.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::Value;

pub const DEFAULT_FULLTOPIC: &str = "%prefix%/%topic%/";
pub const UPGRADE_TIMEOUT: Duration = Duration::from_secs(8 * 60);
/// A device that has not answered a status query within this time counts as unreachable,
/// whatever its (possibly stale, retained) LWT says.
pub const NO_ANSWER_AFTER: Duration = Duration::from_secs(30);

fn default_prefixes() -> [String; 3] {
    ["cmnd".into(), "stat".into(), "tele".into()]
}

#[derive(Debug, Clone)]
pub enum Action {
    Publish { topic: String, payload: String },
    Subscribe(String),
}

#[derive(Debug, Clone)]
pub enum UpgradePhase {
    Sent,
    Progress(String),
    Restarting,
    Done(String),
    Failed(String),
    /// A failed upgrade, kept as a note after the device came back.
    Note(String),
}

#[derive(Debug, Clone)]
pub struct Upgrade {
    pub started: Instant,
    pub from: Option<String>,
    pub phase: UpgradePhase,
    /// For a step of the upgrade path: the version it must end on.
    pub target: Option<[u32; 3]>,
    /// For a step of the upgrade path: (step, steps).
    pub step: Option<(usize, usize)>,
}

/// Why an upgrade did not start.
#[derive(Debug, Clone, PartialEq)]
pub enum StartError {
    Unknown,
    Offline,
    /// Already upgrading this device.
    Running,
    /// Another device is being upgraded: one at a time.
    OtherRunning(String),
    /// The plan says no (too old, not enough flash, size not checked yet...).
    NotPossible(String),
}

impl Upgrade {
    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            UpgradePhase::Sent | UpgradePhase::Progress(_) | UpgradePhase::Restarting
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct Device {
    pub topic: String,
    pub full_topic: String,
    /// cmnd, stat, tele
    pub prefixes: [String; 3],
    pub cmnd_prefix: String,
    pub stat_prefix: String,
    pub tele_prefix: String,

    /// What the LWT says (retained, so it can be stale). Use `is_online` / `reachable`.
    pub online: Option<bool>,
    /// The full status was asked since the last LWT.
    pub queried: bool,
    /// A live (not retained) message came from the device since the last LWT.
    pub answered: bool,
    /// When a status query was sent that has not been answered yet.
    pub asked: Option<Instant>,

    pub device_name: Option<String>,
    pub friendly_name: Option<String>,
    pub module: Option<String>,
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub gateway: Option<String>,
    pub mac: Option<String>,
    pub version: Option<String>,
    pub core: Option<String>,
    pub sdk: Option<String>,
    pub hardware: Option<String>,
    pub ota_url: Option<String>,
    pub restart_reason: Option<String>,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub group_topic: Option<String>,
    pub mqtt_client: Option<String>,
    pub downtime: Option<String>,

    pub program_size: Option<i64>,
    pub free: Option<i64>,
    pub boot_count: Option<i64>,
    pub load_avg: Option<i64>,
    pub sleep: Option<i64>,
    pub mqtt_count: Option<i64>,
    pub link_count: Option<i64>,
    pub channel: Option<i64>,
    pub rssi: Option<i64>,
    pub ap: Option<i64>,

    /// Uptime in seconds and the moment it was received.
    pub uptime: Option<(u64, Instant)>,
    pub power: Vec<Option<bool>>,
    pub sensor: Option<Value>,
    pub upgrade: Option<Upgrade>,
}

impl Device {
    pub fn new(topic: &str, full_topic: &str, prefixes: [String; 3]) -> Self {
        let mut d = Device {
            topic: topic.to_string(),
            full_topic: full_topic.to_string(),
            prefixes,
            ..Default::default()
        };
        d.refresh_topics();
        d
    }

    fn expand(&self, prefix: &str) -> String {
        let mut s = self
            .full_topic
            .replace("%prefix%", prefix)
            .replace("%topic%", &self.topic);
        if let Some(h) = &self.hostname {
            s = s.replace("%hostname%", h);
        }
        if let Some(m) = &self.mac {
            let hex: String = m.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            if hex.len() >= 6 {
                s = s.replace("%id%", &hex[hex.len() - 6..].to_ascii_uppercase());
            }
        }
        if !s.ends_with('/') {
            s.push('/');
        }
        s
    }

    pub fn refresh_topics(&mut self) {
        self.cmnd_prefix = self.expand(&self.prefixes[0]);
        self.stat_prefix = self.expand(&self.prefixes[1]);
        self.tele_prefix = self.expand(&self.prefixes[2]);
    }

    pub fn cmnd(&self, command: &str) -> String {
        format!("{}{}", self.cmnd_prefix, command)
    }

    /// Online = not reported offline, and actually answering. A retained "Online" alone is
    /// not enough: the broker keeps it when a device disappears without its LWT being sent.
    pub fn is_online(&self) -> bool {
        self.online != Some(false) && self.answered
    }

    /// Asked for its status, no answer within `NO_ANSWER_AFTER`.
    pub fn no_answer(&self) -> bool {
        self.online != Some(false)
            && !self.answered
            && self.asked.is_some_and(|t| t.elapsed() > NO_ANSWER_AFTER)
    }

    /// Some(true) online, Some(false) offline or not answering, None still waiting.
    pub fn reachable(&self) -> Option<bool> {
        if self.is_online() {
            Some(true)
        } else if self.online == Some(false) || self.no_answer() {
            Some(false)
        } else {
            None
        }
    }

    pub fn is_offline(&self) -> bool {
        self.reachable() == Some(false)
    }

    /// "Offline", "No answer" or None.
    pub fn offline_label(&self) -> Option<&'static str> {
        if self.online == Some(false) {
            Some("Offline")
        } else if self.no_answer() {
            Some("No answer")
        } else {
            None
        }
    }

    pub fn display_name(&self) -> &str {
        self.device_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.friendly_name.as_deref().filter(|s| !s.is_empty()))
            .unwrap_or(&self.topic)
    }

    pub fn web_url(&self) -> Option<String> {
        self.ip
            .as_deref()
            .filter(|ip| !ip.is_empty() && *ip != "0.0.0.0")
            .or(self.hostname.as_deref())
            .map(|h| format!("http://{h}/"))
    }

    pub fn current_uptime(&self) -> Option<u64> {
        self.uptime.map(|(secs, at)| {
            if self.is_online() {
                secs + at.elapsed().as_secs()
            } else {
                secs
            }
        })
    }

    pub fn fallback_topic(&self) -> Option<String> {
        self.mqtt_client
            .as_ref()
            .map(|c| format!("{}/{}_fb/", self.prefixes[0], c))
    }

    /// Short summary of the most useful telemetry value(s).
    pub fn telemetry(&self) -> Option<String> {
        let s = self.sensor.as_ref()?.as_object()?;
        let mut parts = Vec::new();
        if let Some(p) = s.get("ENERGY").and_then(|e| e.get("Power")) {
            parts.push(format!("{} W", fmt_value(p)));
        }
        let unit = s.get("TempUnit").and_then(Value::as_str).unwrap_or("C");
        for v in s.values() {
            let Some(o) = v.as_object() else { continue };
            if let Some(t) = o.get("Temperature") {
                parts.push(format!("{} °{unit}", fmt_value(t)));
                if let Some(h) = o.get("Humidity") {
                    parts.push(format!("{} %", fmt_value(h)));
                }
                break;
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("  "))
        }
    }

    /// LWT received. Every LWT starts over: the device is asked for its full status again
    /// (when it says Online) and counts as online only once it answers.
    fn set_online(&mut self, online: bool) {
        self.online = Some(online);
        self.queried = false;
        self.answered = false;
        self.asked = None;
        if let Some(u) = &mut self.upgrade {
            if !online && u.is_active() {
                u.phase = UpgradePhase::Restarting;
            } else if let (true, UpgradePhase::Failed(m)) = (online, &u.phase) {
                // Back after a failed upgrade: no more alarm, the reason stays as a note.
                u.phase = UpgradePhase::Note(format!("Last upgrade failed: {m}"));
            }
        }
    }

    fn set_power(&mut self, idx: usize, value: &str) {
        let state = if value.eq_ignore_ascii_case("on") {
            true
        } else if value.eq_ignore_ascii_case("off") {
            false
        } else {
            return;
        };
        if idx >= 32 {
            return;
        }
        if self.power.len() <= idx {
            self.power.resize(idx + 1, None);
        }
        self.power[idx] = Some(state);
    }

    fn parse_power_keys(&mut self, v: &Value) {
        let Some(o) = v.as_object() else { return };
        for (k, val) in o {
            let ku = k.to_ascii_uppercase();
            let Some(rest) = ku.strip_prefix("POWER") else {
                continue;
            };
            let idx = if rest.is_empty() {
                0
            } else if let Ok(n) = rest.parse::<usize>() {
                n.saturating_sub(1)
            } else {
                continue;
            };
            if let Some(s) = val.as_str() {
                self.set_power(idx, s);
            }
        }
    }

    fn parse_state(&mut self, v: &Value) {
        if let Some(s) = v.get("UptimeSec").and_then(Value::as_u64) {
            self.uptime = Some((s, Instant::now()));
        } else if let Some(s) = v
            .get("Uptime")
            .and_then(Value::as_str)
            .and_then(parse_duration)
        {
            self.uptime = Some((s, Instant::now()));
        }
        set_i64(&mut self.load_avg, v, "LoadAvg");
        set_i64(&mut self.sleep, v, "Sleep");
        set_i64(&mut self.mqtt_count, v, "MqttCount");
        self.parse_power_keys(v);
        if let Some(w) = v.get("Wifi") {
            set_i64(&mut self.ap, w, "AP");
            set_str(&mut self.ssid, w, "SSId");
            set_str(&mut self.bssid, w, "BSSId");
            set_i64(&mut self.channel, w, "Channel");
            set_i64(&mut self.rssi, w, "RSSI");
            set_i64(&mut self.link_count, w, "LinkCount");
            set_str(&mut self.downtime, w, "Downtime");
        }
    }

    fn parse_status(&mut self, v: &Value) {
        if let Some(s) = v.get("Status") {
            set_str(&mut self.device_name, s, "DeviceName");
            match s.get("FriendlyName") {
                Some(Value::Array(a)) => {
                    self.friendly_name = a.iter().find_map(|x| x.as_str()).map(str::to_string)
                }
                Some(Value::String(f)) => self.friendly_name = Some(f.clone()),
                _ => {}
            }
        }
        if let Some(s) = v.get("StatusPRM") {
            set_str(&mut self.group_topic, s, "GroupTopic");
            set_str(&mut self.ota_url, s, "OtaUrl");
            set_str(&mut self.restart_reason, s, "RestartReason");
            set_i64(&mut self.boot_count, s, "BootCount");
            set_i64(&mut self.sleep, s, "Sleep");
            if let Some(u) = s
                .get("Uptime")
                .and_then(Value::as_str)
                .and_then(parse_duration)
            {
                self.uptime = Some((u, Instant::now()));
            }
        }
        if let Some(s) = v.get("StatusFWR") {
            set_str(&mut self.version, s, "Version");
            set_str(&mut self.core, s, "Core");
            set_str(&mut self.sdk, s, "SDK");
            set_str(&mut self.hardware, s, "Hardware");
            self.check_upgrade_finished();
        }
        if let Some(s) = v.get("StatusMEM") {
            set_i64(&mut self.program_size, s, "ProgramSize");
            set_i64(&mut self.free, s, "Free");
        }
        if let Some(s) = v.get("StatusNET") {
            set_str(&mut self.hostname, s, "Hostname");
            set_str(&mut self.ip, s, "IPAddress");
            set_str(&mut self.gateway, s, "Gateway");
            set_str(&mut self.mac, s, "Mac");
        }
        if let Some(s) = v.get("StatusMQT") {
            set_str(&mut self.mqtt_client, s, "MqttClient");
            set_i64(&mut self.mqtt_count, s, "MqttCount");
        }
        if let Some(s) = v.get("StatusSTS") {
            self.parse_state(s);
        }
        if let Some(s) = v.get("StatusSNS") {
            self.sensor = Some(s.clone());
        }
    }

    fn parse_info(&mut self, v: &Value) {
        let get = |k: &str| v.get(k).unwrap_or(v);
        let i1 = get("Info1");
        set_str(&mut self.module, i1, "Module");
        set_str(&mut self.version, i1, "Version");
        set_str(&mut self.group_topic, i1, "GroupTopic");
        let i2 = get("Info2");
        set_str(&mut self.hostname, i2, "Hostname");
        set_str(&mut self.ip, i2, "IPAddress");
        let i3 = get("Info3");
        set_str(&mut self.restart_reason, i3, "RestartReason");
        set_i64(&mut self.boot_count, i3, "BootCount");
    }

    fn parse_result(&mut self, v: &Value, ssids: &mut [Option<String>; 2]) {
        self.parse_power_keys(v);
        match v.get("Module") {
            Some(Value::Object(m)) => {
                if let Some(name) = m.values().find_map(Value::as_str) {
                    self.module = Some(name.to_string());
                }
            }
            // Old firmware: "1 (Sonoff Basic)"
            Some(Value::String(s)) => {
                let name = s
                    .split_once('(')
                    .map(|(_, r)| r.trim_end_matches(')').to_string())
                    .unwrap_or_else(|| s.clone());
                self.module = Some(name);
            }
            _ => {}
        }
        for (i, slot) in ssids.iter_mut().enumerate() {
            if let Some(s) = v.get(format!("SSId{}", i + 1)).and_then(Value::as_str) {
                *slot = Some(s.to_string());
            }
        }
        if let Some(gt) = v
            .get("GroupTopic1")
            .or(v.get("GroupTopic"))
            .and_then(Value::as_str)
        {
            self.group_topic = Some(gt.to_string());
        }
        if let Some(msg) = v.get("Upgrade").and_then(Value::as_str) {
            if let Some(u) = &mut self.upgrade {
                u.phase = if msg.to_ascii_lowercase().contains("fail") {
                    UpgradePhase::Failed(msg.to_string())
                } else {
                    UpgradePhase::Progress(msg.to_string())
                };
            }
        }
        // Answer to "OtaUrl <url>".
        set_str(&mut self.ota_url, v, "OtaUrl");
        // Some firmware answers Status commands with RESULT.
        self.parse_status(v);
    }

    fn check_upgrade_finished(&mut self) {
        let version = self.version.clone().unwrap_or_default();
        let Some(u) = &mut self.upgrade else { return };
        let lower = version.to_ascii_lowercase();
        let from = u.from.clone().unwrap_or_else(|| "?".into());
        let changed = short_version(&from) != short_version(&version);
        match &u.phase {
            // ESP8266 goes through tasmota-minimal, ESP32 through its safeboot partition:
            // the real firmware comes after one more restart.
            UpgradePhase::Restarting | UpgradePhase::Progress(_) | UpgradePhase::Sent
                if lower.contains("safeboot") || lower.contains("minimal") =>
            {
                let what = if lower.contains("safeboot") {
                    "Safeboot"
                } else {
                    "Minimal firmware"
                };
                u.phase =
                    UpgradePhase::Progress(format!("{what} running, writing the new firmware…"));
                // A second download and restart follow: the timeout starts again.
                u.started = Instant::now();
            }
            UpgradePhase::Restarting if u.target.is_some() => {
                let target = u.target.unwrap_or_default();
                let now = parse_version(&version)
                    .map(|v| [0, 1, 2].map(|i| v.get(i).copied().unwrap_or(0)));
                let left = u
                    .step
                    .map(|(_, total)| total.saturating_sub(1))
                    .unwrap_or(0);
                u.phase = if now == Some(target) {
                    let next = match left {
                        0 => String::new(),
                        1 => " 1 step left: upgrade again for the last one.".into(),
                        n => format!(" {n} steps left: upgrade again for the next one."),
                    };
                    UpgradePhase::Done(format!(
                        "Upgraded {} → {}.{next}",
                        short_version(&from),
                        short_version(&version)
                    ))
                } else if changed {
                    UpgradePhase::Failed(format!(
                        "Restarted with {}, expected {}.{}.{}: path stopped. Check the device, then \
                         upgrade again to continue from there.",
                        short_version(&version),
                        target[0],
                        target[1],
                        target[2]
                    ))
                } else {
                    UpgradePhase::Failed(format!(
                        "Restarted, still {}: the step did not take. Check free flash and OtaUrl, then retry.",
                        short_version(&version)
                    ))
                };
            }
            UpgradePhase::Restarting => {
                u.phase = if changed {
                    UpgradePhase::Done(format!(
                        "Upgraded {} → {}",
                        short_version(&from),
                        short_version(&version)
                    ))
                } else {
                    UpgradePhase::Done(format!("Restarted, still {}", short_version(&version)))
                };
            }
            // Finished or failed earlier, but the device now runs another version: it did
            // upgrade after all (slow restart, or upgraded from another app).
            UpgradePhase::Failed(_) | UpgradePhase::Note(_) if changed => {
                u.phase = UpgradePhase::Done(format!(
                    "Upgraded {} → {}",
                    short_version(&from),
                    short_version(&version)
                ));
            }
            UpgradePhase::Done(m) if changed && m.starts_with("Restarted, still") => {
                u.phase = UpgradePhase::Done(format!(
                    "Upgraded {} → {}",
                    short_version(&from),
                    short_version(&version)
                ));
            }
            _ => {}
        }
    }
}

#[derive(Default)]
pub struct Store {
    pub devices: BTreeMap<String, Device>,
    /// SSId1 / SSId2 (assumed identical on all devices).
    pub ssids: [Option<String>; 2],
    /// Firmware image sizes, for the free flash check of the upgrade path.
    pub sizes: crate::ota::Sizes,
}

impl Store {
    /// A message that was sent live (not a retained one replayed by the broker).
    pub fn handle(&mut self, topic: &str, payload: &str) -> Vec<Action> {
        self.handle_message(topic, payload, false)
    }

    /// `retained`: replayed by the broker on subscribe. Retained messages fill in data but
    /// never prove that a device is alive.
    pub fn handle_message(&mut self, topic: &str, payload: &str, retained: bool) -> Vec<Action> {
        let mut out = Vec::new();

        if let Some(rest) = topic.strip_prefix("tasmota/discovery/") {
            if rest.ends_with("/config") {
                if let Some(key) = self.handle_discovery(payload, &mut out) {
                    self.query_if_needed(&key, &mut out);
                }
            }
            return out;
        }

        let mut hit: Option<(String, bool, String)> = None;
        for (key, d) in &self.devices {
            if let Some(s) = topic
                .strip_prefix(d.tele_prefix.as_str())
                .filter(|s| !s.contains('/'))
            {
                hit = Some((key.clone(), true, s.to_string()));
                break;
            }
            if let Some(s) = topic
                .strip_prefix(d.stat_prefix.as_str())
                .filter(|s| !s.contains('/'))
            {
                hit = Some((key.clone(), false, s.to_string()));
                break;
            }
        }

        // Fallback discovery from LWT for devices without native discovery.
        if hit.is_none() {
            let parts: Vec<&str> = topic.split('/').collect();
            let lwt =
                payload.eq_ignore_ascii_case("online") || payload.eq_ignore_ascii_case("offline");
            if parts.len() == 3
                && parts[0] == "tele"
                && parts[2] == "LWT"
                && lwt
                && !parts[1].is_empty()
            {
                let d = Device::new(parts[1], DEFAULT_FULLTOPIC, default_prefixes());
                self.devices.insert(parts[1].to_string(), d);
                hit = Some((parts[1].to_string(), true, "LWT".into()));
            }
        }

        let Some((key, is_tele, suffix)) = hit else {
            return out;
        };
        let suffix = suffix.to_ascii_uppercase();
        let json: Option<Value> = serde_json::from_str::<Value>(payload)
            .ok()
            .filter(Value::is_object);

        let Some(d) = self.devices.get_mut(&key) else {
            return out;
        };
        let is_lwt = is_tele && suffix == "LWT";
        if !is_lwt && !retained {
            d.answered = true;
            d.asked = None;
        }
        match (is_tele, suffix.as_str()) {
            (true, "LWT") => d.set_online(payload.eq_ignore_ascii_case("online")),
            (true, "STATE") => {
                if let Some(v) = &json {
                    d.parse_state(v)
                }
            }
            (true, "SENSOR") => d.sensor = json.clone(),
            (true, s) if s.starts_with("INFO") => {
                if let Some(v) = &json {
                    d.parse_info(v)
                }
            }
            (_, "RESULT") | (false, "UPGRADE") => {
                if let Some(v) = &json {
                    d.parse_result(v, &mut self.ssids)
                }
            }
            (false, s) if s.starts_with("STATUS") => {
                if let Some(v) = &json {
                    d.parse_status(v)
                }
            }
            (false, s) if s.starts_with("POWER") => {
                let idx = s[5..].parse::<usize>().unwrap_or(1).saturating_sub(1);
                d.set_power(idx, payload.trim());
            }
            _ => {}
        }

        self.query_if_needed(&key, &mut out);
        out
    }

    /// Asks for the full status once per LWT, unless the device is reported offline.
    fn query_if_needed(&mut self, key: &str, out: &mut Vec<Action>) {
        let Some(d) = self.devices.get_mut(key) else {
            return;
        };
        if d.online == Some(false) || d.queried {
            return;
        }
        d.queried = true;
        if !d.answered {
            d.asked = Some(Instant::now());
        }
        self.query(key, out);
    }

    fn handle_discovery(&mut self, payload: &str, out: &mut Vec<Action>) -> Option<String> {
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return None;
        };
        let t = v
            .get("t")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())?;
        let ft = v
            .get("ft")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_FULLTOPIC)
            .to_string();
        let mut tp = default_prefixes();
        if let Some(a) = v.get("tp").and_then(Value::as_array) {
            for (i, p) in a.iter().take(3).enumerate() {
                if let Some(s) = p.as_str() {
                    tp[i] = s.to_string();
                }
            }
        }

        let d = self
            .devices
            .entry(t.to_string())
            .or_insert_with(|| Device::new(t, &ft, tp.clone()));
        d.full_topic = ft.clone();
        d.prefixes = tp.clone();
        set_str(&mut d.device_name, &v, "dn");
        if let Some(f) = v
            .get("fn")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find_map(Value::as_str))
        {
            d.friendly_name = Some(f.to_string());
        }
        set_str(&mut d.hostname, &v, "hn");
        set_str(&mut d.ip, &v, "ip");
        if d.module.is_none() {
            set_str(&mut d.module, &v, "md");
        }
        if d.version.is_none() {
            set_str(&mut d.version, &v, "sw");
        }
        if let Some(m) = v.get("mac").and_then(Value::as_str) {
            d.mac = Some(format_mac(m));
        }
        if let Some(rl) = v.get("rl").and_then(Value::as_array) {
            let relays = rl.iter().filter(|x| x.as_i64().unwrap_or(0) != 0).count();
            if d.power.len() < relays {
                d.power.resize(relays, None);
            }
        }
        d.refresh_topics();

        if ft != DEFAULT_FULLTOPIC || tp != default_prefixes() {
            out.push(Action::Subscribe(format!("{}+", d.tele_prefix)));
            out.push(Action::Subscribe(format!("{}+", d.stat_prefix)));
        }
        Some(t.to_string())
    }

    fn query(&self, key: &str, out: &mut Vec<Action>) {
        let Some(d) = self.devices.get(key) else {
            return;
        };
        let mut send = |cmd: &str, payload: &str| {
            out.push(Action::Publish {
                topic: d.cmnd(cmd),
                payload: payload.into(),
            })
        };
        send("STATUS", "0");
        send("MODULE", "");
        if self.ssids[0].is_none() {
            send("SSID1", "");
        }
        if self.ssids[1].is_none() {
            send("SSID2", "");
        }
    }

    /// Asks every device not reported offline for a full status again (also the ones that
    /// did not answer: they get another `NO_ANSWER_AFTER` to do it).
    pub fn refresh_all(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        let keys: Vec<String> = self
            .devices
            .iter()
            .filter(|(_, d)| d.online != Some(false))
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys {
            if let Some(d) = self.devices.get_mut(&k).filter(|d| !d.answered) {
                d.asked = Some(Instant::now());
            }
            self.query(&k, &mut out);
        }
        out
    }

    /// A state-changing command. None when the device is not online (offline, not answering
    /// or not answered yet): commands only go to devices that just proved they are there.
    pub fn command(&self, key: &str, cmd: &str, payload: &str) -> Option<Action> {
        self.devices
            .get(key)
            .filter(|d| d.is_online())
            .map(|d| Action::Publish {
                topic: d.cmnd(cmd),
                payload: payload.into(),
            })
    }

    /// What upgrading this device would do now (see `ota`).
    pub fn upgrade_plan(
        &self,
        key: &str,
        targets: &crate::ota::Targets,
    ) -> Option<crate::ota::PlanInfo> {
        self.devices
            .get(key)
            .map(|d| crate::ota::plan_info(d, targets, &self.sizes))
    }

    /// Starts the next upgrade (one step of the path, or the final one). Only one device at
    /// a time, only online devices, only when the plan allows it (free flash checked).
    pub fn start_upgrade(
        &mut self,
        key: &str,
        targets: &crate::ota::Targets,
    ) -> Result<Vec<Action>, StartError> {
        let d = self.devices.get(key).ok_or(StartError::Unknown)?;
        if !d.is_online() {
            return Err(StartError::Offline);
        }
        if let Some((other, _)) = self
            .devices
            .iter()
            .find(|(_, d)| d.upgrade.as_ref().is_some_and(|u| u.is_active()))
        {
            return Err(if other == key {
                StartError::Running
            } else {
                StartError::OtherRunning(self.devices[other].display_name().to_string())
            });
        }
        let info = crate::ota::plan_info(d, targets, &self.sizes);
        if !info.can_start {
            let why = info
                .warnings
                .last()
                .or(info.details.last())
                .cloned()
                .unwrap_or(info.title);
            return Err(StartError::NotPossible(why));
        }
        let plan = crate::ota::plan(
            d.version.as_deref(),
            d.ota_url.as_deref(),
            crate::ota::is_esp32(d),
        );
        let (target, step) = match plan {
            crate::ota::Plan::Step {
                target,
                index,
                total,
                ..
            } => (Some(target), Some((index, total))),
            _ => (None, None),
        };
        let d = self.devices.get_mut(key).ok_or(StartError::Unknown)?;
        let mut out = Vec::new();
        if let (true, Some(url)) = (info.sets_ota_url, info.url.as_ref()) {
            out.push(Action::Publish {
                topic: d.cmnd("OTAURL"),
                payload: url.clone(),
            });
            d.ota_url = Some(url.clone());
        }
        out.push(Action::Publish {
            topic: d.cmnd("UPGRADE"),
            payload: "1".into(),
        });
        d.upgrade = Some(Upgrade {
            started: Instant::now(),
            from: d.version.clone(),
            phase: UpgradePhase::Sent,
            target,
            step,
        });
        Ok(out)
    }

    pub fn check_timeouts(&mut self) {
        for d in self.devices.values_mut() {
            if let Some(u) = &mut d.upgrade {
                if u.is_active() && u.started.elapsed() > UPGRADE_TIMEOUT {
                    u.phase = UpgradePhase::Failed("No answer after 8 minutes".into());
                }
            }
        }
    }
}

fn set_str(field: &mut Option<String>, v: &Value, key: &str) {
    match v.get(key) {
        Some(Value::String(s)) => *field = Some(s.clone()),
        Some(Value::Number(n)) => *field = Some(n.to_string()),
        _ => {}
    }
}

fn set_i64(field: &mut Option<i64>, v: &Value, key: &str) {
    match v.get(key) {
        Some(Value::Number(n)) => *field = n.as_i64().or(n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => {
            if let Ok(n) = s.trim().parse() {
                *field = Some(n)
            }
        }
        _ => {}
    }
}

fn fmt_value(v: &Value) -> String {
    match v {
        Value::Array(a) => a.iter().map(fmt_value).collect::<Vec<_>>().join(" / "),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn format_mac(m: &str) -> String {
    let hex: String = m
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase();
    if hex.len() != 12 {
        return m.to_string();
    }
    hex.as_bytes()
        .chunks(2)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(":")
}

/// Parses Tasmota durations like `1T08:43:46` or `08:43:46` into seconds.
pub fn parse_duration(s: &str) -> Option<u64> {
    let (days, rest) = match s.split_once('T') {
        Some((d, r)) => (d.trim().parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let mut parts = rest.trim().split(':').map(|p| p.parse::<u64>());
    let h = parts.next()?.ok()?;
    let m = parts.next()?.ok()?;
    let sec = parts.next()?.ok()?;
    Some(days * 86400 + h * 3600 + m * 60 + sec)
}

pub fn format_duration(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{d}d {h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

/// "16.0.0(release-tasmota)" -> "16.0.0"
pub fn short_version(v: &str) -> &str {
    v.split('(').next().unwrap_or(v).trim()
}

pub fn parse_version(v: &str) -> Option<Vec<u32>> {
    let s = short_version(v).trim_start_matches(['v', 'V']);
    let nums: Vec<u32> = s.split('.').map_while(|p| p.trim().parse().ok()).collect();
    if nums.is_empty() { None } else { Some(nums) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(
            parse_duration("1T08:43:46"),
            Some(86400 + 8 * 3600 + 43 * 60 + 46)
        );
        assert_eq!(parse_duration("0T00:00:05"), Some(5));
        assert_eq!(format_duration(86400 + 5), "1d 00:00:05");
    }

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("16.0.0(release-tasmota)"),
            Some(vec![16, 0, 0])
        );
        assert_eq!(parse_version("v15.2.1"), Some(vec![15, 2, 1]));
        assert!(parse_version("8.5.1") < parse_version("16.0.0"));
    }

    #[test]
    fn lwt_then_state() {
        let mut s = Store::default();
        let out = s.handle("tele/credenza/LWT", "Online");
        assert!(out.iter().any(
            |a| matches!(a, Action::Publish { topic, .. } if topic == "cmnd/credenza/STATUS")
        ));
        s.handle(
            "tele/credenza/STATE",
            r#"{"UptimeSec":100,"POWER1":"ON","POWER2":"OFF","Wifi":{"AP":1,"RSSI":96,"LinkCount":4}}"#,
        );
        let d = &s.devices["credenza"];
        assert_eq!(d.power, vec![Some(true), Some(false)]);
        assert_eq!(d.rssi, Some(96));
        s.handle("stat/credenza/RESULT", r#"{"POWER2":"ON"}"#);
        assert_eq!(s.devices["credenza"].power[1], Some(true));
    }

    fn status_fwr(version: &str) -> String {
        format!(r#"{{"StatusFWR":{{"Version":"{version}","Core":"3_1_3"}}}}"#)
    }

    fn sent(out: &[Action], cmd: &str) -> bool {
        out.iter()
            .any(|a| matches!(a, Action::Publish { topic, .. } if topic.ends_with(cmd)))
    }

    fn long_ago(d: Duration) -> Instant {
        Instant::now().checked_sub(d).unwrap()
    }

    #[test]
    fn stale_retained_online_is_not_online() {
        let mut s = Store::default();
        // Unplugged for days: the broker still has "Online" and a retained POWER.
        assert!(sent(
            &s.handle_message("tele/plug/LWT", "Online", true),
            "/STATUS"
        ));
        s.handle_message("stat/plug/POWER", "ON", true);
        let d = &s.devices["plug"];
        assert!(!d.is_online());
        assert_eq!(d.reachable(), None, "still waiting for the answer");
        assert!(s.command("plug", "POWER1", "TOGGLE").is_none());
        assert!(s.start_upgrade("plug", &Default::default()).is_err());

        s.devices.get_mut("plug").unwrap().asked = Some(long_ago(NO_ANSWER_AFTER * 2));
        let d = &s.devices["plug"];
        assert!(d.no_answer());
        assert_eq!(d.reachable(), Some(false));
        assert_eq!(d.offline_label(), Some("No answer"));

        // Plugged in: the device sends LWT Online again and answers.
        assert!(sent(&s.handle("tele/plug/LWT", "Online"), "/STATUS"));
        s.handle("stat/plug/STATUS0", &status_fwr("13.3.0(tasmota32)"));
        assert!(s.devices["plug"].is_online());
        assert!(s.command("plug", "POWER1", "TOGGLE").is_some());
    }

    #[test]
    fn every_lwt_online_asks_again() {
        let mut s = Store::default();
        s.handle_message("tele/plug/LWT", "Online", true);
        s.handle("tele/plug/STATE", r#"{"Wifi":{"RSSI":60}}"#);
        assert!(s.devices["plug"].is_online());
        // No Offline in between (a stale "Online" before): still a new full status query.
        let out = s.handle("tele/plug/LWT", "Online");
        assert!(sent(&out, "/STATUS"));
        assert!(
            !s.devices["plug"].is_online(),
            "online again only after it answers"
        );
    }

    #[test]
    fn discovery_without_lwt_is_asked() {
        let mut s = Store::default();
        let out = s.handle_message(
            "tasmota/discovery/AABB/config",
            r#"{"t":"lamp","sw":"15.0.1"}"#,
            true,
        );
        assert!(sent(&out, "/STATUS"));
        assert_eq!(s.devices["lamp"].reachable(), None);
    }

    #[test]
    fn esp32_upgrade_through_safeboot() {
        let mut s = Store::default();
        s.handle("tele/a8t/LWT", "Online");
        s.handle("stat/a8t/STATUS0", &status_fwr("13.3.0(tasmota32)"));
        assert!(s.start_upgrade("a8t", &Default::default()).is_ok());
        s.handle("tele/a8t/LWT", "Offline");
        s.handle("tele/a8t/LWT", "Online");
        s.handle("stat/a8t/STATUS0", &status_fwr("13.3.0(safeboot)"));
        let u = s.devices["a8t"].upgrade.as_ref().unwrap();
        assert!(u.is_active(), "safeboot is not the end: {:?}", u.phase);
        s.handle("tele/a8t/LWT", "Offline");
        s.handle("tele/a8t/LWT", "Online");
        s.handle("stat/a8t/STATUS0", &status_fwr("16.0.0(tasmota32)"));
        let u = s.devices["a8t"].upgrade.as_ref().unwrap();
        assert!(
            matches!(&u.phase, UpgradePhase::Done(m) if m == "Upgraded 13.3.0 → 16.0.0"),
            "{:?}",
            u.phase
        );
    }

    #[test]
    fn same_version_is_not_shown_as_upgrade() {
        let mut s = Store::default();
        s.handle("tele/a8t/LWT", "Online");
        s.handle("stat/a8t/STATUS0", &status_fwr("16.0.0(tasmota32)"));
        s.start_upgrade("a8t", &Default::default()).unwrap();
        s.handle("tele/a8t/LWT", "Offline");
        s.handle("tele/a8t/LWT", "Online");
        s.handle("stat/a8t/STATUS0", &status_fwr("16.0.0(release-tasmota32)"));
        let u = s.devices["a8t"].upgrade.as_ref().unwrap();
        assert!(
            matches!(&u.phase, UpgradePhase::Done(m) if m == "Restarted, still 16.0.0"),
            "{:?}",
            u.phase
        );
    }

    #[test]
    fn failed_upgrade_clears_when_device_comes_back() {
        let mut s = Store::default();
        s.handle("tele/a8t/LWT", "Online");
        s.handle("stat/a8t/STATUS0", &status_fwr("13.3.0(tasmota32)"));
        s.start_upgrade("a8t", &Default::default()).unwrap();
        s.devices
            .get_mut("a8t")
            .unwrap()
            .upgrade
            .as_mut()
            .unwrap()
            .started = long_ago(UPGRADE_TIMEOUT * 2);
        s.check_timeouts();
        assert!(matches!(
            s.devices["a8t"].upgrade.as_ref().unwrap().phase,
            UpgradePhase::Failed(_)
        ));
        // Comes back: the alarm becomes a note.
        s.handle("tele/a8t/LWT", "Online");
        s.handle("stat/a8t/STATUS0", &status_fwr("13.3.0(tasmota32)"));
        assert!(matches!(
            s.devices["a8t"].upgrade.as_ref().unwrap().phase,
            UpgradePhase::Note(_)
        ));
        // Upgraded later (another app): shown as done.
        s.handle("tele/a8t/LWT", "Online");
        s.handle("stat/a8t/STATUS0", &status_fwr("16.0.0(tasmota32)"));
        let u = s.devices["a8t"].upgrade.as_ref().unwrap();
        assert!(
            matches!(&u.phase, UpgradePhase::Done(m) if m == "Upgraded 13.3.0 → 16.0.0"),
            "{:?}",
            u.phase
        );
    }

    #[test]
    fn telemetry_summary() {
        let mut d = Device::new("x", DEFAULT_FULLTOPIC, default_prefixes());
        d.sensor = serde_json::from_str(r#"{"ENERGY":{"Total":12.3,"Power":45}}"#).ok();
        assert_eq!(d.telemetry().as_deref(), Some("45 W"));
        d.sensor =
            serde_json::from_str(r#"{"AM2301":{"Temperature":21.5,"Humidity":40},"TempUnit":"C"}"#)
                .ok();
        assert_eq!(d.telemetry().as_deref(), Some("21.5 °C  40 %"));
    }
}
