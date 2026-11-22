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
    /// The network search running now, or the last one.
    search: Option<crate::http_dev::Search>,
    search_found: usize,
    connected: bool,
    status: String,
}

pub struct Session {
    shared: Arc<Mutex<Shared>>,
    mqtt: Arc<Mqtt>,
    latest: Arc<Mutex<Latest>>,
    latest_dev: Arc<Mutex<Latest>>,
    app_latest: Arc<Mutex<Latest>>,
    read_only: Arc<AtomicBool>,
    broker: String,
    broker_cfg: BrokerConfig,
    notify: Notify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandError {
    ReadOnly,
    UnknownDevice,
    Offline,
    NoSuchRelay,
    BadAp,
    UpgradeRunning,
    UpgradeNotPossible,
    NotOfficialOtaUrl,
    NotALight,
    /// SetOption value refused: it would cut the device off from this app.
    OptionBlocked,
    BadOption,
    /// Only devices that are offline (or not answering) can be removed.
    NotOffline,
    Wifi(crate::wifi::WifiError),
    Console(crate::console::ConsoleError),
    /// "Enable MQTT" sent to a device that already uses MQTT.
    NotHttp,
    /// "Enable MQTT" without a broker address (or with a `;`).
    BadMqttSettings,
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
            CommandError::UpgradeNotPossible => {
                "This upgrade can't start now: see the upgrade details."
            }
            CommandError::NotOfficialOtaUrl => {
                "The OtaUrl is not an official release or development URL."
            }
            CommandError::NotALight => "This device has no light that can do this.",
            CommandError::OptionBlocked => {
                "This value would cut the device off from this app: change it from the device's web UI or console."
            }
            CommandError::BadOption => "No such SetOption, or value out of range.",
            CommandError::NotOffline => "Only offline devices can be removed.",
            CommandError::Wifi(e) => e.message(),
            CommandError::Console(e) => e.message(),
            CommandError::NotHttp => "This device already uses MQTT.",
            CommandError::BadMqttSettings => {
                "Give the broker address the devices can reach (and no ';' in the settings)."
            }
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
        Self::start_with(broker, &Default::default(), read_only, notify)
    }

