//! SetOption decoder and editor support.
//!
//! The current values come from the device (`Status 3` → `StatusLOG.SetOption`, also part of
//! `Status 0`). The categories and the risk of each option are ours. The descriptions are not
//! part of this program: they are downloaded at run time from the Tasmota documentation
//! (`Commands.md`, the full text with the meaning of each value) and from Tasmota's
//! `tools/decode-status.py` (one line per option, for the ones the documentation lacks), and
//! cached. Without them the options are shown by number, with a link to the documentation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::Notify;

const DOCS_RAW: &str = "https://raw.githubusercontent.com/tasmota/docs/master/docs/Commands.md";
const SCRIPT_RAW: &str =
    "https://raw.githubusercontent.com/arendst/Tasmota/development/tools/decode-status.py";

/// Highest option number that `Status 3` can report (6 registers).
pub const MAX_OPTION: u16 = 177;

/// Online documentation of option `n`.
pub fn doc_link(n: u16) -> String {
    format!("https://tasmota.github.io/docs/Commands/#setoption{n}")
}

/// SetOption32..49 hold a number (one byte each); all the others are on/off bits.
pub fn is_numeric(n: u16) -> bool {
    (32..=49).contains(&n)
}

// ------------------------------------------------------------------------ decoding

/// Decodes the `StatusLOG.SetOption` registers into (option, value) pairs, in order.
///
/// Register 0: bits of SO0–31. Register 1: 36 hex digits, one byte for each of SO32–49
/// (8 digits before 6.1.1.14: something else, skipped). Registers 2–5: bits of SO50–81,
/// 82–113, 114–145, 146–177. Very old firmware reports only register 0 and the resolution.
pub fn decode(regs: &[String]) -> Vec<(u16, u32)> {
    let mut out = Vec::new();
    let bits = |out: &mut Vec<(u16, u32)>, reg: &str, base: u16| {
        if let Ok(v) = u32::from_str_radix(reg.trim(), 16) {
            for i in 0..32 {
                out.push((base + i, (v >> i) & 1));
            }
        }
    };
    let Some(r0) = regs.first() else { return out };
    bits(&mut out, r0, 0);
    if regs.len() <= 2 {
        return out;
    }
    let r1 = regs[1].trim();
    if r1.len() == 36 {
        for i in 0..18u16 {
            let byte = &r1[i as usize * 2..i as usize * 2 + 2];
            if let Ok(v) = u32::from_str_radix(byte, 16) {
                out.push((32 + i, v));
            }
        }
    }
    for (r, reg) in regs.iter().enumerate().skip(2).take(4) {
        bits(&mut out, reg, 50 + (r as u16 - 2) * 32);
    }
    out
}

/// Updates the registers after a `{"SetOptionN": value}` answer, so the new value shows at
/// once (the `Status 3` asked after every change confirms it).
pub fn apply(regs: &mut [String], n: u16, value: u32) {
    let set_bit = |reg: &mut String, bit: u16| {
        if let Ok(mut v) = u32::from_str_radix(reg.trim(), 16) {
            if value != 0 {
                v |= 1 << bit;
            } else {
                v &= !(1 << bit);
            }
            *reg = format!("{v:08X}");
        }
    };
    match n {
        0..=31 => {
            if let Some(r) = regs.get_mut(0) {
                set_bit(r, n);
            }
        }
        32..=49 => {
            if regs.len() > 2 {
                if let Some(r) = regs.get_mut(1).filter(|r| r.trim().len() == 36) {
                    let i = (n as usize - 32) * 2;
                    let mut s = r.trim().to_string();
                    s.replace_range(i..i + 2, &format!("{:02X}", value.min(255)));
                    *r = s;
                }
            }
        }
        50..=MAX_OPTION => {
            let reg = 2 + (n as usize - 50) / 32;
            if regs.len() > 2 {
                if let Some(r) = regs.get_mut(reg) {
                    set_bit(r, (n - 50) % 32);
                }
            }
        }
        _ => {}
    }
}

/// `{"SetOption4":"ON"}` / `{"SetOption34":200}` → (4, 1) / (34, 200).
pub fn parse_answer(key: &str, value: &serde_json::Value) -> Option<(u16, u32)> {
    let n: u16 = key
        .strip_prefix("SetOption")
        .or_else(|| key.strip_prefix("SETOPTION"))?
        .parse()
        .ok()?;
    let v = match value {
        serde_json::Value::Number(x) => x.as_u64()? as u32,
        serde_json::Value::String(s) => match s.to_ascii_uppercase().as_str() {
            "ON" => 1,
            "OFF" => 0,
            s => s.parse().ok()?,
        },
        _ => return None,
    };
    Some((n, v))
}

// -------------------------------------------------------------------- our metadata

