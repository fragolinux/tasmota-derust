//! Tasmota Derust core for the Android app, exposed to Kotlin through UniFFI.
//!
//! The Kotlin side creates a [`Session`] with the broker settings, polls [`Session::snapshot`]
//! to draw the screen, and calls the command methods on user actions. A read-only session
//! refuses every command that changes something on a device; only read queries are sent.

use std::sync::Arc;

use tasmota_derust_core::config::BrokerConfig;
use tasmota_derust_core::session;

uniffi::setup_scaffolding!();

#[derive(uniffi::Record)]
pub struct Broker {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

/// Why a command was refused. The Kotlin exception's message is a sentence for the user.
#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum CommandError {
    ReadOnly,
    UnknownDevice,
    Offline,
    NoSuchRelay,
    BadAp,
    UpgradeRunning,
}

impl CommandError {
    fn core(&self) -> session::CommandError {
        use session::CommandError as E;
        match self {
            CommandError::ReadOnly => E::ReadOnly,
            CommandError::UnknownDevice => E::UnknownDevice,
            CommandError::Offline => E::Offline,
            CommandError::NoSuchRelay => E::NoSuchRelay,
            CommandError::BadAp => E::BadAp,
            CommandError::UpgradeRunning => E::UpgradeRunning,
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.core().message())
    }
}

impl std::error::Error for CommandError {}

impl From<session::CommandError> for CommandError {
    fn from(e: session::CommandError) -> Self {
        use session::CommandError as E;
        match e {
            E::ReadOnly => CommandError::ReadOnly,
            E::UnknownDevice => CommandError::UnknownDevice,
            E::Offline => CommandError::Offline,
            E::NoSuchRelay => CommandError::NoSuchRelay,
            E::BadAp => CommandError::BadAp,
            E::UpgradeRunning => CommandError::UpgradeRunning,
        }
    }
}

#[derive(uniffi::Record)]
pub struct Device {
    pub topic: String,
    pub name: String,
    /// true online and answering, false offline or not answering, null waiting.
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
    /// Seconds.
    pub downtime: Option<u64>,
    /// Seconds.
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
    /// One entry per relay: on, off, or not known yet.
    pub power: Vec<Option<bool>>,
    pub telemetry: Option<String>,
    /// Last sensor message, as pretty JSON.
    pub sensor_json: Option<String>,
    /// "sent", "progress", "restarting", "done" or "failed".
    pub upgrade_state: Option<String>,
    pub upgrade_text: Option<String>,
}

#[derive(uniffi::Record)]
pub struct Snapshot {
    pub connected: bool,
    pub status: String,
    pub broker: String,
    pub read_only: bool,
    pub latest: Option<String>,
    pub latest_error: Option<String>,
    pub ssid1: Option<String>,
    pub ssid2: Option<String>,
    pub devices: Vec<Device>,
}

impl From<session::DeviceInfo> for Device {
    fn from(d: session::DeviceInfo) -> Self {
        Device {
            topic: d.topic,
            name: d.name,
            online: d.online,
            no_answer: d.no_answer,
            web_url: d.web_url,
            module: d.module,
            hostname: d.hostname,
            ip: d.ip,
            gateway: d.gateway,
            mac: d.mac,
            version: d.version,
            version_short: d.version_short,
            outdated: d.outdated,
            core: d.core,
            sdk: d.sdk,
            ota_url: d.ota_url,
            restart_reason: d.restart_reason,
            ssid: d.ssid,
            bssid: d.bssid,
            channel: d.channel,
            rssi: d.rssi,
            ap: d.ap,
            link_count: d.link_count,
            downtime: d.downtime,
            uptime: d.uptime,
            boot_count: d.boot_count,
            load_avg: d.load_avg,
            sleep: d.sleep,
            mqtt_count: d.mqtt_count,
            program_size: d.program_size,
            free: d.free,
            full_topic: d.full_topic,
            cmnd_topic: d.cmnd_topic,
            stat_topic: d.stat_topic,
            tele_topic: d.tele_topic,
            fallback_topic: d.fallback_topic,
            group_topic: d.group_topic,
            power: d.power,
            telemetry: d.telemetry,
            // `{:#}` on a serde_json::Value is pretty-printed JSON.
            sensor_json: d.sensor.map(|v| format!("{v:#}")),
            upgrade_state: d.upgrade.as_ref().map(|u| u.state.to_string()),
            upgrade_text: d.upgrade.map(|u| u.text),
        }
    }
}

#[derive(uniffi::Object)]
pub struct Session {
    inner: session::Session,
}

#[uniffi::export]
impl Session {
    /// Connects to the broker in the background. Poll `snapshot()` to see progress.
    #[uniffi::constructor]
    pub fn new(broker: Broker, read_only: bool) -> Arc<Self> {
        let cfg = BrokerConfig {
            host: broker.host,
            port: broker.port,
            username: broker.username,
            password: broker.password,
        };
        Arc::new(Session { inner: session::Session::start(&cfg, read_only, Arc::new(|| {})) })
    }

    /// Turns read-only mode on or off without reconnecting.
    pub fn set_read_only(&self, on: bool) {
        self.inner.set_read_only(on);
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = self.inner.snapshot();
        let [ssid1, ssid2] = s.ssids;
        Snapshot {
            connected: s.connected,
            status: s.status,
            broker: s.broker,
            read_only: s.read_only,
            latest: s.latest,
            latest_error: s.latest_error,
            ssid1,
            ssid2,
            devices: s.devices.into_iter().map(Device::from).collect(),
        }
    }

    /// Read queries only; allowed in read-only mode.
    pub fn refresh(&self) {
        self.inner.refresh();
    }

    /// Toggles relay `n` (1-based).
    pub fn toggle_power(&self, topic: String, n: u32) -> Result<(), CommandError> {
        Ok(self.inner.toggle_power(&topic, n as usize)?)
    }

    /// Restarts the device on SSId `n` (1 or 2).
    pub fn switch_ap(&self, topic: String, n: u8) -> Result<(), CommandError> {
        Ok(self.inner.switch_ap(&topic, n)?)
    }

    pub fn upgrade(&self, topic: String) -> Result<(), CommandError> {
        Ok(self.inner.upgrade(&topic)?)
    }
}