    /// [`Session::start`] with custom MQTT topics (FullTopic patterns, devices with their own).
    pub fn start_with(
        broker: &BrokerConfig,
        topics: &crate::topics::Topics,
        read_only: bool,
        notify: Notify,
    ) -> Session {
        let (mqtt, rx) = Mqtt::start(broker, notify.clone());
        let mqtt = Arc::new(mqtt);
        let mut store = Store {
            topics: topics.clone(),
            ..Default::default()
        };
        for h in crate::http_dev::saved() {
            store.add_http_saved(&h);
        }
        let mut first_poll = store.initial_actions();
        first_poll.extend(store.poll_http());
        let shared = Arc::new(Mutex::new(Shared {
            store,
            search: None,
            search_found: 0,
            connected: false,
            status: "Connecting…".into(),
        }));

        let latest = github::fetch_latest(notify.clone());
        let latest_dev = github::fetch_latest_dev(notify.clone());
        let read_only = Arc::new(AtomicBool::new(read_only));

        // MQTT events -> device store. Only the read queries produced by the store are sent here,
        // plus the next steps of group upgrades (never in read-only mode).
        // The thread ends when the MQTT connection is dropped.
        {
            let shared = shared.clone();
            let mqtt = Arc::downgrade(&mqtt);
            let (latest, latest_dev, read_only, notify) =
                (latest.clone(), latest_dev.clone(), read_only.clone(), notify.clone());
            let mut http_polled = std::time::Instant::now();
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
                                    Some(m) => run_actions(&shared, &m, actions, &notify),
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
                        let (mut actions, gone) = {
                            let mut s = shared.lock().unwrap();
                            (s.store.check_timeouts(), s.store.merge_http())
                        };
                        // HTTP-only devices that now talk MQTT: the MQTT entry takes over.
                        for h in gone {
                            crate::http_dev::forget(&h);
                        }
                        // HTTP-only devices: asked for their status every minute.
                        if http_polled.elapsed() >= Duration::from_secs(60) {
                            http_polled = std::time::Instant::now();
                            actions.extend(shared.lock().unwrap().store.poll_http());
                        }
                        if !read_only.load(Ordering::SeqCst) {
                            let targets = targets_of(&latest, &latest_dev);
                            actions.extend(shared.lock().unwrap().store.continue_upgrades(&targets));
                        }
                        if let Some(m) = mqtt.upgrade() {
                            run_actions(&shared, &m, actions, &notify);
                        }
                    }
                })
                .expect("spawn session thread");
        }

        run_actions(&shared, &mqtt, first_poll, &notify);
        Session {
            shared,
            mqtt,
            latest,
            latest_dev,
            app_latest: github::fetch_app_latest(notify.clone()),
            notify,
            read_only,
            broker: format!("{}:{}", broker.host.trim(), broker.port),
            broker_cfg: broker.clone(),
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
        let search = self.search_state();
        // One lock at a time: two guards of the same mutex in one expression deadlock.
        let (app_update, app_latest) = {
            let l = self.app_latest.lock().unwrap();
            let v = match &*l {
                Latest::Ok(v) => Some(v.clone()),
                _ => None,
            };
            (github::app_update(&l), v)
        };
        let s = self.shared.lock().unwrap();
        let devices = s
            .store
            .devices
            .values()
            .map(|d| {
                let plan = crate::ota::plan_info(d, &targets, &s.store.sizes);
                let status = crate::ota::device_status(d, &targets);
                DeviceInfo::new(d, status, plan)
            })
            .collect();
        Snapshot {
            app_version: env!("CARGO_PKG_VERSION"),
            app_update,
            app_latest,
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
            search,
        }
    }

    /// Asks every online device for its full status. Read queries only (Status 0, Module,
    /// SSId), so this is allowed in read-only mode too.
    pub fn refresh(&self) {
        let actions = self.shared.lock().unwrap().store.refresh_all();
        run_actions(&self.shared, &self.mqtt, actions, &self.notify);
    }

    /// Removes an offline device from the list and clears its retained messages on the
    /// broker (that is a write to the broker, so not in read-only mode).
    pub fn forget(&self, topic: &str) -> Result<(), CommandError> {
        if self.read_only() {
            return Err(CommandError::ReadOnly);
        }
        let actions = {
            let mut s = self.shared.lock().unwrap();
            if !s.store.devices.contains_key(topic) {
                return Err(CommandError::UnknownDevice);
            }
            s.store.forget(topic).ok_or(CommandError::NotOffline)?
        };
        if let Some(h) = crate::http_dev::host_of_key(topic) {
            crate::http_dev::forget(h);
        }
        run_actions(&self.shared, &self.mqtt, actions, &self.notify);
        (self.notify)();
        Ok(())
    }

    /// [`Session::forget`] for every device that is not reachable. Returns how many.
    pub fn forget_offline(&self) -> Result<usize, CommandError> {
        if self.read_only() {
            return Err(CommandError::ReadOnly);
        }
        let actions = {
            let mut s = self.shared.lock().unwrap();
            let keys: Vec<String> = s
                .store
                .devices
                .iter()
                .filter(|(_, d)| d.reachable() == Some(false))
                .map(|(k, _)| k.clone())
                .collect();
            let mut n = 0;
            let mut actions = Vec::new();
            for k in &keys {
                if let Some(a) = s.store.forget(k) {
                    n += 1;
                    actions.extend(a);
                }
            }
            (n, actions)
        };
        let (n, actions) = actions;
        run_actions(&self.shared, &self.mqtt, actions, &self.notify);
        (self.notify)();
        Ok(n)
    }

    /// Searches the local network for Tasmota devices over HTTP (read-only: `Status 0` to
    /// every address). `range`: empty for this network (see `http_dev::hosts`). Devices not
    /// on MQTT are added to the list as HTTP-only, and remembered.
    pub fn search(
        &self,
        range: &str,
        password: Option<String>,
    ) -> Result<(), crate::http_dev::ScanError> {
        let hosts = crate::http_dev::hosts(range)?;
        let search = crate::http_dev::Search {
            total: hosts.len(),
            ..Default::default()
        };
        let (done, result) = (search.done.clone(), search.result.clone());
        {
            let mut s = self.shared.lock().unwrap();
            if s.search.as_ref().is_some_and(|x| x.result.lock().unwrap().is_none()) {
                return Ok(()); // one at a time
            }
            s.search = Some(search);
            s.search_found = 0;
        }
        let (shared, notify) = (self.shared.clone(), self.notify.clone());
        std::thread::spawn(move || {
            let found = crate::http_dev::scan(&hosts, password.as_deref(), &done);
            let mut added = 0;
            {
                let mut s = shared.lock().unwrap();
                for f in &found {
                    if s.store.add_http(&f.host, f.status.as_ref(), password.as_deref()).is_some() {
                        crate::http_dev::remember(&f.host);
                        added += 1;
                    }
                }
                s.search_found = added;
            }
            *result.lock().unwrap() = Some(found);
            notify();
        });
        Ok(())
    }

    /// Progress of the network search: None before the first one.
    pub fn search_state(&self) -> Option<SearchInfo> {
        let s = self.shared.lock().unwrap();
        let x = s.search.as_ref()?;
        let result = x.result.lock().unwrap();
        Some(SearchInfo {
            running: result.is_none(),
            done: x.done.load(Ordering::Relaxed).min(x.total),
            total: x.total,
            tasmota: result.as_ref().map(Vec::len).unwrap_or(0),
            added: s.search_found,
        })
    }

    /// The web password of an HTTP-only device that asked for one (memory only); it is asked
    /// for its status again.
    pub fn set_http_password(&self, key: &str, password: &str) -> Result<(), CommandError> {
        let actions = {
            let mut s = self.shared.lock().unwrap();
            let d = s.store.devices.get_mut(key).ok_or(CommandError::UnknownDevice)?;
            if d.http_host.is_none() {
                return Err(CommandError::UnknownDevice);
            }
            d.http_password = Some(password.to_string()).filter(|p| !p.is_empty());
            s.store.poll_http().into_iter().filter(|a| matches!(a, Action::Http { key: k, .. } if k == key)).collect::<Vec<_>>()
        };
        run_actions(&self.shared, &self.mqtt, actions, &self.notify);
        Ok(())
    }

    /// The custom MQTT topics in use (FullTopic patterns, devices with their own).
    pub fn topics(&self) -> crate::topics::Topics {
        self.shared.lock().unwrap().store.topics.clone()
    }

    /// This app's broker (host, port, user, whether it has a password): what "Enable MQTT"
    /// offers by default. The password itself never leaves the session.
    pub fn broker_settings(&self) -> MqttDefaults {
        let b = &self.broker_cfg;
        let host = b.host.trim().to_string();
        MqttDefaults {
            loopback: matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1"),
            host,
            port: b.port,
            user: b.username.clone(),
            has_password: !b.password.is_empty(),
        }
    }

    /// Connects HTTP-only devices to the MQTT broker: `Backlog SetOption3 1; MqttHost …;
    /// MqttPort …; MqttUser …; MqttPassword …` over HTTP (they restart). They then show up
    /// over MQTT and their HTTP entry goes away. Returns the ones that were not sent, and why.
    pub fn enable_mqtt(
        &self,
        keys: &[String],
        host: &str,
        port: u16,
        user: &str,
        password: Option<&str>,
    ) -> Vec<(String, CommandError)> {
        // None: the app's own broker password.
        let password = password.unwrap_or(&self.broker_cfg.password);
        let Some(line) = crate::http_dev::mqtt_backlog(host, port, user, password) else {
            return keys
                .iter()
                .map(|k| (k.clone(), CommandError::BadMqttSettings))
                .collect();
        };
        let payload = line.trim_start_matches("Backlog ").to_string();
        let mut errors = Vec::new();
        let mut actions = Vec::new();
        {
            let s = self.shared.lock().unwrap();
            for k in keys {
                let r = self.guard(&s, k).and_then(|()| {
                    if s.store.devices.get(k).is_some_and(|d| d.http_host.is_some()) {
                        s.store.command(k, "Backlog", &payload).ok_or(CommandError::Offline)
                    } else {
                        Err(CommandError::NotHttp)
                    }
                });
                match r {
                    Ok(a) => actions.push(a),
                    Err(e) => errors.push((k.clone(), e)),
                }
            }
        }
        run_actions(&self.shared, &self.mqtt, actions, &self.notify);
        errors
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
        run_actions(&self.shared, &self.mqtt, action.into_iter().collect(), &self.notify);
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
        run_actions(&self.shared, &self.mqtt, action.into_iter().collect(), &self.notify);
        Ok(())
    }

    /// Starts the next upgrade of one device: one step of the upgrade path for old firmware,
    /// or the upgrade to the latest release (see `ota`). Devices upgrade independently.
    pub fn upgrade(&self, topic: &str) -> Result<(), CommandError> {
        self.upgrade_with(topic, false)
    }

    /// Starts the upgrade of several devices at once, each on its own; the steps of the
    /// upgrade path follow by themselves. Returns the devices that did not start, and why.
    pub fn upgrade_many(&self, topics: &[String]) -> Vec<(String, CommandError)> {
        topics
            .iter()
            .filter_map(|t| self.upgrade_with(t, true).err().map(|e| (t.clone(), e)))
            .collect()
    }

    fn upgrade_with(&self, topic: &str, auto: bool) -> Result<(), CommandError> {
        let targets = self.targets();
        let actions = {
            let mut s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            s.store
                .start_upgrade(topic, &targets, auto)
                .map_err(|e| match e {
                    StartError::Unknown => CommandError::UnknownDevice,
                    StartError::Offline => CommandError::Offline,
                    StartError::Running => CommandError::UpgradeRunning,
                    StartError::NotPossible(_) => CommandError::UpgradeNotPossible,
                })?
        };
        run_actions(&self.shared, &self.mqtt, actions, &self.notify);
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
        run_actions(&self.shared, &self.mqtt, action.into_iter().collect(), &self.notify);
        Ok(())
    }

    /// `Restart 1`: the device restarts with its current settings.
    pub fn restart(&self, topic: &str) -> Result<(), CommandError> {
        let action = {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            s.store.command(topic, "RESTART", "1")
        };
        run_actions(&self.shared, &self.mqtt, action.into_iter().collect(), &self.notify);
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
        run_actions(&self.shared, &self.mqtt, action.into_iter().collect(), &self.notify);
        Ok(())
    }

    /// The SetOptions of a device, decoded and described (descriptions are downloaded the
    /// first time). None for an unknown device.
    pub fn options(&self, topic: &str) -> Option<crate::setoption::OptionsView> {
        crate::setoption::load_texts(self.notify.clone());
        let s = self.shared.lock().unwrap();
        let d = s.store.devices.get(topic)?;
        Some(crate::setoption::view(&d.set_options))
    }

    /// `SetOption<n> <value>`, then `Status 3` to read the options back. Values that would
    /// cut the device off from this app (MQTT or Wi-Fi off) are refused.
    pub fn set_option(&self, topic: &str, n: u16, value: u32) -> Result<(), CommandError> {
        let payload = crate::setoption::validate(n, value).map_err(|_| {
            if crate::setoption::blocked(n, value).is_some() {
                CommandError::OptionBlocked
            } else {
                CommandError::BadOption
            }
        })?;
        let actions = {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            [
                s.store.command(topic, &format!("SETOPTION{n}"), &payload),
                s.store.command(topic, "STATUS", "3"),
            ]
        };
        run_actions(&self.shared, &self.mqtt, actions.into_iter().flatten().collect(), &self.notify);
        Ok(())
    }

    /// Sets the Wi-Fi network the device is NOT using (`slot` 1 or 2): name and password in one
    /// Backlog, one restart. The network in use can't be changed from here (see `wifi`).
    pub fn set_wifi(
        &self,
        topic: &str,
        slot: u8,
        ssid: &str,
        password: &str,
    ) -> Result<(), CommandError> {
        let action = {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            let d = s
                .store
                .devices
                .get(topic)
                .ok_or(CommandError::UnknownDevice)?;
            let payload =
                crate::wifi::command(slot, d.ap, ssid, password).map_err(CommandError::Wifi)?;
            s.store.command(topic, "BACKLOG", &payload)
        };
        run_actions(&self.shared, &self.mqtt, action.into_iter().collect(), &self.notify);
        Ok(())
    }

    fn device_ip(&self, topic: &str) -> Result<(String, String), crate::backup::BackupError> {
        let s = self.shared.lock().unwrap();
        let d = s.store.devices.get(topic).ok_or_else(|| {
            crate::backup::BackupError::Failed(CommandError::UnknownDevice.message().into())
        })?;
        let ip = d.ip.clone().ok_or(crate::backup::BackupError::NoAddress)?;
        Ok((ip, crate::backup::default_name(topic, d.version.as_deref())))
    }

    /// Downloads the device's whole configuration (Tasmota's `.dmp`) from its web server.
    /// Blocking. Changes nothing, so it is allowed in read-only mode.
    pub fn backup(
        &self,
        topic: &str,
        password: Option<&str>,
    ) -> Result<crate::backup::Backup, crate::backup::BackupError> {
        let (ip, name) = self.device_ip(topic)?;
        crate::backup::download(&ip, password, &name)
    }

    /// Uploads a configuration dump to the device, which restarts with it. Blocking.
    pub fn restore(
        &self,
        topic: &str,
        password: Option<&str>,
        data: &[u8],
    ) -> Result<(), crate::backup::BackupError> {
        {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)
                .map_err(|e| crate::backup::BackupError::Failed(e.message().into()))?;
        }
        let (ip, _) = self.device_ip(topic)?;
        crate::backup::restore(&ip, password, data)
    }

    /// A device's console: its messages after `after` (0: all of them) and its command
    /// history. Allowed in read-only mode (nothing is sent). None for an unknown device.
    pub fn console(&self, topic: &str, after: u64) -> Option<ConsoleView> {
        let lines = {
            let s = self.shared.lock().unwrap();
            s.store.devices.get(topic)?;
            s.store.console.lines(topic, after)
        };
        Some(ConsoleView {
            lines,
            history: crate::console::history(topic),
        })
    }

    /// Sends a line typed in a device's console (`Command payload`, as in Tasmota's own
    /// console) and adds it to the device's history. A line that can cut the device off
    /// (Wi-Fi, MQTT, reset: `console::risk`) needs `confirmed`.
    pub fn console_send(&self, topic: &str, line: &str, confirmed: bool) -> Result<(), CommandError> {
        let (cmd, payload) = crate::console::parse(line).map_err(CommandError::Console)?;
        if !confirmed && crate::console::risk(line).is_some() {
            return Err(CommandError::Console(crate::console::ConsoleError::NeedsConfirm));
        }
        let action = {
            let s = self.shared.lock().unwrap();
            self.guard(&s, topic)?;
            s.store.command(topic, &cmd, &payload)
        };
        crate::console::remember(topic, line);
        run_actions(&self.shared, &self.mqtt, action.into_iter().collect(), &self.notify);
        Ok(())
    }

    fn targets(&self) -> crate::ota::Targets {
        targets_of(&self.latest, &self.latest_dev)
    }
}

