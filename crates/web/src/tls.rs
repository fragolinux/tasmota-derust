//! HTTPS with a self-signed certificate, made once and kept in the app's config directory
//! (`web-cert.pem`, `web-key.pem`), so the desktop app's server and the standalone
//! programs present the same certificate and a browser exception keeps working.
//! Delete both files to make a new one (e.g. after the computer's address changed).

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::crypto::ring::default_provider;
use tokio_rustls::server::TlsStream;

use tasmota_derust_core::config::Config;

/// The certificate in use, ready to accept connections.
#[derive(Clone)]
pub struct Tls {
    acceptor: TlsAcceptor,
    /// SHA-256 of the certificate, as browsers show it (`AB:CD:…`).
    pub fingerprint: String,
    pub cert_path: PathBuf,
}

/// Loads the certificate from the config directory, creating it on first use.
pub fn load_or_create() -> io::Result<Tls> {
    let dir = Config::dir().ok_or_else(|| io::Error::other("no config directory"))?;
    load_or_create_in(&dir)
}

pub fn load_or_create_in(dir: &Path) -> io::Result<Tls> {
    let cert_path = dir.join("web-cert.pem");
    let key_path = dir.join("web-key.pem");
    if !cert_path.exists() || !key_path.exists() {
        create(&cert_path, &key_path)?;
    }
    let bad = |e: rustls_pki_types::pem::Error| io::Error::other(format!("certificate: {e}"));
    let certs = CertificateDer::pem_file_iter(&cert_path)
        .map_err(bad)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(bad)?;
    let key = PrivateKeyDer::from_pem_file(&key_path).map_err(bad)?;
    let first = certs
        .first()
        .ok_or_else(|| io::Error::other("certificate file is empty"))?;
    let digest = ring::digest::digest(&ring::digest::SHA256, first);
    let fingerprint = digest
        .as_ref()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    let config = ServerConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .and_then(|b| b.with_no_client_auth().with_single_cert(certs, key))
        .map_err(|e| io::Error::other(format!("TLS: {e}")))?;
    Ok(Tls {
        acceptor: TlsAcceptor::from(Arc::new(config)),
        fingerprint,
        cert_path,
    })
}

/// A certificate for `localhost`, the loopback addresses and this computer's LAN address.
fn create(cert_path: &Path, key_path: &Path) -> io::Result<()> {
    let mut names = vec!["localhost".to_string(), "127.0.0.1".into(), "::1".into()];
    if let Some(ip) = crate::lan_ip() {
        names.push(ip.to_string());
    }
    let ck = rcgen::generate_simple_self_signed(names)
        .map_err(|e| io::Error::other(format!("certificate: {e}")))?;
    if let Some(dir) = cert_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_private(key_path, ck.signing_key.serialize_pem().as_bytes())?;
    std::fs::write(cert_path, ck.cert.pem())
}

/// Writes a file only its owner can read (on Unix; Windows keeps per-user ACLs on the
/// config directory).
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(data)
    }
    #[cfg(not(unix))]
    std::fs::write(path, data)
}

/// A TCP listener that hands out connections once their TLS handshake is done. Handshakes
/// run side by side, each with a time limit, so a slow or silent client blocks nobody.
pub struct TlsListener {
    addr: SocketAddr,
    rx: mpsc::Receiver<(TlsStream<tokio::net::TcpStream>, SocketAddr)>,
}

impl TlsListener {
    /// Must be called inside the Tokio runtime that serves.
    pub fn new(tcp: TcpListener, tls: &Tls) -> io::Result<TlsListener> {
        let addr = tcp.local_addr()?;
        let (tx, rx) = mpsc::channel(32);
        let acceptor = tls.acceptor.clone();
        tokio::spawn(async move {
            loop {
                let (stream, peer) = match tcp.accept().await {
                    Ok(c) => c,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let (acceptor, out) = (acceptor.clone(), tx.clone());
                tokio::spawn(async move {
                    let hs = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream));
                    if let Ok(Ok(tls)) = hs.await {
                        let _ = out.send((tls, peer)).await;
                    }
                });
                if tx.is_closed() {
                    break;
                }
            }
        });
        Ok(TlsListener { addr, rx })
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.rx.recv().await {
            Some(c) => c,
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_is_made_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("derust-tls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = load_or_create_in(&dir).unwrap();
        let b = load_or_create_in(&dir).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 32 * 3 - 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("web-key.pem"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
