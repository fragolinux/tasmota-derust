//! Per-device console: the MQTT messages of one device, the commands typed for it (history,
//! per device) and a few favourite commands shared by all devices.
//!
//! The traffic lives in memory only ([`Log`], inside the device store). History and
//! favourites are saved in `console.json` in the config directory. Commands that carry a
//! password never enter the history, and passwords are hidden in the log.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// Messages kept per device.
pub const LINES_MAX: usize = 300;
/// Commands remembered per device.
pub const HISTORY_MAX: usize = 50;
/// Favourite commands (slots).
pub const FAVOURITES: usize = 5;
/// Longer payloads are cut in the log.
const PAYLOAD_MAX: usize = 8000;

// ------------------------------------------------------------------- traffic

/// One MQTT message to or from a device.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Line {
    /// Grows with every line of every device: ask for the lines after the last one seen.
    pub seq: u64,
    /// Local time, "HH:MM:SS".
    pub time: String,
    /// Sent by this app (a command), not received.
    pub sent: bool,
    /// Replayed by the broker (retained), not sent by the device just now.
    pub retained: bool,
    pub topic: String,
    pub payload: String,
}

/// The recent messages of every device.
#[derive(Default)]
pub struct Log {
    lines: HashMap<String, VecDeque<Line>>,
    seq: u64,
}

impl Log {
    pub fn push(&mut self, key: &str, sent: bool, retained: bool, topic: &str, payload: &str) {
        self.seq += 1;
        let mut payload = if sent {
            mask(topic, payload)
        } else {
            payload.to_string()
        };
        if payload.len() > PAYLOAD_MAX {
            let mut cut = PAYLOAD_MAX;
            while !payload.is_char_boundary(cut) {
                cut -= 1;
            }
            payload.truncate(cut);
            payload.push('…');
        }
        let list = self.lines.entry(key.to_string()).or_default();
        list.push_back(Line {
            seq: self.seq,
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            sent,
            retained,
            topic: topic.to_string(),
            payload,
        });
        while list.len() > LINES_MAX {
            list.pop_front();
        }
    }

    /// The lines of a device newer than `after` (0: all of them), oldest first.
    pub fn lines(&self, key: &str, after: u64) -> Vec<Line> {
        self.lines
            .get(key)
            .map(|l| l.iter().filter(|x| x.seq > after).cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear(&mut self, key: &str) {
        self.lines.remove(key);
    }
}

/// A sent payload with passwords hidden: `Password1 x` as a topic + payload, or inside a
/// Backlog.
pub fn mask(topic: &str, payload: &str) -> String {
    let cmd = topic.rsplit('/').next().unwrap_or("");
    if is_secret_command(cmd) && !payload.trim().is_empty() {
        return "****".into();
    }
    crate::wifi::mask_passwords(payload)
}

/// Commands whose value is a password: Password1/2, MqttPassword, WebPassword…
fn is_secret_command(cmd: &str) -> bool {
    cmd.to_ascii_lowercase().contains("password")
}

// ------------------------------------------------------------------- commands

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleError {
    /// Nothing typed, or the first word is not a command name.
    BadCommand,
    /// A favourite's `<placeholder>` was not replaced.
    Placeholder,
    /// The command can cut the device off (see [`risk`]): send it again confirmed.
    NeedsConfirm,
}

impl ConsoleError {
    pub fn message(self) -> &'static str {
        match self {
            ConsoleError::BadCommand => {
                "Type a Tasmota command, for example: Status 0 (letters and digits, then the value)."
            }
            ConsoleError::Placeholder => "Replace the <…> parts with real values first.",
            ConsoleError::NeedsConfirm => RISK_TEXT,
        }
    }
}

/// Splits a console line into command and payload, as Tasmota's own console does:
/// `Backlog Power1 ON; Delay 10` → ("Backlog", "Power1 ON; Delay 10").
pub fn parse(line: &str) -> Result<(String, String), ConsoleError> {
    let line = line.trim();
    let (cmd, payload) = match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    };
    // The command becomes part of the topic: no '/', '+', '#' or anything odd.
    if cmd.is_empty()
        || !cmd
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(ConsoleError::BadCommand);
    }
    if has_placeholder(line) {
        return Err(ConsoleError::Placeholder);
    }
    Ok((cmd.to_string(), payload.to_string()))
}

/// Why [`risk`] asks before sending.
pub const RISK_TEXT: &str = "This command changes how the device connects (Wi-Fi, MQTT) or resets its \
settings. A wrong value cuts it off from this app, and fixing it then needs the device's own \
Wi-Fi access point or its serial port.";

/// Commands (without their trailing number) that change the Wi-Fi or MQTT connection.
const CONNECTION: &[&str] = &[
    "ssid", "password", "ap", "wifi", "wificonfig", "mqtthost", "mqttport", "mqttuser",
    "mqttpassword", "mqttclient", "topic", "fulltopic", "prefix", "grouptopic",
];

