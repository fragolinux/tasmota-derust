//! User interface.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use egui::{Align, Align2, Color32, FontId, Layout, RichText, Sense, Stroke, vec2};
use egui_extras::{Column as TCol, TableBuilder};

use tasmota_derust_core::columns::{Col, View};
use tasmota_derust_core::config::{BrokerConfig, Config, WebLogin};
use tasmota_derust_core::device::{self, Action, Device, StartError, Store, UpgradePhase};
use tasmota_derust_core::github::{self, Latest};
use tasmota_derust_core::light;
use tasmota_derust_core::mqtt::{Mqtt, MqttEvent};
use tasmota_derust_core::ota;
use tasmota_derust_core::setoption::{self, OptionInfo, Risk};
use tasmota_derust_core::{backup, wifi};
use tasmota_derust_web::WebServer;

#[cfg(feature = "screenshots")]
#[path = "shots.rs"]
mod shots;

const GREEN: Color32 = Color32::from_rgb(0x7c, 0xc0, 0x42);
const YELLOW: Color32 = Color32::from_rgb(0xf2, 0xc2, 0x1b);
const RED: Color32 = Color32::from_rgb(0xe0, 0x4b, 0x3c);
const ORANGE: Color32 = Color32::from_rgb(0xf0, 0x8c, 0x1e);
const GRAY: Color32 = Color32::from_rgb(0xb0, 0xb0, 0xb0);
const ROW_HEIGHT: f32 = 26.0;
const LOG_MAX: usize = 2000;
const WEB_READ_ONLY_HINT: &str = "It was started read-only from the desktop app: stop it there and start it again as read-write.";
const READ_ONLY_NOTICE: &str =
    "Read-only mode is on: nothing was sent. Untick Read-only in the toolbar to allow commands.";

enum Conn {
    NotConfigured,
    Connecting,
    Connected,
    Error(String),
}

const FORGET_TEXT: &str = "It is removed from the list, and the messages the broker keeps for it \
    (its last will, discovery…) are deleted, so it is not listed again.";
const FORGET_BACK: &str = "If the device comes back online, it shows up again by itself.";

enum Confirm {
    Upgrade(String),
    SwitchAp(String, i64),
    /// Device, option, new value.
    SetOption(String, u16, u32),
    /// Remove an offline device and clear its retained messages.
    Forget(String),
    /// The same for every offline device.
    ForgetOffline,
}

/// Wi-Fi networks window of one device: the slot being edited and what was typed.
struct WifiWindow {
    key: String,
    ssid: String,
    password: String,
    /// The confirmation is open.
    confirm: bool,
}

type BackupSlot = Arc<Mutex<Option<Result<String, backup::BackupError>>>>;

/// Backup / restore window of one device.
struct BackupWindow {
    key: String,
    /// The device's web password, only when it has one (kept while the window is open).
    password: String,
    /// The .dmp file chosen for a restore (Downloads list or dropped on the window).
    chosen: Option<std::path::PathBuf>,
    confirm: bool,
    /// The running job and what it reports (Ok: a sentence for the user).
    job: Option<(&'static str, BackupSlot)>,
    result: Option<Result<String, backup::BackupError>>,
    /// Last backup saved, for "Show in folder".
    saved: Option<std::path::PathBuf>,
}

/// SetOption window of one device.
struct OptionsWindow {
    key: String,
    search: String,
    only_changed: bool,
    /// Numbers being edited (SO32–49), sent with "Set".
    drafts: std::collections::HashMap<u16, u32>,
}

enum UiAction {
    Toggle(String, usize),
    OpenWeb(String),
    AskUpgrade(String),
    AskAp(String, i64),
    /// Point the OtaUrl at the development (true) or release (false) builds.
    SetChannel(String, bool),
    OpenLight(String),
    OpenOptions(String),
    OpenWifi(String),
    OpenBackup(String),
    /// The gear's menu, at that screen position.
    OpenTools(String, egui::Pos2),
    AskForget(String),
    AskForgetOffline,
    Sort(Option<Col>),
}

struct LogLine {
    time: String,
    dir: char,
    topic: String,
    payload: String,
}

/// Read-only data the cell renderers need besides the device.
struct CellCtx<'a> {
    targets: &'a ota::Targets,
    ssids: &'a [Option<String>; 2],
    sizes: &'a ota::Sizes,
    dim: bool,
}

pub struct DerustApp {
    cfg: Config,
    /// Asks for a repaint (background threads).
    repaint: tasmota_derust_core::Notify,
    store: Store,
    mqtt: Option<Mqtt>,
    rx: Option<Receiver<MqttEvent>>,
    conn: Conn,
    latest: Arc<Mutex<Latest>>,
    latest_dev: Arc<Mutex<Latest>>,
    app_latest: Arc<Mutex<Latest>>,
    settings_open: bool,
    draft: BrokerConfig,
    /// Web login and HTTPS choice being edited in Settings.
    login_draft: LoginDraft,
    https_draft: bool,
    /// "Start web server" asks whether to start it read-only or read-write.
    web_ask: bool,
    confirm: Option<Confirm>,
    sort: Option<Col>,
    sort_desc: bool,
    log: VecDeque<LogLine>,
    log_open: bool,
    log_filter: String,
    /// Built-in web server (same page as `tasmota-derust-web`), with its own MQTT session.
    web: Option<WebServer>,
    web_error: Option<String>,
    /// Refuses every command that changes a device (relays, Wi-Fi switch, upgrade) in this
    /// window. On at every start. The web server has its own mode, chosen when it starts.
    read_only: bool,
    /// Short message shown at the bottom of the window, with the time it appeared.
    notice: Option<(String, Instant)>,
    /// Light control window: device topic and the values being edited.
    light_open: Option<(String, LightEdit)>,
    /// Changelog popup: version and its notes (None while loading).
    changelog: Option<(String, ChangelogSlot)>,
    about_open: bool,
    /// What changed in the latest release (or this one), loaded when About opens.
    about_notes: Option<(String, ChangelogSlot)>,
    options_open: Option<OptionsWindow>,
    wifi_open: Option<WifiWindow>,
    backup_open: Option<BackupWindow>,
    /// The gear's menu: device, where it opens, and "opened this frame" (that click is not
    /// a click outside it).
    tools_open: Option<(String, egui::Pos2, bool)>,
    #[cfg(feature = "screenshots")]
    shots: Option<shots::Shots>,
}