fn targets_of(latest: &Mutex<Latest>, dev: &Mutex<Latest>) -> crate::ota::Targets {
    let get = |l: &Mutex<Latest>| match &*l.lock().unwrap() {
        Latest::Ok(v) => Some(v.clone()),
        _ => None,
    };
    crate::ota::Targets {
        release: get(latest),
        development: get(dev),
    }
}

/// Sends the actions; the commands also go to the console of their device. HTTP commands run
/// on their own thread and their answers update the store. The caller must not hold the lock.
fn run_actions(shared: &Arc<Mutex<Shared>>, mqtt: &Mqtt, actions: Vec<Action>, notify: &Notify) {
    if actions.is_empty() {
        return;
    }
    shared.lock().unwrap().store.log_sent(&actions);
    for a in actions {
        match a {
            Action::Publish { topic, payload } => mqtt.publish(&topic, &payload),
            Action::Subscribe(topic) => mqtt.subscribe(&topic),
            Action::ClearRetained(topic) => mqtt.clear_retained(&topic),
            Action::Http {
                key,
                host,
                command,
                password,
            } => {
                let (shared, notify) = (shared.clone(), notify.clone());
                std::thread::spawn(move || {
                    let answer = crate::http_dev::command(&host, &command, password.as_deref());
                    let gone = {
                        let mut s = shared.lock().unwrap();
                        let mut gone = s.store.merge_http();
                        gone.extend(s.store.handle_http(&key, &answer));
                        gone
                    };
                    for h in gone {
                        crate::http_dev::forget(&h);
                    }
                    notify();
                });
            }
        }
    }
}

