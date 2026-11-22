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
/// The status query is sent again after this long without an answer, up to QUERY_TRIES times.
pub const QUERY_RETRY: Duration = Duration::from_secs(5);
pub const QUERY_TRIES: u8 = 3;
/// Tasmota acknowledges "Upgrade 1" at once; without that, the commands are sent again
/// after this long, and the step fails after twice this long.
pub const UPGRADE_ACK: Duration = Duration::from_secs(15);
const RECONNECTED: &str = "Reconnected during the download, waiting for the restart…";

fn default_prefixes() -> [String; 3] {
    ["cmnd".into(), "stat".into(), "tele".into()]
}

#[derive(Debug, Clone)]
pub enum Action {
    Publish {
        topic: String,
        payload: String,
    },
    Subscribe(String),
    /// Deletes a retained message on the broker (empty retained publish).
    ClearRetained(String),
    /// A command to an HTTP-only device (`http_dev`): run it, then give the answer to
    /// [`Store::handle_http`].
    Http {
        key: String,
        host: String,
        command: String,
        password: Option<String>,
    },
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
    /// The commands that started it (OtaUrl, Upgrade 1): sent again once when the device
    /// does not acknowledge them (it can miss commands right after it reconnects).
    pub commands: Vec<Action>,
    pub resent: bool,
    /// Started for a group of devices: once a step of the upgrade path is done, the next one
    /// starts by itself (see [`Store::continue_upgrades`]).
    pub auto: bool,
}

