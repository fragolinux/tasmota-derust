//! Tasmota Derust web server: same MQTT core as the desktop app, served as a web page
//! that works on phones, tablets and desktop browsers.
//!
//! Usage:
//!   tasmota-derust-web [--listen 0.0.0.0:8080] [--read-only]
//!
//! The broker comes from the desktop app's config file and can be overridden with the
//! environment variables DERUST_MQTT_HOST, DERUST_MQTT_PORT, DERUST_MQTT_USER,
//! DERUST_MQTT_PASSWORD. DERUST_LISTEN and DERUST_READ_ONLY=1 work like the flags.
//!
//! Read-only mode: only read queries (Status, Module, SSId) are sent to devices; every
//! command that changes something (relays, Wi-Fi switch, upgrade) is refused.

use std::net::SocketAddr;
use std::sync::Arc;

use tasmota_derust_core::config::Config;
use tasmota_derust_core::session::Session;
use tasmota_derust_web::{DEFAULT_LISTEN, browser_url, router};

struct Options {
    listen: SocketAddr,
    read_only: bool,
}

fn parse_options() -> Options {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut listen = env("DERUST_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.into());
    let mut read_only = env("DERUST_READ_ONLY").is_some_and(|v| v != "0" && v != "false");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().unwrap_or(listen),
            "--read-only" => read_only = true,
            "-h" | "--help" => {
                println!("Usage: tasmota-derust-web [--listen 0.0.0.0:8080] [--read-only]");
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    let listen = listen.parse().unwrap_or_else(|_| {
        eprintln!("Invalid listen address: {listen}");
        std::process::exit(2);
    });
    Options { listen, read_only }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let opts = parse_options();

    let mut cfg = Config::load();
    let env = |k: &str| std::env::var(k).ok();
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
    if cfg.broker.host.trim().is_empty() {
        eprintln!(
            "No MQTT broker configured. Set it in the desktop app or with DERUST_MQTT_HOST \
             (and DERUST_MQTT_PORT, DERUST_MQTT_USER, DERUST_MQTT_PASSWORD)."
        );
        std::process::exit(2);
    }

    let session = Arc::new(Session::start(&cfg.broker, opts.read_only, Arc::new(|| {})));

    let listener = tokio::net::TcpListener::bind(opts.listen)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Cannot listen on {}: {e}", opts.listen);
            std::process::exit(1);
        });
    println!(
        "Tasmota Derust web on {}{}",
        browser_url(opts.listen),
        if opts.read_only { "  (read-only)" } else { "" }
    );
    let hint = if opts.read_only {
        "Restart the server without --read-only to allow commands."
    } else {
        ""
    };
    axum::serve(listener, router(session, hint)).await.unwrap();
}