/// Some([`RISK_TEXT`]) when a line (also inside a Backlog) sets the Wi-Fi or MQTT
/// connection, turns MQTT or Wi-Fi off, or resets the settings. Reading a value (no
/// payload) is never risky.
pub fn risk(line: &str) -> Option<&'static str> {
    let line = line.trim();
    let (first, rest) = match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    };
    let parts: Vec<&str> = if first.eq_ignore_ascii_case("backlog") || first.eq_ignore_ascii_case("backlog0") {
        rest.split(';').collect()
    } else {
        vec![line]
    };
    parts.into_iter().find_map(|part| {
        let part = part.trim();
        let (cmd, value) = match part.find(char::is_whitespace) {
            Some(i) => (&part[..i], part[i..].trim()),
            None => (part, ""),
        };
        let cmd = cmd.to_ascii_lowercase();
        let base = cmd.trim_end_matches(|c: char| c.is_ascii_digit());
        let off = matches!(value.to_ascii_lowercase().as_str(), "0" | "off" | "false");
        let risky = !value.is_empty()
            && (CONNECTION.contains(&base)
                || base == "reset"
                || ((cmd == "setoption3" || cmd == "setoption95") && off));
        risky.then_some(RISK_TEXT)
    })
}

/// `<network>`, `<password>`…: placeholders the user may write in a favourite.
fn has_placeholder(s: &str) -> bool {
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let word: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || *c == ' ')
            .collect();
        if !word.trim().is_empty() && rest[word.len()..].starts_with('>') {
            return true;
        }
    }
    false
}

/// Browsing the history with the arrows: `pos` None is the line being typed. Older goes
/// back from the newest command and stops at the oldest; newer past the newest returns to
/// the empty line.
pub fn browse(history_len: usize, pos: Option<usize>, older: bool) -> Option<usize> {
    match (pos, older) {
        _ if history_len == 0 => None,
        (None, true) => Some(history_len - 1),
        (Some(i), true) => Some(i.saturating_sub(1).min(history_len - 1)),
        (None, false) => None,
        (Some(i), false) => (i + 1 < history_len).then_some(i + 1),
    }
}

/// Telemetry sent on its own every TelePeriod (`tele/…/STATE`, `SENSOR`): the console can
/// hide it.
pub fn is_telemetry(topic: &str) -> bool {
    let last = topic.rsplit('/').next().unwrap_or("");
    last == "STATE" || last == "SENSOR"
}

/// The line carries a password (`Password1 x`, `Backlog …; MqttPassword x`).
fn has_secret(line: &str) -> bool {
    line.split(';').any(|part| {
        let mut words = part.split_whitespace();
        let first = words.next().unwrap_or("");
        let (cmd, mut rest) = if first.eq_ignore_ascii_case("backlog") {
            (words.next().unwrap_or(""), words)
        } else {
            (first, words)
        };
        is_secret_command(cmd) && rest.next().is_some()
    })
}

// ------------------------------------------------------------------- saved state

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Favourite {
    pub name: String,
    pub command: String,
}

impl Favourite {
    fn new(name: &str, command: &str) -> Favourite {
        Favourite {
            name: name.into(),
            command: command.into(),
        }
    }
}

