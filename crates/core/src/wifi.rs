//! Setting a device's second Wi-Fi network (SSId1/SSId2 + Password1/Password2).
//!
//! Only the network the device is NOT connected to can be changed: changing the one in use
//! could leave the device unable to reconnect, cut off from MQTT and from this app. To change
//! the network in use, switch the device to the other one first (AP), then change this one.
//! Both values go in one `Backlog`, so the device restarts once; it then stays on the network
//! it was using, and the new one is tried only if that one is lost (or after an AP switch).
//! Tasmota never sends passwords back, and this program never stores or logs them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiError {
    BadSlot,
    /// That is the network the device is using now (or which one is not known).
    InUse,
    ActiveUnknown,
    BadSsid,
    BadPassword,
}

impl WifiError {
    pub fn message(self) -> &'static str {
        match self {
            WifiError::BadSlot => "Wi-Fi network must be 1 or 2.",
            WifiError::InUse => {
                "That is the network the device is using now: switch it to the other one first, then change this one."
            }
            WifiError::ActiveUnknown => {
                "Which network the device is using is not known yet: press Refresh and try again."
            }
            WifiError::BadSsid => {
                "The network name must be 1 to 32 characters, without ';' (and not just \"0\" or \"1\")."
            }
            WifiError::BadPassword => {
                "The password must be 8 to 63 characters (WPA2), without ';'."
            }
        }
    }
}

impl std::fmt::Display for WifiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for WifiError {}

/// The `Backlog` payload that sets network `slot` (1 or 2) on a device using network `active`.
pub fn command(
    slot: u8,
    active: Option<i64>,
    ssid: &str,
    password: &str,
) -> Result<String, WifiError> {
    if !(1..=2).contains(&slot) {
        return Err(WifiError::BadSlot);
    }
    match active {
        None => return Err(WifiError::ActiveUnknown),
        Some(a) if a == i64::from(slot) => return Err(WifiError::InUse),
        _ => {}
    }
    let ssid = ssid.trim();
    // Tasmota: "0"/"1" as a value mean "clear" / "back to the firmware default".
    if ssid.is_empty()
        || ssid.chars().count() > 32
        || ssid.contains(';')
        || ssid == "0"
        || ssid == "1"
    {
        return Err(WifiError::BadSsid);
    }
    let n = password.chars().count();
    if !(8..=63).contains(&n) || password.contains(';') {
        return Err(WifiError::BadPassword);
    }
    Ok(format!("SSId{slot} {ssid}; Password{slot} {password}"))
}

/// The same text with passwords hidden, for logs.
pub fn mask_passwords(payload: &str) -> String {
    let mut out = Vec::new();
    for part in payload.split(';') {
        let t = part.trim_start();
        let cmd: String = t.chars().take_while(|c| !c.is_whitespace()).collect();
        // Password1/2, MqttPassword, WebPassword…
        if cmd.to_ascii_lowercase().contains("password") {
            let lead = &part[..part.len() - t.len()];
            out.push(if t.len() > cmd.len() {
                format!("{lead}{cmd} ****")
            } else {
                part.to_string()
            });
        } else {
            out.push(part.to_string());
        }
    }
    out.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_unused_network() {
        assert_eq!(
            command(2, Some(1), "Home-Extender", "secret123").as_deref(),
            Ok("SSId2 Home-Extender; Password2 secret123")
        );
        assert_eq!(command(1, Some(1), "x", "secret123"), Err(WifiError::InUse));
        assert_eq!(
            command(2, None, "x", "secret123"),
            Err(WifiError::ActiveUnknown)
        );
        assert_eq!(
            command(3, Some(1), "x", "secret123"),
            Err(WifiError::BadSlot)
        );
        assert_eq!(
            command(2, Some(1), "a;b", "secret123"),
            Err(WifiError::BadSsid)
        );
        assert_eq!(
            command(2, Some(1), "1", "secret123"),
            Err(WifiError::BadSsid)
        );
        assert_eq!(
            command(2, Some(1), "net", "short"),
            Err(WifiError::BadPassword)
        );
        assert_eq!(
            command(2, Some(1), "net", "pass;word1"),
            Err(WifiError::BadPassword)
        );
    }

    #[test]
    fn masks() {
        assert_eq!(
            mask_passwords("SSId2 Home; Password2 secret123"),
            "SSId2 Home; Password2 ****"
        );
        assert_eq!(mask_passwords("Power1 ON"), "Power1 ON");
        assert_eq!(mask_passwords("Password1"), "Password1");
    }
}
