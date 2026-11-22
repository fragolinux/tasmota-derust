//! Tasmota devices reached over HTTP only (no MQTT): finding them on the local network,
//! sending them commands, and the list of the ones found (saved in `http_devices.json`, so
//! they come back at the next start).
//!
//! Every Tasmota with its web server answers `GET /cm?cmnd=<command>` with the same JSON as
//! MQTT (`/cm?user=admin&password=<pw>&cmnd=…` when it has a web password). The search asks
//! `Status 0` to every address of a range, many at a time with short timeouts: read-only.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;

/// Device keys of HTTP-only devices start with this (`http:192.168.1.50`).
pub const KEY_PREFIX: &str = "http:";
/// Most addresses one search may ask.
const MAX_HOSTS: usize = 1024;
/// Addresses asked at the same time.
const PARALLEL: usize = 64;

pub fn key(host: &str) -> String {
    format!("{KEY_PREFIX}{host}")
}

pub fn host_of_key(key: &str) -> Option<&str> {
    key.strip_prefix(KEY_PREFIX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    BadRange,
    TooMany,
    NoNetwork,
}

impl ScanError {
    pub fn message(self) -> &'static str {
        match self {
            ScanError::BadRange => {
                "Addresses: leave empty for this network, or write e.g. 192.168.1.0/24, 192.168.1.20-80 or 192.168.1.50."
            }
            ScanError::TooMany => "Too many addresses: at most 1024 per search.",
            ScanError::NoNetwork => "This device is not on a local network.",
        }
    }
}

/// This device's own network, from its address and netmask (e.g. `192.168.1.0/24`): what an
/// empty range means. The interface is the one the default route uses. Networks bigger
/// than /22 (more than 1022 addresses) are cut to the /24 around this device.
pub fn local_range() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1: nothing is sent
    let std::net::IpAddr::V4(ip) = s.local_addr().ok()?.ip() else {
        return None;
    };
    if ip.is_loopback() || ip.is_unspecified() {
        return None;
    }
    let mask = if_addrs::get_if_addrs()
        .ok()
        .and_then(|list| {
            list.into_iter().find_map(|i| match i.addr {
                if_addrs::IfAddr::V4(a) if a.ip == ip => Some(a.netmask),
                _ => None,
            })
        })
        .unwrap_or(Ipv4Addr::new(255, 255, 255, 0));
    let lead = u32::from(mask).leading_ones();
    let bits = if lead < 22 { 24 } else { lead };
    let m = if bits >= 32 { u32::MAX } else { !(u32::MAX >> bits) };
    Some(format!("{}/{bits}", Ipv4Addr::from(u32::from(ip) & m)))
}

