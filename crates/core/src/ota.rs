//! Upgrade planning: the official step-by-step path for old firmware, and the free flash check.
//!
//! From the Tasmota docs (Upgrading > Upgrade Flow): old firmware must go through
//! 5.14.0 → 6.7.1 → 7.2.0 → 8.5.1 → 9.1 → current release, one step at a time, using the
//! binaries linked there. Devices older than 10.0.0.3 (ESP8266 and ESP32 alike) also stop at
//! 16.0.0 once a newer release is out: 16.0.0 is the last release that still migrates their
//! settings (the development branch dropped every settings step older than 10.0.0.3, the
//! 9.2–9.5 ones shared by every chip and the ESP32-C3 layout fix at 9.5.0.2 among them).
//! `.gz` images work from 8.2 on. If an image does not fit in the free
//! flash, Tasmota first installs `<name>-minimal.bin` from the same folder by itself, then the
//! image. Firmware older than 4.2.0 can't do it over the air (the next files are only on
//! GitHub, over https).
//!
//! The plan is computed again from the version the device reports, so every step needs its
//! own confirmation and an interrupted path continues from wherever the device is.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;

use crate::device::{Device, parse_version, short_version};

pub const OTA_BASE: &str = "http://ota.tasmota.com/tasmota/";
/// Same server, ESP32 images (`.bin` only).
pub const OTA_BASE32: &str = "http://ota.tasmota.com/tasmota32/";

/// ESP32 images in `tasmota32/release-16.0.0/` (safeboot images left out): the milestone
/// uses the device's own build when it is one of these, the chip's plain build otherwise.
const ESP32_BUILDS_16: &[&str] = &[
    "tasmota32",
    "tasmota32-bluetooth",
    "tasmota32-display",
    "tasmota32-ir",
    "tasmota32-lvgl",
    "tasmota32-nspanel",
    "tasmota32-webcam",
    "tasmota32-zbbrdgpro",
    "tasmota32c2",
    "tasmota32c3",
    "tasmota32c5",
    "tasmota32c6",
    "tasmota32c61",
    "tasmota32p4",
    "tasmota32p4r3",
    "tasmota32s2",
    "tasmota32s2cdc",
    "tasmota32s3",
    "tasmota32solo1",
];
/// `.gz` firmware images are accepted from this version on.
pub const GZ_SINCE: [u32; 2] = [8, 2];
/// Older firmware can't follow the path over the air.
pub const OLDEST_OTA: [u32; 3] = [4, 2, 0];
/// Below this the path is needed; from here on, one upgrade to the latest release.
pub const PATH_END: [u32; 3] = [9, 1, 0];
/// The last release that still runs the settings migrations of 9.x (see the module docs).
pub const MILESTONE: [u32; 3] = [16, 0, 0];
/// Devices below this settings version need the milestone before a newer release.
const MILESTONE_BELOW: [u32; 4] = [10, 0, 0, 3];
/// Free flash kept in reserve when checking whether an image fits.
const MARGIN: u64 = 4 * 1024;

struct PathStep {
    version: [u32; 3],
    /// Relative to OTA_BASE.
    file: &'static str,
    /// The same image gzipped, for devices that accept it.
    file_gz: Option<&'static str>,
}

const PATH: &[PathStep] = &[
    PathStep {
        version: [5, 14, 0],
        file: "release-5.14.0/sonoff-classic.bin",
        file_gz: None,
    },
    PathStep {
        version: [6, 7, 1],
        file: "release-6.7.1/sonoff.bin",
        file_gz: None,
    },
    PathStep {
        version: [7, 2, 0],
        file: "release-7.2.0/tasmota-lite.bin",
        file_gz: None,
    },
    PathStep {
        version: [8, 5, 1],
        file: "release-8.5.1/tasmota-lite.bin",
        file_gz: Some("release-8.5.1/tasmota-lite.bin.gz"),
    },
    // Only reached from 8.5.1, which accepts .gz.
    PathStep {
        version: [9, 1, 0],
        file: "release-9.1.0/tasmota-lite.bin.gz",
        file_gz: None,
    },
];

fn ver3(v: &[u32]) -> [u32; 3] {
    [
        v.first().copied().unwrap_or(0),
        v.get(1).copied().unwrap_or(0),
        v.get(2).copied().unwrap_or(0),
    ]
}

fn fmt_ver(v: [u32; 3]) -> String {
    format!("{}.{}.{}", v[0], v[1], v[2])
}

/// The official build name to end on: the device's own build, except the ones the path
/// installs temporarily (lite, minimal) and the old "sonoff" names, which become "tasmota".
pub fn final_build(version: &str) -> String {
    let tag = version
        .split_once('(')
        .map(|(_, t)| t.trim_end_matches(')'))
        .unwrap_or("");
    let tag = tag.trim_start_matches("release-");
    let rest = tag
        .strip_prefix("tasmota-")
        .or_else(|| tag.strip_prefix("sonoff-"))
        .unwrap_or(tag);
    match rest {
        "" | "tasmota" | "sonoff" | "lite" | "minimal" | "classic" | "basic" => "tasmota".into(),
        r if r.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => format!("tasmota-{r}"),
        _ => "tasmota".into(),
    }
}