impl DerustApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let cfg = Config::load();
        let mut app = Self {
            repaint: repaint(&ctx),
            draft: cfg.broker.clone(),
            login_draft: LoginDraft::from(&cfg.web_login),
            https_draft: cfg.web_https,
            web_ask: false,
            cfg,
            store: Store::default(),
            mqtt: None,
            rx: None,
            conn: Conn::NotConfigured,
            latest: github::fetch_latest(repaint(&ctx)),
            latest_dev: github::fetch_latest_dev(repaint(&ctx)),
            app_latest: github::fetch_app_latest(repaint(&ctx)),
            settings_open: false,
            confirm: None,
            sort: None,
            sort_desc: false,
            log: VecDeque::new(),
            log_open: false,
            log_filter: String::new(),
            web: None,
            web_error: None,
            read_only: true,
            notice: None,
            changelog: None,
            about_open: false,
            about_notes: None,
            light_open: None,
            options_open: None,
            wifi_open: None,
            backup_open: None,
            tools_open: None,
            #[cfg(feature = "screenshots")]
            shots: shots::Shots::from_env(),
        };
        ctx.set_theme(theme_pref(&app.cfg.theme));
        if app.cfg.broker.host.trim().is_empty() {
            app.settings_open = true;
        } else {
            app.connect(&ctx);
        }
        app
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.mqtt = None;
        self.rx = None;
        self.store = Store::default();
        if self.cfg.broker.host.trim().is_empty() {
            self.conn = Conn::NotConfigured;
            return;
        }
        let (mqtt, rx) = Mqtt::start(&self.cfg.broker, repaint(ctx));
        self.mqtt = Some(mqtt);
        self.rx = Some(rx);
        self.conn = Conn::Connecting;
        self.push_log(
            '*',
            &format!(
                "connecting to {}:{}",
                self.cfg.broker.host, self.cfg.broker.port
            ),
            "",
        );
    }

    fn push_log(&mut self, dir: char, topic: &str, payload: &str) {
        // Wi-Fi passwords never reach the log.
        let mut payload = wifi::mask_passwords(payload);
        if payload.len() > 400 {
            let mut cut = 400;
            while !payload.is_char_boundary(cut) {
                cut -= 1;
            }
            payload.truncate(cut);
            payload.push('…');
        }
        self.log.push_back(LogLine {
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            dir,
            topic: topic.to_string(),
            payload,
        });
        while self.log.len() > LOG_MAX {
            self.log.pop_front();
        }
    }

    fn process_events(&mut self) {
        let Some(rx) = &self.rx else { return };
        let events: Vec<MqttEvent> = rx.try_iter().collect();
        for ev in events {
            match ev {
                MqttEvent::Connected => {
                    self.conn = Conn::Connected;
                    self.push_log('*', "connected", "");
                }
                MqttEvent::Disconnected(e) => {
                    self.push_log('!', "connection error", &e);
                    self.conn = Conn::Error(e);
                }
                MqttEvent::Message {
                    topic,
                    payload,
                    retained,
                } => {
                    self.push_log('<', &topic, &payload);
                    let actions = self.store.handle_message(&topic, &payload, retained);
                    self.apply(actions);
                }
            }
        }
    }

    fn apply(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Publish { topic, payload } => {
                    if let Some(m) = &self.mqtt {
                        m.publish(&topic, &payload);
                    }
                    self.push_log('>', &topic, &payload);
                }
                Action::Subscribe(topic) => {
                    if let Some(m) = &self.mqtt {
                        m.subscribe(&topic);
                    }
                }
                Action::ClearRetained(topic) => {
                    if let Some(m) = &self.mqtt {
                        m.clear_retained(&topic);
                    }
                    self.push_log('>', &topic, "(retained message cleared)");
                }
            }
        }
    }

    // ------------------------------------------------------------- web server

    fn start_web(&mut self, read_only: bool) {
        self.web = None;
        self.web_error = None;
        let https = self.cfg.web_https;
        let listen = tasmota_derust_web::default_listen(https);
        let login = &self.cfg.web_login;
        match WebServer::start(
            &self.cfg.broker,
            login,
            https,
            listen,
            read_only,
            WEB_READ_ONLY_HINT,
        ) {
            Ok(w) => {
                let mode = if w.read_only() {
                    "read-only"
                } else {
                    "read-write"
                };
                let login = if w.login() {
                    "login required"
                } else {
                    "no login"
                };
                self.push_log(
                    '*',
                    &format!("web server on {} ({mode}, {login})", w.url()),
                    "",
                );
                self.web = Some(w);
            }
            Err(e) => {
                self.web_error = Some(format!("Cannot start the web server on {listen}: {e}"))
            }
        }
    }

    /// Asks how to start the web server: read-only, or read-write (with the risk spelled
    /// out when there is no login).
    fn web_start_window(&mut self, ctx: &egui::Context) {
        if !self.web_ask {
            return;
        }
        let login = self.cfg.web_login.is_set();
        let mut open = true;
        let mut choice = None;
        egui::Window::new("Start web server")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .default_width(420.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    "May web visitors send commands (relays, Wi-Fi, settings, restores, \
                     upgrades)? The Read-only checkbox of this window does not change it.",
                );
                ui.add_space(6.0);
                if login {
                    ui.label(format!(
                        "Visitors must log in as \"{}\" (set in Settings).",
                        self.cfg.web_login.username
                    ));
                    let note = if self.cfg.web_https {
                        "HTTPS with a self-signed certificate: browsers warn on the first visit."
                    } else {
                        "Plain HTTP (no TLS): the login and all traffic travel in clear text \
                         on the network."
                    };
                    ui.label(RichText::new(note).small().weak());
                } else {
                    ui.colored_label(
                        RED,
                        RichText::new("Read-write without a login is a risk.").strong(),
                    );
                    ui.colored_label(RED, tasmota_derust_web::no_login_risk(self.cfg.web_https));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Read-only").clicked() {
                        choice = Some(true);
                    }
                    let rw = if login {
                        RichText::new("Read-write")
                    } else {
                        RichText::new("Read-write, no login").color(RED)
                    };
                    if ui.button(rw).clicked() {
                        choice = Some(false);
                    }
                    if !login && ui.button("Settings…").clicked() {
                        self.draft = self.cfg.broker.clone();
                        self.login_draft = LoginDraft::from(&self.cfg.web_login);
                        self.https_draft = self.cfg.web_https;
                        self.settings_open = true;
                        self.web_ask = false;
                    }
                });
            });
        if let Some(read_only) = choice {
            self.web_ask = false;
            self.start_web(read_only);
        } else if !open {
            self.web_ask = false;
        }
    }

    fn stop_web(&mut self) {
        if let Some(w) = self.web.take() {
            w.stop();
            self.push_log('*', "web server stopped", "");
        }
    }

    fn read_only_toggle(&mut self, ui: &mut egui::Ui) {
        let label = if self.read_only {
            RichText::new("Read-only").color(ORANGE).strong()
        } else {
            RichText::new("Read-only")
        };
        let r = ui.checkbox(&mut self.read_only, label).on_hover_text(
            "When ticked, only read queries are sent from this window: relays, Wi-Fi switching \
             and upgrades are refused. Ticked at every start. The web server's mode is chosen \
             when it starts.",
        );
        if r.changed() {
            let state = if self.read_only { "on" } else { "off" };
            self.push_log('*', &format!("read-only {state}"), "");
        }
    }

    fn show_notice(&mut self, text: &str) {
        self.notice = Some((text.to_string(), Instant::now()));
    }

    fn notice_overlay(&mut self, ctx: &egui::Context) {
        const SHOW: Duration = Duration::from_secs(5);
        let Some((text, since)) = &self.notice else {
            return;
        };
        if since.elapsed() > SHOW {
            self.notice = None;
            return;
        }
        egui::Area::new(egui::Id::unique("notice"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -48.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(ORANGE).show(ui, |ui| {
                    ui.label(RichText::new(text.as_str()).color(Color32::BLACK).strong());
                });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Right-to-left layout: widgets are added in reverse visual order.
    fn web_controls(&mut self, ui: &mut egui::Ui) {
        let can_start = !self.cfg.broker.host.trim().is_empty();
        if let Some(w) = &self.web {
            let url = w.url();
            let read_only = w.read_only();
            if ui.button("Stop web").clicked() {
                self.stop_web();
                return;
            }
            if !w.login() {
                chip(ui, "no login", RED).on_hover_text(
                    "Anyone on this network can open the page. Set a web login in Settings.",
                );
            }
            if read_only {
                chip(ui, "read-only", ORANGE);
            } else {
                chip(ui, "read-write", RED)
                    .on_hover_text("Web visitors can switch relays, change Wi-Fi and upgrade.");
            }
            let tip = match w.fingerprint() {
                Some(f) => format!(
                    "Open it from any phone or computer on this network.\n\nSelf-signed \
                     certificate, SHA-256 fingerprint:\n{f}"
                ),
                None => "Open it from any phone or computer on this network.\n\nPlain HTTP: \
                         traffic is not encrypted."
                    .into(),
            };
            ui.hyperlink_to(&url, &url).on_hover_text(tip);
            ui.label("Web:");
        } else {
            let start = ui
                .add_enabled(can_start, egui::Button::new("Start web server"))
                .on_hover_text(
                    "Serve the web interface to phones and computers on this network. \
                     It asks whether web visitors may send commands.",
                )
                .on_disabled_hover_text("Configure the MQTT broker first");
            if let Some(e) = &self.web_error {
                warn_icon(ui, RED).on_hover_text(e.as_str());
            }
            if start.clicked() {
                self.web_ask = true;
            }
        }
    }

    /// Latest release and latest development build.
    fn targets(&self) -> ota::Targets {
        let dev = match &*self.latest_dev.lock().unwrap() {
            Latest::Ok(v) => Some(v.clone()),
            _ => None,
        };
        ota::Targets {
            release: self.latest_version(),
            development: dev,
        }
    }

    /// Colour wheel and sliders for one light. Values are sent when a drag ends or on a click.
    fn light_window(&mut self, ctx: &egui::Context) {
        let Some((key, mut edit)) = self.light_open.take() else {
            return;
        };
        let Some(d) = self.store.devices.get(&key) else {
            return;
        };
        let Some(light) = d.light.clone() else { return };
        let name = d.display_name().to_string();
        let online = d.is_online();
        let mut open = true;
        let mut send: Vec<(&'static str, String)> = Vec::new();
        egui::Window::new(format!("{name} — {} light", light.kind()))
            .id(egui::Id::unique("light-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_enabled_ui(online, |ui| {
                    if light.has_color() {
                        if let Some((h, sat)) = color_wheel(ui, edit.hue, edit.sat, 200.0) {
                            edit.hue = h;
                            edit.sat = sat;
                            send.push(("HSBCOLOR", format!("{},{},{}", h, sat, edit.dimmer)));
                        }
                        ui.add_space(8.0);
                    }
                    let mut slider =
                        |ui: &mut egui::Ui, v: &mut i64, range, label: &str, cmd: &'static str| {
                            let r = ui.add(egui::Slider::new(v, range).text(label));
                            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                send.push((cmd, v.to_string()));
                            }
                        };
                    slider(ui, &mut edit.dimmer, 0..=100, "Brightness", "DIMMER");
                    if light.has_ct() {
                        slider(
                            ui,
                            &mut edit.ct,
                            light::CT_MIN..=light::CT_MAX,
                            "White temperature (cold … warm)",
                            "CT",
                        );
                    }
                    if light.has_white() {
                        slider(ui, &mut edit.white, 0..=100, "White", "WHITE");
                    }
                });
                if !online {
                    ui.colored_label(RED, "The device is not online.");
                }
                let (r, g, b) = light.swatch();
                ui.horizontal(|ui| {
                    ui.label("Now:");
                    swatch(ui, Color32::from_rgb(r, g, b));
                    ui.label(RichText::new(light.color.as_deref().unwrap_or("")).weak());
                });
            });
        if !send.is_empty() {
            if self.read_only {
                self.show_notice(READ_ONLY_NOTICE);
            } else {
                let actions: Vec<Action> = send
                    .into_iter()
                    .filter_map(|(cmd, payload)| self.store.command(&key, cmd, &payload))
                    .collect();
                self.apply(actions);
            }
        }
        if open {
            self.light_open = Some((key, edit));
        }
    }

    /// `SetOption<n> <v>`, then `Status 3` to read the options back.
    fn set_option_actions(&self, key: &str, n: u16, v: u32) -> Vec<Action> {
        match setoption::validate(n, v) {
            Ok(payload) => [
                self.store.command(key, &format!("SETOPTION{n}"), &payload),
                self.store.command(key, "STATUS", "3"),
            ]
            .into_iter()
            .flatten()
            .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// A change asked from the SetOption window: refused, confirmed first, or sent.
    fn request_option(&mut self, key: &str, o: &OptionInfo, v: u32) {
        if let Some(why) = setoption::blocked(o.n, v) {
            self.show_notice(why);
        } else if self.read_only {
            self.show_notice(READ_ONLY_NOTICE);
        } else if o.risk != Risk::Normal {
            self.confirm = Some(Confirm::SetOption(key.to_string(), o.n, v));
        } else {
            let a = self.set_option_actions(key, o.n, v);
            self.apply(a);
        }
    }

    /// The SetOptions of one device, grouped by category, with search and switches.
    fn options_window(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.options_open.take() else {
            return;
        };
        let Some(d) = self.store.devices.get(&w.key) else {
            return;
        };
        let name = d.display_name().to_string();
        let online = d.is_online();
        let view = setoption::view(&d.set_options);
        let mut open = true;
        let mut change: Option<(OptionInfo, u32)> = None;
        let mut refresh = false;
        let mut open_backup = false;
        egui::Window::new(format!("{name} — SetOptions"))
            .id(egui::Id::unique("options-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 560.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut w.search)
                            .hint_text("Search (number or words)")
                            .desired_width(240.0),
                    );
                    ui.checkbox(&mut w.only_changed, "Only changed from default");
                    if ui.button("Reload").on_hover_text("Ask the device for its options again").clicked() {
                        refresh = true;
                    }
                    if ui
                        .button("Back up first")
                        .on_hover_text("Save the device's whole configuration before changing risky options")
                        .clicked()
                    {
                        open_backup = true;
                    }
                });
                match view.texts {
                    "loading" if view.options.iter().all(|o| o.title.starts_with("SetOption")) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading the descriptions from the Tasmota documentation…");
                        });
                    }
                    "failed" => {
                        ui.colored_label(
                            ORANGE,
                            "Descriptions not available (no internet?): options are shown by number.",
                        );
                    }
                    _ => {}
                }
                if self.read_only {
                    ui.colored_label(ORANGE, "Read-only mode is on: options can't be changed.");
                } else if !online {
                    ui.colored_label(RED, "The device is not online.");
                }
                ui.separator();
                let q = w.search.trim().to_lowercase();
                let shown: Vec<&OptionInfo> = view
                    .options
                    .iter()
                    .filter(|o| !w.only_changed || !o.is_default())
                    .filter(|o| o.matches(&q))
                    .collect();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if shown.is_empty() {
                        ui.label(RichText::new("No option matches.").weak());
                    }
                    for cat in setoption::CATEGORIES {
                        let in_cat: Vec<&&OptionInfo> = shown.iter().filter(|o| o.category == cat).collect();
                        if in_cat.is_empty() {
                            continue;
                        }
                        egui::CollapsingHeader::new(RichText::new(format!("{cat}  ({})", in_cat.len())).strong())
                            .id_salt(("so-cat", cat, !q.is_empty() || w.only_changed))
                            .default_open(!q.is_empty() || w.only_changed || cat == "General")
                            .show(ui, |ui| {
                                for o in in_cat {
                                    if let Some(v) = option_row(ui, o, &mut w.drafts, online && !self.read_only) {
                                        change = Some(((*o).clone(), v));
                                    }
                                    ui.separator();
                                }
                            });
                    }
                });
                ui.hyperlink_to("All SetOptions in the Tasmota documentation", view.docs);
            });
        let key = w.key.clone();
        if open {
            self.options_open = Some(w);
        }
        if let Some((o, v)) = change {
            self.request_option(&key, &o, v);
        }
        if open_backup {
            self.handle_actions(vec![UiAction::OpenBackup(key.clone())]);
        }
        if refresh {
            // Status 0 holds the options too, and it is a read query (allowed in read-only).
            if let Some(a) = self.store.command(&key, "STATUS", "0") {
                self.apply(vec![a]);
            }
        }
    }

    /// The gear's menu: SetOptions, Wi-Fi networks, backup / restore.
    fn tools_menu(&mut self, ctx: &egui::Context) {
        let Some((key, pos, fresh)) = self.tools_open.clone() else {
            return;
        };
        let has_options = self
            .store
            .devices
            .get(&key)
            .is_some_and(|d| !d.set_options.is_empty());
        let mut picked = None;
        let area = egui::Area::new(egui::Id::unique("tools-menu").with(&key))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    if ui
                        .add_enabled(has_options, egui::Button::new("SetOptions…").frame(false))
                        .clicked()
                    {
                        picked = Some(UiAction::OpenOptions(key.clone()));
                    }
                    if ui
                        .add(egui::Button::new("Wi-Fi networks…").frame(false))
                        .clicked()
                    {
                        picked = Some(UiAction::OpenWifi(key.clone()));
                    }
                    if ui
                        .add(egui::Button::new("Backup / restore…").frame(false))
                        .clicked()
                    {
                        picked = Some(UiAction::OpenBackup(key.clone()));
                    }
                });
            });
        let clicked_outside = !fresh
            && ctx.input(|i| i.pointer.any_click())
            && !area
                .response
                .rect
                .contains(ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default());
        if picked.is_some() || clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.tools_open = None;
        } else if fresh {
            self.tools_open = Some((key.clone(), pos, false));
        }
        if let Some(a) = picked {
            self.handle_actions(vec![a]);
        }
    }

    /// Wi-Fi networks of one device; only the one it is not using can be changed.
    fn wifi_window(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.wifi_open.take() else {
            return;
        };
        let Some(d) = self.store.devices.get(&w.key) else {
            return;
        };
        let name = d.display_name().to_string();
        let ssids = d.ssids.clone();
        let active = d.ap;
        let online = d.is_online();
        let mut open = true;
        let mut send = false;
        let slot = match active {
            Some(1) => Some(2u8),
            Some(2) => Some(1u8),
            _ => None,
        };
        if w.ssid.is_empty() && !w.confirm {
            if let Some(s) = slot.and_then(|s| ssids[s as usize - 1].clone()) {
                w.ssid = s;
            }
        }
        egui::Window::new(format!("{name} — Wi-Fi networks"))
            .id(egui::Id::unique("wifi-window").with(&w.key))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Only the network the device is not using can be changed: changing the one \
                         in use could leave it unable to reconnect. To change that one, switch the \
                         device to the other network first (Wifi view, AP column).",
                    )
                    .weak(),
                );
                ui.add_space(6.0);
                egui::Grid::new("wifi-grid").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                    for i in 0..2 {
                        ui.label(format!("SSId{}", i + 1));
                        let text = ssids[i].clone().unwrap_or_else(|| "not set".into());
                        let in_use = active == Some(i as i64 + 1);
                        ui.label(if in_use {
                            RichText::new(format!("{text}  · in use")).color(GREEN)
                        } else {
                            RichText::new(text)
                        });
                        ui.end_row();
                    }
                });
                ui.separator();
                let Some(slot) = slot else {
                    ui.colored_label(ORANGE, "Which network the device is using is not known yet: press Refresh.");
                    return;
                };
                ui.label(RichText::new(format!("Set SSId{slot}")).strong());
                ui.add_enabled_ui(online && !w.confirm, |ui| {
                    egui::Grid::new("wifi-form").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("Network name");
                        ui.add(egui::TextEdit::singleline(&mut w.ssid).char_limit(32).desired_width(240.0));
                        ui.end_row();
                        ui.label("Password");
                        ui.add(
                            egui::TextEdit::singleline(&mut w.password)
                                .password(true)
                                .char_limit(63)
                                .desired_width(240.0)
                                .hint_text("8 to 63 characters"),
                        );
                        ui.end_row();
                    });
                });
                let check = wifi::command(slot, active, &w.ssid, &w.password);
                if let Err(e) = &check {
                    // Only once something was typed (the name is prefilled).
                    if !w.password.is_empty() {
                        ui.colored_label(ORANGE, e.message());
                    }
                }
                if !online {
                    ui.colored_label(RED, "The device is not online.");
                }
                ui.add_space(6.0);
                if !w.confirm {
                    if ui
                        .add_enabled(check.is_ok() && online, egui::Button::new("Save and restart…"))
                        .clicked()
                    {
                        w.confirm = true;
                    }
                } else {
                    ui.label(format!(
                        "SSId{slot} becomes \"{}\". The device restarts and stays on SSId{} (the \
                         network it uses now); it moves to the new one only if that one is lost, or \
                         when you switch it.",
                        w.ssid.trim(),
                        active.unwrap_or(0)
                    ));
                    ui.label(
                        RichText::new("The password goes to the device only: it is neither stored nor shown here.")
                            .weak(),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Save and restart").strong()).clicked() {
                            send = true;
                        }
                        if ui.button("Cancel").clicked() {
                            w.confirm = false;
                        }
                    });
                }
            });
        if send {
            if self.read_only {
                self.show_notice(READ_ONLY_NOTICE);
                w.confirm = false;
            } else if let (Some(slot), Ok(payload)) = (
                slot,
                wifi::command(slot.unwrap_or(0), active, &w.ssid, &w.password),
            ) {
                if let Some(a) = self.store.command(&w.key, "BACKLOG", &payload) {
                    self.apply(vec![a]);
                    self.show_notice(&format!("{name}: SSId{slot} set, the device restarts."));
                    return;
                }
            }
        }
        if open {
            self.wifi_open = Some(w);
        }
    }

    /// Whole-configuration backup (saved in Downloads) and restore of one device.
    fn backup_window(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.backup_open.take() else {
            return;
        };
        let Some(d) = self.store.devices.get(&w.key) else {
            return;
        };
        let name = d.display_name().to_string();
        let ip = d.ip.clone();
        let online = d.is_online();
        let fallback = backup::default_name(&w.key, d.version.as_deref());
        // Finished job?
        let finished = w
            .job
            .as_ref()
            .and_then(|(what, slot)| slot.lock().unwrap().take().map(|r| (*what, r)));
        if let Some((what, r)) = finished {
            // A backup reports the path of the file it saved.
            let r = match r {
                Ok(path) if what == "backup" => {
                    w.saved = Some(std::path::PathBuf::from(&path));
                    Ok(format!("Saved {path}"))
                }
                r => r,
            };
            w.result = Some(r);
            w.job = None;
            w.confirm = false;
        }
        // A .dmp dropped on the window.
        if let Some(p) = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .find_map(|f| Some(f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()))
        }) {
            w.chosen = Some(p);
            w.confirm = false;
        }
        let dir = backup::downloads_dir();
        let mut open = true;
        let mut start: Option<&'static str> = None;
        egui::Window::new(format!("{name} — Backup / restore"))
            .id(egui::Id::unique("backup-window").with(&w.key))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .max_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "The whole device configuration (Wi-Fi, MQTT, topic, GPIOs, rules, SetOptions), \
                         through the device's own web server: the same .dmp file as its \
                         Configuration > Backup / Restore pages.",
                    )
                    .weak(),
                );
                ui.horizontal(|ui| {
                    ui.label("Web password");
                    ui.add(
                        egui::TextEdit::singleline(&mut w.password)
                            .password(true)
                            .desired_width(180.0)
                            .hint_text("only if the device has one"),
                    );
                });
                if ip.is_none() {
                    ui.colored_label(ORANGE, "The device's IP address is not known yet.");
                }
                let busy = w.job.is_some();
                ui.separator();
                ui.label(RichText::new("Backup").strong());
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy && ip.is_some() && online, egui::Button::new("Back up now"))
                        .clicked()
                    {
                        start = Some("backup");
                    }
                });
                if let Some(dir) = &dir {
                    ui.add(egui::Label::new(RichText::new(format!("Saved in {}", dir.display())).weak().small()).wrap());
                }
                if let Some(p) = &w.saved {
                    if ui.link("Show in folder").clicked() {
                        let _ = open::that(p.parent().unwrap_or(p));
                    }
                }
                ui.separator();
                ui.label(RichText::new("Restore").strong());
                ui.label(RichText::new("Pick a .dmp file from Downloads, or drop one on this window.").weak());
                let files = dir.as_deref().map(backup::list_dumps).unwrap_or_default();
                egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                    if files.is_empty() {
                        ui.label(RichText::new("No .dmp files in Downloads.").weak());
                    }
                    for f in files.iter().take(30) {
                        let label = f.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        let sel = w.chosen.as_deref() == Some(f.as_path());
                        if ui.selectable_label(sel, label).clicked() {
                            w.chosen = Some(f.clone());
                            w.confirm = false;
                        }
                    }
                });
                if let Some(c) = &w.chosen {
                    ui.label(format!("Chosen: {}", c.display()));
                    if !w.confirm {
                        if ui
                            .add_enabled(!busy && ip.is_some() && online, egui::Button::new("Restore…"))
                            .clicked()
                        {
                            w.confirm = true;
                        }
                    } else {
                        ui.colored_label(
                            ORANGE,
                            "The device restarts with that whole configuration. A file from another \
                             device copies its name, topic and network settings too: use one made \
                             from this device.",
                        );
                        ui.horizontal(|ui| {
                            if ui.button(RichText::new("Restore and restart").strong()).clicked() {
                                start = Some("restore");
                            }
                            if ui.button("Cancel").clicked() {
                                w.confirm = false;
                            }
                        });
                    }
                }
                ui.separator();
                if let Some((what, _)) = &w.job {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(if *what == "backup" { "Backing up…" } else { "Restoring…" });
                    });
                }
                match &w.result {
                    Some(Ok(msg)) => {
                        ui.colored_label(GREEN, msg.as_str());
                    }
                    Some(Err(backup::BackupError::NeedsPassword)) => {
                        ui.colored_label(ORANGE, "The device's web interface has a password: type it above and try again.");
                    }
                    Some(Err(e)) => {
                        ui.colored_label(RED, e.to_string());
                    }
                    None => {}
                }
            });
        if let (Some(what), Some(ip)) = (start, ip) {
            if what == "restore" && self.read_only {
                self.show_notice(READ_ONLY_NOTICE);
                w.confirm = false;
            } else {
                let slot: BackupSlot = Arc::new(Mutex::new(None));
                let out = slot.clone();
                let pw = Some(w.password.clone()).filter(|p| !p.is_empty());
                let chosen = w.chosen.clone();
                let repaint = self.repaint.clone();
                w.result = None;
                std::thread::spawn(move || {
                    let r = if what == "backup" {
                        backup::download(&ip, pw.as_deref(), &fallback).and_then(|b| {
                            let dir = backup::downloads_dir().ok_or_else(|| {
                                backup::BackupError::Failed("No Downloads folder.".into())
                            })?;
                            backup::save(&dir, &b)
                                .map(|p| p.display().to_string())
                                .map_err(|e| backup::BackupError::Failed(e.to_string()))
                        })
                    } else {
                        let data = chosen
                            .as_deref()
                            .ok_or_else(|| backup::BackupError::Failed("No file chosen.".into()))
                            .and_then(|p| {
                                std::fs::read(p)
                                    .map_err(|e| backup::BackupError::Failed(e.to_string()))
                            });
                        data.and_then(|d| backup::restore(&ip, pw.as_deref(), &d))
                            .map(|_| "Configuration restored: the device restarts.".to_string())
                    };
                    *out.lock().unwrap() = Some(r);
                    repaint();
                });
                w.job = Some((what, slot));
            }
        }
        if open {
            self.backup_open = Some(w);
        }
    }

    /// Fetches the release notes of `version` in the background and shows them.
    fn open_changelog(&mut self, ctx: &egui::Context, version: &str) {
        let slot = Arc::new(Mutex::new(None));
        let (out, v, ctx) = (slot.clone(), version.to_string(), ctx.clone());
        std::thread::spawn(move || {
            *out.lock().unwrap() = Some(github::changelog(&v));
            ctx.request_repaint();
        });
        self.changelog = Some((version.to_string(), slot));
    }

    /// About: first the latest release (download + what changed, read here), then its release
    /// page, what the app is and the project website.
    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            self.about_notes = None;
            return;
        }
        let mine = env!("CARGO_PKG_VERSION");
        // One guard at a time on app_latest.
        let (latest, update) = {
            let g = self.app_latest.lock().unwrap();
            let latest = match &*g {
                Latest::Ok(v) => Some(v.clone()),
                _ => None,
            };
            (latest, github::app_update(&g))
        };
        // Newer than the latest release (a build being released, or a development build):
        // its notes are not published yet, so none are loaded.
        let ahead = update.is_none() && latest.as_deref().is_some_and(|l| l != mine);
        let target = update.clone().unwrap_or_else(|| mine.to_string());
        if ahead {
            self.about_notes = None;
        } else if self.about_notes.as_ref().is_none_or(|(v, _)| *v != target) {
            let slot = Arc::new(Mutex::new(None));
            let (out, v, c) = (slot.clone(), target.clone(), ctx.clone());
            std::thread::spawn(move || {
                *out.lock().unwrap() = Some(github::changelog(&v));
                c.request_repaint();
            });
            self.about_notes = Some((target.clone(), slot));
        }
        let slot = self.about_notes.as_ref().map(|(_, s)| s.clone());
        let newer = update.is_some();
        let mut open = true;
        egui::Window::new(concat!("Tasmota Derust ", env!("CARGO_PKG_VERSION")))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                if newer {
                    ui.label(
                        RichText::new(format!("Version {target} is available"))
                            .strong()
                            .color(ORANGE),
                    );
                } else if ahead {
                    ui.label(
                        RichText::new(format!(
                            "This version is newer than the latest release ({})",
                            latest.as_deref().unwrap_or("?")
                        ))
                        .strong(),
                    );
                    ui.hyperlink_to("Latest release", github::APP_RELEASES);
                } else if latest.is_some() {
                    ui.label(RichText::new("You have the latest version").strong());
                } else {
                    ui.label(RichText::new("This version").strong());
                }
                let notes = slot.as_ref().map(|s| s.lock().unwrap());
                match notes.as_deref() {
                    None => {}
                    Some(None) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading what changed…");
                        });
                    }
                    Some(Some(Err(e))) => {
                        ui.colored_label(RED, e.as_str());
                    }
                    Some(Some(Ok(c))) => {
                        match c.download_for(github::this_platform()) {
                            Some(d) => {
                                ui.hyperlink_to(
                                    RichText::new(format!("Download {}", d.name)).strong(),
                                    &d.url,
                                );
                            }
                            None => {
                                ui.label(
                                    RichText::new("No download for this system in this release.")
                                        .weak(),
                                );
                            }
                        }
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for sec in &c.sections {
                                    ui.label(RichText::new(&sec.title).strong());
                                    for item in &sec.items {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("•");
                                            ui.label(item.as_str());
                                        });
                                    }
                                    ui.add_space(4.0);
                                }
                            });
                        ui.hyperlink_to("Release page", &c.url);
                    }
                }
                ui.separator();
                for p in tasmota_derust_core::ABOUT.split("\n\n") {
                    ui.label(p);
                    ui.add_space(4.0);
                }
                ui.separator();
                ui.hyperlink_to(
                    "Website: screenshots, guides, all downloads",
                    tasmota_derust_core::SITE_URL,
                );
            });
        if !open {
            self.about_open = false;
        }
    }

    fn changelog_window(&mut self, ctx: &egui::Context) {
        let Some((version, slot)) = &self.changelog else {
            return;
        };
        let mut open = true;
        egui::Window::new(format!("Tasmota Derust {version}"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .show(ctx, |ui| match &*slot.lock().unwrap() {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading the release notes…");
                    });
                }
                Some(Err(e)) => {
                    ui.colored_label(RED, e.as_str());
                    ui.hyperlink_to(
                        "All releases",
                        "https://github.com/fragolinux/tasmota-derust/releases",
                    );
                }
                Some(Ok(c)) => {
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            if c.sections.is_empty() {
                                ui.label("No list of changes in these release notes.");
                            }
                            for sec in &c.sections {
                                ui.label(RichText::new(&sec.title).strong());
                                for item in &sec.items {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label("•");
                                        ui.label(item.as_str());
                                    });
                                }
                                ui.add_space(6.0);
                            }
                        });
                    ui.separator();
                    match c.download_for(github::this_platform()) {
                        Some(d) => {
                            ui.hyperlink_to(
                                RichText::new(format!("Download {}", d.name)).strong(),
                                &d.url,
                            );
                        }
                        None => {
                            ui.label(
                                RichText::new("No download for this system in this release.")
                                    .weak(),
                            );
                        }
                    }
                    ui.hyperlink_to("Release page on GitHub (all downloads, full notes)", &c.url);
                }
            });
        if !open {
            self.changelog = None;
        }
    }

    fn latest_version(&self) -> Option<String> {
        match &*self.latest.lock().unwrap() {
            Latest::Ok(v) => Some(v.clone()),
            _ => None,
        }
    }

    // ---------------------------------------------------------------- toolbar

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Tasmota Derust").strong().size(16.0));
            ui.separator();
            for v in View::ALL {
                if ui.selectable_label(self.cfg.view == v, v.label()).clicked()
                    && self.cfg.view != v
                {
                    self.cfg.view = v;
                    self.cfg.save();
                }
            }
            ui.separator();
            if ui
                .checkbox(&mut self.cfg.show_offline, "Show offline")
                .changed()
            {
                self.cfg.save();
            }
            let offline = self.offline_keys().len();
            if self.cfg.show_offline
                && offline > 0
                && ui
                    .button(format!("Remove offline ({offline})"))
                    .on_hover_text(
                        "Remove the offline devices from the list and clear what the broker keeps for them",
                    )
                    .clicked()
            {
                self.handle_actions(vec![UiAction::AskForgetOffline]);
            }
            self.columns_menu(ui);
            if ui
                .button("Refresh")
                .on_hover_text("Ask every online device for its full status")
                .clicked()
            {
                let a = self.store.refresh_all();
                self.apply(a);
            }
            ui.separator();
            self.read_only_toggle(ui);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.draft = self.cfg.broker.clone();
                    self.login_draft = LoginDraft::from(&self.cfg.web_login);
                    self.https_draft = self.cfg.web_https;
                    self.settings_open = true;
                }
                ui.separator();
                self.web_controls(ui);
                ui.separator();
                // Copied first: targets() takes the same lock.
                let latest = self.latest.lock().unwrap().clone();
                match self.cfg.view {
                    View::Wifi => {
                        // Right-to-left: add in reverse order.
                        for (i, s) in self.store.ssids.iter().enumerate().rev() {
                            ui.label(s.as_deref().unwrap_or("?"));
                            chip(ui, &format!("{}", i + 1), GRAY);
                            ui.add_space(8.0);
                        }
                    }
                    View::Firmware => match latest {
                        Latest::Ok(v) => {
                            // Right-to-left: the development build goes first.
                            if let Some(dev) = self.targets().development {
                                ui.hyperlink_to(
                                    format!("dev {dev}"),
                                    "https://github.com/arendst/Tasmota/commits/development",
                                );
                                ui.label("·");
                            }
                            ui.hyperlink_to(
                                format!("Latest {v}"),
                                "https://github.com/arendst/Tasmota/releases/latest",
                            );
                        }
                        Latest::Loading => {
                            ui.label("Checking latest release…");
                        }
                        Latest::Err(e) => {
                            ui.colored_label(RED, "Latest release unknown")
                                .on_hover_text(e.as_str());
                        }
                    },
                    _ => {}
                }
            });
        });
        ui.add_space(4.0);
    }

    fn columns_menu(&mut self, ui: &mut egui::Ui) {
        let view = self.cfg.view;
        let mut changed = false;
        let mut swap: Option<(usize, usize)> = None;
        let mut reset = false;
        let config = egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        egui::containers::menu::MenuButton::new("Columns")
            .config(config)
            .ui(ui, |ui| {
                ui.label(RichText::new(format!("Columns in {}", view.label())).strong());
                egui::ScrollArea::vertical()
                    .max_height(480.0)
                    .show(ui, |ui| {
                        if let Some(list) = self.cfg.columns.get_mut(&view) {
                            let n = list.len();
                            for i in 0..n {
                                ui.horizontal(|ui| {
                                    if arrow_button(ui, true, i > 0) {
                                        swap = Some((i, i - 1));
                                    }
                                    if arrow_button(ui, false, i + 1 < n) {
                                        swap = Some((i, i + 1));
                                    }
                                    let title = list[i].col.title();
                                    if ui.checkbox(&mut list[i].visible, title).changed() {
                                        changed = true;
                                    }
                                });
                            }
                        }
                    });
                ui.separator();
                if ui.button("Reset to defaults").clicked() {
                    reset = true;
                }
            });
        if let Some((a, b)) = swap {
            if let Some(list) = self.cfg.columns.get_mut(&view) {
                list.swap(a, b);
                changed = true;
            }
        }
        if reset {
            self.cfg.reset_columns(view);
            changed = true;
        }
        if changed {
            self.cfg.save();
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (text, color) = match &self.conn {
                Conn::NotConfigured => ("Broker not configured".to_string(), GRAY),
                Conn::Connecting => (format!("Connecting to {}…", self.cfg.broker.host), YELLOW),
                Conn::Connected => (
                    format!(
                        "Connected to {}:{}",
                        self.cfg.broker.host, self.cfg.broker.port
                    ),
                    GREEN,
                ),
                Conn::Error(e) => (format!("Disconnected: {e}"), RED),
            };
            dot(ui, color);
            ui.label(text);
            ui.separator();
            let total = self.store.devices.len();
            let online = self
                .store
                .devices
                .values()
                .filter(|d| d.is_online())
                .count();
            let offline = self
                .store
                .devices
                .values()
                .filter(|d| d.is_offline())
                .count();
            ui.label(format!(
                "{total} devices · {online} online · {offline} offline"
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.toggle_value(&mut self.log_open, "Log").on_hover_text(
                    "MQTT messages of all devices: received (<), sent (>) and connection events",
                );
                let me = ui
                    .link(concat!("About · v", env!("CARGO_PKG_VERSION")))
                    .on_hover_text("What Tasmota Derust is, what changed in this version");
                if me.clicked() {
                    self.about_open = !self.about_open;
                }
                let update = github::app_update(&self.app_latest.lock().unwrap());
                if let Some(v) = update {
                    let r = ui
                        .add(
                            egui::Label::new(
                                RichText::new(format!("v{v} available")).color(ORANGE),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_text("A newer Tasmota Derust is published: see what changed");
                    if r.clicked() {
                        self.open_changelog(ui.ctx(), &v);
                    }
                }
            });
        });
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.add(egui::TextEdit::singleline(&mut self.log_filter).desired_width(240.0));
            if ui.button("Clear").clicked() {
                self.log.clear();
            }
            ui.label(RichText::new("< received   > sent   * / ! connection").weak());
        });
        let filter = self.log_filter.to_lowercase();
        let lines: Vec<&LogLine> = self
            .log
            .iter()
            .filter(|l| {
                filter.is_empty()
                    || l.topic.to_lowercase().contains(&filter)
                    || l.payload.to_lowercase().contains(&filter)
            })
            .collect();
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink(false)
            .show_rows(ui, row_h, lines.len(), |ui, range| {
                for l in &lines[range] {
                    let color = match l.dir {
                        '>' => ORANGE,
                        '!' => RED,
                        '*' => GREEN,
                        _ => ui.visuals().text_color(),
                    };
                    ui.label(
                        RichText::new(format!("{} {} {}  {}", l.time, l.dir, l.topic, l.payload))
                            .monospace()
                            .color(color),
                    );
                }
            });
    }

    // ------------------------------------------------------------------ table

    fn table(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let view = self.cfg.view;
        let cols = self.cfg.visible_columns(view);
        let targets = self.targets();
        let info = CellCtx {
            targets: &targets,
            ssids: &self.store.ssids,
            sizes: &self.store.sizes,
            dim: false,
        };

        let mut rows: Vec<&Device> = self
            .store
            .devices
            .values()
            .filter(|d| self.cfg.show_offline || !d.is_offline() || is_upgrading(d))
            .collect();
        let sort = self.sort;
        rows.sort_by(|a, b| {
            let o = compare(a, b, sort);
            let o = if self.sort_desc { o.reverse() } else { o };
            o.then_with(|| {
                a.display_name()
                    .to_lowercase()
                    .cmp(&b.display_name().to_lowercase())
            })
        });

        if rows.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(match self.conn {
                    Conn::Connected => "Waiting for Tasmota devices…",
                    Conn::NotConfigured => "Open Settings to configure the MQTT broker.",
                    _ => "Not connected to the MQTT broker.",
                });
            });
            return;
        }

        let desc = self.sort_desc;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            let mut tb = TableBuilder::new(ui)
                .id_salt(view)
                .striped(true)
                .resizable(true)
                .auto_shrink(false)
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(TCol::initial(240.0).at_least(120.0).clip(true));
            for c in &cols {
                tb = tb.column(if *c == Col::Update {
                    // Upgrade progress texts vary in length: the column fits them.
                    TCol::auto().at_least(c.width())
                } else {
                    TCol::initial(c.width()).at_least(28.0).clip(true)
                });
            }
            tb.header(ROW_HEIGHT, |mut h| {
                h.col(|ui| header_cell(ui, "Device", sort.is_none(), desc, None, actions));
                for c in &cols {
                    h.col(|ui| {
                        header_cell(ui, c.title(), sort == Some(*c), desc, Some(*c), actions)
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, rows.len(), |mut row| {
                    let d = rows[row.index()];
                    let ci = CellCtx {
                        dim: d.is_offline(),
                        ..info_ref(&info)
                    };
                    let new = d.is_new();
                    row.col(|ui| {
                        if new {
                            new_row_tint(ui);
                        }
                        device_cell(ui, d, actions)
                    });
                    for c in &cols {
                        row.col(|ui| {
                            if new {
                                new_row_tint(ui);
                            }
                            cell(ui, d, *c, &ci, actions)
                        });
                    }
                });
            });
        });
    }

    fn handle_actions(&mut self, actions: Vec<UiAction>) {
        for a in actions {
            match a {
                UiAction::Toggle(..)
                | UiAction::AskUpgrade(_)
                | UiAction::AskAp(..)
                | UiAction::SetChannel(..)
                | UiAction::AskForget(_)
                | UiAction::AskForgetOffline
                    if self.read_only =>
                {
                    self.show_notice(READ_ONLY_NOTICE);
                }
                UiAction::Toggle(key, i) => {
                    if let Some(x) = self
                        .store
                        .command(&key, &format!("POWER{}", i + 1), "TOGGLE")
                    {
                        self.apply(vec![x]);
                    }
                }
                UiAction::SetChannel(key, dev) => {
                    let url = self
                        .store
                        .devices
                        .get(&key)
                        .and_then(|d| d.ota_url.as_deref())
                        .and_then(|u| ota::channel_url(u, dev));
                    if let Some(x) = url.and_then(|u| self.store.command(&key, "OTAURL", &u)) {
                        self.apply(vec![x]);
                    }
                }
                UiAction::OpenLight(key) => {
                    if let Some(l) = self.store.devices.get(&key).and_then(|d| d.light.as_ref()) {
                        self.light_open = Some((key, LightEdit::from(l)));
                    }
                }
                UiAction::OpenOptions(key) => {
                    setoption::load_texts(self.repaint.clone());
                    self.options_open = Some(OptionsWindow {
                        key,
                        search: String::new(),
                        only_changed: false,
                        drafts: Default::default(),
                    });
                }
                UiAction::OpenTools(key, pos) => self.tools_open = Some((key, pos, true)),
                UiAction::OpenWifi(key) => {
                    self.wifi_open = Some(WifiWindow {
                        key,
                        ssid: String::new(),
                        password: String::new(),
                        confirm: false,
                    })
                }
                UiAction::OpenBackup(key) => {
                    self.backup_open = Some(BackupWindow {
                        key,
                        password: String::new(),
                        chosen: None,
                        confirm: false,
                        job: None,
                        result: None,
                        saved: None,
                    })
                }
                UiAction::OpenWeb(url) => {
                    let _ = open::that(url);
                }
                UiAction::AskUpgrade(key) => self.confirm = Some(Confirm::Upgrade(key)),
                UiAction::AskForget(key) => self.confirm = Some(Confirm::Forget(key)),
                UiAction::AskForgetOffline => self.confirm = Some(Confirm::ForgetOffline),
                UiAction::AskAp(key, n) => self.confirm = Some(Confirm::SwitchAp(key, n)),
                UiAction::Sort(c) => {
                    if self.sort == c {
                        self.sort_desc = !self.sort_desc;
                    } else {
                        self.sort = c;
                        self.sort_desc = false;
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- dialogs

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut connect = false;
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                // Applied and saved at once, unlike the broker below.
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Theme").strong());
                    let before = self.cfg.theme.clone();
                    for (k, t) in [("system", "System"), ("light", "Light"), ("dark", "Dark")] {
                        ui.selectable_value(&mut self.cfg.theme, k.to_string(), t);
                    }
                    if self.cfg.theme != before {
                        ui.ctx().set_theme(theme_pref(&self.cfg.theme));
                        self.cfg.save();
                    }
                });
                ui.separator();
                ui.label(RichText::new("MQTT broker").strong());
                ui.add_space(4.0);
                egui::Grid::new("broker")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Host");
                        ui.text_edit_singleline(&mut self.draft.host);
                        ui.end_row();
                        ui.label("Port");
                        ui.add(egui::DragValue::new(&mut self.draft.port).range(1..=65535));
                        ui.end_row();
                        ui.label("Username");
                        ui.text_edit_singleline(&mut self.draft.username);
                        ui.end_row();
                        ui.label("Password");
                        ui.add(egui::TextEdit::singleline(&mut self.draft.password).password(true));
                        ui.end_row();
                    });
                ui.separator();
                ui.label(RichText::new("Web server").strong());
                ui.checkbox(
                    &mut self.https_draft,
                    "HTTPS (self-signed certificate, port 8443)",
                )
                .on_hover_text(
                    "Encrypts the traffic. The certificate is made once and kept next to the \
                     config file, shared with tasmota-derust-web and tasmota-derust-web-rw. \
                     Browsers warn on the first visit, since no authority signed it: compare \
                     the fingerprint shown in the toolbar before accepting.",
                );
                if !self.https_draft {
                    ui.label(
                        RichText::new("Plain HTTP on port 8080: no encryption.")
                            .small()
                            .weak(),
                    );
                }
                ui.add_space(4.0);
                ui.add_space(4.0);
                let had_login = self.cfg.web_login.is_set();
                ui.label(if had_login {
                    format!("Login: set for \"{}\".", self.cfg.web_login.username)
                } else {
                    "Login: none.".to_string()
                });
                let d = &mut self.login_draft;
                ui.add_enabled_ui(!d.remove, |ui| {
                    egui::Grid::new("web_login")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            ui.label("Username");
                            ui.text_edit_singleline(&mut d.username);
                            ui.end_row();
                            ui.label(if had_login {
                                "New password"
                            } else {
                                "Password"
                            });
                            ui.add(
                                egui::TextEdit::singleline(&mut d.password)
                                    .password(true)
                                    .hint_text(if had_login { "unchanged" } else { "" }),
                            );
                            ui.end_row();
                        });
                });
                if had_login {
                    ui.checkbox(&mut d.remove, "Remove the login");
                }
                let problem = d.problem(had_login);
                if let Some(p) = problem {
                    ui.colored_label(RED, p);
                }
                ui.label(
                    RichText::new(
                        "Asked by the browser for every visitor, also by tasmota-derust-web and \
                         tasmota-derust-web-rw, which read this same config. Only a salted hash \
                         of the password is saved, so it cannot be read back: type a new one to \
                         change it. Without HTTPS it travels in clear text.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "The broker password is saved in plain text in the app's config file \
                         (it must be sent to the broker as is).",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(4.0);
                if ui
                    .add_enabled(problem.is_none(), egui::Button::new("Save and connect"))
                    .clicked()
                {
                    connect = true;
                }
            });
        if connect {
            self.cfg.broker = self.draft.clone();
            self.cfg.web_login = self.login_draft.apply(&self.cfg.web_login);
            self.login_draft = LoginDraft::from(&self.cfg.web_login);
            self.cfg.web_https = self.https_draft;
            self.cfg.save();
            self.connect(ctx);
            if let Some(read_only) = self.web.as_ref().map(|w| w.read_only()) {
                // Follow the new broker and login, in the same mode.
                self.start_web(read_only);
            }
            self.settings_open = false;
        } else if !open {
            self.settings_open = false;
        }
    }

    /// Removes an offline device; the actions clear its retained messages on the broker.
    fn forget_actions(&mut self, key: &str) -> Vec<Action> {
        match self.store.forget(key) {
            Some(a) => {
                self.push_log('*', "removed", key);
                a
            }
            None => {
                self.show_notice("Only offline devices can be removed.");
                Vec::new()
            }
        }
    }

    fn offline_keys(&self) -> Vec<String> {
        self.store
            .devices
            .iter()
            .filter(|(_, d)| d.reachable() == Some(false))
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn forget_offline_dialog(&mut self, ctx: &egui::Context) {
        let keys = self.offline_keys();
        if keys.is_empty() {
            self.confirm = None;
            return;
        }
        let mut names: Vec<String> = keys
            .iter()
            .take(12)
            .map(|k| self.store.devices[k].display_name().to_string())
            .collect();
        if keys.len() > 12 {
            names.push(format!("… ({} more)", keys.len() - 12));
        }
        let mut result = None;
        let resp = egui::Modal::new(egui::Id::unique("confirm-forget")).show(ctx, |ui| {
            ui.set_max_width(520.0);
            let n = keys.len();
            ui.heading(format!(
                "Remove {n} offline device{}?",
                if n > 1 { "s" } else { "" }
            ));
            ui.add_space(6.0);
            ui.label(names.join(", "));
            ui.add_space(6.0);
            ui.label(FORGET_TEXT);
            ui.add_space(6.0);
            ui.label(RichText::new(FORGET_BACK).weak());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Remove all").strong()).clicked() {
                    result = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    result = Some(false);
                }
            });
        });
        if result.is_none() && resp.should_close() {
            result = Some(false);
        }
        match result {
            Some(true) if self.read_only => {
                self.confirm = None;
                self.show_notice(READ_ONLY_NOTICE);
            }
            Some(true) => {
                self.confirm = None;
                let actions: Vec<Action> =
                    keys.iter().flat_map(|k| self.forget_actions(k)).collect();
                self.apply(actions);
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(confirm) = &self.confirm else { return };
        let key = match confirm {
            Confirm::Upgrade(k)
            | Confirm::SwitchAp(k, _)
            | Confirm::SetOption(k, ..)
            | Confirm::Forget(k) => k.clone(),
            Confirm::ForgetOffline => return self.forget_offline_dialog(ctx),
        };
        let Some(d) = self.store.devices.get(&key) else {
            self.confirm = None;
            return;
        };
        let latest = self.latest_version();
        let targets = self.targets();
        let mut result: Option<bool> = None;
        let mut ok_enabled = true;

        let resp = egui::Modal::new(egui::Id::unique("confirm")).show(ctx, |ui| {
            ui.set_max_width(520.0);
            let ok_label = match confirm {
                Confirm::Upgrade(_) => {
                    let plan = ota::plan_info(d, &targets, &self.store.sizes);
                    ui.heading(format!("Upgrade {}?", d.display_name()));
                    ui.label(RichText::new(arrows(&plan.title)).strong());
                    ui.add_space(6.0);
                    egui::Grid::new("upg").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                        ui.label("Topic");
                        ui.label(&d.topic);
                        ui.end_row();
                        ui.label("Current firmware");
                        ui.label(d.version.as_deref().unwrap_or("unknown"));
                        ui.end_row();
                        ui.label("Latest release");
                        ui.label(latest.as_deref().unwrap_or("unknown"));
                        ui.end_row();
                        if let Some(dev) = &targets.development {
                            ui.label("Latest development");
                            ui.label(dev.as_str());
                            ui.end_row();
                        }
                        ui.label("Firmware image");
                        ui.label(plan.url.as_deref().or(d.ota_url.as_deref()).unwrap_or("unknown"));
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    for line in &plan.details {
                        ui.label(arrows(line));
                    }
                    ui.label(
                        "The device downloads the firmware and restarts. Do not power it off during \
                         the upgrade.",
                    );
                    for w in &plan.warnings {
                        ui.add_space(4.0);
                        ui.colored_label(ORANGE, arrows(w));
                    }
                    if plan.kind == "step" || plan.kind == "manual" {
                        ui.hyperlink_to(
                            "Tasmota upgrade path",
                            "https://tasmota.github.io/docs/Upgrading/#upgrade-flow",
                        );
                    }
                    if !plan.sets_ota_url && d.ota_url.as_deref().is_some_and(|u| !u.contains("ota.tasmota.com")) {
                        ui.add_space(6.0);
                        ui.colored_label(ORANGE, "This device uses a custom OtaUrl.");
                    }
                    ok_enabled = plan.can_start;
                    if plan.kind == "step" { "Start this step" } else { "Upgrade" }
                }
                Confirm::SwitchAp(_, to) => {
                    let name = |n: i64| {
                        self.store.ssids[(n - 1).clamp(0, 1) as usize].clone().unwrap_or_else(|| "?".into())
                    };
                    let from = d.ap.unwrap_or(1);
                    ui.heading(format!("Switch {} to SSId {}?", d.display_name(), to));
                    ui.add_space(6.0);
                    ui.label(format!("Now connected to SSId {from} ({}).", name(from)));
                    ui.label(format!("It will restart and connect to SSId {to} ({}).", name(*to)));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("If it cannot connect there, Tasmota tries the other network again.")
                            .weak(),
                    );
                    "Restart and switch"
                }
                Confirm::SetOption(_, n, value) => {
                    let view = setoption::view(&d.set_options);
                    let o = view.options.iter().find(|o| o.n == *n);
                    ui.heading(format!("Change SetOption{n} on {}?", d.display_name()));
                    ui.add_space(6.0);
                    if let Some(o) = o {
                        ui.label(RichText::new(&o.title).strong());
                        for l in &o.details {
                            ui.label(RichText::new(l).weak());
                        }
                        ui.add_space(6.0);
                        ui.label(format!(
                            "From {} to {}.",
                            option_value_text(o, o.value),
                            option_value_text(o, *value)
                        ));
                        if let Some(w) = o.warning {
                            ui.add_space(4.0);
                            ui.colored_label(if o.risk == Risk::Danger { RED } else { ORANGE }, w);
                        }
                        ui.hyperlink_to("Documentation", &o.link);
                    } else {
                        ui.label(format!("New value: {value}"));
                    }
                    "Change"
                }
                Confirm::Forget(_) => {
                    ui.heading(format!("Remove {}?", d.display_name()));
                    ui.add_space(6.0);
                    ui.label(FORGET_TEXT);
                    ui.add_space(6.0);
                    ui.label(RichText::new(FORGET_BACK).weak());
                    "Remove"
                }
                Confirm::ForgetOffline => unreachable!("own dialog"),
            };
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(ok_enabled, egui::Button::new(RichText::new(ok_label).strong())).clicked() {
                    result = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    result = Some(false);
                }
            });
        });
        if result.is_none() && resp.should_close() {
            result = Some(false);
        }

        match result {
            Some(true) if self.read_only => {
                // Read-only was ticked while the dialog was open.
                self.confirm = None;
                self.show_notice(READ_ONLY_NOTICE);
            }
            Some(true) => {
                let actions = match self.confirm.take() {
                    Some(Confirm::Upgrade(k)) => match self.store.start_upgrade(&k, &targets) {
                        Ok(a) => a,
                        Err(e) => {
                            self.show_notice(&match e {
                                StartError::OtherRunning(name) => {
                                    format!("{name} is being upgraded: one device at a time.")
                                }
                                StartError::NotPossible(why) => why,
                                StartError::Running => "This device is already upgrading.".into(),
                                StartError::Offline | StartError::Unknown => {
                                    "The device is not online.".into()
                                }
                            });
                            Vec::new()
                        }
                    },
                    Some(Confirm::SwitchAp(k, n)) => self
                        .store
                        .command(&k, "AP", &n.to_string())
                        .into_iter()
                        .collect(),
                    Some(Confirm::SetOption(k, n, v)) => self.set_option_actions(&k, n, v),
                    Some(Confirm::Forget(k)) => self.forget_actions(&k),
                    Some(Confirm::ForgetOffline) | None => Vec::new(),
                };
                self.apply(actions);
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }
}

