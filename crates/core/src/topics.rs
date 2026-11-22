//! Devices on custom MQTT topics: the app finds devices by their LWT (`tele/<topic>/LWT`) and
//! by `tasmota/discovery`. Devices with another FullTopic (e.g.
//! `headquarters/meeting-room-3/%prefix%/%topic%/`) that do not publish `tasmota/discovery`
//! (`SetOption19 1`, or firmware older than 9.x) are found through:
//!
//! - **patterns**: FullTopics with `+` for any one level, e.g. `headquarters/+/%prefix%/%topic%/`.
//!   The device is named by `%topic%` (or `%hostname%` / `%id%` when the FullTopic uses those);
//! - **devices**: one device with its own FullTopic, e.g. topic `plug1` with
//!   `site/garage/%prefix%/%topic%/`.
//!
//! For each, the app subscribes to the LWT topic it implies; when a device's LWT arrives, its
//! real FullTopic is known and it works like any other device.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TopicDevice {
    pub topic: String,
    pub full_topic: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Topics {
    pub patterns: Vec<String>,
    pub devices: Vec<TopicDevice>,
}

const IDS: [&str; 3] = ["%topic%", "%hostname%", "%id%"];

fn segments(full_topic: &str) -> Vec<&str> {
    full_topic.trim().trim_matches('/').split('/').collect()
}

fn check_full_topic(ft: &str, needs_id: bool) -> Result<(), String> {
    let segs = segments(ft);
    let bad = |why: &str| Err(format!("{}: {why}", ft.trim()));
    if ft.trim().is_empty() {
        return bad("empty");
    }
    if segs.iter().any(|s| s.is_empty()) {
        return bad("an empty level (//)");
    }
    if ft.contains('#') || ft.contains(char::is_whitespace) {
        return bad("no spaces or #");
    }
    if segs.iter().filter(|s| **s == "%prefix%").count() != 1 {
        return bad("%prefix% must appear once, as a whole level");
    }
    let ids = segs.iter().filter(|s| IDS.contains(s)).count();
    if needs_id && ids != 1 {
        return bad("needs %topic% (or %hostname% / %id%) once, as a whole level");
    }
    if segs.iter().any(|s| s.contains('+') && *s != "+") {
        return bad("+ must be a whole level");
    }
    Ok(())
}

fn check_topic(t: &str) -> Result<(), String> {
    if t.is_empty() || t.contains(['/', '+', '#']) || t.contains(char::is_whitespace) {
        return Err(format!("{t}: a topic is one level, without / + # or spaces"));
    }
    Ok(())
}

impl Topics {
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty() && self.devices.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        for p in &self.patterns {
            check_full_topic(p, true)?;
        }
        for d in &self.devices {
            check_topic(&d.topic)?;
            check_full_topic(&d.full_topic, false)?;
        }
        Ok(())
    }

    /// From the two text boxes of the settings: one pattern per line; one device per line
    /// as `<topic> <FullTopic>`. Blank lines and lines starting with `#` are skipped.
    pub fn from_lines(patterns: &str, devices: &str) -> Result<Topics, String> {
        let lines = |s: &str| -> Vec<String> {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect()
        };
        let mut t = Topics {
            patterns: lines(patterns),
            devices: Vec::new(),
        };
        for l in lines(devices) {
            let mut parts = l.split_whitespace();
            match (parts.next(), parts.next(), parts.next()) {
                (Some(topic), Some(ft), None) => t.devices.push(TopicDevice {
                    topic: topic.into(),
                    full_topic: ft.into(),
                }),
                _ => return Err(format!("{l}: write the topic, a space, then its FullTopic")),
            }
        }
        t.validate()?;
        Ok(t)
    }

    pub fn patterns_text(&self) -> String {
        self.patterns.join("\n")
    }

    pub fn devices_text(&self) -> String {
        self.devices
            .iter()
            .map(|d| format!("{} {}", d.topic, d.full_topic))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The LWT subscriptions these patterns and devices need.
    pub fn subscriptions(&self) -> Vec<String> {
        let lwt = |ft: &str, topic: Option<&str>| -> String {
            let mut out: Vec<String> = segments(ft)
                .iter()
                .map(|s| match *s {
                    "%prefix%" => "tele".to_string(),
                    "%topic%" if topic.is_some() => topic.unwrap_or("+").to_string(),
                    s if IDS.contains(&s) => "+".to_string(),
                    s => s.to_string(),
                })
                .collect();
            out.push("LWT".into());
            out.join("/")
        };
        let mut subs: Vec<String> = self
            .patterns
            .iter()
            .filter(|p| check_full_topic(p, true).is_ok())
            .map(|p| lwt(p, None))
            .chain(
                self.devices
                    .iter()
                    .filter(|d| check_topic(&d.topic).is_ok() && check_full_topic(&d.full_topic, false).is_ok())
                    .map(|d| lwt(&d.full_topic, Some(&d.topic))),
            )
            .collect();
        subs.sort();
        subs.dedup();
        subs
    }

    /// A device's key and real FullTopic from an LWT topic, when a pattern or a device entry
    /// matches it.
    pub fn match_lwt(&self, topic: &str) -> Option<(String, String)> {
        let parts: Vec<&str> = topic.split('/').collect();
        let (last, levels) = parts.split_last()?;
        if *last != "LWT" {
            return None;
        }
        for d in &self.devices {
            if let Some((key, ft)) = match_one(&d.full_topic, levels, Some(&d.topic)) {
                return Some((key, ft));
            }
        }
        self.patterns
            .iter()
            .find_map(|p| match_one(p, levels, None))
    }
}

