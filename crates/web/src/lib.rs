//! Tasmota Derust web interface: the HTTP routes and the embedded page, shared by the
//! standalone `tasmota-derust-web` binary and the desktop app's "Web server" button.
//!
//! Read-only mode: only read queries (Status, Module, SSId) are sent to devices; every
//! command that changes something (relays, Wi-Fi switch, upgrade) is refused with 403.

use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, UdpSocket};
use std::sync::Arc;
use std::thread::JoinHandle;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use tasmota_derust_core::config::BrokerConfig;
use tasmota_derust_core::session::{CommandError, Session, Snapshot};

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
});

#[derive(Clone)]
struct AppState {
    session: Arc<Session>,
    /// Where read-only can be turned off, shown when a command is refused.
    read_only_hint: Arc<str>,
}

/// `/api/state`: the session snapshot plus the read-only hint.
#[derive(Serialize)]
struct StateReply {
    #[serde(flatten)]
    snapshot: Snapshot,
    read_only_hint: String,
}

/// Default address: every network interface, so phones on the same Wi-Fi can connect.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:8080";

/// All routes of the web interface, backed by `session`. `read_only_hint` tells web
/// visitors where read-only mode can be turned off (shown when a command is refused).
pub fn router(session: Arc<Session>, read_only_hint: &str) -> Router {
    Router::new()
        .route("/", get(|| async { Html(INDEX.as_str()) }))
        .route("/api/state", get(api_state))
        .route("/api/changelog/{version}", get(api_changelog))
        .route("/api/refresh", post(api_refresh))
        .route("/api/devices/{topic}/power/{n}", post(api_power))
        .route("/api/devices/{topic}/ap/{n}", post(api_ap))
        .route("/api/devices/{topic}/upgrade", post(api_upgrade))
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
            "/api/devices/{topic}/option/{n}/{value}",
            post(api_set_option),
        )
        .with_state(AppState {
            session,
            read_only_hint: read_only_hint.into(),
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
pub fn browser_url(addr: SocketAddr) -> String {
    let ip = if addr.ip().is_unspecified() {
        lan_ip().unwrap_or(IpAddr::from([127, 0, 0, 1]))
    } else {
        addr.ip()
    };
    format!("http://{}", SocketAddr::new(ip, addr.port()))
}

/// A web server running on its own thread with its own MQTT session.
/// Dropping it (or calling [`WebServer::stop`]) shuts it down.
pub struct WebServer {
    addr: SocketAddr,
    session: Arc<Session>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl WebServer {
    /// Binds `listen` right away (so a busy port is reported here) and starts serving.
    pub fn start(
        broker: &BrokerConfig,
        listen: SocketAddr,
        read_only: bool,
        read_only_hint: &str,
    ) -> io::Result<WebServer> {
        let listener = TcpListener::bind(listen)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let session = Arc::new(Session::start(broker, read_only, Arc::new(|| {})));
        let app = router(session.clone(), read_only_hint);
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
                    let _ = axum::serve(listener, app)
                        .with_graceful_shutdown(async {
                            let _ = rx.await;
                        })
                        .await;
                });
            })?;
        Ok(WebServer {
            addr,
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

    /// Takes effect immediately for every web visitor.
    pub fn set_read_only(&self, on: bool) {
        self.session.set_read_only(on);
    }

    /// The URL to open from a browser (LAN address when listening on all interfaces).
    pub fn url(&self) -> String {
        browser_url(self.addr)
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
        Err(
            CommandError::Offline
            | CommandError::UpgradeRunning
            | CommandError::OtherUpgradeRunning
            | CommandError::UpgradeNotPossible
            | CommandError::NotOfficialOtaUrl
            | CommandError::NotALight,
        ) => StatusCode::CONFLICT,
        Err(
            CommandError::NoSuchRelay
            | CommandError::BadAp
            | CommandError::OptionBlocked
            | CommandError::BadOption
            | CommandError::Wifi(_),
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
        let w =
            WebServer::start(&broker, "0.0.0.0:0".parse().unwrap(), true, "Untick it.").unwrap();
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

        w.stop();
        assert!(TcpStream::connect(addr).is_err());
    }
}