impl eframe::App for DerustApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_events();
        let again = self.store.check_timeouts();
        self.apply(again);
        let ctx = ui.ctx().clone();
        #[cfg(feature = "screenshots")]
        self.screenshot_tick(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.log_open {
            egui::Panel::bottom("log")
                .resizable(true)
                .default_size(200.0)
                .show(ui, |ui| self.log_panel(ui));
        }
        // Stars of devices that just came online fade out by themselves.
        if self.store.devices.values().any(|d| d.is_new()) {
            ctx.request_repaint_after(Duration::from_secs(5));
        }
        let mut actions = Vec::new();
        egui::CentralPanel::default().show(ui, |ui| self.table(ui, &mut actions));
        self.handle_actions(actions);

        self.settings_window(&ctx);
        self.web_start_window(&ctx);
        self.confirm_dialog(&ctx);
        self.notice_overlay(&ctx);
        self.about_window(&ctx);
        self.changelog_window(&ctx);
        self.light_window(&ctx);
        self.options_window(&ctx);
        self.tools_menu(&ctx);
        self.wifi_window(&ctx);
        self.backup_window(&ctx);

        // Keeps the uptime columns ticking.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

// -------------------------------------------------------------------- helpers

fn theme_pref(name: &str) -> egui::ThemePreference {
    match name {
        "light" => egui::ThemePreference::Light,
        "dark" => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    }
}

fn info_ref<'a>(i: &CellCtx<'a>) -> CellCtx<'a> {
    CellCtx {
        targets: i.targets,
        ssids: i.ssids,
        sizes: i.sizes,
        dim: i.dim,
    }
}

