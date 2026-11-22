//! Tasmota Derust web interface: the HTTP routes and the embedded page, shared by the
//! standalone `tasmota-derust-web` binary and the desktop app's "Web server" button.
//!
//! Read-only mode: only read queries (Status, Module, SSId) are sent to devices; every
//! command that changes something (relays, Wi-Fi switch, upgrade) is refused with 403.

use std::collections::HashSet;
use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use tasmota_derust_core::config::{BrokerConfig, Config, WebLogin};
use tasmota_derust_core::session::{CommandError, Session, Snapshot};

pub mod tls;

const INDEX_HTML: &str = include_str!("../index.html");

/// The page with the About text and the app icon (favicon) filled in.
static INDEX: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let about = tasmota_derust_core::ABOUT
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace("\n\n", "</p><p>");
    // The SVG without its comment, on one line.
    let svg = include_str!("../../../packaging/icon/icon.svg");
    let svg = match (svg.find("<!--"), svg.find("-->")) {
        (Some(a), Some(b)) => format!("{}{}", &svg[..a], &svg[b + 3..]),
        _ => svg.to_string(),
    };
    let icon: String = svg.split_whitespace().collect::<Vec<_>>().join(" ");
    let favicon = icon
        .replace('%', "%25")
        .replace('#', "%23")
        .replace('"', "'")
        .replace('<', "%3C")
        .replace('>', "%3E");
    INDEX_HTML
        .replace("@ABOUT@", &about)
        .replace("@FAVICON@", &favicon)
        .replace("@SITE@", tasmota_derust_core::SITE_URL)
});

#[derive(Clone)]
struct AppState {
    session: Arc<Session>,
    /// Where read-only can be turned off, shown when a command is refused.
    read_only_hint: Arc<str>,
    /// Whether visitors must log in.
    login: bool,
    https: bool,
}

/// `/api/state`: the session snapshot plus the read-only hint.
#[derive(Serialize)]
struct StateReply {
    #[serde(flatten)]
    snapshot: Snapshot,
    read_only_hint: String,
    /// A web login is required (otherwise anyone on the network can use the page).
    login: bool,
    /// Served over HTTPS (otherwise everything travels in clear text).
    https: bool,
}

/// Default address: every network interface, so phones on the same Wi-Fi can connect;
/// port 8080 for HTTP, 8443 for HTTPS.
pub fn default_listen(https: bool) -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], if https { 8443 } else { 8080 }))
}

/// All routes of the web interface, backed by `session`. `read_only_hint` tells web
/// visitors where read-only mode can be turned off (shown when a command is refused).
/// When `login` is set, every request needs it (HTTP Basic: the browser asks for it).
pub fn router(
    session: Arc<Session>,
    read_only_hint: &str,
    login: &WebLogin,
    https: bool,
) -> Router {
    let auth = Arc::new(Auth {
        login: login.is_set().then(|| login.clone()),
        checked: Mutex::new(HashSet::new()),
        gate: tokio::sync::Semaphore::new(1),
    });
    Router::new()
        .route("/", get(|| async { Html(INDEX.as_str()) }))
        .route("/api/state", get(api_state))
        .route("/api/changelog/{version}", get(api_changelog))
        .route("/api/refresh", post(api_refresh))
        .route("/api/devices/{topic}", delete(api_forget))
        .route("/api/forget-offline", post(api_forget_offline))
        .route("/api/devices/{topic}/power/{n}", post(api_power))
        .route("/api/devices/{topic}/ap/{n}", post(api_ap))
        .route("/api/devices/{topic}/upgrade", post(api_upgrade))
        .route("/api/devices/{topic}/restart", post(api_restart))
        .route("/api/devices/{topic}/channel/{channel}", post(api_channel))
        .route("/api/devices/{topic}/dimmer/{n}", post(api_dimmer))
        .route("/api/devices/{topic}/hsb/{h}/{s}/{b}", post(api_hsb))
        .route("/api/devices/{topic}/ct/{n}", post(api_ct))
        .route("/api/devices/{topic}/white/{n}", post(api_white))
        .route("/api/devices/{topic}/options", get(api_options))
        .route("/api/devices/{topic}/wifi/{slot}", post(api_wifi))
        .route("/api/devices/{topic}/backup", post(api_backup))
        .route("/api/devices/{topic}/restore", post(api_restore))
        .route(
            "/api/devices/{topic}/console",
            get(api_console).post(api_console_send),
        )
        .route("/api/search", get(api_search_range).post(api_search))
        .route("/api/topics", get(api_topics))
        .route("/api/upgrade-many", post(api_upgrade_many))
        .route("/api/enable-mqtt", get(api_mqtt_defaults).post(api_enable_mqtt))
        .route("/api/devices/{topic}/http-password", post(api_http_password))
        .route("/api/ota", get(api_ota).post(api_set_ota))
        .route("/api/devices/{topic}/ota", get(api_device_ota).post(api_set_device_ota))
        .route(
            "/api/console/favourites",
            get(api_favourites).post(api_set_favourites),
        )
        .route(
            "/api/devices/{topic}/option/{n}/{value}",
            post(api_set_option),
        )
        .with_state(AppState {
            session,
            read_only_hint: read_only_hint.into(),
            login: login.is_set(),
            https,
        })
        .layer(middleware::from_fn_with_state(auth, guard))
}