/// The addresses of a range: empty (this network), `a.b.c.d/nn` (nn ≥ 22), `a.b.c.x-y`,
/// one address, or `host:port1-port2` (several ports on one host, for tests); comma separated.
pub fn hosts(range: &str) -> Result<Vec<String>, ScanError> {
    let range = range.trim();
    let auto;
    let range = if range.is_empty() {
        auto = local_range().ok_or(ScanError::NoNetwork)?;
        auto.as_str()
    } else {
        range
    };
    let mut out = BTreeSet::new();
    for part in range.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((h, ports)) = part.rsplit_once(':') {
            let (a, b) = ports.split_once('-').unwrap_or((ports, ports));
            let (a, b): (u16, u16) = (
                a.trim().parse().map_err(|_| ScanError::BadRange)?,
                b.trim().parse().map_err(|_| ScanError::BadRange)?,
            );
            if h.parse::<Ipv4Addr>().is_err() || a == 0 || b < a {
                return Err(ScanError::BadRange);
            }
            for p in a..=b {
                out.insert(format!("{h}:{p}"));
            }
        } else if let Some((ip, bits)) = part.split_once('/') {
            let ip: Ipv4Addr = ip.trim().parse().map_err(|_| ScanError::BadRange)?;
            let bits: u32 = bits.trim().parse().map_err(|_| ScanError::BadRange)?;
            if !(22..=32).contains(&bits) {
                return Err(if bits < 22 { ScanError::TooMany } else { ScanError::BadRange });
            }
            let mask = if bits == 32 { u32::MAX } else { !(u32::MAX >> bits) };
            let net = u32::from(ip) & mask;
            let size = 1u32 << (32 - bits);
            let (first, last) = if size > 2 { (1, size - 2) } else { (0, size - 1) };
            for i in first..=last {
                out.insert(Ipv4Addr::from(net + i).to_string());
            }
        } else if let Some((ip, last)) = part.split_once('-') {
            let ip: Ipv4Addr = ip.trim().parse().map_err(|_| ScanError::BadRange)?;
            let o = ip.octets();
            let last: u8 = last.trim().parse().map_err(|_| ScanError::BadRange)?;
            if last < o[3] {
                return Err(ScanError::BadRange);
            }
            for x in o[3]..=last {
                out.insert(Ipv4Addr::new(o[0], o[1], o[2], x).to_string());
            }
        } else {
            let ip: Ipv4Addr = part.parse().map_err(|_| ScanError::BadRange)?;
            out.insert(ip.to_string());
        }
        if out.len() > MAX_HOSTS {
            return Err(ScanError::TooMany);
        }
    }
    if out.is_empty() {
        return Err(ScanError::BadRange);
    }
    Ok(out.into_iter().collect())
}

fn encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// The command URL (the password, when given, goes in the query as Tasmota expects).
pub fn url(host: &str, cmnd: &str, password: Option<&str>) -> String {
    match password.filter(|p| !p.is_empty()) {
        Some(p) => format!(
            "http://{host}/cm?user=admin&password={}&cmnd={}",
            encode(p),
            encode(cmnd)
        ),
        None => format!("http://{host}/cm?cmnd={}", encode(cmnd)),
    }
}

/// What one address answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// A Tasmota JSON answer.
    Json(Value),
    /// A Tasmota with a web password that was not given (or is wrong).
    NeedsPassword,
    /// Something answered, but it is not a Tasmota (Shelly, router, printer…).
    NotTasmota,
    /// Nothing answered (no web server there, or it is off).
    Nothing,
}

fn ask(agent: &ureq::Agent, host: &str, cmnd: &str, password: Option<&str>) -> Answer {
    let Ok(mut resp) = agent
        .get(&url(host, cmnd, password))
        .header("User-Agent", "tasmota-derust")
        .call()
    else {
        return Answer::Nothing;
    };
    let code = resp.status().as_u16();
    // Tasmota names itself on every answer: "Server: Tasmota/<version> (<hardware>)".
    let server_tasmota = resp
        .headers()
        .get("server")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Tasmota/"));
    let body = resp
        .body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_to_string()
        .unwrap_or_default();
    classify(code, server_tasmota, &body)
}

/// What an answer to `/cm` means. Other devices with a web server (Shelly, routers…) also
/// answer 401 to an unknown page: only Tasmota's own answer counts. With a web password
/// Tasmota answers 401 and {"WARNING":"Need user=<username>&password=<password>"}.
pub fn classify(code: u16, server_tasmota: bool, body: &str) -> Answer {
    if body.contains("Need user=") && (server_tasmota || body.trim_start().starts_with('{')) {
        return Answer::NeedsPassword;
    }
    if code == 401 {
        return if server_tasmota {
            Answer::NeedsPassword
        } else {
            Answer::NotTasmota
        };
    }
    match serde_json::from_str::<Value>(body) {
        // Tasmota's own JSON: a known device's answer, or a Status 0 with its sections.
        Ok(v) if v.is_object() && (server_tasmota || is_tasmota(&v) || looks_like_result(&v)) => {
            Answer::Json(v)
        }
        _ => Answer::NotTasmota,
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(timeout))
        .timeout_global(Some(timeout * 3))
        .http_status_as_error(false)
        .build()
        .into()
}

