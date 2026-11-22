//! Where upgrades take their firmware from, and how far they go: a default for every device,
//! and exceptions for single devices. Saved in `ota.json` in the config directory (Android:
//! [`set_dir`]).
//!
//! - **Firmware server**: the official Tasmota server, or a custom one given as a URL template
//!   where `{build}` becomes the device's build (`tasmota`, `tasmota-sensors`, `tasmota32c3`…),
//!   e.g. `http://192.168.1.5/fw/{build}.bin.gz`. The app can't know which version a custom
//!   server holds: the upgrade sets that OtaUrl and installs whatever it serves, when asked.
//! - **Highest version** (official server only): upgrades stop there, using that release's
//!   folder (`release-11.1.0/…`); devices already at or above it are left alone.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// The default for every device. Empty strings: official server, no limit.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct OtaSettings {
    pub custom_url: String,
    pub max_version: String,
}

/// One device's exception. None: use the default; Some("") on purpose: official server /
/// no limit for this device even when the default has one.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct DeviceOta {
    pub custom_url: Option<String>,
    pub max_version: Option<String>,
}

impl DeviceOta {
    pub fn is_default(&self) -> bool {
        self.custom_url.is_none() && self.max_version.is_none()
    }
}

/// What applies to one device.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Policy {
    /// URL template with `{build}`, None: official server.
    pub custom_url: Option<String>,
    /// Highest version to install from the official server.
    pub max: Option<[u32; 3]>,
    /// The device has its own exception.
    pub own: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtaError {
    BadUrl,
    BadVersion,
}

impl OtaError {
    pub fn message(self) -> &'static str {
        match self {
            OtaError::BadUrl => {
                "The firmware URL must start with http:// (Tasmota can't use https here), and may contain {build}."
            }
            OtaError::BadVersion => "The highest version must look like 11.1.0.",
        }
    }
}

/// A version like "11", "11.1" or "11.1.0" as [major, minor, patch].
pub fn parse_max(s: &str) -> Result<Option<[u32; 3]>, OtaError> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    if s.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() > 3 {
        return Err(OtaError::BadVersion);
    }
    let mut v = [0u32; 3];
    for (i, p) in parts.iter().enumerate() {
        v[i] = p.parse().map_err(|_| OtaError::BadVersion)?;
    }
    if v[0] == 0 {
        return Err(OtaError::BadVersion);
    }
    Ok(Some(v))
}

fn check_url(u: &str) -> Result<(), OtaError> {
    let u = u.trim();
    if u.is_empty() {
        return Ok(());
    }
    let lower = u.to_ascii_lowercase();
    if !lower.starts_with("http://") || u.contains(char::is_whitespace) || u.len() < 12 {
        return Err(OtaError::BadUrl);
    }
    Ok(())
}

/// The firmware URL for one build from a template.
pub fn fill(template: &str, build: &str) -> String {
    template.trim().replace("{build}", build)
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Saved {
    default: OtaSettings,
    devices: BTreeMap<String, DeviceOta>,
}

struct State {
    dir: Option<PathBuf>,
    saved: Option<Saved>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State {
            dir: crate::config::Config::dir(),
            saved: None,
        })
    })
}

/// Where `ota.json` goes, for platforms without a standard config directory (Android).
pub fn set_dir(dir: PathBuf) {
    let mut s = state().lock().unwrap();
    s.dir = Some(dir);
    s.saved = None;
}

fn with_saved<T>(f: impl FnOnce(&mut Saved) -> (T, bool)) -> T {
    let mut s = state().lock().unwrap();
    let path = s.dir.as_ref().map(|d| d.join("ota.json"));
    let saved = s.saved.get_or_insert_with(|| {
        path.as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    });
    let (out, changed) = f(saved);
    if changed && let Some(p) = path {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(saved) {
            let _ = std::fs::write(p, t);
        }
    }
    out
}

pub fn default_settings() -> OtaSettings {
    with_saved(|s| (s.default.clone(), false))
}

pub fn set_default(o: OtaSettings) -> Result<(), OtaError> {
    check_url(&o.custom_url)?;
    parse_max(&o.max_version)?;
    let o = OtaSettings {
        custom_url: o.custom_url.trim().into(),
        max_version: o.max_version.trim().into(),
    };
    with_saved(|s| {
        s.default = o;
        ((), true)
    });
    Ok(())
}

pub fn device(topic: &str) -> DeviceOta {
    with_saved(|s| (s.devices.get(topic).cloned().unwrap_or_default(), false))
}

pub fn set_device(topic: &str, o: DeviceOta) -> Result<(), OtaError> {
    if let Some(u) = &o.custom_url {
        check_url(u)?;
    }
    if let Some(v) = &o.max_version {
        parse_max(v)?;
    }
    let o = DeviceOta {
        custom_url: o.custom_url.map(|u| u.trim().to_string()),
        max_version: o.max_version.map(|v| v.trim().to_string()),
    };
    with_saved(|s| {
        if o.is_default() {
            s.devices.remove(topic);
        } else {
            s.devices.insert(topic.to_string(), o);
        }
        ((), true)
    });
    Ok(())
}

/// What applies to `topic`: its exception, else the default.
pub fn policy(topic: &str) -> Policy {
    with_saved(|s| {
        let d = s.devices.get(topic);
        let url = d
            .and_then(|d| d.custom_url.clone())
            .unwrap_or_else(|| s.default.custom_url.clone());
        let max = d
            .and_then(|d| d.max_version.clone())
            .unwrap_or_else(|| s.default.max_version.clone());
        (
            Policy {
                custom_url: Some(url).filter(|u| !u.trim().is_empty()),
                max: parse_max(&max).ok().flatten(),
                own: d.is_some_and(|d| !d.is_default()),
            },
            false,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_urls() {
        assert_eq!(parse_max(""), Ok(None));
        assert_eq!(parse_max("11"), Ok(Some([11, 0, 0])));
        assert_eq!(parse_max("v11.1"), Ok(Some([11, 1, 0])));
        assert_eq!(parse_max("11.1.0"), Ok(Some([11, 1, 0])));
        assert!(parse_max("11.x").is_err());
        assert!(parse_max("1.2.3.4").is_err());
        assert!(check_url("http://10.0.0.5/fw/{build}.bin.gz").is_ok());
        assert!(check_url("https://example.org/x.bin").is_err());
        assert!(check_url("ftp://x").is_err());
        assert_eq!(
            fill("http://h/fw/{build}.bin.gz", "tasmota-sensors"),
            "http://h/fw/tasmota-sensors.bin.gz"
        );
    }
}