// --------------------------------------------------------------------- snapshot

/// The app's broker, offered by "Enable MQTT".
#[derive(Serialize, Clone, Debug)]
pub struct MqttDefaults {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub has_password: bool,
    /// 127.0.0.1 / localhost: right for this app, wrong for the devices.
    pub loopback: bool,
}

/// The network search, for a progress bar.
#[derive(Serialize, Clone, Debug)]
pub struct SearchInfo {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    /// Tasmota devices that answered.
    pub tasmota: usize,
    /// Of those, added to the list (not already on MQTT).
    pub added: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct ConsoleView {
    pub lines: Vec<crate::console::Line>,
    /// Commands typed for this device, oldest first.
    pub history: Vec<String>,
}

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
    /// Started in the last 5 minutes (plugged in or restarted; a mere reconnect does not count).
    pub new: bool,
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
    /// The device reported its SetOptions (see `Session::options`).
    pub has_options: bool,
    /// This device's own SSId1 / SSId2.
    pub ssid1: Option<String>,
    pub ssid2: Option<String>,
    /// Reached over HTTP only (no MQTT): its address.
    pub http: Option<String>,
    /// HTTP-only and it wants a web password.
    pub http_locked: bool,
    /// Its own firmware server / highest version (not the default): see `ota_policy`.
    pub ota_own: bool,
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
    /// The latest published Tasmota Derust release, when known.
    pub app_latest: Option<String>,
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
    /// The network search (HTTP), when one was started.
    pub search: Option<SearchInfo>,
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
            new: d.is_new(),
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
            has_options: !d.set_options.is_empty(),
            ssid1: d.ssids[0].clone(),
            ssid2: d.ssids[1].clone(),
            http: d.http_host.clone(),
            http_locked: d.http_locked,
            ota_own: crate::ota_policy::policy(&d.topic).own,
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
