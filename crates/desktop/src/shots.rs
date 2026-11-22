//! Development only (feature `screenshots`), for the README images: with
//! `DERUST_SCREENSHOTS=<dir>` the app walks through its screens, saves one PNG per screen
//! and quits. Run it against the device simulator, never a real broker (see README).

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use super::*;

const STEPS: &[&str] = &[
    "dashboard",
    "find",
    "selection",
    "home",
    "health",
    "firmware",
    "wifi",
    "mqtt",
    "log",
    "settings",
    "light",
    "options",
    "tools",
    "wifi-networks",
    "backup",
    "console",
    "upgrade",
    "changelog",
    "about",
    "about-light",
];

pub(super) struct Shots {
    dir: PathBuf,
    step: usize,
    at: Instant,
    waiting: bool,
    done_tx: mpsc::Sender<()>,
    done_rx: mpsc::Receiver<()>,
}

impl Shots {
    pub(super) fn from_env() -> Option<Shots> {
        let dir = PathBuf::from(std::env::var_os("DERUST_SCREENSHOTS")?);
        std::fs::create_dir_all(&dir).ok()?;
        // Leaves time for the devices to report in.
        let (done_tx, done_rx) = mpsc::channel();
        Some(Shots {
            dir,
            step: 0,
            at: Instant::now() + Duration::from_secs(6),
            waiting: false,
            done_tx,
            done_rx,
        })
    }
}

impl DerustApp {
    pub(super) fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some(mut s) = self.shots.take() else {
            return;
        };

        if s.done_rx.try_recv().is_ok() {
            s.waiting = false;
            s.step += 1;
            self.settings_open = false;
            self.log_open = false;
            self.light_open = None;
            self.options_open = None;
            self.tools_open = None;
            self.wifi_open = None;
            self.backup_open = None;
            self.consoles.clear();
            self.search_open = None;
            self.confirm = None;
            self.selected.clear();
            self.changelog = None;
            self.about_open = false;
            ctx.set_theme(theme_pref(&self.cfg.theme));
            if s.step == STEPS.len() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            s.at = Instant::now() + Duration::from_secs(2);
        }

        if !s.waiting {
            match STEPS[s.step] {
                "dashboard" => self.cfg.view = View::Dashboard,
                "find" => {
                    // The simulator's HTTP-only devices.
                    self.cfg.view = View::Home;
                    if self.search_open.is_none() {
                        self.search_open = Some(SearchWindow {
                            range: "127.0.0.1:18081-18084".into(),
                            ..Default::default()
                        });
                        if let Ok(hosts) = http_dev::hosts("127.0.0.1:18081-18084") {
                            let found = http_dev::scan(&hosts, None, &Default::default());
                            for f in &found {
                                self.store.add_http(&f.host, f.status.as_ref(), None);
                            }
                            if let Some(w) = &mut self.search_open {
                                w.message = Some(format!("{} Tasmota found; {} reached over HTTP only, added to the list.", found.len(), found.len()));
                            }
                        }
                    }
                }
                "selection" => {
                    self.cfg.view = View::Firmware;
                    if self.confirm.is_none() {
                        let keys: Vec<String> = ["vecchio_interruttore", "presa_esp32", "http:127.0.0.1:18081"]
                            .iter()
                            .filter(|k| self.store.devices.contains_key(**k))
                            .map(|k| k.to_string())
                            .collect();
                        self.selected.extend(keys.iter().cloned());
                        self.confirm = Some(Confirm::UpgradeMany(keys));
                    }
                }
                "home" => self.cfg.view = View::Home,
                "health" => self.cfg.view = View::Health,
                "firmware" => self.cfg.view = View::Firmware,
                "wifi" => self.cfg.view = View::Wifi,
                "mqtt" => self.cfg.view = View::Mqtt,
                "about" => {
                    self.cfg.view = View::Home;
                    self.about_open = true;
                }
                "about-light" => {
                    self.cfg.view = View::Home;
                    self.about_open = true;
                    ctx.set_theme(egui::ThemePreference::Light);
                }
                "log" => {
                    self.cfg.view = View::Home;
                    self.log_open = true;
                }
                "light" => {
                    self.cfg.view = View::Home;
                    if self.light_open.is_none() {
                        let lamp = self
                            .store
                            .devices
                            .values()
                            .find(|d| d.light.as_ref().is_some_and(|l| l.has_color()));
                        if let Some((k, l)) =
                            lamp.and_then(|d| Some((d.topic.clone(), d.light.as_ref()?)))
                        {
                            self.light_open = Some((k, LightEdit::from(l)));
                        }
                    }
                }
                "options" => {
                    self.cfg.view = View::Home;
                    if self.options_open.is_none() {
                        if let Some(d) = self.store.devices.values().find(|d| d.light.is_some()) {
                            self.handle_actions(vec![UiAction::OpenOptions(d.topic.clone())]);
                        }
                    }
                }
                "tools" => {
                    self.cfg.view = View::Home;
                    if self.tools_open.is_none() {
                        self.tools_open = Some(("cucina".into(), egui::pos2(60.0, 120.0), false));
                    }
                }
                "wifi-networks" => {
                    self.cfg.view = View::Wifi;
                    if self.wifi_open.is_none() {
                        self.handle_actions(vec![UiAction::OpenWifi("cucina".into())]);
                    }
                }
                "backup" => {
                    self.cfg.view = View::Home;
                    if self.backup_open.is_none() {
                        self.handle_actions(vec![UiAction::OpenBackup("cucina".into())]);
                    }
                }
                "console" => {
                    self.cfg.view = View::Home;
                    if self.consoles.is_empty() {
                        self.handle_actions(vec![UiAction::OpenConsole("cucina".into())]);
                        if let Some(w) = self.consoles.first_mut() {
                            w.input = "Power1 TOGGLE".into();
                        }
                    }
                }
                "upgrade" => {
                    // The old switch half way through its first step, to show the Update column.
                    self.cfg.view = View::Firmware;
                    if let Some(d) = self.store.devices.get_mut("vecchio_interruttore") {
                        d.upgrade = Some(device::Upgrade {
                            started: Instant::now(),
                            from: d.version.clone(),
                            phase: UpgradePhase::Done(
                                "Upgraded 6.7.1 → 7.2.0. 3 steps left: upgrade again for the next one.".into(),
                            ),
                            target: None,
                            step: None,
                            commands: Vec::new(),
                            resent: false,
                            auto: false,
                        });
                    }
                }
                "changelog" => {
                    if self.changelog.is_none() {
                        self.open_changelog(ctx, env!("CARGO_PKG_VERSION"));
                    }
                }
                "settings" => {
                    self.draft = self.cfg.broker.clone();
                    self.settings_open = true;
                }
                _ => {}
            }
            if Instant::now() >= s.at {
                let path = s.dir.join(format!("desktop-{}.png", STEPS[s.step]));
                let done = s.done_tx.clone();
                let repaint = ctx.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::ScreenshotCallback::new(move |image| {
                        if let Err(e) = save_png(&path, &image) {
                            eprintln!("{}: {e}", path.display());
                        }
                        let _ = done.send(());
                        repaint.request_repaint();
                    }),
                ));
                s.waiting = true;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        self.shots = Some(s);
    }
}

fn save_png(path: &Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path)?),
        w as u32,
        h as u32,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    enc.write_header()?.write_image_data(&data)?;
    Ok(())
}
