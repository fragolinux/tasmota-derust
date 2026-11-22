//! Persisted settings (JSON in the user's config directory).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::columns::{Col, View};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrokerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 1883,
            username: String::new(),
            password: String::new(),
        }
    }
}

/// Login of the web interface (HTTP Basic). Shared by the desktop app's built-in server
/// and the standalone `tasmota-derust-web` programs, which read the same config file.
///
/// The password is never stored: only a salted PBKDF2-HMAC-SHA256 hash
/// (`pbkdf2-sha256$<iterations>$<salt hex>$<hash hex>`), enough to check a login and useless
/// to log in elsewhere. Off while the username or the hash is empty.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebLogin {
    pub username: String,
    pub password_hash: String,
    /// A plain password in an older config file: read once, hashed by [`Config::load`],
    /// never written back.
    #[serde(skip_serializing)]
    password: String,
}

/// PBKDF2 rounds: slow enough to make guessing a stolen hash expensive, fast enough for
/// one check per browser session (the web server remembers logins it has checked).
const PBKDF2_ROUNDS: u32 = 100_000;

impl WebLogin {
    /// A login for `username` with `password` hashed under a fresh random salt.
    pub fn new(username: &str, password: &str) -> WebLogin {
        use ring::rand::SecureRandom;
        let mut salt = [0u8; 16];
        ring::rand::SystemRandom::new()
            .fill(&mut salt)
            .expect("system random generator");
        let mut hash = [0u8; 32];
        ring::pbkdf2::derive(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            std::num::NonZeroU32::new(PBKDF2_ROUNDS).expect("non-zero"),
            &salt,
            password.as_bytes(),
            &mut hash,
        );
        WebLogin {
            username: username.into(),
            password_hash: format!(
                "pbkdf2-sha256${PBKDF2_ROUNDS}${}${}",
                hex(&salt),
                hex(&hash)
            ),
            password: String::new(),
        }
    }

    pub fn is_set(&self) -> bool {
        !self.username.is_empty() && !self.password_hash.is_empty()
    }

    /// Whether `username` / `password` match (the comparison takes the same time
    /// wherever the first difference is).
    pub fn verify(&self, username: &str, password: &str) -> bool {
        let mut parts = self.password_hash.split('$');
        let (Some("pbkdf2-sha256"), Some(rounds), Some(salt), Some(hash), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return false;
        };
        let (Some(rounds), Some(salt), Some(hash)) = (
            rounds.parse().ok().and_then(std::num::NonZeroU32::new),
            unhex(salt),
            unhex(hash),
        ) else {
            return false;
        };
        let pw_ok = ring::pbkdf2::verify(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            rounds,
            &salt,
            password.as_bytes(),
            &hash,
        )
        .is_ok();
        // Same time wherever the first difference is (the length may show).
        let (a, b) = (username.as_bytes(), self.username.as_bytes());
        let user_ok =
            a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0;
        pw_ok && user_ok && self.is_set()
    }

    /// Hashes a plain password left by an older version. True when something changed.
    fn migrate(&mut self) -> bool {
        if self.password.is_empty() {
            return false;
        }
        let plain = std::mem::take(&mut self.password);
        *self = WebLogin::new(&self.username, &plain);
        true
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnSetting {
    pub col: Col,
    pub visible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub broker: BrokerConfig,
    pub show_offline: bool,
    pub view: View,
    pub columns: BTreeMap<View, Vec<ColumnSetting>>,
    /// Desktop app colours: "system", "light" or "dark".
    pub theme: String,
    pub web_login: WebLogin,
    /// Serve the web interface over HTTPS (self-signed certificate, port 8443).
    pub web_https: bool,
}

impl Default for Config {
    fn default() -> Self {
        let mut c = Self {
            broker: BrokerConfig::default(),
            show_offline: false,
            view: View::Home,
            columns: BTreeMap::new(),
            theme: "system".into(),
            web_login: WebLogin::default(),
            web_https: false,
        };
        c.normalize();
        c
    }
}

impl Config {
    /// The app's config directory, shared by the desktop app and the web servers.
    pub fn dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "tasmota-derust").map(|d| d.config_dir().into())
    }

    fn path() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("config.json"))
    }

    pub fn load() -> Self {
        let mut cfg: Config = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        cfg.normalize();
        if cfg.web_login.migrate() {
            // Drop the plain password from the file right away.
            cfg.save();
        }
        cfg
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }

    /// Makes sure every view lists every column exactly once.
    fn normalize(&mut self) {
        // Up to 0.3.0 the Firmware view ended with OtaUrl, Update: the Update column was
        // often off screen. Untouched layouts move to the new default.
        {
            use Col::*;
            const OLD_FIRMWARE: [Col; 7] = [Version, Core, Sdk, ProgramSize, Free, OtaUrl, Update];
            if self.visible_columns(View::Firmware) == OLD_FIRMWARE {
                self.columns
                    .insert(View::Firmware, default_columns(View::Firmware));
            }
        }
        for view in View::ALL {
            let list = self
                .columns
                .entry(view)
                .or_insert_with(|| default_columns(view));
            let mut seen = Vec::new();
            list.retain(|c| {
                let fresh = !seen.contains(&c.col);
                seen.push(c.col);
                fresh
            });
            for col in Col::ALL {
                if !list.iter().any(|c| c.col == col) {
                    list.push(ColumnSetting {
                        col,
                        visible: false,
                    });
                }
            }
        }
    }

    pub fn reset_columns(&mut self, view: View) {
        self.columns.insert(view, default_columns(view));
        self.normalize();
    }

    pub fn visible_columns(&self, view: View) -> Vec<Col> {
        self.columns
            .get(&view)
            .map(|l| l.iter().filter(|c| c.visible).map(|c| c.col).collect())
            .unwrap_or_default()
    }
}

fn default_columns(view: View) -> Vec<ColumnSetting> {
    view.default_columns()
        .iter()
        .map(|&col| ColumnSetting { col, visible: true })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn web_login_is_hashed_and_checked() {
        let l = WebLogin::new("me", "sécret pw");
        assert!(l.is_set());
        assert!(!l.password_hash.contains("sécret"));
        assert!(l.verify("me", "sécret pw"));
        assert!(!l.verify("me", "secret pw"));
        assert!(!l.verify("you", "sécret pw"));
        // A new salt every time.
        assert_ne!(
            l.password_hash,
            WebLogin::new("me", "sécret pw").password_hash
        );
        assert!(!WebLogin::default().verify("", ""));
        // Never written in plain text, even from an old file.
        let mut old: WebLogin =
            serde_json::from_str(r#"{"username":"me","password":"pw"}"#).unwrap();
        assert!(old.migrate());
        assert!(old.verify("me", "pw"));
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("\"password\""), "{json}");
    }

    use super::*;

    fn firmware(cols: &[Col]) -> Config {
        let mut c = Config::default();
        c.columns.insert(
            View::Firmware,
            cols.iter()
                .map(|&col| ColumnSetting { col, visible: true })
                .collect(),
        );
        c.normalize();
        c
    }

    #[test]
    fn old_firmware_layout_moves_update_forward() {
        use Col::*;
        let c = firmware(&[Version, Core, Sdk, ProgramSize, Free, OtaUrl, Update]);
        assert_eq!(
            c.visible_columns(View::Firmware),
            View::Firmware.default_columns()
        );
        // A layout the user changed stays as it is.
        let mine = [Version, OtaUrl, Update];
        assert_eq!(firmware(&mine).visible_columns(View::Firmware), mine);
    }
}
