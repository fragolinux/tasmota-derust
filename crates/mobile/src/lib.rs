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
    OtherUpgradeRunning,
    UpgradeNotPossible,
    NotOfficialOtaUrl,
    NotALight,
    OptionBlocked,
    BadOption,
    NotOffline,
    Wifi(tasmota_derust_core::wifi::WifiError),
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
            CommandError::OtherUpgradeRunning => E::OtherUpgradeRunning,
            CommandError::UpgradeNotPossible => E::UpgradeNotPossible,
            CommandError::NotOfficialOtaUrl => E::NotOfficialOtaUrl,
            CommandError::NotALight => E::NotALight,
            CommandError::OptionBlocked => E::OptionBlocked,
            CommandError::BadOption => E::BadOption,
            CommandError::NotOffline => E::NotOffline,
            CommandError::Wifi(w) => E::Wifi(*w),
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
            E::OtherUpgradeRunning => CommandError::OtherUpgradeRunning,
            E::UpgradeNotPossible => CommandError::UpgradeNotPossible,
            E::NotOfficialOtaUrl => CommandError::NotOfficialOtaUrl,
            E::NotALight => CommandError::NotALight,
            E::OptionBlocked => CommandError::OptionBlocked,
            E::BadOption => CommandError::BadOption,
            E::NotOffline => CommandError::NotOffline,
            E::Wifi(w) => CommandError::Wifi(w),
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
    /// An upgrade is running: keep the device in view even while it restarts.
    pub upgrading: bool,
    /// Runs the latest release or newer: no upgrade button.
    pub up_to_date: bool,
    pub upgrade_plan: UpgradePlan,
    /// "release", "development" or "custom": what the OtaUrl points at.
    pub ota_channel: String,
    pub can_switch_channel: bool,
    /// The device reported its SetOptions.
    pub has_options: bool,
    /// This device's own SSId1 / SSId2.
    pub ssid1: Option<String>,
    pub ssid2: Option<String>,
    pub light: Option<Light>,
}

/// A dimmable / CCT / RGB light.
#[derive(uniffi::Record)]
pub struct Light {
    /// "Dimmer", "CCT", "RGB", "RGBW" or "RGBCCT".
    pub kind: String,
    pub dimmer: Option<i64>,
    pub hue: Option<i64>,
    pub sat: Option<i64>,
    pub bri: Option<i64>,
    pub ct: Option<i64>,
    pub white: Option<i64>,
    pub has_color: bool,
    pub has_ct: bool,
    pub has_white: bool,
    /// "#RRGGBB"
    pub swatch: String,
}

/// What the upgrade button does now (see the core's `ota::PlanInfo`).
#[derive(uniffi::Record)]
pub struct UpgradePlan {
    /// "final", "step", "manual" or "unknown".
    pub kind: String,
    pub title: String,
    pub url: Option<String>,
    pub details: Vec<String>,
    pub warnings: Vec<String>,
    pub can_start: bool,
    pub button: String,
}

#[derive(uniffi::Record)]
pub struct Snapshot {
    /// A newer Tasmota Derust release, when there is one.
    pub app_update: Option<String>,
    /// The latest published Tasmota Derust release, when known (checked every hour).
    pub app_latest: Option<String>,
    pub app_releases: String,
    pub connected: bool,
    pub status: String,
    pub broker: String,
    pub read_only: bool,
    pub latest: Option<String>,
    pub latest_dev: Option<String>,
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
            upgrading: d.upgrading,
            ota_channel: d.ota_channel.to_string(),
            light: d.light.map(|l| Light {
                kind: l.kind.to_string(),
                dimmer: l.dimmer,
                hue: l.hue,
                sat: l.sat,
                bri: l.bri,
                ct: l.ct,
                white: l.white,
                has_color: l.has_color,
                has_ct: l.has_ct,
                has_white: l.has_white,
                swatch: l.swatch,
            }),
            can_switch_channel: d.can_switch_channel,
            has_options: d.has_options,
            ssid1: d.ssid1,
            ssid2: d.ssid2,
            up_to_date: d.up_to_date,
            upgrade_plan: UpgradePlan {
                kind: d.upgrade_plan.kind.to_string(),
                title: d.upgrade_plan.title,
                url: d.upgrade_plan.url,
                details: d.upgrade_plan.details,
                warnings: d.upgrade_plan.warnings,
                can_start: d.upgrade_plan.can_start,
                button: d.upgrade_plan.button,
            },
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
        Arc::new(Session {
            inner: session::Session::start(&cfg, read_only, Arc::new(|| {})),
        })
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
            app_update: s.app_update,
            app_latest: s.app_latest,
            app_releases: s.app_releases.to_string(),
            latest: s.latest,
            latest_dev: s.latest_dev,
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