/// Display order of the categories.
pub const CATEGORIES: [&str; 14] = [
    "General",
    "Relays",
    "Buttons & switches",
    "Lights",
    "MQTT",
    "Home Assistant",
    "Network",
    "Web UI",
    "Energy",
    "Sensors",
    "IR & RF",
    "Zigbee",
    "Tuya",
    "Other",
];

// Category of each option; "" = unused slot.
const CATEGORY: [&str; 178] = [
    "General",
    "Buttons & switches",
    "MQTT",
    "MQTT",
    "MQTT",
    "MQTT", // 0
    "MQTT",
    "MQTT",
    "Sensors",
    "MQTT",
    "MQTT",
    "Buttons & switches", // 6
    "General",
    "Buttons & switches",
    "Relays",
    "Lights",
    "Lights",
    "Lights", // 12
    "Lights",
    "Home Assistant",
    "Lights",
    "Energy",
    "MQTT",
    "MQTT", // 18
    "Sensors",
    "Other",
    "MQTT",
    "Other",
    "IR & RF",
    "IR & RF", // 24
    "Home Assistant",
    "Network",
    "Buttons & switches",
    "Energy",
    "General",
    "", // 30
    "General",
    "Lights",
    "IR & RF",
    "Energy",
    "Buttons & switches",
    "Network", // 36
    "Energy",
    "Lights",
    "IR & RF",
    "Relays",
    "Relays",
    "Relays", // 42
    "Energy",
    "",
    "General",
    "General",
    "General",
    "Web UI", // 48
    "Tuya",
    "Network",
    "Network",
    "Network",
    "IR & RF",
    "MQTT", // 54
    "General",
    "Buttons & switches",
    "MQTT",
    "Relays",
    "MQTT",
    "General", // 60
    "Tuya",
    "Other",
    "Lights",
    "Other",
    "Energy",
    "Energy", // 66
    "Energy",
    "Buttons & switches",
    "Sensors",
    "MQTT",
    "General",
    "Lights", // 72
    "",
    "Sensors",
    "Buttons & switches",
    "Other",
    "Lights",
    "Zigbee", // 78
    "MQTT",
    "Lights",
    "Lights",
    "Lights",
    "Lights",
    "Zigbee", // 84
    "MQTT",
    "Lights",
    "Lights",
    "General",
    "Sensors",
    "Network", // 90
    "Network",
    "Tuya",
    "Lights",
    "Lights",
    "Zigbee",
    "Zigbee", // 96
    "Energy",
    "MQTT",
    "MQTT",
    "Lights",
    "Lights",
    "Lights", // 102
    "Energy",
    "Lights",
    "Zigbee",
    "Other",
    "Zigbee",
    "Lights", // 108
    "Buttons & switches",
    "Other",
    "Zigbee",
    "Lights",
    "Zigbee",
    "Zigbee", // 114
    "Zigbee",
    "MQTT",
    "MQTT",
    "Sensors",
    "Sensors",
    "Zigbee", // 120
    "Sensors",
    "Network",
    "Web UI",
    "Energy",
    "General",
    "Tuya", // 126
    "MQTT",
    "Other",
    "Lights",
    "Other",
    "Tuya",
    "Tuya", // 132
    "Web UI",
    "Sensors",
    "MQTT",
    "Web UI",
    "Network",
    "Zigbee", // 138
    "Zigbee",
    "MQTT",
    "Sensors",
    "MQTT",
    "Lights",
    "Network", // 144
    "Energy",
    "Other",
    "Relays",
    "Other",
    "Other",
    "Lights", // 150
    "Sensors",
    "Sensors",
    "MQTT",
    "Sensors",
    "Sensors",
    "Web UI", // 156
    "Energy",
    "Web UI",
    "Other",
    "MQTT",
    "General",
    "", // 162
    "",
    "",
    "",
    "",
    "",
    "", // 168
    "",
    "",
    "",
    "", // 174
];

pub fn category(n: u16) -> &'static str {
    CATEGORY.get(n as usize).copied().unwrap_or("")
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Normal,
    /// Changes how this app (and Home Assistant) read the device: confirm.
    App,
    /// Can make the device misbehave or harder to recover: confirm.
    Danger,
}