fn default_favourites() -> Vec<Favourite> {
    vec![
        Favourite::new("Status", "Status 0"),
        // Read-only on purpose: they show the settings; the user can turn them into
        // changes, which then ask first (see `risk`).
        Favourite::new("Wi-Fi info", "Backlog SSId1; SSId2; AP"),
        Favourite::new("MQTT info", "Backlog MqttHost; MqttPort; MqttUser; Topic; FullTopic"),
        Favourite::new("Telemetry", "TelePeriod 60"),
        Favourite::new("Restart", "Restart 1"),
    ]
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Saved {
    /// Per device topic, oldest first.
    history: BTreeMap<String, Vec<String>>,
    /// None: never edited, the defaults apply.
    favourites: Option<Vec<Favourite>>,
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

/// Where `console.json` goes, for platforms without a standard config directory (Android).
pub fn set_dir(dir: PathBuf) {
    let mut s = state().lock().unwrap();
    s.dir = Some(dir);
    s.saved = None;
}

fn with_saved<T>(f: impl FnOnce(&mut Saved) -> (T, bool)) -> T {
    let mut s = state().lock().unwrap();
    let path = s.dir.as_ref().map(|d| d.join("console.json"));
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

/// The commands typed for a device, oldest first.
pub fn history(topic: &str) -> Vec<String> {
    with_saved(|s| (s.history.get(topic).cloned().unwrap_or_default(), false))
}

/// Adds a command to a device's history (moved to the end if it was there already).
/// Commands with a password are not kept.
pub fn remember(topic: &str, line: &str) {
    let line = line.trim();
    if line.is_empty() || has_secret(line) {
        return;
    }
    with_saved(|s| {
        let h = s.history.entry(topic.to_string()).or_default();
        h.retain(|x| x != line);
        h.push(line.to_string());
        let extra = h.len().saturating_sub(HISTORY_MAX);
        h.drain(..extra);
        ((), true)
    })
}

/// Always [`FAVOURITES`] slots; an empty command is an unused slot.
pub fn favourites() -> Vec<Favourite> {
    with_saved(|s| {
        let mut f = s.favourites.clone().unwrap_or_else(default_favourites);
        f.resize(FAVOURITES, Favourite::default());
        (f, false)
    })
}

pub fn set_favourites(list: Vec<Favourite>) {
    let mut list: Vec<Favourite> = list
        .into_iter()
        .take(FAVOURITES)
        .map(|f| Favourite {
            name: f.name.trim().to_string(),
            command: f.command.trim().to_string(),
        })
        .collect();
    list.resize(FAVOURITES, Favourite::default());
    with_saved(|s| {
        s.favourites = Some(list);
        ((), true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_tasmota() {
        assert_eq!(parse("Status 0").unwrap(), ("Status".into(), "0".into()));
        assert_eq!(parse("  Power1  ").unwrap(), ("Power1".into(), "".into()));
        assert_eq!(
            parse("Backlog Power1 ON; Delay 10").unwrap(),
            ("Backlog".into(), "Power1 ON; Delay 10".into())
        );
        assert_eq!(
            parse("Rule1 on Temp>20 do Power ON endon").unwrap().0,
            "Rule1"
        );
        assert_eq!(parse(""), Err(ConsoleError::BadCommand));
        assert_eq!(parse("a/b 1"), Err(ConsoleError::BadCommand));
        assert_eq!(parse("# 1"), Err(ConsoleError::BadCommand));
        assert_eq!(
            parse("Backlog SSId1 <network>; Password1 <password>"),
            Err(ConsoleError::Placeholder)
        );
    }

    #[test]
    fn risky_commands() {
        assert!(risk("Backlog SSId1 Home; Password1 x").is_some());
        assert!(risk("ssid2 Other").is_some());
        assert!(risk("MqttHost 10.0.0.1").is_some());
        assert!(risk("Topic kitchen").is_some());
        assert!(risk("Reset 1").is_some());
        assert!(risk("SetOption3 0").is_some());
        assert!(risk("Backlog Power1 ON; Wifi 0").is_some());
        assert!(risk("AP 2").is_some());
        // Reading is fine, and so are other commands.
        assert!(risk("Backlog SSId1; SSId2; AP").is_none());
        assert!(risk("Backlog MqttHost; MqttPort; MqttUser; Topic; FullTopic").is_none());
        assert!(risk("SetOption3 1").is_none());
        assert!(risk("Power1 TOGGLE").is_none());
        assert!(risk("Restart 1").is_none());
        assert!(risk("TelePeriod 60").is_none());
        for f in default_favourites() {
            assert!(risk(&f.command).is_none(), "{}", f.command);
        }
    }

    #[test]
    fn browsing() {
        assert_eq!(browse(0, None, true), None);
        assert_eq!(browse(3, None, true), Some(2));
        assert_eq!(browse(3, Some(2), true), Some(1));
        assert_eq!(browse(3, Some(0), true), Some(0));
        assert_eq!(browse(3, Some(1), false), Some(2));
        assert_eq!(browse(3, Some(2), false), None);
        assert_eq!(browse(3, None, false), None);
    }

    #[test]
    fn secrets() {
        assert!(has_secret("Password1 abc"));
        assert!(has_secret("Backlog SSId1 Home; Password1 abc"));
        assert!(has_secret("backlog MqttPassword abc"));
        assert!(!has_secret("Password1"));
        assert!(!has_secret("Status 0"));
        assert_eq!(mask("cmnd/x/PASSWORD1", "abc"), "****");
        assert_eq!(mask("cmnd/x/WebPassword", "abc"), "****");
        assert_eq!(
            mask("cmnd/x/BACKLOG", "SSId1 Home; Password1 abc"),
            "SSId1 Home; Password1 ****"
        );
        assert_eq!(mask("cmnd/x/POWER", "ON"), "ON");
    }

    #[test]
    fn log_is_per_device_and_capped() {
        let mut l = Log::default();
        for i in 0..LINES_MAX + 5 {
            l.push("a", false, false, "stat/a/RESULT", &i.to_string());
        }
        l.push("b", true, false, "cmnd/b/Password1", "secret");
        let a = l.lines("a", 0);
        assert_eq!(a.len(), LINES_MAX);
        assert_eq!(a[0].payload, "5");
        let b = l.lines("b", 0);
        assert_eq!(b[0].payload, "****");
        assert!(l.lines("a", b[0].seq).is_empty());
    }
}