/// The URL Tasmota itself tries when an image does not fit: `<name>-minimal.<ext>` in the
/// same folder (name up to the last '-', so tasmota-lite.bin → tasmota-minimal.bin).
pub fn minimal_url(url: &str) -> String {
    let (dir, file) = url
        .rsplit_once('/')
        .map(|(d, f)| (format!("{d}/"), f))
        .unwrap_or((String::new(), url));
    let ext = file.find('.').map(|i| &file[i..]).unwrap_or("");
    let stem = &file[..file.len() - ext.len()];
    let name = stem.rsplit_once('-').map(|(n, _)| n).unwrap_or(stem);
    format!("{dir}{name}-minimal{ext}")
}

fn is_path_url(url: &str) -> bool {
    url.starts_with(&format!("{OTA_BASE}release-{}/", fmt_ver(MILESTONE)))
        || url.starts_with(&format!("{OTA_BASE32}release-{}/", fmt_ver(MILESTONE)))
        || PATH.iter().any(|s| {
            url == format!("{OTA_BASE}{}", s.file)
                || s.file_gz.is_some_and(|g| url == format!("{OTA_BASE}{g}"))
        })
}

/// The milestone is needed: a device (any chip) whose settings are older than 10.0.0.3, when
/// the latest release is newer than 16.0.0 (or not known: better one step too many than
/// missed migrations).
fn needs_milestone(v: &[u32], latest: Option<&str>) -> bool {
    let mut full = [0u32; 4];
    for (i, x) in v.iter().take(4).enumerate() {
        full[i] = *x;
    }
    let newer = latest
        .and_then(parse_version)
        .is_none_or(|l| ver3(&l) > MILESTONE);
    full < MILESTONE_BELOW && newer
}