/// Risk of changing option `n`, with the reason.
pub fn risk(n: u16) -> (Risk, Option<&'static str>) {
    use Risk::*;
    match n {
        3 => (
            Danger,
            Some("With MQTT off the device disappears from this app and from Home Assistant."),
        ),
        4 => (
            App,
            Some(
                "Changes the topic of command answers (RESULT or one per command): apps that expect the other one stop seeing them.",
            ),
        ),
        19 => (
            App,
            Some(
                "Changes how Home Assistant discovers the device; this app falls back to the LWT topic.",
            ),
        ),
        26 => (
            App,
            Some(
                "Renames the relay of single-relay devices (POWER or POWER1): automations that use the old name stop working.",
            ),
        ),
        90 => (
            App,
            Some(
                "Stops non-JSON answers (like stat/…/POWER): apps that read them stop seeing the state.",
            ),
        ),
        104 => (
            Danger,
            Some(
                "The device restarts. Without retained messages Home Assistant and this app don't know the state of the device until it reports again.",
            ),
        ),
        12 => (
            Danger,
            Some("A fixed flash slot wears out one flash sector: change it only if you know why."),
        ),
        36 => (
            Danger,
            Some(
                "Boot loop protection: at 0 a configuration that makes the device crash is never reset.",
            ),
        ),
        65 => (
            Danger,
            Some(
                "Fast power cycle recovery: when disabled, switching the power on and off quickly no longer resets a misconfigured device.",
            ),
        ),
        78 => (
            Danger,
            Some("Without the OTA compatibility check a wrong firmware can be installed."),
        ),
        95 => (
            Danger,
            Some("With Wi-Fi off the device is cut off from the network."),
        ),
        96 => (
            Danger,
            Some("On a wired ESP32, turning Ethernet off cuts the device off from the network."),
        ),
        99 => (
            Danger,
            Some("Only for zero-cross dimmer hardware: the wrong setting makes lights flicker."),
        ),
        103 => (
            Danger,
            Some("With a broker without TLS the device can no longer connect."),
        ),
        128 => (
            Danger,
            Some(
                "At 1 any web page can send commands to the device (no check of where requests come from).",
            ),
        ),
        132 => (
            Danger,
            Some("A wrong TLS validation setting can stop the MQTT connection."),
        ),
        _ => (Normal, None),
    }
}

/// Where the documentation is wrong or out of date, checked against the firmware source
/// (`tasmota/tasmota_support/settings.ino`, 16.0): the default of a freshly flashed device and,
/// when the documentation describes the values wrongly, what they really mean.
fn correction(n: u16) -> Option<(u32, &'static [&'static str])> {
    match n {
        // Not set by SettingsDefaultSet2: 0 on a new device (the documentation says 1).
        40 | 93 => Some((0, &[])),
        // A different option since 14.2.0.3 (upgrades reset it to 0); the documentation
        // still describes the old Tuya dimmer limit.
        69 => Some((
            0,
            &[
                "0 = normal serial receive (default)",
                "1 = inverted serial receive",
            ],
        )),
        // The bit is `disable_referer_chk`: the documentation has the values reversed.
        128 => Some((
            0,
            &[
                "0 = HTTP API commands are accepted only from the device's own web pages (default)",
                "1 = HTTP API commands are accepted from any page (no referer check)",
            ],
        )),
        _ => None,
    }
}

/// How the default changed over time, when an upgraded device can differ from a new one
/// (checked against the firmware source).
fn history(n: u16) -> Option<&'static str> {
    match n {
        57 => Some(
            "History: default 0 up to 9.1, 1 since 9.2.0.3. Upgrades up to 16.0.0 turn it on; the \
             development builds after 16.0.0 no longer do, so a device upgraded from 9.1 or older \
             can keep 0.",
        ),
        _ => None,
    }
}

/// Why the value is refused, when it is: options that would cut the device off from this
/// app with no way back from here.
pub fn blocked(n: u16, value: u32) -> Option<&'static str> {
    match (n, value) {
        (3, 0) => Some(
            "Turning MQTT off from here would cut the device off from this app: use the device's web UI or console.",
        ),
        (95, 0) => Some(
            "Turning Wi-Fi off from here would cut the device off from the network: use the serial console.",
        ),
        _ => None,
    }
}

// --------------------------------------------------------------------- descriptions

/// One option as described by the documentation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocEntry {
    pub lines: Vec<String>,
    pub default: Option<u32>,
    pub range: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Default)]
pub struct Texts {
    pub docs: HashMap<u16, DocEntry>,
    /// One-line description from decode-status.py, without the "(Category)" prefix.
    pub script: HashMap<u16, String>,
}