fn rssi_color(rssi: i64) -> Color32 {
    if rssi >= 85 {
        GREEN
    } else if rssi >= 50 {
        YELLOW
    } else {
        RED
    }
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

fn chip(ui: &mut egui::Ui, text: &str, fill: Color32) -> egui::Response {
    egui::Frame::NONE
        .fill(fill)
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(5, 0))
        .show(ui, |ui| ui.label(RichText::new(text).color(Color32::BLACK)))
        .response
}

/// Fills the cell with a colored box and centered text.
fn boxed(ui: &mut egui::Ui, text: &str, fill: Color32, sense: Sense) -> egui::Response {
    let size = vec2(ui.available_width(), ROW_HEIGHT - 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    ui.painter()
        .rect_filled(rect.shrink2(vec2(2.0, 1.0)), 2.0, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        Color32::BLACK,
    );
    resp
}

fn header_cell(
    ui: &mut egui::Ui,
    title: &str,
    active: bool,
    desc: bool,
    col: Option<Col>,
    actions: &mut Vec<UiAction>,
) {
    let r = ui.add(egui::Button::new(RichText::new(title).strong()).frame(false));
    if active {
        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
        triangle(ui, rect, !desc, ui.visuals().strong_text_color());
    }
    if r.clicked() {
        actions.push(UiAction::Sort(col));
    }
}

/// Small filled triangle pointing up or down (the default font has no arrow glyphs).
fn triangle(ui: &egui::Ui, rect: egui::Rect, up: bool, color: Color32) {
    let c = rect.center();
    let (w, h) = (rect.width() * 0.4, rect.height() * 0.3);
    let pts = if up {
        vec![c + vec2(0.0, -h), c + vec2(w, h), c + vec2(-w, h)]
    } else {
        vec![c + vec2(-w, -h), c + vec2(w, -h), c + vec2(0.0, h)]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Tiny up/down button used to reorder columns.
fn arrow_button(ui: &mut egui::Ui, up: bool, enabled: bool) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), sense);
    let color = if !enabled {
        GRAY.gamma_multiply(0.4)
    } else if resp.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    triangle(ui, rect, up, color);
    resp.clicked()
}