    /// Removes an offline device and clears its retained messages on the broker.
    pub fn forget(&self, topic: String) -> Result<(), CommandError> {
        Ok(self.inner.forget(&topic)?)
    }

    /// Removes every offline device; returns how many.
    pub fn forget_offline(&self) -> Result<u32, CommandError> {
        Ok(self.inner.forget_offline()? as u32)
    }

    pub fn upgrade(&self, topic: String) -> Result<(), CommandError> {
        Ok(self.inner.upgrade(&topic)?)
    }

    /// Light brightness 0..100.
    pub fn set_dimmer(&self, topic: String, dimmer: i64) -> Result<(), CommandError> {
        Ok(self.inner.set_dimmer(&topic, dimmer)?)
    }

    /// Colour: hue 0..359, saturation and brightness 0..100.
    pub fn set_hsb(&self, topic: String, hue: i64, sat: i64, bri: i64) -> Result<(), CommandError> {
        Ok(self.inner.set_hsb(&topic, hue, sat, bri)?)
    }

    /// White temperature 153 (cold) ..500 (warm).
    pub fn set_ct(&self, topic: String, ct: i64) -> Result<(), CommandError> {
        Ok(self.inner.set_ct(&topic, ct)?)
    }

    /// White channels brightness 0..100.
    pub fn set_white(&self, topic: String, white: i64) -> Result<(), CommandError> {
        Ok(self.inner.set_white(&topic, white)?)
    }

    /// Points the OtaUrl at the development (true) or release (false) builds.
    pub fn set_channel(&self, topic: String, development: bool) -> Result<(), CommandError> {
        Ok(self.inner.set_channel(&topic, development)?)
    }

    /// The SetOptions of a device, decoded and described (the descriptions are downloaded
    /// the first time: poll again). None for an unknown device.
    pub fn options(&self, topic: String) -> Option<Options> {
        let v = self.inner.options(&topic)?;
        Some(Options {
            reported: v.reported,
            texts: v.texts.to_string(),
            texts_error: v.texts_error,
            docs: v.docs.to_string(),
            options: v
                .options
                .into_iter()
                .map(|o| SetOption {
                    is_default: o.is_default(),
                    n: o.n,
                    category: o.category.to_string(),
                    numeric: o.numeric,
                    value: o.value,
                    default_value: o.default,
                    min: o.min,
                    max: o.max,
                    title: o.title,
                    details: o.details,
                    risk: match o.risk {
                        tasmota_derust_core::setoption::Risk::Normal => "normal",
                        tasmota_derust_core::setoption::Risk::App => "app",
                        tasmota_derust_core::setoption::Risk::Danger => "danger",
                    }
                    .to_string(),
                    warning: o.warning.map(str::to_string),
                    blocked_value: o.blocked_value,
                    blocked_reason: o.blocked_reason.map(str::to_string),
                    link: o.link,
                })
                .collect(),
        })
    }

    /// Sets the Wi-Fi network the device is NOT using (`slot` 1 or 2): one restart. The
    /// password is sent to the device only.
    pub fn set_wifi(
        &self,
        topic: String,
        slot: u8,
        ssid: String,
        password: String,
    ) -> Result<(), CommandError> {
        Ok(self.inner.set_wifi(&topic, slot, &ssid, &password)?)
    }

    /// The device's whole configuration (Tasmota's .dmp), from its web server. Blocking:
    /// call it off the main thread. Allowed in read-only mode (it changes nothing).
    pub fn backup(
        &self,
        topic: String,
        password: Option<String>,
    ) -> Result<BackupFile, BackupError> {
        let b = self.inner.backup(&topic, password.as_deref())?;
        Ok(BackupFile {
            name: b.name,
            data: b.data,
        })
    }

    /// Uploads a configuration file; the device restarts with it. Blocking.
    pub fn restore(
        &self,
        topic: String,
        password: Option<String>,
        data: Vec<u8>,
    ) -> Result<(), BackupError> {
        Ok(self.inner.restore(&topic, password.as_deref(), &data)?)
    }