/// The documentation's command table rows: `SetOption34<a … id="setoption34"></a>|text`.
pub fn parse_docs(md: &str) -> HashMap<u16, DocEntry> {
    let mut out = HashMap::new();
    for line in md.lines() {
        // "SetOption34<a …>|text", or with another command first:
        // "BuzzerActive<a …><BR>SetOption67<a …>|text".
        let Some((head, text)) = line.split_once('|') else {
            continue;
        };
        let Some(i) = head.find("SetOption") else {
            continue;
        };
        let alias = &head[..i];
        if !alias.is_empty() && !alias.to_ascii_lowercase().ends_with("<br>") {
            continue;
        }
        let digits: String = head[i + 9..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let Ok(n) = digits.parse::<u16>() else {
            continue;
        };
        let alias = alias.split('<').next().unwrap_or("").trim();
        let mut lines: Vec<String> = split_br(text)
            .iter()
            .map(|l| clean_markdown(l))
            .filter(|l| !l.is_empty())
            .collect();
        if !alias.is_empty() && !lines.is_empty() {
            lines.push(format!("Same as the {alias} command."));
        }
        if lines.is_empty() {
            continue;
        }
        let mut e = DocEntry {
            default: None,
            range: None,
            lines,
        };
        for l in &e.lines {
            let lower = l.to_ascii_lowercase();
            // "1 = enable (default)", "0..255 = … (default = 200)"
            // The last one: "(default 10 seconds) … (default = 1)" describes the option itself.
            if let Some(i) = lower.rfind("(default") {
                let after = &lower[i + 8..];
                let explicit: String = after
                    .trim_start_matches([' ', '=', ':'])
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                if let Ok(v) = explicit.parse() {
                    e.default = Some(v);
                } else if let Some(v) = leading_value(l) {
                    e.default = Some(v);
                }
            }
            if let Some((a, b)) = leading_range(l) {
                e.range = Some((a, b));
            }
        }
        out.insert(n, e);
    }
    out
}

/// "1 = keep it (default)" → "1 = keep it"; also "(default = 1)", "(default =1)".
fn strip_default(l: &str) -> String {
    let mut out = l.to_string();
    while let Some(i) = out.to_ascii_lowercase().find("(default") {
        let Some(j) = out[i..].find(')') else { break };
        out.replace_range(i..i + j + 1, "");
    }
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" .", ".")
}

/// `0 = …` → 0.
fn leading_value(l: &str) -> Option<u32> {
    let (v, _) = l.split_once('=')?;
    v.trim().parse().ok()
}

/// `0..255 = …`, `1..100 to set …` → (0, 255), (1, 100).
fn leading_range(l: &str) -> Option<(u32, u32)> {
    let first = l.split_whitespace().next()?.trim_end_matches(['=', ':']);
    let (a, b) = first.split_once("..")?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// "0 = …", "1..200 = …": a line that explains a value, not a heading.
fn is_value_line(l: &str) -> bool {
    l.starts_with(|c: char| c.is_ascii_digit()) && (l.contains('=') || l.contains(".."))
}

fn split_br(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        let hit = ["<br>", "<br/>", "<br />"]
            .iter()
            .filter_map(|t| lower.find(t).map(|i| (i, t.len())))
            .min();
        match hit {
            Some((i, len)) => {
                parts.push(rest[..i].to_string());
                rest = &rest[i + len..];
            }
            None => {
                parts.push(rest.to_string());
                return parts;
            }
        }
    }
}

/// Markdown + inline HTML → plain text: links keep their text, tags, backticks and
/// emphasis go (underscores inside words like BOOT_LOOP_TIME stay).
pub fn clean_markdown(s: &str) -> String {
    // HTML tags.
    let mut no_tags = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => no_tags.push(c),
            _ => {}
        }
    }
    // [text](link) → text
    let mut out = String::new();
    let mut rest = no_tags.as_str();
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find("](") {
            Some(j) if !after[..j].contains('[') => {
                let label = &after[..j];
                let tail = &after[j + 2..];
                match tail.find(')') {
                    Some(k) => {
                        out.push_str(label);
                        rest = &tail[k + 1..];
                    }
                    None => {
                        out.push('[');
                        rest = after;
                    }
                }
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    let chars: Vec<char> = out.chars().collect();
    let mut text = String::new();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '`' | '*' => {}
            '_' => {
                let prev = i.checked_sub(1).map(|p| chars[p]);
                let next = chars.get(i + 1).copied();
                let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
                if word(prev) && word(next) {
                    text.push('_');
                }
            }
            _ => text.push(c),
        }
    }
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .split_whitespace()
        .filter(|w| !is_shortcode(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// MkDocs emoji like `:rotating_light:`.
fn is_shortcode(w: &str) -> bool {
    w.len() > 2
        && w.starts_with(':')
        && w.ends_with(':')
        && w[1..w.len() - 1]
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The `a_setoption = [[ "…", … ],[ … ]]` list of decode-status.py: six groups that start at
/// SO0, 32, 50, 82, 114, 146. Empty strings are unused slots.
pub fn parse_script(py: &str) -> HashMap<u16, String> {
    const BASE: [u16; 6] = [0, 32, 50, 82, 114, 146];
    let mut out = HashMap::new();
    let Some(start) = py.find("a_setoption = [[") else {
        return out;
    };
    let body = &py[start + "a_setoption = [[".len()..];
    let mut group = 0usize;
    let mut index = 0u16;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(e) = chars.next() {
                                s.push(e)
                            }
                        }
                        '"' => break,
                        c => s.push(c),
                    }
                }
                if let Some(base) = BASE.get(group) {
                    let text = match s.strip_prefix('(').and_then(|r| r.split_once(')')) {
                        // "(Timers) Enabled": a short line needs its subject.
                        Some((cat, t)) if t.trim().len() < 24 => {
                            format!("{}: {}", cat.trim(), t.trim())
                        }
                        Some((_, t)) => t.trim().to_string(),
                        None => s.trim().to_string(),
                    };
                    if !s.trim().is_empty() {
                        out.insert(base + index, text);
                    }
                }
                index += 1;
            }
            ']' => {
                if chars.peek() == Some(&']') {
                    break;
                }
                group += 1;
                index = 0;
            }
            _ => {}
        }
    }
    out
}