/// Why an upgrade did not start.
#[derive(Debug, Clone, PartialEq)]
pub enum StartError {
    Unknown,
    Offline,
    /// Already upgrading this device.
    Running,
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

/// How long a device that just came online keeps its star.
pub const NEW_FOR: Duration = Duration::from_secs(5 * 60);

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
    /// A STATUS answer came since the last LWT.
    pub status_seen: bool,
    /// When the full status was last asked, and how many times since the last LWT.
    pub queried_at: Option<Instant>,
    pub query_tries: u8,

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
    /// Dimmable / CCT / RGB light, when the device reports one.
    pub light: Option<crate::light::Light>,
    pub upgrade: Option<Upgrade>,
    /// `StatusLOG.SetOption` registers (see `setoption::decode`); empty until reported.
    pub set_options: Vec<String>,
    /// This device's SSId1 / SSId2 (`StatusLOG.SSId`, or the answer to `SSId<n>`).
    pub ssids: [Option<String>; 2],
    /// Topics the broker keeps a retained message on for this device (LWT, discovery…):
    /// what [`Store::forget`] clears so a removed device does not come back.
    pub retained_topics: std::collections::BTreeSet<String>,
    /// Reached over HTTP only (no MQTT): its address (`ip` or `ip:port`). See `http_dev`.
    pub http_host: Option<String>,
    /// Its web password, when it has one (memory only, never saved).
    pub http_password: Option<String>,
    /// It answered that it wants a web password.
    pub http_locked: bool,
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
            .or(self.http_host.as_deref())
            .unwrap_or(&self.topic)
    }

    pub fn web_url(&self) -> Option<String> {
        if let Some(h) = &self.http_host {
            return Some(format!("http://{h}/"));
        }
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

    /// It started less than [`NEW_FOR`] ago (plugged in or restarted): shown with a star.
    /// Judged only by the uptime it reports: an LWT "Online" also comes when a device merely
    /// reconnects (Wi-Fi drop, broker restart), and the broker replays old ones at connect.
    pub fn is_new(&self) -> bool {
        self.reachable() == Some(true)
            && self.current_uptime().is_some_and(|u| u < NEW_FOR.as_secs())
    }

    /// LWT received. Every LWT starts over: the device is asked for its full status again
    /// (when it says Online) and counts as online only once it answers.
    fn set_online(&mut self, online: bool) {
        self.online = Some(online);
        self.queried = false;
        self.answered = false;
        self.asked = None;
        self.status_seen = false;
        self.query_tries = 0;
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
        self.parse_light(v);
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
        if v.as_object().is_some_and(|o| o.contains_key("StatusMEM")) {
            self.status_seen = true;
        }
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
            // Restarted during the upgrade only if it has been up for less time than the
            // upgrade has been running (a device configured just before can have a small
            // uptime without restarting for the upgrade).
            let booted = match (&self.upgrade, self.current_uptime()) {
                (Some(u), Some(up)) => up < u.started.elapsed().as_secs() + 5,
                _ => false,
            };
            self.check_upgrade_finished(booted);
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
        if let Some(list) = v
            .get("StatusLOG")
            .and_then(|s| s.get("SSId"))
            .and_then(Value::as_array)
        {
            for (i, slot) in self.ssids.iter_mut().enumerate() {
                *slot = list
                    .get(i)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
            }
        }
        if let Some(regs) = v
            .get("StatusLOG")
            .and_then(|s| s.get("SetOption"))
            .and_then(Value::as_array)
        {
            self.set_options = regs
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
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
        // Sent at every boot: also tells when an upgrade step has finished, even if the
        // status query got lost.
        if i1.get("Version").is_some() {
            // INFO1 is sent at boot only: the device did restart.
            self.check_upgrade_finished(true);
        }
        set_str(&mut self.group_topic, i1, "GroupTopic");
        let i2 = get("Info2");
        set_str(&mut self.hostname, i2, "Hostname");
        set_str(&mut self.ip, i2, "IPAddress");
        let i3 = get("Info3");
        set_str(&mut self.restart_reason, i3, "RestartReason");
        set_i64(&mut self.boot_count, i3, "BootCount");
    }

    fn parse_light(&mut self, v: &Value) {
        let mut l = self.light.clone().unwrap_or_default();
        if l.update(v) {
            self.light = Some(l);
        }
    }

    fn parse_result(&mut self, v: &Value, ssids: &mut [Option<String>; 2]) {
        self.parse_power_keys(v);
        self.parse_light(v);
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
                self.ssids[i] = Some(s.to_string()).filter(|s| !s.is_empty());
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
        // Answer to "SetOption<n> <value>".
        if let Some(o) = v.as_object() {
            for (k, val) in o {
                if let Some((n, value)) = crate::setoption::parse_answer(k, val) {
                    crate::setoption::apply(&mut self.set_options, n, value);
                }
            }
        }
        // Some firmware answers Status commands with RESULT.
        self.parse_status(v);
    }

    /// `booted`: the device has just restarted (INFO1, or a small uptime). Tasmota 6.x
    /// drops and restores its MQTT connection while it downloads, before it restarts: a
    /// status with the old version and a long uptime means "still downloading".
    fn check_upgrade_finished(&mut self, booted: bool) {
        let version = self.version.clone().unwrap_or_default();
        let Some(u) = &mut self.upgrade else { return };
        let lower = version.to_ascii_lowercase();
        let from = u.from.clone().unwrap_or_else(|| "?".into());
        let changed = short_version(&from) != short_version(&version);
        let intermediate = lower.contains("safeboot") || lower.contains("minimal");
        if changed
            && !intermediate
            && matches!(u.phase, UpgradePhase::Sent | UpgradePhase::Progress(_))
        {
            // Back with another version without an LWT "Offline" in between: Tasmota 7 and
            // later disconnect cleanly when they restart, so the broker sends no LWT.
            u.phase = UpgradePhase::Restarting;
        }
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
                } else if !booted {
                    UpgradePhase::Progress(RECONNECTED.into())
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
                } else if !booted {
                    UpgradePhase::Progress(RECONNECTED.into())
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
    /// Recent MQTT messages of each device, for the console.
    pub console: crate::console::Log,
    /// Custom FullTopics to find devices by their LWT (see `topics`).
    pub topics: crate::topics::Topics,
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

        // Another app (or the web server) removed an offline device: its retained messages
        // are cleared with an empty payload, which reaches us too. Drop it here as well.
        if payload.is_empty() && !retained && self.forgotten_elsewhere(topic) {
            return out;
        }

        if let Some(rest) = topic.strip_prefix("tasmota/discovery/") {
            if rest.ends_with("/config")
                && let Some(key) = self.handle_discovery(payload, &mut out)
            {
                if retained && let Some(d) = self.devices.get_mut(&key) {
                    d.retained_topics.insert(topic.to_string());
                    // Tasmota also retains its sensors message next to the config.
                    let base = topic.trim_end_matches("config");
                    d.retained_topics.insert(format!("{base}sensors"));
                }
                self.query_if_needed(&key, &mut out);
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

        // Custom FullTopics (patterns and device entries): the LWT tells the real FullTopic.
        if hit.is_none()
            && (payload.eq_ignore_ascii_case("online") || payload.eq_ignore_ascii_case("offline"))
            && let Some((key, ft)) = self.topics.match_lwt(topic)
        {
            let d = Device::new(&key, &ft, default_prefixes());
            out.push(Action::Subscribe(format!("{}+", d.tele_prefix)));
            out.push(Action::Subscribe(format!("{}+", d.stat_prefix)));
            self.devices.insert(key.clone(), d);
            hit = Some((key, true, "LWT".into()));
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

        self.console.push(&key, false, retained, topic, payload);
        let Some(d) = self.devices.get_mut(&key) else {
            return out;
        };
        if retained && !payload.is_empty() {
            d.retained_topics.insert(topic.to_string());
        }
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
            (false, s) if s.starts_with("POWER") && json.is_none() => {
                let idx = s[5..].parse::<usize>().unwrap_or(1).saturating_sub(1);
                d.set_power(idx, payload.trim());
            }
            // SetOption4 1: answers on stat/<topic>/<COMMAND> instead of RESULT.
            (false, _) => {
                if let Some(v) = &json {
                    d.parse_result(v, &mut self.ssids)
                }
            }
            _ => {}
        }

        self.query_if_needed(&key, &mut out);
        out
    }

    /// An empty message on a device's discovery config or LWT: it was removed elsewhere.
    /// Only devices that are not answering are dropped (a live device never sends these).
    fn forgotten_elsewhere(&mut self, topic: &str) -> bool {
        let key = if let Some(mac) = topic
            .strip_prefix("tasmota/discovery/")
            .and_then(|r| r.strip_suffix("/config"))
        {
            let mac = format_mac(mac);
            self.devices
                .iter()
                .find(|(_, d)| d.mac.as_deref() == Some(mac.as_str()))
                .map(|(k, _)| k.clone())
        } else if topic.ends_with("/LWT") {
            self.devices
                .iter()
                .find(|(_, d)| {
                    topic
                        .strip_prefix(d.tele_prefix.as_str())
                        .is_some_and(|s| s == "LWT")
                })
                .map(|(k, _)| k.clone())
        } else {
            None
        };
        let Some(key) = key else { return false };
        if self.devices[&key].reachable() == Some(true) {
            return false;
        }
        self.devices.remove(&key);
        self.console.clear(&key);
        true
    }

    /// Removes a device that is not reachable, and clears the retained messages the broker
    /// keeps for it, or it would be listed again on the next connection. A device that comes
    /// back online announces itself again. `None`: unknown device or still reachable.
    pub fn forget(&mut self, key: &str) -> Option<Vec<Action>> {
        let d = self.devices.get(key)?;
        if d.reachable() != Some(false) && d.http_host.is_none() {
            return None;
        }
        let d = self.devices.remove(key)?;
        self.console.clear(key);
        Some(
            d.retained_topics
                .into_iter()
                .map(Action::ClearRetained)
                .collect(),
        )
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
        d.queried_at = Some(Instant::now());
        d.query_tries = 1;
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
        if d.http_host.is_some() {
            out.extend(http_action(d, "Status 0"));
            return;
        }
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

    /// Records the commands among `actions` in the console of the device they go to.
    pub fn log_sent(&mut self, actions: &[Action]) {
        for a in actions {
            if let Action::Http { key, host, command, .. } = a {
                let (cmd, payload) = command.split_once(' ').unwrap_or((command, ""));
                let topic = format!("http://{host}/cm {cmd}");
                let masked = crate::console::mask(cmd, payload);
                self.console.push(key, true, false, &topic, &masked);
                continue;
            }
            let Action::Publish { topic, payload } = a else {
                continue;
            };
            let key = self
                .devices
                .iter()
                .find(|(_, d)| {
                    topic
                        .strip_prefix(d.cmnd_prefix.as_str())
                        .is_some_and(|c| !c.contains('/'))
                })
                .map(|(k, _)| k.clone());
            if let Some(k) = key {
                self.console.push(&k, true, false, topic, payload);
            }
        }
    }

    /// A state-changing command. None when the device is not online (offline, not answering
    /// or not answered yet): commands only go to devices that just proved they are there.
    pub fn command(&self, key: &str, cmd: &str, payload: &str) -> Option<Action> {
        self.devices
            .get(key)
            .filter(|d| d.is_online())
            .and_then(|d| {
                if d.http_host.is_some() {
                    http_action(d, format!("{cmd} {payload}").trim())
                } else {
                    Some(Action::Publish {
                        topic: d.cmnd(cmd),
                        payload: payload.into(),
                    })
                }
            })
    }

    /// Adds (or updates) an HTTP-only device found by a search: `status` is its `Status 0`
    /// answer, None when it wants a web password. Returns its key. A device already known
    /// over MQTT (same MAC or address) is not added: None.
    pub fn add_http(
        &mut self,
        host: &str,
        status: Option<&Value>,
        password: Option<&str>,
    ) -> Option<String> {
        let ip = host.split(':').next().unwrap_or(host);
        let mac = status
            .and_then(|v| v.get("StatusNET"))
            .and_then(|n| n.get("Mac"))
            .and_then(Value::as_str)
            .map(format_mac);
        let on_mqtt = self.devices.values().any(|d| {
            d.http_host.is_none()
                && (d.ip.as_deref() == Some(ip) && !host.contains(':')
                    || mac.is_some() && d.mac == mac)
        });
        if on_mqtt {
            return None;
        }
        let key = crate::http_dev::key(host);
        let d = self.devices.entry(key.clone()).or_insert_with(|| {
            let mut d = Device::new(&key, DEFAULT_FULLTOPIC, default_prefixes());
            d.http_host = Some(host.to_string());
            if !host.contains(':') {
                d.ip = Some(host.to_string());
            }
            d
        });
        if let Some(p) = password.filter(|p| !p.is_empty()) {
            d.http_password = Some(p.to_string());
        }
        match status {
            Some(v) => {
                d.http_locked = false;
                d.online = Some(true);
                d.answered = true;
                d.parse_status(v);
                d.status_seen = true;
                if host.contains(':') {
                    // Tests on one machine: keep the address with its port.
                    d.ip = Some(host.to_string());
                }
            }
            None => {
                d.http_locked = true;
                d.online = Some(true);
                d.answered = true;
            }
        }
        Some(key)
    }

    /// An HTTP-only device saved earlier, not asked yet (see [`Store::poll_http`]).
    pub fn add_http_saved(&mut self, host: &str) {
        let key = crate::http_dev::key(host);
        self.devices.entry(key.clone()).or_insert_with(|| {
            let mut d = Device::new(&key, DEFAULT_FULLTOPIC, default_prefixes());
            d.http_host = Some(host.to_string());
            d.ip = Some(host.to_string());
            d
        });
    }

    /// The answer to an [`Action::Http`]. Returns the device's address when it turned out
    /// not to be a Tasmota (it is then removed: drop it from the saved list too).
    pub fn handle_http(&mut self, key: &str, answer: &crate::http_dev::Answer) -> Option<String> {
        use crate::http_dev::Answer;
        if *answer == Answer::NotTasmota {
            let d = self.devices.remove(key)?;
            self.console.clear(key);
            return d.http_host;
        }
        let Some(d) = self.devices.get_mut(key) else {
            return None;
        };
        let topic = format!("http://{}/cm", d.http_host.clone().unwrap_or_default());
        match answer {
            Answer::Json(v) => {
                self.console.push(key, false, false, &topic, &v.to_string());
                d.online = Some(true);
                d.answered = true;
                d.http_locked = false;
                if v.get("StatusFWR").is_some() || v.get("StatusSTS").is_some() {
                    let port_host = d.http_host.clone().filter(|h| h.contains(':'));
                    d.parse_status(v);
                    d.status_seen = true;
                    if let Some(h) = port_host {
                        d.ip = Some(h);
                    }
                } else {
                    d.parse_result(v, &mut self.ssids);
                }
            }
            Answer::NeedsPassword => {
                d.http_locked = true;
                d.online = Some(true);
                d.answered = true;
            }
            Answer::Nothing | Answer::NotTasmota => {
                d.online = Some(false);
                d.answered = false;
            }
        }
        None
    }

    /// The subscriptions the custom topics need (at every connection).
    pub fn initial_actions(&self) -> Vec<Action> {
        self.topics
            .subscriptions()
            .into_iter()
            .map(Action::Subscribe)
            .collect()
    }

    /// HTTP-only devices that now also talk MQTT (same MAC or address): removed, since the
    /// MQTT entry takes over. Returns their addresses (to drop them from the saved list).
    pub fn merge_http(&mut self) -> Vec<String> {
        let gone: Vec<(String, String)> = self
            .devices
            .iter()
            .filter_map(|(k, d)| {
                let host = d.http_host.as_ref()?;
                let ip = host.split(':').next().unwrap_or(host);
                self.devices
                    .values()
                    .any(|m| {
                        m.http_host.is_none()
                            && m.is_online()
                            && (d.mac.is_some() && m.mac == d.mac
                                || !host.contains(':') && m.ip.as_deref() == Some(ip))
                    })
                    .then(|| (k.clone(), host.clone()))
            })
            .collect();
        for (k, _) in &gone {
            self.devices.remove(k);
            self.console.clear(k);
        }
        gone.into_iter().map(|(_, h)| h).collect()
    }

    /// The `Status 0` poll of every HTTP-only device (online state, relays, telemetry).
    pub fn poll_http(&self) -> Vec<Action> {
        self.devices
            .values()
            .filter(|d| d.http_host.is_some())
            .filter_map(|d| http_action(d, "Status 0"))
            .collect()
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

    /// Starts the next upgrade (one step of the path, or the final one). Devices upgrade on
    /// their own, several at once; only online devices, only when the plan allows it (free
    /// flash checked). `auto`: the following steps of the path start by themselves.
    pub fn start_upgrade(
        &mut self,
        key: &str,
        targets: &crate::ota::Targets,
        auto: bool,
    ) -> Result<Vec<Action>, StartError> {
        let d = self.devices.get(key).ok_or(StartError::Unknown)?;
        if !d.is_online() {
            return Err(StartError::Offline);
        }
        if d.upgrade.as_ref().is_some_and(|u| u.is_active()) {
            return Err(StartError::Running);
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
        let plan = crate::ota::plan_with(
            d.version.as_deref(),
            d.ota_url.as_deref(),
            crate::ota::esp32_build(d).as_deref(),
            targets.release.as_deref(),
            &crate::ota_policy::policy(key),
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
            commands: out.clone(),
            resent: false,
            auto,
        });
        Ok(out)
    }

    /// Group upgrades: a device whose path step is done, back with a fresh status, starts its
    /// next step (or the final upgrade). It stops after the final upgrade, on a failure, or
    /// when the plan can't go on (it is then shown as usual).
    pub fn continue_upgrades(&mut self, targets: &crate::ota::Targets) -> Vec<Action> {
        let ready: Vec<String> = self
            .devices
            .iter()
            .filter(|(_, d)| {
                d.is_online()
                    && d.status_seen
                    && d.upgrade.as_ref().is_some_and(|u| {
                        u.auto && u.target.is_some() && matches!(u.phase, UpgradePhase::Done(_))
                    })
            })
            .map(|(k, _)| k.clone())
            .collect();
        let mut out = Vec::new();
        for k in ready {
            let info = crate::ota::plan_info(&self.devices[&k], targets, &self.sizes);
            if !matches!(info.kind, "step" | "final") {
                if let Some(u) = self.devices.get_mut(&k).and_then(|d| d.upgrade.as_mut()) {
                    u.auto = false;
                }
                continue;
            }
            if !info.can_start {
                continue; // size or free flash still being checked: next round
            }
            if let Ok(a) = self.start_upgrade(&k, targets, true) {
                out.extend(a);
            }
        }
        out
    }

    /// Upgrade timeouts, and the status query asked again when it got no answer: Tasmota
    /// publishes its LWT "Online" before it subscribes to its command topic, so a query
    /// sent right away can be lost.
    pub fn check_timeouts(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        let again: Vec<String> = self
            .devices
            .iter_mut()
            .filter(|(_, d)| {
                d.online != Some(false)
                    && d.queried
                    && !d.status_seen
                    && d.query_tries < QUERY_TRIES
                    && d.queried_at.is_some_and(|t| t.elapsed() >= QUERY_RETRY)
            })
            .map(|(k, d)| {
                d.query_tries += 1;
                d.queried_at = Some(Instant::now());
                k.clone()
            })
            .collect();
        for k in again {
            self.query(&k, &mut out);
        }
        for d in self.devices.values_mut() {
            if let Some(u) = &mut d.upgrade
                && matches!(u.phase, UpgradePhase::Sent)
            {
                if !u.resent && u.started.elapsed() > UPGRADE_ACK {
                    u.resent = true;
                    out.extend(u.commands.iter().cloned());
                } else if u.resent && u.started.elapsed() > UPGRADE_ACK * 2 {
                    u.phase = UpgradePhase::Failed(
                        "The device did not answer the upgrade command (sent twice): nothing was \
                         installed. Check it is reachable, then try again."
                            .into(),
                    );
                }
            }
            if let Some(u) = &mut d.upgrade {
                if u.is_active() && u.started.elapsed() > UPGRADE_TIMEOUT {
                    u.phase = UpgradePhase::Failed("No answer after 8 minutes".into());
                }
            }
        }
        out
    }
}

fn http_action(d: &Device, command: &str) -> Option<Action> {
    Some(Action::Http {
        key: d.topic.clone(),
        host: d.http_host.clone()?,
        command: command.to_string(),
        password: d.http_password.clone(),
    })
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
    #[test]
    fn new_means_just_started_not_reconnected() {
        let mut s = Store::default();
        // A Wi-Fi drop: LWT Online again, but up for two hours. Not new.
        s.handle("tele/fan/LWT", "Online");
        s.handle("stat/fan/STATUS11", r#"{"StatusSTS":{"UptimeSec":7200}}"#);
        assert!(!s.devices["fan"].is_new());
        // Just booted: new, even when its LWT was a retained one.
        s.handle_message("tele/plug/LWT", "Online", true);
        s.handle("stat/plug/STATUS11", r#"{"StatusSTS":{"UptimeSec":40}}"#);
        assert!(s.devices["plug"].is_new());
        // An LWT alone (uptime not known yet) is not enough, and offline ends it.
        s.handle("tele/other/LWT", "Online");
        assert!(!s.devices["other"].is_new());
        s.handle("tele/plug/LWT", "Offline");
        assert!(!s.devices["plug"].is_new());
    }

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

    /// A full status answer of a device that has just booted.
    fn status_fwr(version: &str) -> String {
        status_up(version, "0T00:00:01")
    }

    fn status_up(version: &str, uptime: &str) -> String {
        format!(
            r#"{{"StatusPRM":{{"Uptime":"{uptime}"}},"StatusFWR":{{"Version":"{version}","Core":"3_1_3"}},"StatusMEM":{{"ProgramSize":500,"Free":400}}}}"#
        )
    }

    fn sent(out: &[Action], cmd: &str) -> bool {
        out.iter()
            .any(|a| matches!(a, Action::Publish { topic, .. } if topic.ends_with(cmd)))
    }

    fn long_ago(d: Duration) -> Instant {
        Instant::now().checked_sub(d).unwrap()
    }

    #[test]
    fn set_options_from_status_and_answers() {
        let mut s = Store::default();
        s.handle("tele/plug/LWT", "Online");
        s.handle(
            "stat/plug/STATUS3",
            r#"{"StatusLOG":{"SerialLog":2,"SetOption":["00008009","2805C80001000600003C5A0A190000000000","00000000","00000000"]}}"#,
        );
        assert_eq!(s.devices["plug"].set_options.len(), 4);
        s.handle("stat/plug/RESULT", r#"{"SetOption4":"ON"}"#);
        // SetOption4 1: the next answers arrive on stat/plug/<COMMAND>.
        s.handle("stat/plug/SETOPTION34", r#"{"SetOption34":100}"#);
        s.handle("stat/plug/POWER", r#"{"POWER":"ON"}"#);
        let d: std::collections::HashMap<u16, u32> =
            crate::setoption::decode(&s.devices["plug"].set_options)
                .into_iter()
                .collect();
        assert_eq!((d[&4], d[&34]), (1, 100));
        assert_eq!(s.devices["plug"].power, vec![Some(true)]);
        s.handle("stat/plug/POWER", "OFF");
        assert_eq!(s.devices["plug"].power, vec![Some(false)]);
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
        assert!(s.start_upgrade("plug", &Default::default(), false).is_err());

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
    fn custom_full_topic_from_lwt() {
        let mut s = Store {
            topics: crate::topics::Topics {
                patterns: vec!["headquarters/+/%prefix%/%topic%/".into()],
                devices: vec![],
            },
            ..Default::default()
        };
        assert_eq!(s.initial_actions().len(), 1);
        let a = s.handle("headquarters/room-3/tele/fan/LWT", "Online");
        let d = &s.devices["fan"];
        assert_eq!(d.cmnd("STATUS"), "headquarters/room-3/cmnd/fan/STATUS");
        assert!(a.iter().any(|x| matches!(x, Action::Subscribe(t) if t == "headquarters/room-3/stat/fan/+")));
        assert!(a.iter().any(|x| matches!(x, Action::Publish { topic, .. } if topic == "headquarters/room-3/cmnd/fan/STATUS")));
        s.handle("headquarters/room-3/stat/fan/STATUS0", &status_fwr("8.5.1(tasmota)"));
        assert!(s.devices["fan"].is_online());
    }

    #[test]
    fn removed_elsewhere_goes_away() {
        let mut s = Store::default();
        s.handle_message(
            "tasmota/discovery/AABBCCDDEEFF/config",
            r#"{"t":"old","ft":"%prefix%/%topic%/","dn":"Old","mac":"AABBCCDDEEFF"}"#,
            true,
        );
        s.handle_message("tele/old/LWT", "Offline", true);
        s.handle("tele/live/LWT", "Online");
        s.handle("stat/live/STATUS0", &status_fwr("15.0.0(tasmota)"));
        // The empty messages another app sends when it removes them.
        s.handle("tele/live/LWT", "");
        assert!(s.devices.contains_key("live"), "a live device stays");
        s.handle("tasmota/discovery/AABBCCDDEEFF/config", "");
        assert!(!s.devices.contains_key("old"));
    }

    #[test]
        fn forget_offline_clears_retained() {
        let mut s = Store::default();
        s.handle_message(
            "tasmota/discovery/AABBCCDDEEFF/config",
            r#"{"t":"old","ft":"%prefix%/%topic%/","dn":"Old"}"#,
            true,
        );
        s.handle_message("tele/old/LWT", "Offline", true);
        s.handle_message("stat/old/POWER", "ON", true);
        s.handle("tele/live/LWT", "Online");
        s.handle("stat/live/STATUS0", &status_fwr("15.0.0(tasmota)"));

        assert!(s.forget("live").is_none(), "online devices stay");
        assert!(s.forget("nope").is_none());
        let out = s.forget("old").expect("offline device removed");
        let mut cleared: Vec<&str> = out
            .iter()
            .map(|a| match a {
                Action::ClearRetained(t) => t.as_str(),
                _ => panic!("only clears"),
            })
            .collect();
        cleared.sort();
        assert_eq!(
            cleared,
            [
                "stat/old/POWER",
                "tasmota/discovery/AABBCCDDEEFF/config",
                "tasmota/discovery/AABBCCDDEEFF/sensors",
                "tele/old/LWT",
            ]
        );
        assert!(!s.devices.contains_key("old"));
        // The broker echoes the empty retained messages: they must not bring it back.
        s.handle("tele/old/LWT", "");
        s.handle("tasmota/discovery/AABBCCDDEEFF/config", "");
        assert!(!s.devices.contains_key("old"));
        // Back online: listed again.
        s.handle("tele/old/LWT", "Online");
        assert!(s.devices.contains_key("old"));
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
        assert!(s.start_upgrade("a8t", &Default::default(), false).is_ok());
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
        s.start_upgrade("a8t", &Default::default(), false).unwrap();
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
        s.start_upgrade("a8t", &Default::default(), false).unwrap();
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
    fn status_query_is_asked_again_when_lost() {
        let mut s = Store::default();
        assert!(sent(&s.handle("tele/sw/LWT", "Online"), "/STATUS"));
        // Lost (Tasmota was not subscribed yet): nothing comes back.
        assert!(s.check_timeouts().is_empty(), "not before QUERY_RETRY");
        s.devices.get_mut("sw").unwrap().queried_at = Some(long_ago(QUERY_RETRY * 2));
        assert!(sent(&s.check_timeouts(), "/STATUS"));
        s.handle("stat/sw/STATUS0", &status_fwr("7.2.0(lite)"));
        s.devices.get_mut("sw").unwrap().queried_at = Some(long_ago(QUERY_RETRY * 2));
        assert!(s.check_timeouts().is_empty(), "answered: no more queries");
    }

    #[test]
    fn reconnect_during_download_is_not_a_failure() {
        let mut s = Store::default();
        s.handle("tele/sw/LWT", "Online");
        s.handle("stat/sw/STATUS0", &status_up("6.7.1(basic)", "0T00:00:50"));
        s.devices.get_mut("sw").unwrap().upgrade = Some(Upgrade {
            started: Instant::now(),
            from: Some("6.7.1(basic)".into()),
            phase: UpgradePhase::Progress("Version 6.7.1 from …".into()),
            target: Some([7, 2, 0]),
            step: Some((1, 4)),
            commands: Vec::new(),
            resent: false,
            auto: false,
        });
        // Tasmota 6.x drops MQTT while it downloads and comes back, still on 6.7.1. Its
        // uptime is small (configured a minute ago) but longer than the upgrade has run.
        s.handle("tele/sw/LWT", "Offline");
        s.handle("tele/sw/LWT", "Online");
        s.handle("stat/sw/STATUS0", &status_up("6.7.1(basic)", "0T00:01:02"));
        let u = s.devices["sw"].upgrade.as_ref().unwrap();
        assert!(
            u.is_active() && !matches!(u.phase, UpgradePhase::Failed(_)),
            "{:?}",
            u.phase
        );
        // Then it restarts with the new firmware.
        s.handle("tele/sw/LWT", "Offline");
        s.handle("tele/sw/LWT", "Online");
        s.handle(
            "tele/sw/INFO1",
            r#"{"Module":"Sonoff Basic","Version":"7.2.0(lite)"}"#,
        );
        let u = s.devices["sw"].upgrade.as_ref().unwrap();
        assert!(
            matches!(&u.phase, UpgradePhase::Done(m) if m.starts_with("Upgraded 6.7.1 → 7.2.0")),
            "{:?}",
            u.phase
        );
        // A real restart on the old version is still a failure.
        let mut s = Store::default();
        s.handle("tele/sw/LWT", "Online");
        s.handle("stat/sw/STATUS0", &status_fwr("6.7.1(basic)"));
        s.devices.get_mut("sw").unwrap().upgrade = Some(Upgrade {
            started: Instant::now(),
            from: Some("6.7.1(basic)".into()),
            phase: UpgradePhase::Restarting,
            target: Some([7, 2, 0]),
            step: Some((1, 4)),
            commands: Vec::new(),
            resent: false,
            auto: false,
        });
        s.handle(
            "tele/sw/INFO1",
            r#"{"Module":"Sonoff Basic","Version":"6.7.1(basic)"}"#,
        );
        assert!(matches!(
            s.devices["sw"].upgrade.as_ref().unwrap().phase,
            UpgradePhase::Failed(_)
        ));
    }

    #[test]
    fn restart_without_lwt_offline_still_finishes() {
        let mut s = Store::default();
        s.handle("tele/sw/LWT", "Online");
        s.handle("stat/sw/STATUS0", &status_fwr("7.2.0(lite)"));
        s.devices.get_mut("sw").unwrap().upgrade = Some(Upgrade {
            started: Instant::now(),
            from: Some("7.2.0(lite)".into()),
            phase: UpgradePhase::Sent,
            target: Some([8, 5, 1]),
            step: Some((1, 3)),
            commands: Vec::new(),
            resent: false,
            auto: false,
        });
        s.handle("stat/sw/RESULT", r#"{"Upgrade":"Successful. Restarting"}"#);
        // Clean disconnect: no "Offline", the device is simply back with the new version.
        s.handle("tele/sw/LWT", "Online");
        s.handle(
            "tele/sw/INFO1",
            r#"{"Module":"Sonoff Basic","Version":"8.5.1(lite)"}"#,
        );
        let u = s.devices["sw"].upgrade.as_ref().unwrap();
        assert!(
            matches!(&u.phase, UpgradePhase::Done(m) if m.starts_with("Upgraded 7.2.0 → 8.5.1")),
            "{:?}",
            u.phase
        );
    }

    #[test]
    fn unanswered_upgrade_is_sent_again_then_fails() {
        let mut s = Store::default();
        s.handle("tele/sw/LWT", "Online");
        s.handle("stat/sw/STATUS0", &status_fwr("16.0.0(tasmota)"));
        let sent_first = s.start_upgrade("sw", &Default::default(), false).unwrap();
        s.devices
            .get_mut("sw")
            .unwrap()
            .upgrade
            .as_mut()
            .unwrap()
            .started = long_ago(UPGRADE_ACK + Duration::from_secs(1));
        let again = s.check_timeouts();
        assert_eq!(again.len(), sent_first.len(), "same commands once more");
        assert!(s.check_timeouts().is_empty(), "only once");
        s.devices
            .get_mut("sw")
            .unwrap()
            .upgrade
            .as_mut()
            .unwrap()
            .started = long_ago(UPGRADE_ACK * 3);
        s.check_timeouts();
        assert!(matches!(
            s.devices["sw"].upgrade.as_ref().unwrap().phase,
            UpgradePhase::Failed(_)
        ));
    }

    #[test]
    fn info1_finishes_an_upgrade_step() {
        let mut s = Store::default();
        s.handle("tele/sw/LWT", "Online");
        s.handle("stat/sw/STATUS0", &status_fwr("6.7.1(basic)"));
        s.devices.get_mut("sw").unwrap().upgrade = Some(Upgrade {
            started: Instant::now(),
            from: Some("6.7.1(basic)".into()),
            phase: UpgradePhase::Restarting,
            target: Some([7, 2, 0]),
            step: Some((1, 4)),
            commands: Vec::new(),
            resent: false,
            auto: false,
        });
        s.handle("tele/sw/LWT", "Online");
        // Only INFO1 arrives, the status query is lost.
        s.handle(
            "tele/sw/INFO1",
            r#"{"Module":"Sonoff Basic","Version":"7.2.0(lite)"}"#,
        );
        let u = s.devices["sw"].upgrade.as_ref().unwrap();
        assert!(
            matches!(&u.phase, UpgradePhase::Done(m) if m.starts_with("Upgraded 6.7.1 → 7.2.0")),
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
