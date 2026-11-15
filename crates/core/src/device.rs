//! Device model and parsing of Tasmota MQTT messages.
//!
//! Parsing is deliberately tolerant: fields may be missing or named differently on
//! older firmware, and every value is optional.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::Value;

pub const DEFAULT_FULLTOPIC: &str = "%prefix%/%topic%/";
pub const UPGRADE_TIMEOUT: Duration = Duration::from_secs(8 * 60);

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
}

#[derive(Debug, Clone)]
pub struct Upgrade {
    pub started: Instant,
    pub from: Option<String>,
    pub phase: UpgradePhase,
}

impl Upgrade {
    pub fn is_active(&self) -> bool {
        matches!(self.phase, UpgradePhase::Sent | UpgradePhase::Progress(_) | UpgradePhase::Restarting)
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

    pub online: Option<bool>,
    pub queried: bool,

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
        let mut s = self.full_topic.replace("%prefix%", prefix).replace("%topic%", &self.topic);
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

    pub fn is_online(&self) -> bool {
        self.online == Some(true)
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
            if self.is_online() { secs + at.elapsed().as_secs() } else { secs }
        })
    }

    pub fn fallback_topic(&self) -> Option<String> {
        self.mqtt_client.as_ref().map(|c| format!("{}/{}_fb/", self.prefixes[0], c))
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
        if parts.is_empty() { None } else { Some(parts.join("  ")) }
    }

    fn set_online(&mut self, online: bool) {
        self.online = Some(online);
        if !online {
            self.queried = false;
            if let Some(u) = &mut self.upgrade {
                if u.is_active() {
                    u.phase = UpgradePhase::Restarting;
                }
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
            let Some(rest) = ku.strip_prefix("POWER") else { continue };
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
        } else if let Some(s) = v.get("Uptime").and_then(Value::as_str).and_then(parse_duration) {
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
            if let Some(u) = s.get("Uptime").and_then(Value::as_str).and_then(parse_duration) {
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
        if let Some(gt) = v.get("GroupTopic1").or(v.get("GroupTopic")).and_then(Value::as_str) {
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
        // Some firmware answers Status commands with RESULT.
        self.parse_status(v);
    }

    fn check_upgrade_finished(&mut self) {
        let version = self.version.clone().unwrap_or_default();
        let Some(u) = &mut self.upgrade else { return };
        if !matches!(u.phase, UpgradePhase::Restarting) {
            return;
        }
        if version.to_ascii_lowercase().contains("minimal") {
            u.phase = UpgradePhase::Progress("Minimal firmware running, finishing upgrade…".into());
            return;
        }
        let from = u.from.clone().unwrap_or_else(|| "?".into());
        u.phase = if from == version {
            UpgradePhase::Done(format!("Restarted, still {}", short_version(&version)))
        } else {
            UpgradePhase::Done(format!("{} → {}", short_version(&from), short_version(&version)))
        };
    }
}

#[derive(Default)]
pub struct Store {
    pub devices: BTreeMap<String, Device>,
    /// SSId1 / SSId2 (assumed identical on all devices).
    pub ssids: [Option<String>; 2],
}

impl Store {
    pub fn handle(&mut self, topic: &str, payload: &str) -> Vec<Action> {
        let mut out = Vec::new();

        if let Some(rest) = topic.strip_prefix("tasmota/discovery/") {
            if rest.ends_with("/config") {
                self.handle_discovery(payload, &mut out);
            }
            return out;
        }

        let mut hit: Option<(String, bool, String)> = None;
        for (key, d) in &self.devices {
            if let Some(s) = topic.strip_prefix(d.tele_prefix.as_str()).filter(|s| !s.contains('/')) {
                hit = Some((key.clone(), true, s.to_string()));
                break;
            }
            if let Some(s) = topic.strip_prefix(d.stat_prefix.as_str()).filter(|s| !s.contains('/')) {
                hit = Some((key.clone(), false, s.to_string()));
                break;
            }
        }

        // Fallback discovery from LWT for devices without native discovery.
        if hit.is_none() {
            let parts: Vec<&str> = topic.split('/').collect();
            let lwt = payload.eq_ignore_ascii_case("online") || payload.eq_ignore_ascii_case("offline");
            if parts.len() == 3 && parts[0] == "tele" && parts[2] == "LWT" && lwt && !parts[1].is_empty() {
                let d = Device::new(parts[1], DEFAULT_FULLTOPIC, default_prefixes());
                self.devices.insert(parts[1].to_string(), d);
                hit = Some((parts[1].to_string(), true, "LWT".into()));
            }
        }

        let Some((key, is_tele, suffix)) = hit else { return out };
        let suffix = suffix.to_ascii_uppercase();
        let json: Option<Value> = serde_json::from_str::<Value>(payload).ok().filter(Value::is_object);

        let Some(d) = self.devices.get_mut(&key) else { return out };
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

        if d.is_online() && !d.queried {
            d.queried = true;
            self.query(&key, &mut out);
        }
        out
    }

    fn handle_discovery(&mut self, payload: &str, out: &mut Vec<Action>) {
        let Ok(v) = serde_json::from_str::<Value>(payload) else { return };
        let Some(t) = v.get("t").and_then(Value::as_str).filter(|t| !t.is_empty()) else { return };
        let ft = v.get("ft").and_then(Value::as_str).unwrap_or(DEFAULT_FULLTOPIC).to_string();
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
        if let Some(f) = v.get("fn").and_then(Value::as_array).and_then(|a| a.iter().find_map(Value::as_str)) {
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
    }

    fn query(&self, key: &str, out: &mut Vec<Action>) {
        let Some(d) = self.devices.get(key) else { return };
        let mut send = |cmd: &str, payload: &str| {
            out.push(Action::Publish { topic: d.cmnd(cmd), payload: payload.into() })
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

    /// Asks every online device for a full status again.
    pub fn refresh_all(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        let keys: Vec<String> =
            self.devices.iter().filter(|(_, d)| d.is_online()).map(|(k, _)| k.clone()).collect();
        for k in keys {
            self.query(&k, &mut out);
        }
        out
    }

    pub fn command(&self, key: &str, cmd: &str, payload: &str) -> Option<Action> {
        self.devices
            .get(key)
            .map(|d| Action::Publish { topic: d.cmnd(cmd), payload: payload.into() })
    }

    pub fn start_upgrade(&mut self, key: &str) -> Option<Action> {
        let d = self.devices.get_mut(key)?;
        d.upgrade = Some(Upgrade {
            started: Instant::now(),
            from: d.version.clone(),
            phase: UpgradePhase::Sent,
        });
        Some(Action::Publish { topic: d.cmnd("UPGRADE"), payload: "1".into() })
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
    let hex: String = m.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_uppercase();
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
    if d > 0 { format!("{d}d {h:02}:{m:02}:{s:02}") } else { format!("{h:02}:{m:02}:{s:02}") }
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
        assert_eq!(parse_duration("1T08:43:46"), Some(86400 + 8 * 3600 + 43 * 60 + 46));
        assert_eq!(parse_duration("0T00:00:05"), Some(5));
        assert_eq!(format_duration(86400 + 5), "1d 00:00:05");
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("16.0.0(release-tasmota)"), Some(vec![16, 0, 0]));
        assert_eq!(parse_version("v15.2.1"), Some(vec![15, 2, 1]));
        assert!(parse_version("8.5.1") < parse_version("16.0.0"));
    }

    #[test]
    fn lwt_then_state() {
        let mut s = Store::default();
        let out = s.handle("tele/credenza/LWT", "Online");
        assert!(out.iter().any(|a| matches!(a, Action::Publish { topic, .. } if topic == "cmnd/credenza/STATUS")));
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

    #[test]
    fn telemetry_summary() {
        let mut d = Device::new("x", DEFAULT_FULLTOPIC, default_prefixes());
        d.sensor = serde_json::from_str(r#"{"ENERGY":{"Total":12.3,"Power":45}}"#).ok();
        assert_eq!(d.telemetry().as_deref(), Some("45 W"));
        d.sensor = serde_json::from_str(r#"{"AM2301":{"Temperature":21.5,"Humidity":40},"TempUnit":"C"}"#).ok();
        assert_eq!(d.telemetry().as_deref(), Some("21.5 °C  40 %"));
    }
}
