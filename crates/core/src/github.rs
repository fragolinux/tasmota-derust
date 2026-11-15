//! Latest official Tasmota release, fetched once in the background.

use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug)]
pub enum Latest {
    Loading,
    Ok(String),
    Err(String),
}

pub fn fetch_latest(notify: crate::Notify) -> Arc<Mutex<Latest>> {
    let state = Arc::new(Mutex::new(Latest::Loading));
    let out = state.clone();
    thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let mut resp =
                ureq::get("https://api.github.com/repos/arendst/Tasmota/releases/latest")
                    .header("User-Agent", "tasmota-derust")
                    .header("Accept", "application/vnd.github+json")
                    .call()
                    .map_err(|e| e.to_string())?;
            let v: serde_json::Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            v.get("tag_name")
                .and_then(|t| t.as_str())
                .map(|t| t.trim_start_matches(['v', 'V']).to_string())
                .ok_or_else(|| "no tag_name in response".to_string())
        })();
        *out.lock().unwrap() = match result {
            Ok(v) => Latest::Ok(v),
            Err(e) => Latest::Err(e),
        };
        notify();
    });
    state
}