fn signal_icon(ui: &mut egui::Ui, d: &Device) {
    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 16.0), Sense::hover());
    let p = ui.painter();
    if d.is_offline() {
        let c = rect.center();
        p.circle_filled(c, 6.0, RED);
        let s = Stroke::new(1.6, Color32::WHITE);
        p.line_segment([c + vec2(-2.8, -2.8), c + vec2(2.8, 2.8)], s);
        p.line_segment([c + vec2(-2.8, 2.8), c + vec2(2.8, -2.8)], s);
        return;
    }
    let (level, color) = match d.rssi {
        Some(r) => (
            match r {
                75.. => 4,
                50.. => 3,
                25.. => 2,
                _ => 1,
            },
            rssi_color(r),
        ),
        None => (0, GRAY),
    };
    for i in 0..4 {
        let h = 4.0 + i as f32 * 3.0;
        let x = rect.left() + 1.0 + i as f32 * 4.0;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x, rect.bottom() - 1.0 - h),
            egui::pos2(x + 3.0, rect.bottom() - 1.0),
        );
        p.rect_filled(
            bar,
            0.0,
            if i < level {
                color
            } else {
                GRAY.gamma_multiply(0.5)
            },
        );
    }
}

fn device_cell(ui: &mut egui::Ui, d: &Device, actions: &mut Vec<UiAction>) {
    signal_icon(ui, d);
    if d.online != Some(false) {
        let r = gear_button(ui).on_hover_text("SetOptions, Wi-Fi networks, backup / restore");
        if r.clicked() {
            actions.push(UiAction::OpenTools(d.topic.clone(), r.rect.left_bottom()));
        }
    }
    if d.reachable() == Some(false)
        && remove_button(ui)
            .on_hover_text("Remove this offline device from the list")
            .clicked()
    {
        actions.push(UiAction::AskForget(d.topic.clone()));
    }
    let name = d.display_name();
    if d.is_new() {
        star_icon(ui).on_hover_text("Came online in the last 5 minutes");
    }
    if let Some(label) = d.offline_label() {
        let why = if label == "No answer" {
            "did not answer the status query (its LWT may be stale)"
        } else {
            "is offline"
        };
        ui.label(RichText::new(name).italics().weak())
            .on_hover_text(format!("{} {why}", d.topic));
    } else if let Some(url) = d.web_url() {
        let r = ui.link(name).on_hover_text(format!("Open web UI: {url}"));
        if r.clicked() {
            actions.push(UiAction::OpenWeb(url));
        }
    } else {
        ui.label(name);
    }
}