    /// `SetOption<n> <value>`, then the options are read back. MQTT or Wi-Fi off is refused.
    pub fn set_option(&self, topic: String, n: u16, value: u32) -> Result<(), CommandError> {
        Ok(self.inner.set_option(&topic, n, value)?)
    }
}

/// A configuration dump: Tasmota's file name and its bytes.
#[derive(uniffi::Record)]
pub struct BackupFile {
    pub name: String,
    pub data: Vec<u8>,
}

/// Why a backup or restore did not work.
#[derive(Debug, uniffi::Error)]
pub enum BackupError {
    /// The device's web interface has a password: ask for it and try again.
    NeedsPassword,
    WrongPassword,
    NoAddress,
    Failed {
        reason: String,
    },
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::NeedsPassword => f.write_str("The device's web interface has a password."),
            BackupError::WrongPassword => f.write_str("Wrong web password."),
            BackupError::NoAddress => f.write_str("The device's IP address is not known yet."),
            BackupError::Failed { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<tasmota_derust_core::backup::BackupError> for BackupError {
    fn from(e: tasmota_derust_core::backup::BackupError) -> Self {
        use tasmota_derust_core::backup::BackupError as E;
        match e {
            E::NeedsPassword => BackupError::NeedsPassword,
            E::WrongPassword => BackupError::WrongPassword,
            E::NoAddress => BackupError::NoAddress,
            E::Failed(reason) => BackupError::Failed { reason },
        }
    }
}

/// The SetOptions of one device.
#[derive(uniffi::Record)]
pub struct Options {
    /// The device has reported them.
    pub reported: bool,
    /// Descriptions: "loading", "ready" or "failed".
    pub texts: String,
    pub texts_error: Option<String>,
    /// Documentation page of all SetOptions.
    pub docs: String,
    /// Ordered by category, then number.
    pub options: Vec<SetOption>,
}

#[derive(uniffi::Record)]
pub struct SetOption {
    pub n: u16,
    pub category: String,
    /// A number (SO32–49) instead of on/off.
    pub numeric: bool,
    pub value: u32,
    pub default_value: Option<u32>,
    pub is_default: bool,
    pub min: u32,
    pub max: u32,
    pub title: String,
    pub details: Vec<String>,
    /// "normal", "app" (changes how apps read the device) or "danger": ask first.
    pub risk: String,
    pub warning: Option<String>,
    /// Value that is refused, and why.
    pub blocked_value: Option<u32>,
    pub blocked_reason: Option<String>,
    pub link: String,
}

/// Release notes of a Tasmota Derust version ("Changes" part). Blocking: call it off the
/// main thread.
#[derive(uniffi::Record)]
pub struct Changelog {
    pub version: String,
    pub url: String,
    pub sections: Vec<ChangelogSection>,
    /// The APK of that release, when it has one.
    pub apk_url: Option<String>,
    pub apk_name: Option<String>,
    /// Why they could not be loaded.
    pub error: Option<String>,
}

#[derive(uniffi::Record)]
pub struct ChangelogSection {
    pub title: String,
    pub items: Vec<String>,
}

/// What the app is (About box), shared with the desktop app and the web page.
#[uniffi::export]
pub fn about_text() -> String {
    tasmota_derust_core::ABOUT.to_string()
}

#[uniffi::export]
pub fn repo_url() -> String {
    tasmota_derust_core::REPO_URL.to_string()
}

#[uniffi::export]
pub fn site_url() -> String {
    tasmota_derust_core::SITE_URL.to_string()
}

#[uniffi::export]
pub fn changelog(version: String) -> Changelog {
    match tasmota_derust_core::github::changelog(&version) {
        Ok(c) => Changelog {
            apk_url: c.download_for("android").map(|d| d.url.clone()),
            apk_name: c.download_for("android").map(|d| d.name.clone()),
            version: c.version,
            url: c.url,
            sections: c
                .sections
                .into_iter()
                .map(|s| ChangelogSection {
                    title: s.title,
                    items: s.items,
                })
                .collect(),
            error: None,
        },
        Err(e) => Changelog {
            version,
            url: "https://github.com/fragolinux/tasmota-derust/releases".into(),
            sections: Vec::new(),
            apk_url: None,
            apk_name: None,
            error: Some(e),
        },
    }
}
