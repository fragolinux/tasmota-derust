//! Development only (feature `screenshots`), for the README images: with
//! `DERUST_SCREENSHOTS=<dir>` the app walks through its screens, saves one PNG per screen
//! and quits. Run it against the device simulator, never a real broker (see README).

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use super::*;

const STEPS: &[&str] = &[
    "home",
    "health",
    "firmware",
    "wifi",
    "mqtt",
    "log",
    "settings",
    "light",
    "options",
    "upgrade",
    "changelog",
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
            self.changelog = None;
            if s.step == STEPS.len() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            s.at = Instant::now() + Duration::from_secs(2);
        }

        if !s.waiting {
            match STEPS[s.step] {
                "home" => self.cfg.view = View::Home,
                "health" => self.cfg.view = View::Health,
                "firmware" => self.cfg.view = View::Firmware,
                "wifi" => self.cfg.view = View::Wifi,
                "mqtt" => self.cfg.view = View::Mqtt,
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