fn is_upgrading(d: &Device) -> bool {
    d.upgrade.as_ref().is_some_and(|u| u.is_active())
}

/// Release notes being loaded in the background (None until they arrive).
type ChangelogSlot = Arc<Mutex<Option<Result<github::Changelog, String>>>>;

/// Values being edited in the light window.
#[derive(Clone, Copy)]
struct LightEdit {
    hue: i64,
    sat: i64,
    dimmer: i64,
    ct: i64,
    white: i64,
}

impl From<&light::Light> for LightEdit {
    fn from(l: &light::Light) -> Self {
        let (hue, sat, _) = l.hsb.unwrap_or((0, 0, 100));
        LightEdit {
            hue,
            sat,
            dimmer: l.dimmer.unwrap_or(100),
            ct: l.ct.unwrap_or(326),
            white: l.white.unwrap_or(0),
        }
    }
}

/// A small clickable colour square.
fn swatch(ui: &mut egui::Ui, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 16.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, color);
    p.rect_stroke(rect, 3.0, Stroke::new(1.0, GRAY), egui::StrokeKind::Inside);
    resp
}

/// HSV colour wheel (hue around, saturation from the centre). Returns the picked
/// (hue, saturation) when the user clicks or ends a drag.
fn color_wheel(ui: &mut egui::Ui, hue: i64, sat: i64, size: f32) -> Option<(i64, i64)> {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let c = rect.center();
    let radius = size / 2.0 - 2.0;
    let mut mesh = egui::epaint::Mesh::default();
    let (segs, rings) = (90, 12);
    let col = |h: f32, s: f32| {
        let (r, g, b) = light::hsv_to_rgb(h, s, 1.0);
        Color32::from_rgb(r, g, b)
    };
    for ring in 0..=rings {
        let s = ring as f32 / rings as f32;
        for seg in 0..=segs {
            let h = seg as f32 * 360.0 / segs as f32;
            let a = h.to_radians();
            let pos = c + vec2(a.cos(), -a.sin()) * radius * s;
            mesh.colored_vertex(pos, col(h, s));
        }
    }
    let row = segs as u32 + 1;
    for ring in 0..rings as u32 {
        for seg in 0..segs as u32 {
            let i = ring * row + seg;
            mesh.add_triangle(i, i + 1, i + row);
            mesh.add_triangle(i + 1, i + row + 1, i + row);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    // Marker at the current colour.
    let a = (hue as f32).to_radians();
    let at = c + vec2(a.cos(), -a.sin()) * radius * (sat as f32 / 100.0);
    ui.painter()
        .circle_stroke(at, 6.0, Stroke::new(2.0, Color32::WHITE));
    ui.painter()
        .circle_stroke(at, 7.5, Stroke::new(1.0, Color32::BLACK));

    let pick = |p: egui::Pos2| light::wheel_pick(p.x - c.x, p.y - c.y, radius);
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() {
            // Live marker while dragging.
            let (h, s) = pick(p);
            let a = (h as f32).to_radians();
            let at = c + vec2(a.cos(), -a.sin()) * radius * (s as f32 / 100.0);
            ui.painter()
                .circle_filled(at, 5.0, col(h as f32, s as f32 / 100.0));
        }
        if resp.clicked() || resp.drag_stopped() {
            return Some(pick(p));
        }
    }
    None
}

/// The bundled fonts have no "→": draw it as "->".
fn arrows(s: &str) -> String {
    s.replace('→', "->")
}

fn opt<T: ToString>(v: &Option<T>) -> String {
    v.as_ref().map(|x| x.to_string()).unwrap_or_default()
}

/// Plain-text value of a column (also used for sorting).
fn cell_text(d: &Device, col: Col) -> String {
    match col {
        Col::Power => d
            .power
            .iter()
            .filter(|p| **p == Some(true))
            .count()
            .to_string(),
        Col::Telemetry => d.telemetry().unwrap_or_default(),
        Col::Module => opt(&d.module),
        Col::Uptime => d
            .current_uptime()
            .map(device::format_duration)
            .unwrap_or_default(),
        Col::LoadAvg => opt(&d.load_avg),
        Col::LinkCount => opt(&d.link_count),
        Col::BootCount => opt(&d.boot_count),
        Col::RestartReason => opt(&d.restart_reason),
        Col::Sleep => opt(&d.sleep),
        Col::MqttCount => opt(&d.mqtt_count),
        Col::Downtime => d
            .downtime
            .as_deref()
            .map(|s| {
                device::parse_duration(s)
                    .map(device::format_duration)
                    .unwrap_or_else(|| s.to_string())
            })
            .unwrap_or_default(),
        Col::Rssi => opt(&d.rssi),
        Col::Version => d
            .version
            .as_deref()
            .map(device::short_version)
            .unwrap_or_default()
            .to_string(),
        Col::Core => d
            .core
            .as_deref()
            .map(|c| c.replace('_', "."))
            .unwrap_or_default(),
        Col::Sdk => opt(&d.sdk),
        Col::ProgramSize => opt(&d.program_size),
        Col::Free => opt(&d.free),
        Col::OtaUrl => opt(&d.ota_url),
        Col::Update => String::new(),
        Col::Hostname => opt(&d.hostname),
        Col::Mac => opt(&d.mac),
        Col::Ip => opt(&d.ip),
        Col::Gateway => opt(&d.gateway),
        Col::Ssid => opt(&d.ssid),
        Col::Ap => opt(&d.ap),
        Col::Bssid => opt(&d.bssid),
        Col::Channel => opt(&d.channel),
        Col::Topic => d.topic.clone(),
        Col::FullTopic => d.full_topic.clone(),
        Col::CommandTopic => d.cmnd_prefix.trim_end_matches('/').to_string(),
        Col::StatTopic => d.stat_prefix.trim_end_matches('/').to_string(),
        Col::TeleTopic => d.tele_prefix.trim_end_matches('/').to_string(),
        Col::FallbackTopic => d.fallback_topic().unwrap_or_default(),
        Col::GroupTopic => opt(&d.group_topic),
    }
}

fn sort_num(d: &Device, col: Col) -> Option<f64> {
    let n = |v: Option<i64>| v.map(|x| x as f64);
    match col {
        Col::Uptime => d.current_uptime().map(|x| x as f64),
        Col::Downtime => d
            .downtime
            .as_deref()
            .and_then(device::parse_duration)
            .map(|x| x as f64),
        Col::LoadAvg => n(d.load_avg),
        Col::LinkCount => n(d.link_count),
        Col::BootCount => n(d.boot_count),
        Col::Sleep => n(d.sleep),
        Col::MqttCount => n(d.mqtt_count),
        Col::Rssi => n(d.rssi),
        Col::ProgramSize => n(d.program_size),
        Col::Free => n(d.free),
        Col::Channel => n(d.channel),
        Col::Ap => n(d.ap),
        Col::Power => Some(d.power.iter().filter(|p| **p == Some(true)).count() as f64),
        Col::Telemetry => d
            .telemetry()
            .and_then(|t| t.split_whitespace().next().and_then(|x| x.parse().ok())),
        _ => None,
    }
}

fn compare(a: &Device, b: &Device, col: Option<Col>) -> Ordering {
    let Some(col) = col else {
        return a
            .display_name()
            .to_lowercase()
            .cmp(&b.display_name().to_lowercase());
    };
    match (sort_num(a, col), sort_num(b, col)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => {
            let (x, y) = (cell_text(a, col), cell_text(b, col));
            match (x.is_empty(), y.is_empty()) {
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                _ => x.to_lowercase().cmp(&y.to_lowercase()),
            }
        }
    }
}

fn text_label(ui: &mut egui::Ui, text: String, numeric: bool, dim: bool) -> egui::Response {
    let rt = if dim {
        RichText::new(text).weak()
    } else {
        RichText::new(text)
    };
    if numeric {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(rt))
            .inner
    } else {
        ui.label(rt)
    }
}

