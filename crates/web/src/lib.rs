//! Tasmota Derust web interface: the HTTP routes and the embedded page, shared by the
//! standalone `tasmota-derust-web` binary and the desktop app's "Web server" button.
//!
//! Read-only mode: only read queries (Status, Module, SSId) are sent to devices; every
//! command that changes something (relays, Wi-Fi switch, upgrade) is refused with 403.

use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, UdpSocket};
use std::sync::Arc;
use std::thread::JoinHandle;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::sync::oneshot;

use tasmota_derust_core::config::BrokerConfig;
use tasmota_derust_core::session::{CommandError, Session, Snapshot};

const INDEX_HTML: &str = include_str!("../index.html");

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
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route("/api/state", get(api_state))
        .route("/api/refresh", post(api_refresh))
        .route("/api/devices/{topic}/power/{n}", post(api_power))
        .route("/api/devices/{topic}/ap/{n}", post(api_ap))
        .route("/api/devices/{topic}/upgrade", post(api_upgrade))
        .route("/api/devices/{topic}/channel/{channel}", post(api_channel))
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
            | CommandError::NotOfficialOtaUrl,
        ) => StatusCode::CONFLICT,
        Err(CommandError::NoSuchRelay | CommandError::BadAp) => StatusCode::BAD_REQUEST,
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