/// Refuses requests that other web sites can make through a visitor's browser.
///
/// - The `Host` must be an IP address or a local name (see [`local_host`]), so a public
///   domain re-pointed at this server (DNS rebinding) can neither read nor command.
/// - A request that changes something (anything but GET/HEAD) must come from this page:
///   its `Origin` (or `Referer` when there is none) must name the same host. A form or
///   script on another site sends its own origin and is refused. Requests without either
///   header (curl, scripts) are not browser requests and are let through.
/// - With a login set, every request must carry it (`Authorization: Basic …`).
async fn guard(State(auth): State<Arc<Auth>>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return (StatusCode::BAD_REQUEST, "Missing Host header.").into_response();
    };
    if !local_host(host) {
        return (StatusCode::FORBIDDEN, "Unknown host name.").into_response();
    }
    if auth.login.is_some() {
        let given = headers
            .get(header::AUTHORIZATION)
            .map(|h| h.as_bytes().to_vec())
            .unwrap_or_default();
        if !auth.check(given).await {
            return (
                StatusCode::UNAUTHORIZED,
                [(
                    header::WWW_AUTHENTICATE,
                    r#"Basic realm="Tasmota Derust", charset="UTF-8""#,
                )],
                "Login required.",
            )
                .into_response();
        }
    }
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        let source = headers
            .get(header::ORIGIN)
            .or_else(|| headers.get(header::REFERER))
            .and_then(|h| h.to_str().ok());
        if let Some(source) = source
            && origin_host(source) != Some(host)
        {
            return (
                StatusCode::FORBIDDEN,
                "Refused: the request did not come from this page.",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// The web login and the logins already checked.
///
/// Only a hash of the password is stored, and checking one costs a PBKDF2 run, too slow
/// for every request of a page that refreshes itself. So an `Authorization` header that
/// passed is remembered (as its SHA-256, in memory only, forgotten at restart) and later
/// requests with it are let through at once. Unknown headers are checked one at a time
/// (`gate`), so wrong guesses use at most one core and stay slow.
struct Auth {
    login: Option<WebLogin>,
    checked: Mutex<HashSet<[u8; 32]>>,
    gate: tokio::sync::Semaphore,
}

impl Auth {
    async fn check(&self, header: Vec<u8>) -> bool {
        let Some(login) = &self.login else {
            return true;
        };
        let key: [u8; 32] = ring::digest::digest(&ring::digest::SHA256, &header)
            .as_ref()
            .try_into()
            .expect("32 bytes");
        if self.checked.lock().unwrap().contains(&key) {
            return true;
        }
        let Some((user, pass)) = basic_credentials(&header) else {
            return false;
        };
        let Ok(_permit) = self.gate.acquire().await else {
            return false;
        };
        let login = login.clone();
        let ok = tokio::task::spawn_blocking(move || login.verify(&user, &pass))
            .await
            .unwrap_or(false);
        if ok {
            let mut set = self.checked.lock().unwrap();
            if set.len() >= 64 {
                set.clear();
            }
            set.insert(key);
        }
        ok
    }
}

/// User and password of an `Authorization: Basic …` header.
fn basic_credentials(header: &[u8]) -> Option<(String, String)> {
    let b64 = std::str::from_utf8(header)
        .ok()?
        .strip_prefix("Basic ")?
        .trim();
    let text = String::from_utf8(unbase64(b64)?).ok()?;
    let (user, pass) = text.split_once(':')?;
    Some((user.into(), pass.into()))
}

/// Standard base64 (padding optional); None on anything else.
fn unbase64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Standard base64 with padding (tests build `Authorization` headers with it).
#[cfg(test)]
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() {
                T[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// `host[:port]` of an `Origin` or `Referer` value (`http://192.168.1.20:8080/...`).
fn origin_host(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    Some(rest.split(['/', '?', '#']).next().unwrap_or(rest))
}

/// Whether a `Host` header names this server in a way no outside site can: an IP
/// address, `localhost`, a name without dots (`nas`) or one under a local-only suffix
/// (`.local`, `.lan`, `.home.arpa`, `.internal`, `.localhost`). Other names can be
/// added with `DERUST_ALLOWED_HOSTS` (comma-separated), e.g. behind a reverse proxy.
fn local_host(host: &str) -> bool {
    // Strip the port, keeping IPv6 literals (`[::1]:8080`) whole.
    let name = match host.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') || h.ends_with(']') => {
            if p.chars().all(|c| c.is_ascii_digit()) {
                h
            } else {
                host
            }
        }
        _ => host,
    };
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name.parse::<IpAddr>().is_ok()
        || name
            .strip_prefix('[')
            .and_then(|n| n.strip_suffix(']'))
            .is_some_and(|n| n.parse::<IpAddr>().is_ok())
    {
        return true;
    }
    const LOCAL: [&str; 5] = [".local", ".lan", ".home.arpa", ".internal", ".localhost"];
    if name == "localhost" || !name.contains('.') || LOCAL.iter().any(|s| name.ends_with(s)) {
        return !name.is_empty();
    }
    std::env::var("DERUST_ALLOWED_HOSTS").is_ok_and(|list| {
        list.split(',')
            .any(|h| h.trim().trim_end_matches('.').eq_ignore_ascii_case(&name))
    })
}

/// This computer's address on the local network (the interface that would be used to
/// reach the internet). No packet is sent: connecting a UDP socket only picks a route.
pub fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1, never actually contacted
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// The URL to open in a browser for a server listening on `addr`. A wildcard address
/// (0.0.0.0) is replaced by the LAN address, or by localhost when there is none.
pub fn browser_url(addr: SocketAddr, https: bool) -> String {
    let ip = if addr.ip().is_unspecified() {
        lan_ip().unwrap_or(IpAddr::from([127, 0, 0, 1]))
    } else {
        addr.ip()
    };
    let scheme = if https { "https" } else { "http" };
    format!("{scheme}://{}", SocketAddr::new(ip, addr.port()))
}

/// A web server running on its own thread with its own MQTT session.
/// Dropping it (or calling [`WebServer::stop`]) shuts it down.
pub struct WebServer {
    addr: SocketAddr,
    login: bool,
    /// SHA-256 fingerprint of the certificate when serving HTTPS.
    fingerprint: Option<String>,
    session: Arc<Session>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl WebServer {
    /// Binds `listen` right away (so a busy port is reported here) and starts serving.
    /// With `https`, the self-signed certificate is loaded (or made) first.
    pub fn start(
        broker: &BrokerConfig,
        login: &WebLogin,
        https: bool,
        listen: SocketAddr,
        read_only: bool,
        read_only_hint: &str,
    ) -> io::Result<WebServer> {
        let tls = if https {
            Some(tls::load_or_create()?)
        } else {
            None
        };
        let fingerprint = tls.as_ref().map(|t| t.fingerprint.clone());
        let listener = TcpListener::bind(listen)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        // The desktop app's custom topics (same config file).
        let topics = Config::load().topics;
        let session = Arc::new(Session::start_with(broker, &topics, read_only, Arc::new(|| {})));
        let app = router(session.clone(), read_only_hint, login, https);
        let login = login.is_set();
        let (tx, rx) = oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("web".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let Ok(rt) = rt else { return };
                rt.block_on(async move {
                    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                        return;
                    };
                    let stop = async {
                        let _ = rx.await;
                    };
                    match tls {
                        Some(tls) => {
                            let Ok(listener) = tls::TlsListener::new(listener, &tls) else {
                                return;
                            };
                            let _ = axum::serve(listener, app)
                                .with_graceful_shutdown(stop)
                                .await;
                        }
                        None => {
                            let _ = axum::serve(listener, app)
                                .with_graceful_shutdown(stop)
                                .await;
                        }
                    }
                });
            })?;
        Ok(WebServer {
            addr,
            login,
            fingerprint,
            session,
            shutdown: Some(tx),
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn read_only(&self) -> bool {
        self.session.read_only()
    }

    /// Whether visitors must log in.
    pub fn login(&self) -> bool {
        self.login
    }

    /// The certificate's SHA-256 fingerprint when serving HTTPS.
    pub fn fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    /// Takes effect immediately for every web visitor.
    pub fn set_read_only(&self, on: bool) {
        self.session.set_read_only(on);
    }

    /// The URL to open from a browser (LAN address when listening on all interfaces).
    pub fn url(&self) -> String {
        browser_url(self.addr, self.fingerprint.is_some())
    }

    /// Stops the server and waits for it to finish.
    pub fn stop(mut self) {
        self.shutdown_now();
    }

    fn shutdown_now(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WebServer {
    fn drop(&mut self) {
        self.shutdown_now();
    }
}

/// Warning for a server started read-write without a login (desktop dialog, console).
pub fn no_login_risk(https: bool) -> String {
    let wire = if https {
        "Traffic is encrypted (HTTPS), but that does not keep anyone out."
    } else {
        "All traffic travels in clear text on the network (plain HTTP, no TLS), so anyone on \
         it can also read it, Wi-Fi passwords sent to devices included."
    };
    format!(
        "No web login is set: anyone who can reach this computer on the network can switch \
         relays, change Wi-Fi settings, restore configurations and start firmware upgrades. \
         {wire} Set a username and password in the desktop app's Settings (or with \
         DERUST_WEB_USER and DERUST_WEB_PASSWORD)."
    )
}

/// How the connection is protected, shown with a login set.
pub fn transport_note(fingerprint: Option<&str>) -> String {
    match fingerprint {
        Some(f) => format!(
            "HTTPS with a self-signed certificate: browsers warn on the first visit. Accept it \
             only if its SHA-256 fingerprint is {f}"
        ),
        None => "The server uses plain HTTP (no TLS): the login and all traffic travel in \
                 clear text on the network."
            .into(),
    }
}

/// Entry point of the standalone programs: `tasmota-derust-web` (`read_only`) and
/// `tasmota-derust-web-rw`. The mode is fixed by the program, so which one runs is a
/// visible choice. Broker and login come from the desktop app's config file; environment
/// variables override them.
pub fn standalone_main(read_only: bool) {
    let name = if read_only {
        "tasmota-derust-web"
    } else {
        "tasmota-derust-web-rw"
    };
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut cfg = Config::load();
    if let Some(v) = env("DERUST_WEB_HTTPS") {
        cfg.web_https = v != "0" && v != "false";
    }
    let mut listen = env("DERUST_LISTEN");
    let mut open_browser = env("DERUST_NO_BROWSER").is_none();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().or(listen),
            "--no-browser" => open_browser = false,
            "--https" => cfg.web_https = true,
            "--http" => cfg.web_https = false,
            // Read-only is this program's only mode; kept so old scripts keep working.
            "--read-only" if read_only => {}
            "-h" | "--help" => {
                let other = if read_only {
                    "Read-only: nothing is ever switched, restarted or upgraded. To allow \
                     commands, run tasmota-derust-web-rw instead."
                } else {
                    "Read-write: commands are sent to devices. For a server that only \
                     looks, run tasmota-derust-web instead."
                };
                println!(
                    "Usage: {name} [--https | --http] [--listen ADDRESS:PORT] [--no-browser]\n\n{other}\n\n\
                     --https serves HTTPS with a self-signed certificate (default port 8443),\n\
                     --http plain HTTP (default port 8080). Without either, the desktop app's\n\
                     setting is used (or DERUST_WEB_HTTPS=1/0).\n\
                     The page opens in the default browser when there is a screen; --no-browser\n\
                     (or DERUST_NO_BROWSER=1) leaves it closed, e.g. on a server or as a service."
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    let listen = match listen {
        None => default_listen(cfg.web_https),
        Some(l) => l.parse::<SocketAddr>().unwrap_or_else(|_| {
            eprintln!("Invalid listen address: {l}");
            std::process::exit(2);
        }),
    };
    let tls = cfg.web_https.then(|| {
        tls::load_or_create().unwrap_or_else(|e| {
            eprintln!("Cannot set up HTTPS: {e}");
            std::process::exit(1);
        })
    });
    if let Some(v) = env("DERUST_MQTT_HOST") {
        cfg.broker.host = v;
    }
    if let Some(v) = env("DERUST_MQTT_PORT").and_then(|v| v.parse().ok()) {
        cfg.broker.port = v;
    }
    if let Some(v) = env("DERUST_MQTT_USER") {
        cfg.broker.username = v;
    }
    if let Some(v) = env("DERUST_MQTT_PASSWORD") {
        cfg.broker.password = v;
    }
    // A login from the environment replaces the config's (hashed here, kept in memory).
    if let Some(v) = env("DERUST_WEB_USER") {
        cfg.web_login.username = v;
    }
    if let Some(v) = env("DERUST_WEB_PASSWORD") {
        cfg.web_login = WebLogin::new(&cfg.web_login.username, &v);
    }
    if cfg.broker.host.trim().is_empty() {
        eprintln!(
            "No MQTT broker configured. Set it in the desktop app or with DERUST_MQTT_HOST \
             (and DERUST_MQTT_PORT, DERUST_MQTT_USER, DERUST_MQTT_PASSWORD)."
        );
        std::process::exit(2);
    }

    let hint = if read_only {
        "This server is the read-only one: run tasmota-derust-web-rw to allow commands."
    } else {
        ""
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let session = Arc::new(Session::start_with(&cfg.broker, &cfg.topics, read_only, Arc::new(|| {})));
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Cannot listen on {listen}: {e}");
                std::process::exit(1);
            });
        let mode = if read_only { "read-only" } else { "READ-WRITE" };
        let login = if cfg.web_login.is_set() {
            "login required"
        } else {
            "no login"
        };
        let https = tls.is_some();
        let url = browser_url(listen, https);
        println!("Tasmota Derust web on {url}  ({mode}, {login})");
        let click = if cfg!(target_os = "macos") {
            "Cmd-click"
        } else {
            "Ctrl-click"
        };
        println!("Open it in a browser: {click} the address above, or copy it into the address bar.");
        // The default browser, when this runs on a computer with a screen and a terminal
        // (not as a service or on a headless server).
        if open_browser && has_screen() {
            let u = url.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let _ = open::that(u);
            });
        }
        if let Some(t) = &tls {
            println!("Certificate: {}", t.cert_path.display());
        }
        if !read_only && !cfg.web_login.is_set() {
            eprintln!(
                "\n!!! WARNING: commands allowed without a login.\n!!! {}\n",
                no_login_risk(https)
            );
        } else {
            println!(
                "{}",
                transport_note(tls.as_ref().map(|t| t.fingerprint.as_str()))
            );
        }
        let app = router(session, hint, &cfg.web_login, https);
        match tls {
            Some(t) => {
                let listener = tls::TlsListener::new(listener, &t).expect("listener address");
                axum::serve(listener, app).await.unwrap();
            }
            None => axum::serve(listener, app).await.unwrap(),
        }
    });
}

/// A screen and someone in front of it: macOS and Windows from a terminal, Linux with a
/// graphical session. Not when started as a service (no terminal).
fn has_screen() -> bool {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        return false;
    }
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return true;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Release notes of a Tasmota Derust version (fetched from GitHub by the server).
async fn api_changelog(Path(version): Path<String>) -> Response {
    let r =
        tokio::task::spawn_blocking(move || tasmota_derust_core::github::changelog(&version)).await;
    match r {
        Ok(Ok(c)) => Json(c).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn api_state(State(st): State<AppState>) -> Json<StateReply> {
    Json(StateReply {
        snapshot: st.session.snapshot(),
        read_only_hint: st.read_only_hint.to_string(),
        login: st.login,
        https: st.https,
    })
}

/// Read queries only (Status 0, Module, SSId): allowed in read-only mode too.
async fn api_refresh(State(st): State<AppState>) -> StatusCode {
    st.session.refresh();
    StatusCode::NO_CONTENT
}

fn reply(st: &AppState, r: Result<(), CommandError>) -> Response {
    let code = match r {
        Ok(()) => return StatusCode::NO_CONTENT.into_response(),
        Err(CommandError::ReadOnly) => {
            let msg = format!("{} {}", CommandError::ReadOnly.message(), st.read_only_hint);
            return (StatusCode::FORBIDDEN, msg).into_response();
        }
        Err(CommandError::UnknownDevice) => StatusCode::NOT_FOUND,
        // The page asks, then sends again with "confirm": true.
        Err(CommandError::Console(tasmota_derust_core::console::ConsoleError::NeedsConfirm)) => {
            StatusCode::PRECONDITION_REQUIRED
        }
        Err(
            CommandError::Offline
            | CommandError::UpgradeRunning
            | CommandError::UpgradeNotPossible
            | CommandError::NotOfficialOtaUrl
            | CommandError::NotALight
            | CommandError::NotOffline,
        ) => StatusCode::CONFLICT,
        Err(
            CommandError::NoSuchRelay
            | CommandError::BadAp
            | CommandError::OptionBlocked
            | CommandError::BadOption
            | CommandError::Wifi(_)
            | CommandError::Console(_)
            | CommandError::NotHttp
            | CommandError::BadMqttSettings,
        ) => StatusCode::BAD_REQUEST,
    };
    (code, r.unwrap_err().message()).into_response()
}

async fn api_power(
    State(st): State<AppState>,
    Path((topic, n)): Path<(String, usize)>,
) -> Response {
    reply(&st, st.session.toggle_power(&topic, n))
}

async fn api_ap(State(st): State<AppState>, Path((topic, n)): Path<(String, u8)>) -> Response {
    reply(&st, st.session.switch_ap(&topic, n))
}

async fn api_dimmer(State(st): State<AppState>, Path((topic, n)): Path<(String, i64)>) -> Response {
    reply(&st, st.session.set_dimmer(&topic, n))
}

async fn api_hsb(
    State(st): State<AppState>,
    Path((topic, h, s, b)): Path<(String, i64, i64, i64)>,
) -> Response {
    reply(&st, st.session.set_hsb(&topic, h, s, b))
}

async fn api_ct(State(st): State<AppState>, Path((topic, n)): Path<(String, i64)>) -> Response {
    reply(&st, st.session.set_ct(&topic, n))
}

async fn api_white(State(st): State<AppState>, Path((topic, n)): Path<(String, i64)>) -> Response {
    reply(&st, st.session.set_white(&topic, n))
}

/// channel: "release" or "development". Changes the OtaUrl only.
async fn api_channel(
    State(st): State<AppState>,
    Path((topic, channel)): Path<(String, String)>,
) -> Response {
    match channel.as_str() {
        "release" => reply(&st, st.session.set_channel(&topic, false)),
        "development" => reply(&st, st.session.set_channel(&topic, true)),
        _ => (
            StatusCode::BAD_REQUEST,
            "channel must be release or development",
        )
            .into_response(),
    }
}

/// The SetOptions of a device, decoded and described.
async fn api_options(State(st): State<AppState>, Path(topic): Path<String>) -> Response {
    match st.session.options(&topic) {
        Some(v) => Json(v).into_response(),
        None => (StatusCode::NOT_FOUND, CommandError::UnknownDevice.message()).into_response(),
    }
}

async fn api_set_option(
    State(st): State<AppState>,
    Path((topic, n, value)): Path<(String, u16, u32)>,
) -> Response {
    reply(&st, st.session.set_option(&topic, n, value))
}

#[derive(Deserialize)]
struct After {
    #[serde(default)]
    after: u64,
}

/// A device's console: its messages after `?after=<seq>` and its command history.
async fn api_console(
    State(st): State<AppState>,
    Path(topic): Path<String>,
    Query(q): Query<After>,
) -> Response {
    match st.session.console(&topic, q.after) {
        Some(v) => Json(v).into_response(),
        None => (StatusCode::NOT_FOUND, CommandError::UnknownDevice.message()).into_response(),
    }
}

#[derive(Deserialize)]
struct ConsoleBody {
    line: String,
    /// The user confirmed a command that can cut the device off (428 without it).
    #[serde(default)]
    confirm: bool,
}

/// Sends a line typed in the console (`Command payload`).
async fn api_console_send(
    State(st): State<AppState>,
    Path(topic): Path<String>,
    Json(body): Json<ConsoleBody>,
) -> Response {
    reply(&st, st.session.console_send(&topic, &body.line, body.confirm))
}

/// The custom MQTT topics in use (set in the desktop app's Settings or the config file).
async fn api_topics(State(st): State<AppState>) -> Json<tasmota_derust_core::topics::Topics> {
    Json(st.session.topics())
}

#[derive(Serialize)]
struct RangeReply {
    range: Option<String>,
}

/// The server's own network (address and netmask), offered as the search range.
async fn api_search_range() -> Json<RangeReply> {
    Json(RangeReply {
        range: tasmota_derust_core::http_dev::local_range(),
    })
}

#[derive(Deserialize)]
struct SearchBody {
    #[serde(default)]
    range: String,
    #[serde(default)]
    password: String,
}

/// Searches the network for Tasmota devices over HTTP (read-only: Status 0 to each address).
async fn api_search(State(st): State<AppState>, Json(b): Json<SearchBody>) -> Response {
    let pw = Some(b.password).filter(|p| !p.is_empty());
    match st.session.search(&b.range, pw) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.message()).into_response(),
    }
}

#[derive(Deserialize)]
struct TopicsBody {
    topics: Vec<String>,
}

#[derive(Serialize)]
struct Failure {
    topic: String,
    error: &'static str,
}

fn failures(v: Vec<(String, CommandError)>) -> Json<Vec<Failure>> {
    Json(
        v.into_iter()
            .map(|(topic, e)| Failure {
                topic,
                error: e.message(),
            })
            .collect(),
    )
}

/// Upgrades several devices at once, each on its own (the path steps follow by themselves).
async fn api_upgrade_many(State(st): State<AppState>, Json(b): Json<TopicsBody>) -> Response {
    if st.session.read_only() {
        return reply(&st, Err(CommandError::ReadOnly));
    }
    failures(st.session.upgrade_many(&b.topics)).into_response()
}

async fn api_mqtt_defaults(State(st): State<AppState>) -> Json<tasmota_derust_core::session::MqttDefaults> {
    Json(st.session.broker_settings())
}

#[derive(Deserialize)]
struct EnableMqttBody {
    topics: Vec<String>,
    host: String,
    port: u16,
    #[serde(default)]
    user: String,
    /// None: the app's own broker password (never sent to the page).
    #[serde(default)]
    password: Option<String>,
}

/// Connects HTTP-only devices to the broker (MqttHost, MqttPort, MqttUser, MqttPassword).
async fn api_enable_mqtt(State(st): State<AppState>, Json(b): Json<EnableMqttBody>) -> Response {
    if st.session.read_only() {
        return reply(&st, Err(CommandError::ReadOnly));
    }
    failures(st.session.enable_mqtt(
        &b.topics,
        &b.host,
        b.port,
        &b.user,
        b.password.as_deref(),
    ))
    .into_response()
}

#[derive(Deserialize)]
struct PasswordBody {
    password: String,
}

async fn api_http_password(
    State(st): State<AppState>,
    Path(topic): Path<String>,
    Json(b): Json<PasswordBody>,
) -> Response {
    reply(&st, st.session.set_http_password(&topic, &b.password))
}

use tasmota_derust_core::ota_policy::{self, DeviceOta, OtaSettings};

async fn api_ota() -> Json<OtaSettings> {
    Json(ota_policy::default_settings())
}

/// Firmware server and highest version for every device. Not in read-only mode.
async fn api_set_ota(State(st): State<AppState>, Json(o): Json<OtaSettings>) -> Response {
    if st.session.read_only() {
        return reply(&st, Err(CommandError::ReadOnly));
    }
    match ota_policy::set_default(o) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.message()).into_response(),
    }
}