/// The ESP32 build to install on `d`, or None for an ESP8266: the build in its version tag
/// (`9.5.0(tasmota32c3)`), with the chip from `Hardware` (`ESP32-C3`) when the tag is the
/// plain `tasmota32` or missing.
pub fn esp32_build(d: &Device) -> Option<String> {
    if !is_esp32(d) {
        return None;
    }
    let tag = d
        .version
        .as_deref()
        .and_then(|v| v.split_once('('))
        .map(|(_, t)| {
            t.trim_end_matches(')')
                .trim_start_matches("release-")
                .to_ascii_lowercase()
        })
        .filter(|t| {
            t.starts_with("tasmota32")
                && !t.ends_with("-safeboot")
                && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    let chip = d.hardware.as_deref().and_then(|h| {
        let rest = h.to_ascii_lowercase();
        let rest = rest.strip_prefix("esp32-")?;
        let model: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        ["c2", "c3", "c5", "c6", "c61", "s2", "s3", "p4"]
            .contains(&model.as_str())
            .then(|| format!("tasmota32{model}"))
    });
    Some(match (tag, chip) {
        (Some(t), Some(c)) if t == "tasmota32" => c,
        (Some(t), _) => t,
        (None, Some(c)) => c,
        (None, None) => "tasmota32".into(),
    })
}

/// The build for the 16.0.0 archive: `build` when the archive has it, else the plain
/// build of the same chip (`tasmota32c3-sensors` → `tasmota32c3`).
fn esp32_build_16(build: &str) -> String {
    if ESP32_BUILDS_16.contains(&build) {
        return build.into();
    }
    let chip = build.split('-').next().unwrap_or("tasmota32");
    if ESP32_BUILDS_16.contains(&chip) {
        chip.into()
    } else {
        "tasmota32".into()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Version not known yet.
    Unknown,
    /// Can't be done over the air.
    Manual(String),
    /// One step of the path: set OtaUrl, then Upgrade 1.
    Step {
        target: [u32; 3],
        url: String,
        gz: bool,
        index: usize,
        total: usize,
        /// The 16.0.0 milestone: a full release image, no free flash check (like the last step).
        milestone: bool,
    },
    /// Upgrade to the latest release. `set_url`: OtaUrl to set first (when the device still
    /// points at a path image or at a dead default), otherwise its own OtaUrl is used.
    Final {
        set_url: Option<String>,
        index: usize,
        total: usize,
    },
}

/// `latest`: the latest release, to know whether the 16.0.0 milestone is needed.
/// `esp32`: the device's ESP32 build ([`esp32_build`]), None for an ESP8266.
pub fn plan(
    version: Option<&str>,
    ota_url: Option<&str>,
    esp32: Option<&str>,
    latest: Option<&str>,
) -> Plan {
    let Some(version) = version else {
        return Plan::Unknown;
    };
    let Some(full) = parse_version(version) else {
        return Plan::Unknown;
    };
    let v = ver3(&full);
    if v < PATH_END && esp32.is_some() {
        return Plan::Manual(
            "ESP32 with firmware older than 9.1: upgrade it with the Tasmota web installer or by serial.".into(),
        );
    }
    if v < OLDEST_OTA {
        return Plan::Manual(format!(
            "Firmware older than {} can't follow the upgrade path over the air: flash 6.7.1 or newer \
             by serial or file upload first (see the Tasmota docs, Upgrading > Upgrade Flow).",
            fmt_ver(OLDEST_OTA)
        ));
    }
    let remaining: Vec<&PathStep> = PATH.iter().filter(|s| s.version > v).collect();
    let milestone = needs_milestone(&full, latest);
    let total = remaining.len() + usize::from(milestone) + 1;
    if let Some(step) = remaining.first() {
        let gz_ok = [v[0], v[1]] >= GZ_SINCE;
        let file = match step.file_gz {
            Some(g) if gz_ok => g,
            _ => step.file,
        };
        return Plan::Step {
            target: step.version,
            url: format!("{OTA_BASE}{file}"),
            gz: file.ends_with(".gz"),
            index: 1,
            total,
            milestone: false,
        };
    }
    if milestone {
        // Every path image is 9.1 or newer here: .gz is fine (ESP32 images are .bin only).
        let url = match esp32 {
            Some(b) => format!(
                "{OTA_BASE32}release-{}/{}.bin",
                fmt_ver(MILESTONE),
                esp32_build_16(b)
            ),
            None => format!(
                "{OTA_BASE}release-{}/{}.bin.gz",
                fmt_ver(MILESTONE),
                final_build(version)
            ),
        };
        return Plan::Step {
            target: MILESTONE,
            gz: url.ends_with(".gz"),
            url,
            index: 1,
            total,
            milestone: true,
        };
    }
    let own_ok = ota_url
        .is_some_and(|u| !u.trim().is_empty() && !is_path_url(u) && !u.contains("thehackbox.org"));
    let set_url = if own_ok {
        None
    } else if let Some(b) = esp32 {
        // The chip's plain build: always published, whatever variants a release has.
        let chip = b.split('-').next().unwrap_or("tasmota32");
        Some(format!("{OTA_BASE32}release/{chip}.bin"))
    } else {
        Some(format!("{OTA_BASE}release/{}.bin.gz", final_build(version)))
    };
    Plan::Final {
        set_url,
        index: 1,
        total: 1,
    }
}

// ---------------------------------------------------------------------- channels

/// Latest versions of the two official channels.
#[derive(Clone, Debug, Default)]
pub struct Targets {
    /// Latest release (GitHub releases).
    pub release: Option<String>,
    /// Latest development build (development branch).
    pub development: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Release,
    Development,
    /// Not an official OtaUrl: no idea what it serves.
    Custom,
}

/// What the OtaUrl points at: official ".../release/..." (or a path step) is the release
/// channel, any other official URL the development builds.
pub fn channel(ota_url: Option<&str>) -> Channel {
    let Some(u) = ota_url.map(str::trim).filter(|u| !u.is_empty()) else {
        return Channel::Release;
    };
    let u = u.to_ascii_lowercase();
    let official = u.starts_with("http://ota.tasmota.com/tasmota")
        || u.starts_with("https://ota.tasmota.com/tasmota");
    if u.contains("thehackbox.org")
        || official && (u.contains("/release/") || u.contains("/release-"))
    {
        Channel::Release
    } else if official {
        Channel::Development
    } else {
        Channel::Custom
    }
}

/// The same official image on the other channel: ".../tasmota/release/x" ↔ ".../tasmota/x"
/// (also tasmota32). None when the OtaUrl is not an official channel URL or is already there.
pub fn channel_url(url: &str, development: bool) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let root_end = [
        "://ota.tasmota.com/tasmota/",
        "://ota.tasmota.com/tasmota32/",
    ]
    .iter()
    .find_map(|r| lower.find(r).map(|i| i + r.len()))?;
    let (root, rest) = url.split_at(root_end);
    if let Some(file) = rest.strip_prefix("release/") {
        (development && !file.is_empty() && !file.contains('/')).then(|| format!("{root}{file}"))
    } else if !rest.is_empty() && !rest.contains('/') {
        (!development).then(|| format!("{root}release/{rest}"))
    } else {
        None
    }
}

/// Version parts without trailing zeros, so 16.0.0 == 16.0.0.0.
fn norm(v: &str) -> Option<Vec<u32>> {
    let mut v = parse_version(v)?;
    while v.len() > 1 && v.last() == Some(&0) {
        v.pop();
    }
    Some(v)
}

/// Where a device stands against the channel its OtaUrl points at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VersionStatus {
    /// Latest version of that channel (None: unknown, or custom OtaUrl).
    pub target: Option<String>,
    pub channel_dev: bool,
    pub up_to_date: bool,
    /// Older than the latest of its channel.
    pub outdated: bool,
    /// Newer than its channel's latest: going back (e.g. development → release).
    pub downgrade: bool,
}

pub fn version_status(
    version: Option<&str>,
    ota_url: Option<&str>,
    targets: &Targets,
) -> VersionStatus {
    let ch = channel(ota_url);
    let target = match ch {
        Channel::Release => targets.release.clone(),
        Channel::Development => targets.development.clone(),
        Channel::Custom => None,
    };
    let mut st = VersionStatus {
        channel_dev: ch == Channel::Development,
        ..Default::default()
    };
    if let (Some(v), Some(t)) = (version.and_then(norm), target.as_deref().and_then(norm)) {
        st.up_to_date = v == t;
        st.outdated = v < t;
        st.downgrade = v > t;
    }
    st.target = target;
    st
}

// ------------------------------------------------------------------------- sizes

#[derive(Debug, Clone)]
enum Size {
    Checking,
    Known(u64),
    Failed(String),
}

/// Download sizes of firmware images (HTTP HEAD), fetched in the background and cached.
#[derive(Clone)]
pub struct Sizes {
    map: Arc<Mutex<HashMap<String, Size>>>,
    fetch: bool,
}

impl Default for Sizes {
    fn default() -> Self {
        Sizes {
            map: Arc::default(),
            fetch: true,
        }
    }
}

impl Sizes {
    /// No network: only sizes put in with `insert` (tests).
    pub fn offline() -> Self {
        Sizes {
            map: Arc::default(),
            fetch: false,
        }
    }

    pub fn insert(&self, url: &str, bytes: Option<u64>) {
        let v = bytes
            .map(Size::Known)
            .unwrap_or_else(|| Size::Failed("not found".into()));
        self.map.lock().unwrap().insert(url.to_string(), v);
    }

    /// None while it is being checked; starts the check the first time.
    fn get(&self, url: &str) -> Option<Result<u64, String>> {
        let mut map = self.map.lock().unwrap();
        match map.get(url) {
            Some(Size::Known(n)) => return Some(Ok(*n)),
            Some(Size::Failed(e)) => return Some(Err(e.clone())),
            Some(Size::Checking) => return None,
            None if !self.fetch => return Some(Err("not checked".into())),
            None => {}
        }
        map.insert(url.to_string(), Size::Checking);
        let (map, url) = (self.map.clone(), url.to_string());
        thread::spawn(move || {
            let result = ureq::head(&url)
                .header("User-Agent", "tasmota-derust")
                .call()
                .map_err(|e| e.to_string())
                .and_then(|r| {
                    r.headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .ok_or_else(|| "no size in the answer".to_string())
                });
            let v = match result {
                Ok(n) => Size::Known(n),
                Err(e) => Size::Failed(e),
            };
            map.lock().unwrap().insert(url, v);
        });
        None
    }
}

// ------------------------------------------------------------------- plan for the UI

/// What the upgrade dialog shows, and whether it may start.
#[derive(Serialize, Clone, Debug, Default)]
pub struct PlanInfo {
    /// "final", "step", "manual" or "unknown".
    pub kind: &'static str,
    /// One line, e.g. "Step 2 of 4: 6.7.1 → 7.2.0".
    pub title: String,
    /// The firmware image the device will download (None: its own OtaUrl).
    pub url: Option<String>,
    /// OtaUrl is changed to `url` before the upgrade.
    pub sets_ota_url: bool,
    pub details: Vec<String>,
    pub warnings: Vec<String>,
    pub can_start: bool,
    /// Short label for the upgrade button.
    pub button: String,
}

pub fn is_esp32(d: &Device) -> bool {
    d.hardware
        .as_deref()
        .is_some_and(|h| h.to_ascii_uppercase().starts_with("ESP32"))
        || d.version
            .as_deref()
            .is_some_and(|v| v.contains("tasmota32"))
        || d.core
            .as_deref()
            .is_some_and(|c| c.starts_with('3') || c.starts_with("2_0") || c.starts_with("2.0"))
}

fn kb(bytes: u64) -> u64 {
    bytes.div_ceil(1024)
}

pub fn plan_info(d: &Device, targets: &Targets, sizes: &Sizes) -> PlanInfo {
    let build32 = esp32_build(d);
    let esp32 = build32.is_some();
    let status = version_status(d.version.as_deref(), d.ota_url.as_deref(), targets);
    let current = d
        .version
        .as_deref()
        .map(short_version)
        .unwrap_or("?")
        .to_string();
    let backup =
        "Back up the configuration first (device web UI: Configuration > Backup Configuration).";
    match plan(
        d.version.as_deref(),
        d.ota_url.as_deref(),
        build32.as_deref(),
        targets.release.as_deref(),
    ) {
        Plan::Unknown => PlanInfo {
            kind: "unknown",
            title: "Firmware version not known yet".into(),
            button: "Upgrade…".into(),
            ..Default::default()
        },
        Plan::Manual(why) => PlanInfo {
            kind: "manual",
            title: format!("{current}: no over-the-air upgrade"),
            warnings: vec![why],
            button: "Upgrade…".into(),
            ..Default::default()
        },
        Plan::Final { set_url, .. } => {
            // After the path the OtaUrl is set to the release channel.
            let status = if set_url.is_some() {
                version_status(d.version.as_deref(), set_url.as_deref(), targets)
            } else {
                status
            };
            let to = status.target.clone().unwrap_or_else(|| {
                if channel(d.ota_url.as_deref()) == Channel::Custom {
                    "what its OtaUrl serves".into()
                } else {
                    "the latest version".into()
                }
            });
            let mut info = PlanInfo {
                kind: "final",
                title: format!("{current} → {to}"),
                sets_ota_url: set_url.is_some(),
                url: set_url.clone().or_else(|| d.ota_url.clone()),
                can_start: true,
                button: if status.downgrade {
                    "Switch to release…".into()
                } else {
                    "Upgrade…".into()
                },
                ..Default::default()
            };
            if let Some(u) = &set_url {
                info.details.push(format!("OtaUrl will be set to {u}"));
            }
            if status.channel_dev {
                info.details
                    .push("OtaUrl points to the development builds.".into());
            }
            if status.downgrade {
                info.warnings.push(format!(
                    "This installs {to}, older than the running {current} (OtaUrl points to the \
                     release channel). Tasmota does not guarantee downgrades: back up the \
                     configuration first."
                ));
            }
            info
        }
        Plan::Step {
            target,
            url,
            gz,
            total,
            milestone,
            ..
        } => {
            let to = fmt_ver(target);
            let mut info = PlanInfo {
                kind: "step",
                title: format!(
                    "{current} → {to} (upgrade path: {total} steps left, this one included)"
                ),
                url: Some(url.clone()),
                sets_ota_url: true,
                button: format!("Step to {to}…"),
                ..Default::default()
            };
            info.warnings.push(format!(
                "This firmware is too old to go straight to the latest release: Tasmota has to be \
                 upgraded through {total} steps, one at a time, each confirmed here."
            ));
            info.warnings.push(backup.into());
            if let Some(v) = d.version.as_deref().and_then(parse_version)
                && v.len() >= 4
                && v[..3] == [6, 6, 0]
                && (7..=11).contains(&v[3])
            {
                {
                    info.warnings.push(
                        "Development versions 6.6.0.7 to 6.6.0.11 need \"Reset 6\" before upgrading \
                         (Tasmota docs): back up and do it from the device console first."
                            .into(),
                    );
                }
            }
            if !gz && url.contains("-lite") {
                info.details.push("Intermediate steps install the small \"lite\" build; the full firmware comes with the last step.".into());
            }
            info.details.push(format!("OtaUrl will be set to {url}"));
            if milestone {
                info.details.push(format!(
                    "{to} is the last release that still migrates the settings of firmware older \
                     than 10.0: the newer releases skip those steps."
                ));
                // The same full image the last step installs: Tasmota goes through its minimal
                // image by itself when needed.
                info.can_start = true;
                return info;
            }
            if esp32 {
                info.can_start = true;
                return info;
            }
            // The free flash changes with every image: only a status answer from the
            // firmware running now counts (after a restart, the old one is stale).
            if !d.status_seen {
                info.details.push(
                    "Waiting for the device's status (free flash) since its last restart…".into(),
                );
                return info;
            }
            let free = d.free.and_then(|f| u64::try_from(f).ok()).map(|f| f * 1024);
            let Some(free) = free else {
                info.details.push(
                    "Free flash space not known (no answer to Status 4): press Refresh.".into(),
                );
                return info;
            };
            let size = match sizes.get(&url) {
                None => {
                    info.details.push("Checking the image size…".into());
                    return info;
                }
                Some(Err(e)) => {
                    info.details
                        .push(format!("Could not check the image size ({e})."));
                    return info;
                }
                Some(Ok(n)) => n,
            };
            if size + MARGIN <= free {
                info.details.push(format!(
                    "Image {} KB, free flash {} KB: it fits.",
                    kb(size),
                    kb(free)
                ));
                info.can_start = true;
                return info;
            }
            if gz {
                info.warnings.push(format!(
                    "Image {} KB, free flash only {} KB: it does not fit. Upgrade by serial or file upload.",
                    kb(size),
                    kb(free)
                ));
                return info;
            }
            let min_url = minimal_url(&url);
            match sizes.get(&min_url) {
                None => info.details.push("Checking the minimal image…".into()),
                Some(Ok(m)) if m + MARGIN <= free => {
                    info.details.push(format!(
                        "Image {} KB, free flash {} KB: Tasmota installs {} ({} KB) first, then the \
                         image, by itself. Two restarts.",
                        kb(size),
                        kb(free),
                        min_url.rsplit('/').next().unwrap_or(""),
                        kb(m)
                    ));
                    info.can_start = true;
                }
                Some(Ok(m)) => info.warnings.push(format!(
                    "Free flash {} KB: not enough for the image ({} KB) nor for the minimal one ({} KB). \
                     Upgrade by serial or file upload.",
                    kb(free),
                    kb(size),
                    kb(m)
                )),
                Some(Err(e)) => info.warnings.push(format!(
                    "Image {} KB, free flash {} KB, and no minimal image at {min_url} ({e}).",
                    kb(size),
                    kb(free)
                )),
            }
            info
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Latest release newer than the milestone (the usual case from now on).
    fn step(version: &str) -> Plan {
        plan(Some(version), None, None, Some("16.1.0"))
    }

    #[test]
    fn path_steps() {
        let Plan::Step {
            target,
            url,
            gz,
            total,
            ..
        } = step("6.7.1(sonoff)")
        else {
            panic!()
        };
        assert_eq!(target, [7, 2, 0]);
        assert_eq!(
            url,
            "http://ota.tasmota.com/tasmota/release-7.2.0/tasmota-lite.bin"
        );
        assert!(!gz);
        assert_eq!(total, 5, "7.2.0, 8.5.1, 9.1.0, 16.0.0, latest");

        // 8.1 does not take .gz, 8.2 does.
        let Plan::Step { url, gz, .. } = step("8.1.0(tasmota)") else {
            panic!()
        };
        assert!(url.ends_with("release-8.5.1/tasmota-lite.bin") && !gz);
        let Plan::Step { url, gz, .. } = step("8.2.0(tasmota)") else {
            panic!()
        };
        assert!(url.ends_with("release-8.5.1/tasmota-lite.bin.gz") && gz);

        let Plan::Step { target, .. } = step("4.2.0") else {
            panic!()
        };
        assert_eq!(target, [5, 14, 0]);
        let Plan::Step { target, .. } = step("6.6.0.9") else {
            panic!()
        };
        assert_eq!(target, [6, 7, 1]);
        let Plan::Step { target, total, .. } = step("9.0.0.3(tasmota)") else {
            panic!()
        };
        assert_eq!((target, total), ([9, 1, 0], 3));
    }

    #[test]
    fn milestone_16() {
        // From 9.1 (lite, from the path): the full image of its build in release-16.0.0.
        let Plan::Step {
            target,
            url,
            gz,
            total,
            milestone,
            ..
        } = step("9.1.0(lite)")
        else {
            panic!()
        };
        assert_eq!(target, [16, 0, 0]);
        assert_eq!(
            url,
            "http://ota.tasmota.com/tasmota/release-16.0.0/tasmota.bin.gz"
        );
        assert!(gz && milestone);
        assert_eq!(total, 2);
        let Plan::Step { url, .. } = step("9.5.0(sensors)") else {
            panic!()
        };
        assert!(url.ends_with("release-16.0.0/tasmota-sensors.bin.gz"));
        // Settings already 10.0.0.3 or newer: straight to the latest.
        assert!(matches!(step("10.0.0.3(tasmota)"), Plan::Final { .. }));
        assert!(matches!(step("10.0.0(tasmota)"), Plan::Step { .. }));
        assert!(matches!(step("12.0.2(tasmota)"), Plan::Final { .. }));
        // Latest is 16.0.0 itself: no extra step.
        assert!(matches!(
            plan(Some("9.1.0(lite)"), None, None, Some("16.0.0")),
            Plan::Final { .. }
        ));
        // Latest not known: stop at 16.0.0 to be safe.
        assert!(matches!(
            plan(Some("9.1.0(lite)"), None, None, None),
            Plan::Step {
                milestone: true,
                ..
            }
        ));
        // ESP32 too (the 9.2–9.5 steps are shared, 9.5.0.2 fixes the C3 layout): the
        // 16.0.0 image of its own build, .bin.
        let m32 = |version: &str, build: &str| match plan(
            Some(version),
            None,
            Some(build),
            Some("16.1.0"),
        ) {
            Plan::Step {
                url,
                gz,
                milestone: true,
                target,
                ..
            } => {
                assert_eq!(target, MILESTONE);
                assert!(!gz);
                url
            }
            other => panic!("{version} {build}: {other:?}"),
        };
        assert_eq!(
            m32("9.5.0(tasmota32)", "tasmota32"),
            "http://ota.tasmota.com/tasmota32/release-16.0.0/tasmota32.bin"
        );
        assert!(m32("9.5.0.2(tasmota32c3)", "tasmota32c3").ends_with("/tasmota32c3.bin"));
        assert!(
            m32("10.0.0(tasmota32-bluetooth)", "tasmota32-bluetooth")
                .ends_with("/tasmota32-bluetooth.bin")
        );
        // A build the 16.0.0 archive does not have: the plain image of the same chip.
        assert!(
            m32("9.5.0(tasmota32c3-sensors)", "tasmota32c3-sensors").ends_with("/tasmota32c3.bin")
        );
        assert!(m32("9.5.0(tasmota32-knx)", "tasmota32-knx").ends_with("/tasmota32.bin"));
        // Settings 10.0.0.3 or newer, or latest is 16.0.0: no milestone.
        assert!(matches!(
            plan(
                Some("10.0.0.3(tasmota32)"),
                None,
                Some("tasmota32"),
                Some("16.1.0")
            ),
            Plan::Final { .. }
        ));
        assert!(matches!(
            plan(
                Some("9.5.0(tasmota32)"),
                None,
                Some("tasmota32"),
                Some("16.0.0")
            ),
            Plan::Final { .. }
        ));
        // On 16.0.0 from the ESP32 milestone: back to the release channel of its chip.
        assert_eq!(
            plan(
                Some("16.0.0(release-tasmota32c3)"),
                Some("http://ota.tasmota.com/tasmota32/release-16.0.0/tasmota32c3.bin"),
                Some("tasmota32c3"),
                Some("16.1.0"),
            ),
            Plan::Final {
                set_url: Some(format!("{OTA_BASE32}release/tasmota32c3.bin")),
                index: 1,
                total: 1
            }
        );
        // On 16.0.0 from the milestone: the OtaUrl goes back to the release channel.
        let p = plan(
            Some("16.0.0(release-tasmota)"),
            Some("http://ota.tasmota.com/tasmota/release-16.0.0/tasmota.bin.gz"),
            None,
            Some("16.1.0"),
        );
        assert_eq!(
            p,
            Plan::Final {
                set_url: Some(format!("{OTA_BASE}release/tasmota.bin.gz")),
                index: 1,
                total: 1
            }
        );
    }

    #[test]
    fn esp32_build_from_tag_and_hardware() {
        let dev = |version: &str, hardware: Option<&str>| {
            esp32_build(&Device {
                version: Some(version.into()),
                hardware: hardware.map(Into::into),
                ..Default::default()
            })
        };
        assert_eq!(dev("9.5.0(tasmota)", Some("ESP8266EX")), None);
        assert_eq!(
            dev("9.5.0(tasmota32)", Some("ESP32-D0WDQ6")).as_deref(),
            Some("tasmota32")
        );
        assert_eq!(
            dev("9.5.0(tasmota32)", Some("ESP32-C3")).as_deref(),
            Some("tasmota32c3")
        );
        assert_eq!(
            dev("12.0.0(release-tasmota32s3)", Some("ESP32-S3")).as_deref(),
            Some("tasmota32s3")
        );
        assert_eq!(
            dev("9.5.0(tasmota32-bluetooth)", Some("ESP32-D0WD-V3")).as_deref(),
            Some("tasmota32-bluetooth")
        );
        assert_eq!(
            dev("9.5.0", Some("ESP32-S2")).as_deref(),
            Some("tasmota32s2")
        );
        assert_eq!(esp32_build_16("tasmota32c3-sensors"), "tasmota32c3");
        assert_eq!(esp32_build_16("tasmota32-odroidgo"), "tasmota32");
    }

    #[test]
    fn too_old_or_unknown() {
        assert!(matches!(step("3.9.22"), Plan::Manual(_)));
        assert!(matches!(
            plan(Some("8.5.1(tasmota32)"), None, Some("tasmota32"), None),
            Plan::Manual(_)
        ));
        assert_eq!(plan(None, None, None, None), Plan::Unknown);
    }

    #[test]
    fn final_step_url() {
        // Still pointing at a path image: back to the official release of its build.
        let p = plan(
            Some("9.1.0(lite)"),
            Some("http://ota.tasmota.com/tasmota/release-9.1.0/tasmota-lite.bin.gz"),
            None,
            Some("16.0.0"),
        );
        assert_eq!(
            p,
            Plan::Final {
                set_url: Some(format!("{OTA_BASE}release/tasmota.bin.gz")),
                index: 1,
                total: 1
            }
        );
        // Its own OtaUrl is kept.
        let own = "http://ota.tasmota.com/tasmota/release/tasmota-sensors.bin.gz";
        assert_eq!(
            plan(Some("15.0.1(sensors)"), Some(own), None, Some("16.0.0")),
            Plan::Final {
                set_url: None,
                index: 1,
                total: 1
            }
        );
        // Dead old default.
        let p = plan(
            Some("9.2.0(sensors)"),
            Some("http://thehackbox.org/tasmota/release/tasmota.bin"),
            None,
            Some("16.0.0"),
        );
        assert_eq!(
            p,
            Plan::Final {
                set_url: Some(format!("{OTA_BASE}release/tasmota-sensors.bin.gz")),
                index: 1,
                total: 1
            }
        );
    }

    #[test]
    fn channel_switch() {
        let rel = "http://ota.tasmota.com/tasmota/release/tasmota.bin.gz";
        let dev = "http://ota.tasmota.com/tasmota/tasmota.bin.gz";
        assert_eq!(channel_url(rel, true).as_deref(), Some(dev));
        assert_eq!(channel_url(dev, false).as_deref(), Some(rel));
        assert_eq!(channel_url(rel, false), None);
        assert_eq!(channel_url(dev, true), None);
        assert_eq!(
            channel_url(
                "http://ota.tasmota.com/tasmota32/release/tasmota32c3.bin",
                true
            )
            .as_deref(),
            Some("http://ota.tasmota.com/tasmota32/tasmota32c3.bin")
        );
        assert_eq!(
            channel_url(
                "http://ota.tasmota.com/tasmota/release-9.1.0/tasmota-lite.bin.gz",
                true
            ),
            None
        );
        assert_eq!(channel_url("http://myserver/fw.bin", true), None);
        assert_eq!(channel(Some(dev)), Channel::Development);
        assert_eq!(channel(Some(rel)), Channel::Release);
        let t = Targets {
            release: Some("16.0.0".into()),
            development: Some("16.0.0.4".into()),
        };
        assert!(version_status(Some("16.0.0.4(tasmota)"), Some(dev), &t).up_to_date);
        let back = version_status(Some("16.0.0.4(tasmota)"), Some(rel), &t);
        assert!(back.downgrade && !back.up_to_date);
        let forward = version_status(Some("16.0.0(release-tasmota)"), Some(dev), &t);
        assert!(forward.outdated);
        assert!(version_status(Some("16.0.0(release-tasmota)"), Some(rel), &t).up_to_date);
    }

    #[test]
    fn dev_build_back_to_release() {
        let t = Targets {
            release: Some("16.0.0".into()),
            development: Some("16.0.0.4".into()),
        };
        let mut d = device("16.0.0.4(tasmota)", 364);
        d.ota_url = Some("http://ota.tasmota.com/tasmota/release/tasmota.bin.gz".into());
        let p = plan_info(&d, &t, &Sizes::offline());
        assert_eq!(p.button, "Switch to release…");
        assert!(
            p.warnings
                .iter()
                .any(|w| w.contains("older than the running")),
            "{p:?}"
        );
        d.ota_url = Some("http://ota.tasmota.com/tasmota/tasmota.bin.gz".into());
        let p = plan_info(&d, &t, &Sizes::offline());
        assert_eq!(p.button, "Upgrade…");
        assert!(version_status(d.version.as_deref(), d.ota_url.as_deref(), &t).up_to_date);
    }

    #[test]
    fn builds_and_minimal_names() {
        assert_eq!(final_build("6.7.1(sonoff)"), "tasmota");
        assert_eq!(final_build("8.5.1(lite)"), "tasmota");
        assert_eq!(final_build("16.0.0(release-tasmota)"), "tasmota");
        assert_eq!(final_build("15.0.1(sensors)"), "tasmota-sensors");
        assert_eq!(final_build("6.7.1(sonoff-sensors)"), "tasmota-sensors");
        assert_eq!(
            minimal_url("http://x/release-7.2.0/tasmota-lite.bin"),
            "http://x/release-7.2.0/tasmota-minimal.bin"
        );
        assert_eq!(
            minimal_url("http://x/release-6.7.1/sonoff.bin"),
            "http://x/release-6.7.1/sonoff-minimal.bin"
        );
        assert_eq!(
            minimal_url("http://x/r/sonoff-classic.bin"),
            "http://x/r/sonoff-minimal.bin"
        );
    }

    fn device(version: &str, free_kb: i64) -> Device {
        let mut d = Device::new(
            "old",
            crate::device::DEFAULT_FULLTOPIC,
            ["cmnd".into(), "stat".into(), "tele".into()],
        );
        d.version = Some(version.into());
        d.free = Some(free_kb);
        d.hardware = Some("ESP8266EX".into());
        d.status_seen = true;
        d
    }

    #[test]
    fn free_space_check() {
        let sizes = Sizes::offline();
        let url = format!("{OTA_BASE}release-7.2.0/tasmota-lite.bin");
        sizes.insert(&url, Some(474_576));
        sizes.insert(&minimal_url(&url), Some(382_208));

        let fits = plan_info(&device("6.7.1(sonoff)", 600), &Targets::default(), &sizes);
        assert!(fits.can_start, "{fits:?}");
        let minimal = plan_info(&device("6.7.1(sonoff)", 420), &Targets::default(), &sizes);
        assert!(
            minimal.can_start
                && minimal
                    .details
                    .iter()
                    .any(|l| l.contains("tasmota-minimal.bin")),
            "{minimal:?}"
        );
        let no = plan_info(&device("6.7.1(sonoff)", 300), &Targets::default(), &sizes);
        assert!(!no.can_start, "{no:?}");
        let mut stale = device("6.7.1(sonoff)", 600);
        stale.status_seen = false;
        assert!(
            !plan_info(&stale, &Targets::default(), &sizes).can_start,
            "no fresh status yet"
        );
        let mut unknown = device("6.7.1(sonoff)", 0);
        unknown.free = None;
        assert!(!plan_info(&unknown, &Targets::default(), &sizes).can_start);
    }
}