fn cell(ui: &mut egui::Ui, d: &Device, col: Col, ci: &CellCtx<'_>, actions: &mut Vec<UiAction>) {
    let online = d.is_online();
    match col {
        Col::Power => {
            ui.spacing_mut().item_spacing.x = 3.0;
            if let Some(l) = &d.light {
                let (r, g, b) = l.swatch();
                let resp = swatch(ui, Color32::from_rgb(r, g, b));
                let resp = resp.on_hover_text(format!(
                    "{} light: click for colour and brightness",
                    l.kind()
                ));
                if resp.clicked() {
                    actions.push(UiAction::OpenLight(d.topic.clone()));
                }
            }
            for (i, st) in d.power.iter().enumerate() {
                let label = format!("{}", i + 1);
                let btn = match st {
                    Some(true) => {
                        egui::Button::new(RichText::new(label).color(Color32::BLACK).strong())
                            .fill(GREEN)
                    }
                    Some(false) => egui::Button::new(RichText::new(label)),
                    None => egui::Button::new(RichText::new(label).weak()),
                }
                .min_size(vec2(24.0, 20.0));
                let state = match st {
                    Some(true) => "ON",
                    Some(false) => "OFF",
                    None => "unknown",
                };
                let r = ui
                    .add_enabled(online, btn)
                    .on_hover_text(format!("Relay {}: {state}. Click to toggle.", i + 1));
                if r.clicked() {
                    actions.push(UiAction::Toggle(d.topic.clone(), i));
                }
            }
        }
        Col::Rssi => {
            if let Some(r) = d.rssi.filter(|_| !ci.dim) {
                let signal = format!("RSSI {r}%");
                boxed(ui, &r.to_string(), rssi_color(r), Sense::hover()).on_hover_text(signal);
            }
        }
        Col::Ap => {
            let Some(ap) = d.ap.filter(|_| !ci.dim) else {
                return;
            };
            let has_second = ci.ssids[1].as_deref().is_some_and(|s| !s.is_empty());
            let other = if ap == 1 { 2 } else { 1 };
            let size = vec2(ui.available_width(), ROW_HEIGHT - 4.0);
            let sense = if online && has_second {
                Sense::click()
            } else {
                Sense::hover()
            };
            let (rect, resp) = ui.allocate_exact_size(size, sense);
            let n = if has_second { 2 } else { 1 };
            let w = 20.0;
            let start = rect.center().x - (n as f32 * w + (n - 1) as f32 * 3.0) / 2.0;
            for k in 0..n {
                let num = k as i64 + 1;
                let fill = if num == ap { GREEN } else { RED };
                let x = start + k as f32 * (w + 3.0);
                let r = egui::Rect::from_min_size(
                    egui::pos2(x, rect.top() + 1.0),
                    vec2(w, rect.height() - 2.0),
                );
                ui.painter().rect_filled(r, 2.0, fill);
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    num.to_string(),
                    FontId::proportional(14.0),
                    Color32::BLACK,
                );
            }
            let name = |i: i64| {
                ci.ssids[(i - 1).clamp(0, 1) as usize]
                    .clone()
                    .unwrap_or_default()
            };
            let resp = if has_second {
                resp.on_hover_text(format!(
                    "Connected to SSId {ap} ({}). Click to switch to SSId {other} ({}).",
                    name(ap),
                    name(other)
                ))
            } else {
                resp.on_hover_text(format!("Connected to SSId {ap} ({})", name(ap)))
            };
            if resp.clicked() {
                actions.push(UiAction::AskAp(d.topic.clone(), other));
            }
        }
        Col::Telemetry => {
            if let Some(t) = d.telemetry() {
                let r = text_label(ui, t, false, ci.dim);
                if let Some(s) = &d.sensor {
                    let pretty = serde_json::to_string_pretty(s).unwrap_or_default();
                    r.on_hover_ui(|ui| {
                        ui.label(RichText::new(pretty).monospace());
                    });
                }
            }
        }
        Col::Version => {
            let text = cell_text(d, col);
            let st = ota::version_status(d.version.as_deref(), d.ota_url.as_deref(), ci.targets);
            if st.outdated && !ci.dim {
                let which = if st.channel_dev {
                    "development build"
                } else {
                    "release"
                };
                ui.label(RichText::new(text).color(ORANGE).strong())
                    .on_hover_text(format!(
                        "{} — latest {which} is {}",
                        d.version.as_deref().unwrap_or(""),
                        st.target.as_deref().unwrap_or("?")
                    ));
            } else {
                text_label(ui, text, false, ci.dim)
                    .on_hover_text(d.version.as_deref().unwrap_or(""));
            }
        }
        Col::Update => {
            let st = ota::version_status(d.version.as_deref(), d.ota_url.as_deref(), ci.targets);
            let outdated = st.outdated;
            let busy = is_upgrading(d);
            let plan = ota::plan_info(d, ci.targets, ci.sizes);
            // Release ↔ development: changes the OtaUrl only; the upgrade button then shows
            // whether there is something to install.
            // Fixed-width slot, also when there is no channel button, so what follows
            // starts at the same place on every row.
            const CHANNEL_W: f32 = 62.0;
            if !busy {
                let dev = ota::channel(d.ota_url.as_deref()) == ota::Channel::Development;
                let url = d.ota_url.as_deref().and_then(|u| ota::channel_url(u, !dev));
                if url.is_none() {
                    ui.add_space(CHANNEL_W + ui.spacing().item_spacing.x);
                }
                if let Some(url) = url {
                    let label = if dev { "Dev" } else { "Release" };
                    let hint = format!(
                        "OtaUrl points to the {} builds. Click to use the {} builds:\n{url}",
                        if dev { "development" } else { "release" },
                        if dev { "release" } else { "development" },
                    );
                    if ui
                        .add_enabled(
                            online,
                            egui::Button::new(label)
                                .small()
                                .min_size(vec2(CHANNEL_W, 0.0)),
                        )
                        .on_hover_text(hint)
                        .clicked()
                    {
                        actions.push(UiAction::SetChannel(d.topic.clone(), !dev));
                    }
                }
            }
            if !busy && st.up_to_date && plan.kind == "final" {
                let which = if st.channel_dev {
                    "development build"
                } else {
                    "release"
                };
                ui.label(RichText::new("Up to date").weak())
                    .on_hover_text(format!(
                        "Runs the latest {which} ({})",
                        st.target.as_deref().unwrap_or("?")
                    ));
            } else if !busy {
                let label = plan.button.as_str();
                let btn = if outdated || plan.kind == "step" {
                    egui::Button::new(RichText::new(label).color(Color32::BLACK)).fill(ORANGE)
                } else {
                    egui::Button::new(label)
                };
                let title = arrows(&plan.title);
                let hint = if plan.kind == "step" || st.downgrade {
                    title.as_str()
                } else if outdated {
                    "A newer version is available"
                } else {
                    "Upgrade from OtaUrl"
                };
                if ui.add_enabled(online, btn).on_hover_text(hint).clicked() {
                    actions.push(UiAction::AskUpgrade(d.topic.clone()));
                }
            }
            if let Some(u) = &d.upgrade {
                match &u.phase {
                    UpgradePhase::Sent => {
                        ui.spinner();
                        ui.label("Command sent…");
                    }
                    UpgradePhase::Progress(m) => {
                        ui.spinner();
                        ui.label(arrows(m)).on_hover_text(arrows(m));
                    }
                    UpgradePhase::Restarting => {
                        ui.spinner();
                        ui.label("Restarting…");
                    }
                    UpgradePhase::Done(m) => {
                        ui.colored_label(GREEN, arrows(m)).on_hover_text(arrows(m));
                    }
                    UpgradePhase::Failed(m) => {
                        ui.colored_label(RED, arrows(m)).on_hover_text(arrows(m));
                    }
                    UpgradePhase::Note(m) => {
                        ui.label(RichText::new(m.as_str()).weak())
                            .on_hover_text(m.as_str());
                    }
                }
            }
        }
        _ => {
            let text = cell_text(d, col);
            if !text.is_empty() {
                let r = text_label(ui, text.clone(), col.numeric(), ci.dim);
                if matches!(
                    col,
                    Col::OtaUrl | Col::Hostname | Col::RestartReason | Col::Module
                ) {
                    r.on_hover_text(text);
                }
            }
        }
    }
}

