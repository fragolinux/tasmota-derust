//! A running connection to the broker plus the device store, for front ends that poll
//! (web server, Android app). The desktop app drives the store itself.
//!
//! Read-only sessions only ever send read queries (Status 0, Module, SSId with an empty
//! payload). Every command that changes something on a device goes through [`Session::guard`]
//! and is refused when the session is read-only.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::RecvTimeoutError;
use serde::Serialize;
use serde_json::Value;

use crate::Notify;
use crate::config::BrokerConfig;
use crate::device::{self, Action, Device, StartError, Store, UpgradePhase};
use crate::github::{self, Latest};
use crate::mqtt::{Mqtt, MqttEvent};

struct Shared {
    store: Store,
    connected: bool,
    status: String,
}

pub struct Session {
    shared: Arc<Mutex<Shared>>,
    mqtt: Arc<Mqtt>,
    latest: Arc<Mutex<Latest>>,
    latest_dev: Arc<Mutex<Latest>>,
    app_latest: Arc<Mutex<Latest>>,
    read_only: AtomicBool,
    broker: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandError {
    ReadOnly,
    UnknownDevice,
    Offline,
    NoSuchRelay,
    BadAp,
    UpgradeRunning,
    OtherUpgradeRunning,
    UpgradeNotPossible,
    NotOfficialOtaUrl,
    NotALight,
}

impl CommandError {
    pub fn message(self) -> &'static str {
        match self {
            CommandError::ReadOnly => "Read-only mode is on: commands are disabled.",
            CommandError::UnknownDevice => "Unknown device.",
            CommandError::Offline => "The device is offline.",
            CommandError::NoSuchRelay => "No such relay.",
            CommandError::BadAp => "AP must be 1 or 2.",
            CommandError::UpgradeRunning => "An upgrade is already running on this device.",
            CommandError::OtherUpgradeRunning => "Another device is being upgraded: one at a time.",
            CommandError::UpgradeNotPossible => {
                "This upgrade can't start now: see the upgrade details."
            }
            CommandError::NotOfficialOtaUrl => {
                "The OtaUrl is not an official release or development URL."
            }
            CommandError::NotALight => "This device has no light that can do this.",
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CommandError {}

impl Session {
    /// Connects to the broker and starts the background threads. `notify` is called when
    /// something may have changed (it can be a no-op for front ends that poll).
    pub fn start(broker: &BrokerConfig, read_only: bool, notify: Notify) -> Session {
        let (mqtt, rx) = Mqtt::start(broker, notify.clone());
        let mqtt = Arc::new(mqtt);
        let shared = Arc::new(Mutex::new(Shared {
            store: Store::default(),
            connected: false,
            status: "Connecting…".into(),
        }));

        // MQTT events -> device store. Only the read queries produced by the store are sent here.
        // The thread ends when the MQTT connection is dropped.
        {
            let shared = shared.clone();
            let mqtt = Arc::downgrade(&mqtt);
            std::thread::Builder::new()
                .name("session".into())
                .spawn(move || {
                    loop {
                        match rx.recv_timeout(Duration::from_secs(1)) {
                            Ok(ev) => {
                                let actions = {
                                    let mut s = shared.lock().unwrap();
                                    match ev {
                                        MqttEvent::Connected => {
                                            s.connected = true;
                                            s.status = "Connected".into();
                                            Vec::new()
                                        }
                                        MqttEvent::Disconnected(e) => {
                                            s.connected = false;
                                            s.status = e;
                                            Vec::new()
                                        }
                                        MqttEvent::Message {
                                            topic,
                                            payload,
                                            retained,
                                        } => s.store.handle_message(&topic, &payload, retained),
                                    }
                                };
                                match mqtt.upgrade() {
                                    Some(m) => run_actions(&m, actions),
                                    None => break,
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => {
                                if mqtt.strong_count() == 0 {
                                    break;
                                }
                            }
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                        let actions = shared.lock().unwrap().store.check_timeouts();
                        if let Some(m) = mqtt.upgrade() {
                            run_actions(&m, actions);
                        }
                    }
                })
                .expect("spawn session thread");
        }

        Session {
            shared,
            mqtt,
            latest: github::fetch_latest(notify.clone()),
            latest_dev: github::fetch_latest_dev(notify.clone()),
            app_latest: github::fetch_app_latest(notify),
            read_only: AtomicBool::new(read_only),
            broker: format!("{}:{}", broker.host.trim(), broker.port),
        }
    }

    pub fn read_only(&self) -> bool {
        self.read_only.load(Ordering::SeqCst)
    }

    /// Turns read-only mode on or off while connected. Read-only refuses every command
    /// that changes something on a device (relays, Wi-Fi switch, upgrade).
    pub fn set_read_only(&self, on: bool) {
        self.read_only.store(on, Ordering::SeqCst);
    }

    /// Everything a front end needs to draw, in one consistent copy.
    pub fn snapshot(&self) -> Snapshot {
        let (latest, latest_error) = match &*self.latest.lock().unwrap() {
            Latest::Ok(v) => (Some(v.clone()), None),
            Latest::Err(e) => (None, Some(e.clone())),
            Latest::Loading => (None, None),
        };
        let targets = self.targets();
        let s = self.shared.lock().unwrap();
        let devices = s
            .store
            .devices
            .values()
            .map(|d| {
                let plan = crate::ota::plan_info(d, &targets, &s.store.sizes);
                let status = crate::ota::version_status(
                    d.version.as_deref(),
                    d.ota_url.as_deref(),
                    &targets,
                );
                DeviceInfo::new(d, status, plan)
            })
            .collect();
        Snapshot {
            app_version: env!("CARGO_PKG_VERSION"),
            app_update: github::app_update(&self.app_latest.lock().unwrap()),
            app_releases: github::APP_RELEASES,
            connected: s.connected,
            status: s.status.clone(),
            broker: self.broker.clone(),
            read_only: self.read_only(),
            latest,
            latest_dev: targets.development.clone(),
            latest_error,
            ssids: s.store.ssids.clone(),
            devices,
        }
    }

    /// Asks every online device for its full status. Read queries only (Status 0, Module,
    /// SSId), so this is allowed in read-only mode too.
    pub fn refresh(&self) {
        let actions = self.shared.lock().unwrap().store.refresh_all();
        run_actions(&self.mqtt, actions);
    }

    /// Checks that a state-changing command is allowed for this device.
    fn guard(&self, s: &Shared, topic: &str) -> Result<(), CommandError> {
        if self.read_only() {
            return Err(CommandError::ReadOnly);
        }
        match s.store.devices.get(topic) {
            None => Err(CommandError::UnknownDevice),
            Some(d) if !d.is_online() => Err(CommandError::Offline),
            Some(_) => Ok(()),
        }
    }

    /// Toggles relay `n` (1-based).
    pub fn toggle_power(&self, topic: &str, n: usize) -> Result<(), CommandError> {
        let action = {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            let relays = s
                .store
                .devices
                .get(topic)
                .map(|d| d.power.len())
                .unwrap_or(0);
            if n == 0 || n > relays {
                return Err(CommandError::NoSuchRelay);
            }
            s.store.command(topic, &format!("POWER{n}"), "TOGGLE")
        };
        run_actions(&self.mqtt, action.into_iter().collect());
        Ok(())
    }

    /// Restarts the device and makes it connect to SSId `n` (1 or 2).
    pub fn switch_ap(&self, topic: &str, n: u8) -> Result<(), CommandError> {
        if !(1..=2).contains(&n) {
            return Err(CommandError::BadAp);
        }
        let action = {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            s.store.command(topic, "AP", &n.to_string())
        };
        run_actions(&self.mqtt, action.into_iter().collect());
        Ok(())
    }

    /// Starts the next upgrade of one device: one step of the upgrade path for old firmware,
    /// or the upgrade to the latest release (see `ota`). One device at a time.
    pub fn upgrade(&self, topic: &str) -> Result<(), CommandError> {
        let targets = self.targets();
        let actions = {
            let mut s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            s.store
                .start_upgrade(topic, &targets)
                .map_err(|e| match e {
                    StartError::Unknown => CommandError::UnknownDevice,
                    StartError::Offline => CommandError::Offline,
                    StartError::Running => CommandError::UpgradeRunning,
                    StartError::OtherRunning(_) => CommandError::OtherUpgradeRunning,
                    StartError::NotPossible(_) => CommandError::UpgradeNotPossible,
                })?
        };
        run_actions(&self.mqtt, actions);
        Ok(())
    }

    /// Light brightness, 0..100 (`Dimmer`).
    pub fn set_dimmer(&self, topic: &str, dimmer: i64) -> Result<(), CommandError> {
        self.light_command(topic, |_| true, "DIMMER", dimmer.clamp(0, 100).to_string())
    }

    /// Colour of an RGB light: hue 0..359, saturation and brightness 0..100 (`HSBColor`).
    pub fn set_hsb(&self, topic: &str, hue: i64, sat: i64, bri: i64) -> Result<(), CommandError> {
        let v = format!(
            "{},{},{}",
            hue.rem_euclid(360),
            sat.clamp(0, 100),
            bri.clamp(0, 100)
        );
        self.light_command(topic, |l| l.has_color(), "HSBCOLOR", v)
    }

    /// White colour temperature, 153 (cold) ..500 (warm) mireds (`CT`).
    pub fn set_ct(&self, topic: &str, ct: i64) -> Result<(), CommandError> {
        let v = ct
            .clamp(crate::light::CT_MIN, crate::light::CT_MAX)
            .to_string();
        self.light_command(topic, |l| l.has_ct(), "CT", v)
    }

    /// Brightness of the white channels of an RGBW/RGBCCT light, 0..100 (`White`).
    pub fn set_white(&self, topic: &str, white: i64) -> Result<(), CommandError> {
        self.light_command(
            topic,
            |l| l.has_white(),
            "WHITE",
            white.clamp(0, 100).to_string(),
        )
    }

    fn light_command(
        &self,
        topic: &str,
        supports: impl Fn(&crate::light::Light) -> bool,
        cmd: &str,
        payload: String,
    ) -> Result<(), CommandError> {
        let action = {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            let light = s.store.devices.get(topic).and_then(|d| d.light.as_ref());
            if !light.is_some_and(supports) {
                return Err(CommandError::NotALight);
            }
            s.store.command(topic, cmd, &payload)
        };
        run_actions(&self.mqtt, action.into_iter().collect());
        Ok(())
    }

    /// Points the device's OtaUrl at the release or the development builds (same image name).
    /// Nothing is installed: the upgrade button does that.
    pub fn set_channel(&self, topic: &str, development: bool) -> Result<(), CommandError> {
        let action = {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            let d = s
                .store
                .devices
                .get(topic)
                .ok_or(CommandError::UnknownDevice)?;
            let url = d
                .ota_url
                .as_deref()
                .and_then(|u| crate::ota::channel_url(u, development))
                .ok_or(CommandError::NotOfficialOtaUrl)?;
            s.store.command(topic, "OTAURL", &url)
        };
        run_actions(&self.mqtt, action.into_iter().collect());
        Ok(())
    }

    fn targets(&self) -> crate::ota::Targets {
        let get = |l: &Mutex<Latest>| match &*l.lock().unwrap() {
            Latest::Ok(v) => Some(v.clone()),
            _ => None,
        };
        crate::ota::Targets {
            release: get(&self.latest),
            development: get(&self.latest_dev),
        }
    }
}

fn run_actions(mqtt: &Mqtt, actions: Vec<Action>) {
    for a in actions {
        match a {
            Action::Publish { topic, payload } => mqtt.publish(&topic, &payload),
            Action::Subscribe(topic) => mqtt.subscribe(&topic),
        }
    }
}

// --------------------------------------------------------------------- snapshot

#[derive(Serialize, Clone, Debug)]
pub struct UpgradeInfo {
    /// "sent", "progress", "restarting", "done", "failed" or "note" (an old failure, shown
    /// only in the device details).
    pub state: &'static str,
    pub text: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DeviceInfo {
    pub topic: String,
    pub name: String,
    /// true: online and answering. false: offline, or not answering (`no_answer`).
    /// null: just discovered, waiting for its answer.
    pub online: Option<bool>,
    /// Not reported offline, but it did not answer the status query.
    pub no_answer: bool,
    pub web_url: Option<String>,
    pub module: Option<String>,
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub gateway: Option<String>,
    pub mac: Option<String>,
    pub version: Option<String>,
    pub version_short: Option<String>,
    pub outdated: bool,
    pub core: Option<String>,
    pub sdk: Option<String>,
    pub ota_url: Option<String>,
    pub restart_reason: Option<String>,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub channel: Option<i64>,
    pub rssi: Option<i64>,
    pub ap: Option<i64>,
    pub link_count: Option<i64>,
    pub downtime: Option<u64>,
    pub uptime: Option<u64>,
    pub boot_count: Option<i64>,
    pub load_avg: Option<i64>,
    pub sleep: Option<i64>,
    pub mqtt_count: Option<i64>,
    pub program_size: Option<i64>,
    pub free: Option<i64>,
    pub full_topic: String,
    pub cmnd_topic: String,
    pub stat_topic: String,
    pub tele_topic: String,
    pub fallback_topic: Option<String>,
    pub group_topic: Option<String>,
    pub power: Vec<Option<bool>>,
    pub telemetry: Option<String>,
    pub sensor: Option<Value>,
    pub upgrade: Option<UpgradeInfo>,
    /// An upgrade is running: keep the device in view (dimmed) even while it restarts.
    pub upgrading: bool,
    /// Runs the latest release (or newer): no upgrade button.
    pub up_to_date: bool,
    /// What the upgrade button would do now.
    pub upgrade_plan: crate::ota::PlanInfo,
    pub light: Option<LightInfo>,
    /// Firmware channel the OtaUrl points at: "release", "development" or "custom".
    pub ota_channel: &'static str,
    /// The OtaUrl can be switched to the other channel.
    pub can_switch_channel: bool,
}

/// A light, as the front ends draw it.
#[derive(Serialize, Clone, Debug)]
pub struct LightInfo {
    /// "Dimmer", "CCT", "RGB", "RGBW" or "RGBCCT".
    pub kind: &'static str,
    pub channels: u8,
    pub dimmer: Option<i64>,
    pub hue: Option<i64>,
    pub sat: Option<i64>,
    pub bri: Option<i64>,
    pub ct: Option<i64>,
    pub white: Option<i64>,
    pub has_color: bool,
    pub has_ct: bool,
    pub has_white: bool,
    /// Current colour for a swatch, "#RRGGBB".
    pub swatch: String,
}

impl LightInfo {
    fn new(l: &crate::light::Light) -> LightInfo {
        let (r, g, b) = l.swatch();
        LightInfo {
            kind: l.kind(),
            channels: l.channels,
            dimmer: l.dimmer,
            hue: l.hsb.map(|h| h.0),
            sat: l.hsb.map(|h| h.1),
            bri: l.hsb.map(|h| h.2),
            ct: l.ct,
            white: l.white,
            has_color: l.has_color(),
            has_ct: l.has_ct(),
            has_white: l.has_white(),
            swatch: format!("#{r:02X}{g:02X}{b:02X}"),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Snapshot {
    /// Version of this program (the server, for the web page).
    pub app_version: &'static str,
    /// A newer Tasmota Derust release, when there is one.
    pub app_update: Option<String>,
    pub app_releases: &'static str,
    pub connected: bool,
    pub status: String,
    pub broker: String,
    pub read_only: bool,
    pub latest: Option<String>,
    /// Latest development build (for devices whose OtaUrl points there).
    pub latest_dev: Option<String>,
    pub latest_error: Option<String>,
    pub ssids: [Option<String>; 2],
    pub devices: Vec<DeviceInfo>,
}

impl DeviceInfo {
    fn new(
        d: &Device,
        status: crate::ota::VersionStatus,
        upgrade_plan: crate::ota::PlanInfo,
    ) -> DeviceInfo {
        let outdated = status.outdated;
        let up_to_date = status.up_to_date;
        let trim = |s: &str| s.trim_end_matches('/').to_string();
        DeviceInfo {
            topic: d.topic.clone(),
            name: d.display_name().to_string(),
            online: d.reachable(),
            no_answer: d.no_answer(),
            web_url: d.web_url(),
            module: d.module.clone(),
            hostname: d.hostname.clone(),
            ip: d.ip.clone(),
            gateway: d.gateway.clone(),
            mac: d.mac.clone(),
            version: d.version.clone(),
            version_short: d
                .version
                .as_deref()
                .map(|v| device::short_version(v).to_string()),
            outdated,
            core: d.core.as_deref().map(|c| c.replace('_', ".")),
            sdk: d.sdk.clone(),
            ota_url: d.ota_url.clone(),
            restart_reason: d.restart_reason.clone(),
            ssid: d.ssid.clone(),
            bssid: d.bssid.clone(),
            channel: d.channel,
            rssi: d.rssi,
            ap: d.ap,
            link_count: d.link_count,
            downtime: d.downtime.as_deref().and_then(device::parse_duration),
            uptime: d.current_uptime(),
            boot_count: d.boot_count,
            load_avg: d.load_avg,
            sleep: d.sleep,
            mqtt_count: d.mqtt_count,
            program_size: d.program_size,
            free: d.free,
            full_topic: d.full_topic.clone(),
            cmnd_topic: trim(&d.cmnd_prefix),
            stat_topic: trim(&d.stat_prefix),
            tele_topic: trim(&d.tele_prefix),
            fallback_topic: d.fallback_topic(),
            group_topic: d.group_topic.clone(),
            power: d.power.clone(),
            telemetry: d.telemetry(),
            sensor: d.sensor.clone(),
            upgrading: d.upgrade.as_ref().is_some_and(|u| u.is_active()),
            light: d.light.as_ref().map(LightInfo::new),
            ota_channel: match crate::ota::channel(d.ota_url.as_deref()) {
                crate::ota::Channel::Release => "release",
                crate::ota::Channel::Development => "development",
                crate::ota::Channel::Custom => "custom",
            },
            can_switch_channel: d.ota_url.as_deref().is_some_and(|u| {
                crate::ota::channel_url(u, true).is_some()
                    || crate::ota::channel_url(u, false).is_some()
            }),
            up_to_date,
            upgrade_plan,
            upgrade: d.upgrade.as_ref().map(|u| {
                let (state, text) = match &u.phase {
                    UpgradePhase::Sent => ("sent", "Command sent…".to_string()),
                    UpgradePhase::Progress(m) => ("progress", m.clone()),
                    UpgradePhase::Restarting => ("restarting", "Restarting…".to_string()),
                    UpgradePhase::Done(m) => ("done", m.clone()),
                    UpgradePhase::Failed(m) => ("failed", m.clone()),
                    UpgradePhase::Note(m) => ("note", m.clone()),
                };
                UpgradeInfo { state, text }
            }),
        }
    }
}