async fn api_device_ota(Path(topic): Path<String>) -> Json<DeviceOta> {
    Json(ota_policy::device(&topic))
}

async fn api_set_device_ota(
    State(st): State<AppState>,
    Path(topic): Path<String>,
    Json(o): Json<DeviceOta>,
) -> Response {
    if st.session.read_only() {
        return reply(&st, Err(CommandError::ReadOnly));
    }
    match ota_policy::set_device(&topic, o) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.message()).into_response(),
    }
}

async fn api_favourites() -> Json<Vec<tasmota_derust_core::console::Favourite>> {
    Json(tasmota_derust_core::console::favourites())
}

/// Saves the favourite commands (shared with the desktop app). Not in read-only mode: the
/// file is shared by every program on this computer.
async fn api_set_favourites(
    State(st): State<AppState>,
    Json(list): Json<Vec<tasmota_derust_core::console::Favourite>>,
) -> Response {
    if st.session.read_only() {
        return reply(&st, Err(CommandError::ReadOnly));
    }
    tasmota_derust_core::console::set_favourites(list);
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct WifiBody {
    ssid: String,
    password: String,
}

/// Sets the Wi-Fi network the device is not using (name + password, one restart).
async fn api_wifi(
    State(st): State<AppState>,
    Path((topic, slot)): Path<(String, u8)>,
    Json(body): Json<WifiBody>,
) -> Response {
    reply(
        &st,
        st.session
            .set_wifi(&topic, slot, &body.ssid, &body.password),
    )
}

/// The device's web password, when the user gave one (header, never in the URL).
fn web_password(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-web-password")
        .and_then(|v| v.to_str().ok())
        .filter(|p| !p.is_empty())
        .map(str::to_string)
}

fn backup_error(e: tasmota_derust_core::backup::BackupError) -> Response {
    use tasmota_derust_core::backup::BackupError as E;
    let code = match e {
        // 401 from the device: the page asks for its web password.
        E::NeedsPassword | E::WrongPassword => StatusCode::UNAUTHORIZED,
        E::NoAddress => StatusCode::CONFLICT,
        E::Failed(_) => StatusCode::BAD_GATEWAY,
    };
    (code, e.to_string()).into_response()
}

/// The device's whole configuration (Tasmota's .dmp), downloaded from its web server.
async fn api_backup(
    State(st): State<AppState>,
    Path(topic): Path<String>,
    headers: HeaderMap,
) -> Response {
    let pw = web_password(&headers);
    let r = tokio::task::spawn_blocking(move || st.session.backup(&topic, pw.as_deref())).await;
    match r {
        Ok(Ok(b)) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", b.name),
                ),
            ],
            b.data,
        )
            .into_response(),
        Ok(Err(e)) => backup_error(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Uploads a configuration file (request body) to the device, which restarts with it.
async fn api_restore(
    State(st): State<AppState>,
    Path(topic): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if st.session.read_only() {
        let msg = format!("{} {}", CommandError::ReadOnly.message(), st.read_only_hint);
        return (StatusCode::FORBIDDEN, msg).into_response();
    }
    let pw = web_password(&headers);
    let r =
        tokio::task::spawn_blocking(move || st.session.restore(&topic, pw.as_deref(), &body)).await;
    match r {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => backup_error(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Removes an offline device and clears its retained messages on the broker.
async fn api_forget(State(st): State<AppState>, Path(topic): Path<String>) -> Response {
    reply(&st, st.session.forget(&topic))
}

#[derive(serde::Serialize)]
struct Removed {
    removed: usize,
}

/// Removes every offline device; answers how many.
async fn api_forget_offline(State(st): State<AppState>) -> Response {
    match st.session.forget_offline() {
        Ok(n) => Json(Removed { removed: n }).into_response(),
        Err(e) => reply(&st, Err(e)),
    }
}

/// `Restart 1`.
async fn api_restart(State(st): State<AppState>, Path(topic): Path<String>) -> Response {
    reply(&st, st.session.restart(&topic))
}

async fn api_upgrade(State(st): State<AppState>, Path(topic): Path<String>) -> Response {
    reply(&st, st.session.upgrade(&topic))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn http(addr: SocketAddr, req: &str) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn embedded_server_serves_and_stops() {
        // Port 1 on loopback: nothing listens there, so no real broker is ever contacted.
        let broker = BrokerConfig {
            host: "127.0.0.1".into(),
            port: 1,
            ..Default::default()
        };
        let w = WebServer::start(
            &broker,
            &WebLogin::default(),
            false,
            "0.0.0.0:0".parse().unwrap(),
            true,
            "Untick it.",
        )
        .unwrap();
        let addr = SocketAddr::from(([127, 0, 0, 1], w.addr().port()));
        assert!(!w.url().contains("0.0.0.0"), "url: {}", w.url());

        let page = http(
            addr,
            "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("<html"));

        let state = http(
            addr,
            "GET /api/state HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(state.contains("\"read_only\":true"));

        let cmd = http(
            addr,
            "POST /api/devices/x/power/1 HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(
            cmd.starts_with("HTTP/1.1 403") && cmd.contains("Untick it."),
            "{cmd}"
        );
        assert!(state.contains("\"read_only_hint\":\"Untick it.\""));

        // Switching read-only off reaches the web session at once (the device is unknown,
        // so nothing is sent: the refusal is now 404 instead of 403).
        w.set_read_only(false);
        let cmd = http(
            addr,
            "POST /api/devices/x/power/1 HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(cmd.starts_with("HTTP/1.1 404"), "{cmd}");

        // A command posted by another web site (form or script) is refused even when
        // read-only is off; one from this page reaches the session (404: unknown device).
        let post = |origin: &str| {
            http(
                addr,
                &format!(
                    "POST /api/devices/x/power/1 HTTP/1.1\r\nHost: 192.168.1.20:8080\r\n\
                     {origin}Content-Length: 0\r\nConnection: close\r\n\r\n"
                ),
            )
        };
        assert!(post("Origin: http://evil.example\r\n").starts_with("HTTP/1.1 403"));
        assert!(post("Origin: null\r\n").starts_with("HTTP/1.1 403"));
        assert!(post("Referer: http://evil.example/x\r\n").starts_with("HTTP/1.1 403"));
        assert!(post("Origin: http://192.168.1.20:8080\r\n").starts_with("HTTP/1.1 404"));
        assert!(post("Referer: http://192.168.1.20:8080/\r\n").starts_with("HTTP/1.1 404"));

        // DNS rebinding: a public name pointed at this server is refused, reads included.
        let rebound = http(
            addr,
            "GET /api/state HTTP/1.1\r\nHost: evil.example:8080\r\nConnection: close\r\n\r\n",
        );
        assert!(rebound.starts_with("HTTP/1.1 403"), "{rebound}");

        w.stop();
        assert!(TcpStream::connect(addr).is_err());
    }

    #[test]
    fn login_is_required_when_set() {
        let broker = BrokerConfig {
            host: "127.0.0.1".into(),
            port: 1,
            ..Default::default()
        };
        let login = WebLogin::new("me", "sécret");
        let w = WebServer::start(
            &broker,
            &login,
            false,
            "127.0.0.1:0".parse().unwrap(),
            true,
            "",
        )
        .unwrap();
        let get = |auth: &str| {
            http(
                w.addr(),
                &format!("GET /api/state HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n"),
            )
        };
        let none = get("");
        assert!(
            none.starts_with("HTTP/1.1 401") && none.contains("Basic realm"),
            "{none}"
        );
        assert!(get("Authorization: Basic bWU6d3Jvbmc=\r\n").starts_with("HTTP/1.1 401"));
        let ok = get(&format!(
            "Authorization: Basic {}\r\n",
            base64("me:sécret".as_bytes())
        ));
        assert!(
            ok.starts_with("HTTP/1.1 200") && ok.contains("\"login\":true"),
            "{ok}"
        );
        w.stop();
    }

    #[test]
    fn basic_header_parsing() {
        assert_eq!(
            basic_credentials(format!("Basic {}", base64("me:a:b".as_bytes())).as_bytes()),
            Some(("me".into(), "a:b".into()))
        );
        assert_eq!(basic_credentials(b"Basic !!!"), None);
        assert_eq!(basic_credentials(b"Bearer x"), None);
        for s in ["", "f", "fo", "foo", "foobar", "sécret"] {
            assert_eq!(unbase64(&base64(s.as_bytes())).unwrap(), s.as_bytes());
        }
    }

    #[test]
    fn base64_matches_rfc4648() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn host_and_origin_parsing() {
        for ok in [
            "127.0.0.1:8080",
            "192.168.1.20",
            "[::1]:8080",
            "[fe80::1]",
            "localhost:8080",
            "nas:8080",
            "pi.local",
            "box.lan:80",
            "x.home.arpa",
            "LocalHost.",
        ] {
            assert!(local_host(ok), "{ok}");
        }
        for bad in [
            "evil.example",
            "evil.example:8080",
            "192.168.1.20.nip.io",
            "",
            ":8080",
        ] {
            assert!(!local_host(bad), "{bad}");
        }
        assert_eq!(origin_host("http://1.2.3.4:8080"), Some("1.2.3.4:8080"));
        assert_eq!(origin_host("https://a.lan/x?y"), Some("a.lan"));
        assert_eq!(origin_host("null"), None);
    }
}