/// One command, blocking (a few seconds at most).
pub fn command(host: &str, cmnd: &str, password: Option<&str>) -> Answer {
    ask(&agent(Duration::from_secs(3)), host, cmnd, password)
}

/// A Tasmota found by a search.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub host: String,
    /// `Status 0` answer; None: it wants a web password.
    pub status: Option<Value>,
}

/// An answer to a command from a Tasmota without the Server header (old firmware): the
/// usual RESULT keys.
fn looks_like_result(v: &Value) -> bool {
    v.as_object().is_some_and(|o| {
        o.keys().any(|k| {
            k.starts_with("POWER") || k.starts_with("Status") || k == "Command" || k == "Backlog"
                || k.starts_with("SetOption") || k.starts_with("Mqtt") || k == "Restart"
        })
    })
}

/// Is this `Status 0` answer a Tasmota's?
pub fn is_tasmota(v: &Value) -> bool {
    v.get("Status").is_some_and(Value::is_object)
        && (v.get("StatusFWR").is_some() || v.get("StatusNET").is_some())
}

/// Asks `Status 0` to every host, `PARALLEL` at a time. `done` counts the hosts asked so far
/// (for a progress bar). Blocking: run it on a thread.
pub fn scan(hosts: &[String], password: Option<&str>, done: &AtomicUsize) -> Vec<Found> {
    let queue = Mutex::new(hosts.to_vec());
    let found = Mutex::new(Vec::new());
    let agent = agent(Duration::from_millis(800));
    std::thread::scope(|sc| {
        for _ in 0..PARALLEL.min(hosts.len().max(1)) {
            sc.spawn(|| {
                loop {
                    let Some(h) = queue.lock().unwrap().pop() else { break };
                    match ask(&agent, &h, "Status 0", password) {
                        Answer::Json(v) if is_tasmota(&v) => found.lock().unwrap().push(Found {
                            host: h,
                            status: Some(v),
                        }),
                        Answer::NeedsPassword => found.lock().unwrap().push(Found {
                            host: h,
                            status: None,
                        }),
                        _ => {}
                    }
                    done.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    let mut v = found.into_inner().unwrap();
    v.sort_by_key(|f| natural(&f.host));
    v
}

fn natural(host: &str) -> (Vec<u32>, u32) {
    let (ip, port) = host.split_once(':').unwrap_or((host, "80"));
    (
        ip.split('.').filter_map(|x| x.parse().ok()).collect(),
        port.parse().unwrap_or(80),
    )
}

/// A search running in the background: its progress and, at the end, what it found.
#[derive(Default)]
pub struct Search {
    pub total: usize,
    pub done: Arc<AtomicUsize>,
    pub result: Arc<Mutex<Option<Vec<Found>>>>,
}

// ------------------------------------------------------------------ saved list

struct State {
    dir: Option<PathBuf>,
    hosts: Option<BTreeSet<String>>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State {
            dir: crate::config::Config::dir(),
            hosts: None,
        })
    })
}

/// Where `http_devices.json` goes, for platforms without a standard config directory.
pub fn set_dir(dir: PathBuf) {
    let mut s = state().lock().unwrap();
    s.dir = Some(dir);
    s.hosts = None;
}

fn with_hosts<T>(f: impl FnOnce(&mut BTreeSet<String>) -> (T, bool)) -> T {
    let mut s = state().lock().unwrap();
    let path = s.dir.as_ref().map(|d| d.join("http_devices.json"));
    let hosts = s.hosts.get_or_insert_with(|| {
        path.as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    });
    let (out, changed) = f(hosts);
    if changed && let Some(p) = path {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(hosts) {
            let _ = std::fs::write(p, t);
        }
    }
    out
}

/// HTTP-only devices added earlier (asked again at start).
pub fn saved() -> Vec<String> {
    with_hosts(|h| (h.iter().cloned().collect(), false))
}

pub fn remember(host: &str) {
    with_hosts(|h| ((), h.insert(host.to_string())))
}

pub fn forget(host: &str) {
    with_hosts(|h| ((), h.remove(host)))
}

/// The MQTT settings line for "enable MQTT": MQTT on, broker, port, user and password
/// (user and password only when given). `;` is the Backlog separator, so it is refused.
pub fn mqtt_backlog(host: &str, port: u16, user: &str, password: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() || [host, user, password].iter().any(|s| s.contains(';')) {
        return None;
    }
    let mut parts = vec![
        "SetOption3 1".to_string(),
        format!("MqttHost {host}"),
        format!("MqttPort {port}"),
    ];
    if !user.trim().is_empty() {
        parts.push(format!("MqttUser {}", user.trim()));
    }
    if !password.is_empty() {
        parts.push(format!("MqttPassword {password}"));
    }
    Some(format!("Backlog {}", parts.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(hosts("192.168.1.0/30").unwrap(), ["192.168.1.1", "192.168.1.2"]);
        assert_eq!(hosts("192.168.1.0/24").unwrap().len(), 254);
        assert_eq!(hosts("10.0.0.5-7").unwrap(), ["10.0.0.5", "10.0.0.6", "10.0.0.7"]);
        assert_eq!(hosts("10.0.0.9").unwrap(), ["10.0.0.9"]);
        assert_eq!(
            hosts("127.0.0.1:18081-18082").unwrap(),
            ["127.0.0.1:18081", "127.0.0.1:18082"]
        );
        assert_eq!(hosts("10.0.0.0/16"), Err(ScanError::TooMany));
        assert!(hosts("bad").is_err());
        assert!(hosts("10.0.0.9-3").is_err());
    }

    #[test]
    fn urls_and_mqtt_line() {
        assert_eq!(url("1.2.3.4", "Status 0", None), "http://1.2.3.4/cm?cmnd=Status%200");
        assert_eq!(
            url("1.2.3.4", "Power1 TOGGLE", Some("p&w")),
            "http://1.2.3.4/cm?user=admin&password=p%26w&cmnd=Power1%20TOGGLE"
        );
        assert_eq!(
            mqtt_backlog("192.168.1.99", 1883, "u", "p").unwrap(),
            "Backlog SetOption3 1; MqttHost 192.168.1.99; MqttPort 1883; MqttUser u; MqttPassword p"
        );
        assert_eq!(
            mqtt_backlog("broker", 1883, "", "").unwrap(),
            "Backlog SetOption3 1; MqttHost broker; MqttPort 1883"
        );
        assert!(mqtt_backlog("", 1883, "", "").is_none());
        assert!(mqtt_backlog("h", 1883, "a;b", "").is_none());
        // A Shelly (or any web server) answering 401 is not a Tasmota.
        assert_eq!(classify(401, false, "401 Unauthorized"), Answer::NotTasmota);
        assert_eq!(
            classify(401, true, r#"{"WARNING":"Need user=<username>&password=<password>"}"#),
            Answer::NeedsPassword
        );
        assert_eq!(
            classify(401, false, r#"{"WARNING":"Need user=<username>&password=<password>"}"#),
            Answer::NeedsPassword
        );
        assert_eq!(classify(401, true, ""), Answer::NeedsPassword);
        assert_eq!(classify(200, false, "<html>router</html>"), Answer::NotTasmota);
        assert_eq!(classify(200, false, r#"{"type":"SHSW-1","mac":"x"}"#), Answer::NotTasmota);
        assert!(matches!(classify(200, false, r#"{"POWER":"ON"}"#), Answer::Json(_)));
        let v: Value = serde_json::json!({"Status": {"Topic": "x"}, "StatusFWR": {"Version": "14.0.0"}});
        assert!(is_tasmota(&v));
        assert!(!is_tasmota(&serde_json::json!({"Status": 1})));
    }
}