#[derive(Clone)]
enum TextState {
    Idle,
    Loading,
    Ready(Arc<Texts>),
    Failed(String, Instant),
}

struct Loader {
    state: TextState,
    /// From the disk cache while the download runs or after it failed.
    cached: Option<Arc<Texts>>,
}

fn loader() -> &'static Mutex<Loader> {
    static L: OnceLock<Mutex<Loader>> = OnceLock::new();
    L.get_or_init(|| {
        Mutex::new(Loader {
            state: TextState::Idle,
            cached: read_cache(),
        })
    })
}

fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "tasmota-derust").map(|d| d.cache_dir().to_path_buf())
}

fn read_cache() -> Option<Arc<Texts>> {
    let dir = cache_dir()?;
    let docs = std::fs::read_to_string(dir.join("Commands.md")).unwrap_or_default();
    let py = std::fs::read_to_string(dir.join("decode-status.py")).unwrap_or_default();
    let t = Texts {
        docs: parse_docs(&docs),
        script: parse_script(&py),
    };
    (!t.docs.is_empty() || !t.script.is_empty()).then(|| Arc::new(t))
}

fn download(url: &str) -> Result<String, String> {
    ureq::get(url)
        .header("User-Agent", "tasmota-derust")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// Starts downloading the descriptions, once (again a minute after a failure).
pub fn load_texts(notify: Notify) {
    {
        let mut l = loader().lock().unwrap();
        match &l.state {
            TextState::Loading | TextState::Ready(_) => return,
            TextState::Failed(_, at) if at.elapsed() < Duration::from_secs(60) => return,
            _ => {}
        }
        l.state = TextState::Loading;
    }
    std::thread::spawn(move || {
        let docs = download(DOCS_RAW);
        let py = download(SCRIPT_RAW);
        let dir = cache_dir();
        let save = |name: &str, text: &str| {
            if let Some(d) = &dir {
                let _ = std::fs::create_dir_all(d);
                let _ = std::fs::write(d.join(name), text);
            }
        };
        let mut t = Texts::default();
        if let Ok(md) = &docs {
            t.docs = parse_docs(md);
            if !t.docs.is_empty() {
                save("Commands.md", md);
            }
        }
        if let Ok(p) = &py {
            t.script = parse_script(p);
            if !t.script.is_empty() {
                save("decode-status.py", p);
            }
        }
        let mut l = loader().lock().unwrap();
        l.state = if t.docs.is_empty() && t.script.is_empty() {
            let e = docs
                .err()
                .or(py.err())
                .unwrap_or_else(|| "no descriptions found".into());
            TextState::Failed(e, Instant::now())
        } else {
            // Keep what the cache had for the source that failed this time.
            if let Some(c) = &l.cached {
                if t.docs.is_empty() {
                    t.docs = c.docs.clone();
                }
                if t.script.is_empty() {
                    t.script = c.script.clone();
                }
            }
            TextState::Ready(Arc::new(t))
        };
        drop(l);
        notify();
    });
}

/// The descriptions available now and the state of the download.
fn current_texts() -> (Option<Arc<Texts>>, &'static str, Option<String>) {
    let l = loader().lock().unwrap();
    match &l.state {
        TextState::Ready(t) => (Some(t.clone()), "ready", None),
        TextState::Loading | TextState::Idle => (l.cached.clone(), "loading", None),
        TextState::Failed(e, _) => (l.cached.clone(), "failed", Some(e.clone())),
    }
}

// ------------------------------------------------------------------------ the view

/// One option as the front ends draw it.
#[derive(Serialize, Clone, Debug)]
pub struct OptionInfo {
    pub n: u16,
    pub category: &'static str,
    /// A number (SO32–49) instead of an on/off switch.
    pub numeric: bool,
    pub value: u32,
    pub default: Option<u32>,
    pub min: u32,
    pub max: u32,
    /// Short description ("SetOption N" without descriptions).
    pub title: String,
    /// Meaning of the values and notes, from the documentation.
    pub details: Vec<String>,
    pub risk: Risk,
    pub warning: Option<&'static str>,
    /// Value that is refused (SO3 = 0, SO95 = 0), and why.
    pub blocked_value: Option<u32>,
    pub blocked_reason: Option<&'static str>,
    pub link: String,
    /// SO32–49: a 0 means the firmware's built-in default (the documentation does not
    /// allow 0, or says "0 = reset to default").
    pub zero_is_default: bool,
}

impl OptionInfo {
    /// Same as the default the documentation gives. For SO32–49 a 0 the documentation
    /// does not allow means "the firmware's built-in default" (seen on real devices).
    pub fn is_default(&self) -> bool {
        (self.numeric && self.value == 0 && self.zero_is_default)
            || self.default.is_none_or(|d| d == self.value)
    }

    /// Search box: every word must match. A number ("4", "so4", "setoption4") matches that
    /// option only, not SO40–49.
    pub fn matches(&self, query: &str) -> bool {
        let hay = self.haystack();
        query.split_whitespace().all(|t| {
            let t = t.to_lowercase();
            let digits = t
                .strip_prefix("setoption")
                .or_else(|| t.strip_prefix("so"))
                .unwrap_or(&t);
            match digits.parse::<u16>() {
                Ok(n) if !digits.is_empty() => n == self.n,
                _ => hay.contains(&t),
            }
        })
    }

    /// Lower-case text the search box matches against.
    pub fn haystack(&self) -> String {
        format!(
            "so{n} setoption{n} {} {} {}",
            self.title,
            self.category,
            self.details.join(" "),
            n = self.n
        )
        .to_lowercase()
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct OptionsView {
    /// The device has reported its options (a `Status 0` or `Status 3` answer).
    pub reported: bool,
    /// Descriptions: "loading", "ready" or "failed" (shown by number then).
    pub texts: &'static str,
    pub texts_error: Option<String>,
    /// Ordered by category (in `CATEGORIES` order), then number.
    pub options: Vec<OptionInfo>,
    pub docs: &'static str,
}

/// The options of a device, from its registers.
pub fn view(regs: &[String]) -> OptionsView {
    let (texts, state, error) = current_texts();
    let mut options: Vec<OptionInfo> = decode(regs)
        .into_iter()
        .filter_map(|(n, value)| {
            let doc = texts.as_ref().and_then(|t| t.docs.get(&n));
            let line = texts.as_ref().and_then(|t| t.script.get(&n));
            let mut category = category(n);
            if category.is_empty() {
                // Unused slot, unless the documentation knows a newer option.
                doc?;
                category = "Other";
            }
            let numeric = is_numeric(n);
            let (mut title, mut details) = (format!("SetOption{n}"), Vec::new());
            match (doc, line) {
                (Some(d), l) => {
                    let first = &d.lines[0];
                    match l {
                        // No heading, only the values: the one-line summary is the title.
                        Some(l) if is_value_line(first) => {
                            title = l.clone();
                            details.extend(d.lines.iter().cloned());
                        }
                        // A very short heading ("MQTT") needs the summary too.
                        // ("MQTT: Enable" already starts with the heading.)
                        Some(l) if first.len() < 16 => {
                            let head = first.trim_end_matches(':');
                            title = if l.to_lowercase().starts_with(&head.to_lowercase()) {
                                l.clone()
                            } else {
                                format!("{head}: {l}")
                            };
                            details.extend(d.lines[1..].iter().cloned());
                        }
                        _ => {
                            title = first.clone();
                            details.extend(d.lines[1..].iter().cloned());
                        }
                    }
                }
                (None, Some(l)) => title = l.clone(),
                (None, None) => {}
            }
            let fix = correction(n);
            if let Some((d, lines)) = fix {
                if lines.is_empty() {
                    // Move "(default)" to the line of the real default.
                    for l in details.iter_mut().filter(|l| is_value_line(l)) {
                        *l = strip_default(l);
                        if leading_value(l) == Some(d) {
                            l.push_str(" (default)");
                        }
                    }
                } else {
                    // The documentation describes something else: only our lines.
                    details.clear();
                    details.extend(lines.iter().map(|l| l.to_string()));
                }
            }
            if let Some(h) = history(n) {
                details.push(h.to_string());
            }
            let mut zero_is_default = false;
            let (min, max) = if numeric {
                let (min, max) = doc.and_then(|d| d.range).unwrap_or((0, 255));
                // "0 = reset to default on next restart": 0 is allowed too.
                let zero = doc.and_then(|d| d.lines.iter().find(|l| l.starts_with("0 =")));
                zero_is_default = match zero {
                    Some(l) => l.to_lowercase().contains("default"),
                    None => min > 0,
                };
                (if zero.is_some() { 0 } else { min }, max)
            } else {
                (0, 1)
            };
            let default = fix
                .map(|(d, _)| d)
                .or_else(|| doc.and_then(|d| d.default))
                .or_else(|| line.and_then(|l| script_default(l)))
                .filter(|d| *d <= max);
            let (risk, warning) = risk(n);
            let blocked_value = [0u32, 1].into_iter().find(|v| blocked(n, *v).is_some());
            Some(OptionInfo {
                n,
                category,
                numeric,
                value,
                default,
                min,
                max: max.max(min),
                title,
                details,
                risk,
                warning,
                blocked_value,
                blocked_reason: blocked_value.and_then(|v| blocked(n, v)),
                zero_is_default,
                link: doc_link(n),
            })
        })
        .collect();
    let rank = |c: &str| {
        CATEGORIES
            .iter()
            .position(|x| *x == c)
            .unwrap_or(CATEGORIES.len())
    };
    options.sort_by_key(|o| (rank(o.category), o.n));
    OptionsView {
        reported: !regs.is_empty(),
        texts: state,
        texts_error: error,
        options,
        docs: "https://tasmota.github.io/docs/Commands/#setoptions",
    }
}

/// "… (default 40)" → 40.
fn script_default(l: &str) -> Option<u32> {
    let i = l.find("(default ")?;
    let d: String = l[i + 9..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    d.parse().ok()
}

/// Checks a new value; the payload of `SetOption<n>` when it is allowed.
pub fn validate(n: u16, value: u32) -> Result<String, &'static str> {
    if n > MAX_OPTION {
        return Err("No such SetOption.");
    }
    if let Some(why) = blocked(n, value) {
        return Err(why);
    }
    let max = if is_numeric(n) { 255 } else { 1 };
    if value > max {
        return Err("Value out of range.");
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn decode_current_firmware() {
        let r = regs(&[
            "00008009",
            "2805C80001000600003C5A0A192800000000",
            "00000080",
            "00006000",
            "00006000",
            "00000001",
        ]);
        let d: HashMap<u16, u32> = decode(&r).into_iter().collect();
        assert_eq!(d.len(), 32 + 18 + 4 * 32);
        assert_eq!(d[&0], 1);
        assert_eq!(d[&3], 1);
        assert_eq!(d[&4], 0);
        assert_eq!(d[&15], 1);
        assert_eq!(d[&32], 0x28); // 40
        assert_eq!(d[&34], 0xC8); // 200
        assert_eq!(d[&57], 1);
        assert_eq!(d[&95], 1); // 82 + 13
        assert_eq!(d[&146], 1);
    }

    #[test]
    fn decode_old_firmware() {
        // Only SO0-31 and the resolution.
        assert_eq!(decode(&regs(&["00008009", "55818000"])).len(), 32);
        // Pre 6.1.1.14: register 1 is 8 digits, skipped.
        let d = decode(&regs(&["00008009", "55818000", "00000001"]));
        assert_eq!(d.len(), 64);
        assert!(d.contains(&(50, 1)));
        // 6.7.1: four registers.
        let d = decode(&regs(&[
            "00008009",
            "2805C80001000600003C5A0A000000000000",
            "00000000",
            "00000000",
        ]));
        assert_eq!(d.len(), 32 + 18 + 64);
    }

    #[test]
    fn apply_answers() {
        let mut r = regs(&[
            "00008009",
            "2805C80001000600003C5A0A192800000000",
            "00000080",
            "00006000",
        ]);
        apply(&mut r, 4, 1);
        apply(&mut r, 0, 0);
        apply(&mut r, 34, 100);
        apply(&mut r, 59, 1);
        let d: HashMap<u16, u32> = decode(&r).into_iter().collect();
        assert_eq!((d[&4], d[&0], d[&34], d[&59]), (1, 0, 100, 1));
        assert_eq!(
            parse_answer("SetOption4", &serde_json::json!("ON")),
            Some((4, 1))
        );
        assert_eq!(
            parse_answer("SetOption34", &serde_json::json!(200)),
            Some((34, 200))
        );
        assert_eq!(parse_answer("Power", &serde_json::json!("ON")), None);
    }

    #[test]
    fn docs_rows() {
        let md = "SetOption0<a class=\"cmnd\" id=\"setoption0\"></a>|Save power state and use after restart (=SaveState)<BR> `0` = disable (see note below)<BR> `1` = enable *(default)*<BR>Note: see [`SaveData`](#savedata).\n\
SetOption34<a class=\"cmnd\" id=\"setoption34\"></a>|`0..255` = set [Backlog](#backlog) inter-command delay in milliseconds *(default = `200`)*\n\
SetOption36<a class=\"cmnd\" id=\"setoption36\"></a>|Boot loop control (`BOOT_LOOP_TIME`)<BR>`0` = disable<BR> `1..200` = set number of boot loops (within `BOOT_LOOP_TIME` *(default 10 seconds)*) *(default = `1`)*\n\
SetOption113<a class=\"cmnd\" id=\"setoption113\"></a>|_works only with rotary dial button_<BR>`0` = _(default)_<BR>`1` = set dimmer low\n\
NoDelay<a class=\"cmnd\" id=\"nodelay\"></a>|Delay defined by `SetOption34`\n\
BuzzerActive<a class=\"cmnd\" id=\"buzzeractive\"></a><BR>SetOption67<a class=\"cmnd\" id=\"setoption67\"></a>|iFan03 Buzzer control<BR>`0` = disable *(default)*<BR>`1` = enable";
        let d = parse_docs(md);
        assert_eq!(d.len(), 5);
        assert_eq!(d[&67].lines[0], "iFan03 Buzzer control");
        assert_eq!(d[&67].default, Some(0));
        assert_eq!(
            d[&67].lines.last().unwrap(),
            "Same as the BuzzerActive command."
        );
        assert_eq!(
            d[&0].lines[0],
            "Save power state and use after restart (=SaveState)"
        );
        assert_eq!(d[&0].lines[2], "1 = enable (default)");
        assert_eq!(d[&0].lines[3], "Note: see SaveData.");
        assert_eq!(d[&0].default, Some(1));
        assert_eq!(d[&34].default, Some(200));
        assert_eq!(d[&34].range, Some((0, 255)));
        assert_eq!(d[&36].lines[0], "Boot loop control (BOOT_LOOP_TIME)");
        assert_eq!(d[&36].range, Some((1, 200)));
        assert_eq!(d[&36].default, Some(1));
        assert_eq!(d[&113].lines[0], "works only with rotary dial button");
        assert_eq!(d[&113].default, Some(0));
        assert_eq!(
            leading_range("1..100 to set button hold time"),
            Some((1, 100))
        );
        assert_eq!(strip_default("1 = keep (default)"), "1 = keep");
        assert_eq!(
            strip_default("0..250 = hold time.(default =1)"),
            "0..250 = hold time."
        );
        assert!(is_value_line("0..255 = set delay"));
        assert!(!is_value_line("Boot loop control"));
    }

    #[test]
    fn script_list() {
        let py = "x = 1\na_setoption = [[\n    \"(Settings) Save power state\",\n    \"(Button) Single press\"\n    ],[\n    \"(Button/Switch) Key hold time (default 40)\",\n    \"\",\n    ],[\n    \"(Timers) Enabled\"\n    ]]\n\na_features = [[\"x\"]]";
        let s = parse_script(py);
        assert_eq!(s[&0], "Settings: Save power state");
        assert_eq!(s[&1], "Button: Single press");
        assert_eq!(s[&32], "Key hold time (default 40)");
        assert!(!s.contains_key(&33));
        assert_eq!(s[&50], "Timers: Enabled");
        assert_eq!(
            clean_markdown(":rotating_light: WARNING Do not use: 0 = off"),
            "WARNING Do not use: 0 = off"
        );
        assert_eq!(s.len(), 4);
        assert_eq!(script_default(&s[&32]), Some(40));
    }

    #[test]
    fn view_without_texts_and_rules() {
        let v = view(&regs(&["00000008", "55818000"]));
        assert!(v.reported);
        let so3 = v.options.iter().find(|o| o.n == 3).unwrap();
        assert_eq!(so3.value, 1);
        assert_eq!(so3.category, "MQTT");
        assert_eq!(so3.blocked_value, Some(0));
        // Unused slots are left out.
        assert!(v.options.iter().all(|o| !o.category.is_empty()));
        assert!(validate(3, 0).is_err());
        assert!(validate(3, 1).is_ok());
        assert!(validate(95, 0).is_err());
        assert!(validate(4, 2).is_err());
        assert_eq!(validate(34, 250).as_deref(), Ok("250"));
        assert!(validate(34, 256).is_err());
        assert!(!view(&[]).reported);
        // SO44 = 0 with a documented range 1..100: the built-in default.
        let mut o = so3.clone();
        (o.numeric, o.value, o.min, o.default) = (true, 0, 1, Some(25));
        o.zero_is_default = true;
        assert!(o.is_default());
        o.zero_is_default = false;
        assert!(!o.is_default());
        let so4 = v.options.iter().find(|o| o.n == 4).unwrap();
        let so40 = v.options.iter().find(|o| o.n == 40);
        assert!(so4.matches("so4") && so4.matches("SetOption4") && so4.matches("4 mqtt"));
        assert!(!so4.matches("so40") && !so4.matches("light"));
        assert!(so40.is_none_or(|o| !o.matches("so4")));
    }
}
