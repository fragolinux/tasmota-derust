//! Latest official Tasmota release, fetched once in the background.

use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug)]
pub enum Latest {
    Loading,
    Ok(String),
    Err(String),
}

/// Latest development build: the version in tasmota_version.h on the development branch
/// (what ota.tasmota.com/tasmota/ serves), e.g. 0x10000004 → "16.0.0.4".
pub fn fetch_latest_dev(notify: crate::Notify) -> Arc<Mutex<Latest>> {
    let state = Arc::new(Mutex::new(Latest::Loading));
    let out = state.clone();
    thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let text = ureq::get(
                "https://raw.githubusercontent.com/arendst/Tasmota/development/tasmota/include/tasmota_version.h",
            )
            .header("User-Agent", "tasmota-derust")
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
            parse_version_header(&text)
                .ok_or_else(|| "no TASMOTA_VERSION in tasmota_version.h".to_string())
        })();
        *out.lock().unwrap() = match result {
            Ok(v) => Latest::Ok(v),
            Err(e) => Latest::Err(e),
        };
        notify();
    });
    state
}

/// `const uint32_t TASMOTA_VERSION = 0x10000004;` → "16.0.0.4" (one byte per part).
pub fn parse_version_header(text: &str) -> Option<String> {
    let line = text
        .lines()
        .find(|l| l.contains("TASMOTA_VERSION") && l.contains("0x"))?;
    let hex = line.split("0x").nth(1)?;
    let hex: String = hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    let n = u32::from_str_radix(&hex, 16).ok()?;
    let [a, b, c, d] = n.to_be_bytes();
    Some(if d == 0 {
        format!("{a}.{b}.{c}")
    } else {
        format!("{a}.{b}.{c}.{d}")
    })
}

/// Release page of this program.
pub const APP_RELEASES: &str = "https://github.com/fragolinux/tasmota-derust/releases/latest";

/// Latest published release of Tasmota Derust itself (pre-releases are not "latest").
pub fn fetch_app_latest(notify: crate::Notify) -> Arc<Mutex<Latest>> {
    fetch_tag(
        "https://api.github.com/repos/fragolinux/tasmota-derust/releases/latest",
        notify,
    )
}

/// The newer Tasmota Derust version, if the latest release is newer than this program.
pub fn app_update(latest: &Latest) -> Option<String> {
    let Latest::Ok(v) = latest else { return None };
    let newer =
        crate::device::parse_version(v)? > crate::device::parse_version(env!("CARGO_PKG_VERSION"))?;
    newer.then(|| v.clone())
}

pub fn fetch_latest(notify: crate::Notify) -> Arc<Mutex<Latest>> {
    fetch_tag(
        "https://api.github.com/repos/arendst/Tasmota/releases/latest",
        notify,
    )
}

/// tag_name of a GitHub "latest release", without the leading v.
fn fetch_tag(url: &'static str, notify: crate::Notify) -> Arc<Mutex<Latest>> {
    let state = Arc::new(Mutex::new(Latest::Loading));
    let out = state.clone();
    thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let mut resp = ureq::get(url)
                .header("User-Agent", "tasmota-derust")
                .header("Accept", "application/vnd.github+json")
                .call()
                .map_err(|e| e.to_string())?;
            let v: serde_json::Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            v.get("tag_name")
                .and_then(|t| t.as_str())
                .map(|t| t.trim_start_matches(['v', 'V']).to_string())
                .ok_or_else(|| "no tag_name in response".to_string())
        })();
        *out.lock().unwrap() = match result {
            Ok(v) => Latest::Ok(v),
            Err(e) => Latest::Err(e),
        };
        notify();
    });
    state
}

// ------------------------------------------------------------------ changelog

/// The "Changes" part of a Tasmota Derust release's notes.
#[derive(serde::Serialize, Clone, Debug, Default, PartialEq)]
pub struct Changelog {
    pub version: String,
    /// Release page on GitHub.
    pub url: String,
    pub sections: Vec<ChangelogSection>,
}

#[derive(serde::Serialize, Clone, Debug, Default, PartialEq)]
pub struct ChangelogSection {
    /// "New", "Fixes", "Other".
    pub title: String,
    pub items: Vec<String>,
}

/// Release notes of Tasmota Derust `version` (blocking; cached for the session).
pub fn changelog(version: &str) -> Result<Changelog, String> {
    static CACHE: std::sync::OnceLock<Mutex<std::collections::HashMap<String, Changelog>>> =
        std::sync::OnceLock::new();
    let version = version.trim_start_matches(['v', 'V']).to_string();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(c) = cache.lock().unwrap().get(&version) {
        return Ok(c.clone());
    }
    let url =
        format!("https://api.github.com/repos/fragolinux/tasmota-derust/releases/tags/v{version}");
    let mut resp = ureq::get(&url)
        .header("User-Agent", "tasmota-derust")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => {
                format!("No published release v{version} (a local build?).")
            }
            e => e.to_string(),
        })?;
    let v: serde_json::Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
    let body = v.get("body").and_then(|b| b.as_str()).unwrap_or("");
    let page = v
        .get("html_url")
        .and_then(|u| u.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!("https://github.com/fragolinux/tasmota-derust/releases/tag/v{version}")
        });
    let c = Changelog {
        version: version.clone(),
        url: page,
        sections: parse_changes(body),
    };
    cache.lock().unwrap().insert(version, c.clone());
    Ok(c)
}

/// The "## Changes" section of release notes: "### Title" headings with "- item" lists.
pub fn parse_changes(body: &str) -> Vec<ChangelogSection> {
    let mut out: Vec<ChangelogSection> = Vec::new();
    let mut inside = false;
    for line in body.lines().map(str::trim) {
        if let Some(h) = line.strip_prefix("## ") {
            inside = h.trim().eq_ignore_ascii_case("changes");
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(t) = line.strip_prefix("### ") {
            out.push(ChangelogSection {
                title: t.trim().to_string(),
                items: Vec::new(),
            });
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            if out.is_empty() {
                out.push(ChangelogSection {
                    title: "Changes".into(),
                    items: Vec::new(),
                });
            }
            out.last_mut().unwrap().items.push(item.trim().to_string());
        }
    }
    out.retain(|s| !s.items.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_version_header() {
        let h = "const uint32_t TASMOTA_VERSION = 0x10000004;   // 16.0.0.4";
        assert_eq!(parse_version_header(h).as_deref(), Some("16.0.0.4"));
        assert_eq!(parse_version_header("x = 0x10000000;").as_deref(), None);
        assert_eq!(
            parse_version_header("TASMOTA_VERSION = 0x0F040100;").as_deref(),
            Some("15.4.1")
        );
    }

    #[test]
    fn changes_section() {
        let body =
            "## Changes\n\n### New\n\n- A\n- B\n\n### Other\n\n- C\n\n## Downloads\n\n- not this";
        let c = parse_changes(body);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].title, "New");
        assert_eq!(c[0].items, vec!["A", "B"]);
        assert_eq!(c[1].items, vec!["C"]);
        assert!(parse_changes("## Downloads\n- x").is_empty());
    }

    #[test]
    fn app_update_only_when_newer() {
        let ok = |v: &str| Latest::Ok(v.into());
        assert_eq!(app_update(&ok("999.0.0")).as_deref(), Some("999.0.0"));
        assert_eq!(app_update(&ok(env!("CARGO_PKG_VERSION"))), None);
        assert_eq!(app_update(&ok("0.0.1")), None);
        assert_eq!(app_update(&Latest::Loading), None);
    }
}