/// `levels`: the LWT topic without "LWT". `fixed`: a device entry's own topic.
fn match_one(full_topic: &str, levels: &[&str], fixed: Option<&str>) -> Option<(String, String)> {
    let segs = segments(full_topic);
    if segs.len() != levels.len() || check_full_topic(full_topic, fixed.is_none()).is_err() {
        return None;
    }
    let mut key = fixed.map(str::to_string);
    let mut resolved = Vec::new();
    for (s, l) in segs.iter().zip(levels) {
        match *s {
            "%prefix%" if *l == "tele" => resolved.push("%prefix%".to_string()),
            "%prefix%" => return None,
            "%topic%" => {
                if fixed.is_some_and(|t| t != *l) {
                    return None;
                }
                key = Some(l.to_string());
                resolved.push("%topic%".into());
            }
            "%hostname%" | "%id%" if fixed.is_none() => {
                key = Some(l.to_string());
                resolved.push(l.to_string());
            }
            "+" => resolved.push(l.to_string()),
            lit if lit == *l => resolved.push(lit.to_string()),
            _ => return None,
        }
    }
    Some((key?, format!("{}/", resolved.join("/"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(p: &[&str], d: &[(&str, &str)]) -> Topics {
        Topics {
            patterns: p.iter().map(|s| s.to_string()).collect(),
            devices: d
                .iter()
                .map(|(a, b)| TopicDevice { topic: a.to_string(), full_topic: b.to_string() })
                .collect(),
        }
    }

    #[test]
    fn patterns_and_devices() {
        let x = t(
            &["headquarters/+/%prefix%/%topic%/", "%prefix%/lab/%hostname%/"],
            &[("plug1", "site/garage/%prefix%/%topic%/")],
        );
        assert!(x.validate().is_ok());
        assert_eq!(
            x.subscriptions(),
            ["headquarters/+/tele/+/LWT", "site/garage/tele/plug1/LWT", "tele/lab/+/LWT"]
        );
        assert_eq!(
            x.match_lwt("headquarters/meeting-room-3/tele/fan/LWT"),
            Some(("fan".into(), "headquarters/meeting-room-3/%prefix%/%topic%/".into()))
        );
        assert_eq!(
            x.match_lwt("tele/lab/bench-1/LWT"),
            Some(("bench-1".into(), "%prefix%/lab/bench-1/".into()))
        );
        assert_eq!(
            x.match_lwt("site/garage/tele/plug1/LWT"),
            Some(("plug1".into(), "site/garage/%prefix%/%topic%/".into()))
        );
        assert_eq!(x.match_lwt("site/garage/tele/plug2/LWT"), None);
        assert_eq!(x.match_lwt("headquarters/room/stat/fan/LWT"), None);
        assert_eq!(x.match_lwt("tele/fan/LWT"), None);
    }

    #[test]
    fn checks_and_text() {
        assert!(t(&["a/b/%topic%/"], &[]).validate().is_err(), "no %prefix%");
        assert!(t(&["a/%prefix%/"], &[]).validate().is_err(), "no %topic%");
        assert!(t(&["a/#/%prefix%/%topic%"], &[]).validate().is_err());
        assert!(t(&["a/x+/%prefix%/%topic%"], &[]).validate().is_err());
        assert!(t(&[], &[("a/b", "%prefix%/%topic%/")]).validate().is_err());
        let x = Topics::from_lines(
            "# rooms\nheadquarters/+/%prefix%/%topic%/\n\n",
            "plug1 site/garage/%prefix%/%topic%/\n",
        )
        .unwrap();
        assert_eq!(x.patterns, ["headquarters/+/%prefix%/%topic%/"]);
        assert_eq!(x.devices_text(), "plug1 site/garage/%prefix%/%topic%/");
        assert!(Topics::from_lines("", "plug1").is_err());
    }
}