fn repaint(ctx: &egui::Context) -> tasmota_derust_core::Notify {
    let ctx = ctx.clone();
    std::sync::Arc::new(move || ctx.request_repaint())
}

fn option_value_text(o: &OptionInfo, v: u32) -> String {
    if o.numeric {
        v.to_string()
    } else if v == 1 {
        "ON (1)".into()
    } else {
        "OFF (0)".into()
    }
}

/// One option: number, description, risk, current value and its control. Returns the new
/// value when the user changes it.
fn option_row(
    ui: &mut egui::Ui,
    o: &OptionInfo,
    drafts: &mut std::collections::HashMap<u16, u32>,
    enabled: bool,
) -> Option<u32> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("SO{}", o.n)).monospace().weak());
        match o.risk {
            Risk::Danger => {
                warn_icon(ui, RED).on_hover_text(o.warning.unwrap_or(""));
            }
            Risk::App => {
                warn_icon(ui, ORANGE).on_hover_text(o.warning.unwrap_or(""));
            }
            Risk::Normal => {}
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                if o.numeric {
                    let draft = drafts.entry(o.n).or_insert(o.value);
                    let dirty = *draft != o.value;
                    if ui.add_enabled(dirty, egui::Button::new("Set")).clicked() {
                        out = Some(*draft);
                    }
                    // The device can hold 0 ("built-in default") below the documented minimum.
                    ui.add(egui::DragValue::new(draft).range(o.min.min(o.value)..=o.max));
                } else {
                    let mut on = o.value == 1;
                    let blocked = o.blocked_value == Some(if on { 0 } else { 1 });
                    let r = ui.add(toggle(&mut on));
                    let r = match (blocked, o.blocked_reason) {
                        (true, Some(why)) => r.on_hover_text(why),
                        _ => r,
                    };
                    if r.clicked() {
                        out = Some(on as u32);
                    }
                }
            });
            if !o.is_default() {
                ui.label(
                    RichText::new(format!("default {}", o.default.unwrap_or(0)))
                        .color(ORANGE)
                        .small(),
                )
                .on_hover_text(
                    "Default of a freshly flashed current Tasmota (from its documentation, \
                     checked against the firmware source where they disagree). Devices \
                     upgraded from old versions can keep older values.",
                );
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(&o.title)).wrap());
            });
        });
    });
    if !o.details.is_empty() || o.warning.is_some() {
        ui.indent(("so-details", o.n), |ui| {
            for l in &o.details {
                ui.add(egui::Label::new(RichText::new(l).weak().small()).wrap());
            }
            if let Some(w) = o.warning {
                let c = if o.risk == Risk::Danger { RED } else { ORANGE };
                ui.add(egui::Label::new(RichText::new(w).color(c).small()).wrap());
            }
            ui.hyperlink_to(RichText::new("Documentation").small(), &o.link);
        });
    }
    out
}

/// Warning triangle with "!" (the default fonts have no ⚠).
/// Amber, see-through: readable in the light and the dark theme.
const NEW_TINT: Color32 = Color32::from_rgba_unmultiplied_const(0xf2, 0xc2, 0x1b, 0x38);

/// Background of a device that just came online, under the cell's content.
fn new_row_tint(ui: &mut egui::Ui) {
    let gap = ui.spacing().item_spacing.x / 2.0;
    let rect = ui.max_rect().expand2(vec2(gap, 0.0));
    // The cell clips to itself: widen the clip sideways (keeping the scroll area's top and
    // bottom) so the gaps between columns are tinted too.
    let mut p = ui.painter().clone();
    p.set_clip_rect(egui::Rect::from_x_y_ranges(
        rect.x_range(),
        ui.clip_rect().y_range(),
    ));
    p.rect_filled(rect, 0.0, NEW_TINT);
}

/// Painted five-pointed star (the default fonts have no ★), for devices that just came
/// online: a pentagon plus one triangle per point, all convex.
fn star_icon(ui: &mut egui::Ui) -> egui::Response {
    let h = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, resp) = ui.allocate_exact_size(vec2(h, h), Sense::hover());
    let c = rect.center();
    let (outer, inner) = (h * 0.5, h * 0.2);
    let at = |i: usize, r: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())
    };
    let p = ui.painter();
    let inner_pts: Vec<_> = (0..5).map(|k| at(2 * k + 1, inner)).collect();
    p.add(egui::Shape::convex_polygon(inner_pts, YELLOW, Stroke::NONE));
    for k in 0..5 {
        let tri = vec![
            at((2 * k + 9) % 10, inner),
            at(2 * k, outer),
            at(2 * k + 1, inner),
        ];
        p.add(egui::Shape::convex_polygon(tri, YELLOW, Stroke::NONE));
    }
    p.add(egui::Shape::closed_line(
        (0..10)
            .map(|i| at(i, if i % 2 == 0 { outer } else { inner }))
            .collect(),
        Stroke::new(1.0, ORANGE),
    ));
    resp
}

fn warn_icon(ui: &mut egui::Ui, color: Color32) -> egui::Response {
    let h = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, resp) = ui.allocate_exact_size(vec2(h, h), Sense::hover());
    let r = rect.shrink(1.0);
    let p = ui.painter();
    p.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(r.center().x, r.top()),
            egui::pos2(r.right(), r.bottom()),
            egui::pos2(r.left(), r.bottom()),
        ],
        color,
        Stroke::NONE,
    ));
    p.text(
        egui::pos2(r.center().x, r.bottom() - r.height() * 0.05),
        Align2::CENTER_BOTTOM,
        "!",
        FontId::proportional(h * 0.7),
        Color32::BLACK,
    );
    resp
}

/// Small ✕ button (painted, like the gear).
fn remove_button(ui: &mut egui::Ui) -> egui::Response {
    let h = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, resp) = ui.allocate_exact_size(vec2(h, h), Sense::click());
    let color = if resp.hovered() {
        RED
    } else {
        ui.visuals().weak_text_color()
    };
    let c = rect.center();
    let r = h * 0.25;
    let stroke = Stroke::new(h * 0.12, color);
    let p = ui.painter();
    p.line_segment([c + vec2(-r, -r), c + vec2(r, r)], stroke);
    p.line_segment([c + vec2(-r, r), c + vec2(r, -r)], stroke);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Small gear button (the default fonts have no ⚙).
fn gear_button(ui: &mut egui::Ui) -> egui::Response {
    let h = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, resp) = ui.allocate_exact_size(vec2(h, h), Sense::click());
    let color = if resp.hovered() {
        ui.visuals().hyperlink_color
    } else {
        ui.visuals().weak_text_color()
    };
    let c = rect.center();
    let r = h * 0.3;
    let p = ui.painter();
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let d = vec2(a.cos(), a.sin());
        p.line_segment(
            [c + d * r, c + d * (r + h * 0.16)],
            Stroke::new(h * 0.13, color),
        );
    }
    p.circle_stroke(c, r, Stroke::new(h * 0.12, color));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// An iOS-style on/off switch.
fn toggle(on: &mut bool) -> impl egui::Widget + '_ {
    move |ui: &mut egui::Ui| {
        let size = vec2(2.0, 1.0) * ui.spacing().interact_size.y;
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
        if response.clicked() {
            *on = !*on;
            response.mark_changed();
        }
        if ui.is_rect_visible(rect) {
            let t = ui.ctx().animate_bool_responsive(response.id, *on);
            let visuals = ui.style().interact_selectable(&response, *on);
            let rect = rect.expand(visuals.expansion);
            let radius = 0.5 * rect.height();
            let fill = if *on { GREEN } else { visuals.bg_fill };
            ui.painter().rect(
                rect,
                radius,
                fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
            let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
            let center = egui::pos2(x, rect.center().y);
            ui.painter().circle(
                center,
                0.75 * radius,
                visuals.fg_stroke.color,
                visuals.fg_stroke,
            );
        }
        response
    }
}

/// The web login as edited in Settings. The stored password cannot be shown (only its
/// hash is kept): an empty password field keeps it.
#[derive(Default)]
struct LoginDraft {
    username: String,
    password: String,
    remove: bool,
}

impl From<&WebLogin> for LoginDraft {
    fn from(l: &WebLogin) -> Self {
        LoginDraft {
            username: l.username.clone(),
            ..Default::default()
        }
    }
}

impl LoginDraft {
    /// Why it cannot be saved as it is.
    fn problem(&self, had_login: bool) -> Option<&'static str> {
        let (user, pass) = (self.username.trim(), &self.password);
        match () {
            _ if self.remove => None,
            _ if user.is_empty() && !pass.is_empty() => Some("Type a username too."),
            _ if user.is_empty() && had_login => {
                Some("Tick \"Remove the login\" to turn the login off.")
            }
            _ if !user.is_empty() && pass.is_empty() && !had_login => {
                Some("Type a password too, or clear the username.")
            }
            _ => None,
        }
    }

    /// The login to save, from the current one and this draft.
    fn apply(&self, current: &WebLogin) -> WebLogin {
        let user = self.username.trim();
        if self.remove || user.is_empty() {
            WebLogin::default()
        } else if !self.password.is_empty() {
            WebLogin::new(user, &self.password)
        } else {
            let mut l = current.clone();
            l.username = user.into();
            l
        }
    }
}
