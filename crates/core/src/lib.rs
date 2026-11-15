//! Shared core of Tasmota Derust: device model, MQTT connection, settings.
//! Used by the desktop app, the web server and the Android library.

pub mod columns;
pub mod config;
pub mod device;
pub mod github;
pub mod light;
pub mod mqtt;
pub mod ota;
pub mod session;

use std::sync::Arc;

/// Callback used by background threads to say "something changed".
pub type Notify = Arc<dyn Fn() + Send + Sync>;
