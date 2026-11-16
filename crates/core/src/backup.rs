//! Whole-device configuration backup and restore, through the device's own web server.
//!
//! The backup is Tasmota's config dump: `GET /dl`, the same `.dmp` file as the web UI's
//! "Backup Configuration" (readable by decode-config, restorable from the web UI too). The
//! restore does what the web UI's "Restore Configuration" does: `GET /rs` (selects the
//! configuration upload), then a multipart POST of the file to `/u2`; the device then
//! restarts with that configuration.
//!
//! A device with a WebPassword answers 401: the caller asks the user for it and tries again.
//! The password is used for that request only (HTTP Basic, user "admin"), never stored.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum BackupError {
    /// The device wants its web password (none given).
    NeedsPassword,
    /// The password given was refused.
    WrongPassword,
    /// The device's IP address is not known yet.
    NoAddress,
    Failed(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::NeedsPassword => f.write_str("The device's web interface has a password."),
            BackupError::WrongPassword => f.write_str("Wrong web password."),
            BackupError::NoAddress => f.write_str("The device's IP address is not known yet."),
            BackupError::Failed(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for BackupError {}

/// A configuration dump.
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    /// File name, as Tasmota names it (`Config_<hostname>_<version>.dmp`) when it says so.
    pub name: String,
    pub data: Vec<u8>,
}

const TIMEOUT: Duration = Duration::from_secs(15);
/// Smallest real dump (ESP8266 settings are 4 KB; a web page is not a dump).
const MIN_SIZE: usize = 1024;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into()
}

fn auth(password: Option<&str>) -> Option<String> {
    password
        .filter(|p| !p.is_empty())
        .map(|p| format!("Basic {}", base64(format!("admin:{p}").as_bytes())))
}

fn unreachable(e: ureq::Error) -> BackupError {
    let why = match e {
        ureq::Error::Timeout(_) => "no answer".to_string(),
        ureq::Error::ConnectionFailed => "connection refused".to_string(),
        e => e.to_string(),
    };
    BackupError::Failed(format!(
        "Could not reach the device's web server ({why}): check that it is on and that this device can reach its IP address."
    ))
}

fn status_error(code: u16, password: Option<&str>) -> BackupError {
    match code {
        401 if password.is_some_and(|p| !p.is_empty()) => BackupError::WrongPassword,
        401 => BackupError::NeedsPassword,
        404 => BackupError::Failed(
            "The device's web server has no backup page (web server off, or a build without it)."
                .into(),
        ),
        c => BackupError::Failed(format!("The device answered HTTP {c}.")),
    }
}

fn get(url: &str, password: Option<&str>) -> Result<ureq::http::Response<ureq::Body>, BackupError> {
    let mut req = agent()
        .get(url)
        .header("User-Agent", "tasmota-derust")
        // Tasmota refuses privileged pages without it (referer check, SetOption128 0).
        .header("Referer", format!("http://{}/", host_of(url)));
    if let Some(a) = auth(password) {
        req = req.header("Authorization", a);
    }
    let resp = req.call().map_err(unreachable)?;
    let code = resp.status().as_u16();
    if code != 200 {
        return Err(status_error(code, password));
    }
    Ok(resp)
}

/// "http://192.168.1.60/dl" → "192.168.1.60".
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split('/').next().unwrap_or(rest)
}

fn check_ip(ip: &str) -> Result<&str, BackupError> {
    let ip = ip.trim();
    if ip.is_empty() || ip == "0.0.0.0" {
        return Err(BackupError::NoAddress);
    }
    // Only a plain address or host name: nothing that changes the URL.
    if !ip
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
    {
        return Err(BackupError::Failed("Not a valid device address.".into()));
    }
    Ok(ip)
}

/// Downloads the configuration dump of the device at `ip` (blocking).
pub fn download(
    ip: &str,
    password: Option<&str>,
    fallback_name: &str,
) -> Result<Backup, BackupError> {
    let ip = check_ip(ip)?;
    let mut resp = get(&format!("http://{ip}/dl"), password)?;
    let name = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(filename_from_disposition)
        .unwrap_or_else(|| fallback_name.to_string());
    let html = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.contains("html"));
    let data = resp
        .body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_to_vec()
        .map_err(|e| BackupError::Failed(e.to_string()))?;
    if html || data.len() < MIN_SIZE {
        // Webserver 1 (user mode) shows a page instead of the file.
        return Err(BackupError::Failed(
            "The device did not send a configuration file: its web server must be in admin mode (Webserver 2)."
                .into(),
        ));
    }
    Ok(Backup { name, data })
}

/// Uploads a configuration dump to the device at `ip` (blocking). The device restarts.
pub fn restore(ip: &str, password: Option<&str>, data: &[u8]) -> Result<(), BackupError> {
    let ip = check_ip(ip)?;
    if data.len() < MIN_SIZE {
        return Err(BackupError::Failed(
            "This is not a Tasmota configuration file.".into(),
        ));
    }
    // Selects the configuration upload (as opening the Restore page does).
    get(&format!("http://{ip}/rs?"), password)?;
    let boundary = "----tasmota-derust-7f3a91c2";
    let mut body = Vec::with_capacity(data.len() + 256);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"u2\"; filename=\"config.dmp\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let mut req = agent()
        .post(&format!("http://{ip}/u2"))
        .header("User-Agent", "tasmota-derust")
        // Tasmota refuses privileged pages without it (referer check, SetOption128 0).
        .header("Referer", format!("http://{}/", ip))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        );
    if let Some(a) = auth(password) {
        req = req.header("Authorization", a);
    }
    let mut resp = req.send(&body[..]).map_err(unreachable)?;
    let code = resp.status().as_u16();
    if code != 200 {
        return Err(status_error(code, password));
    }
    let page = resp.body_mut().read_to_string().unwrap_or_default();
    upload_result(&page)
}

/// The result page of an upload: "Successful", or the reason it failed.
fn upload_result(page: &str) -> Result<(), BackupError> {
    let lower = page.to_ascii_lowercase();
    if lower.contains("successful") {
        return Ok(());
    }
    if let Some(i) = lower.find("failed") {
        let text: String = strip_tags(&page[i..]).chars().take(120).collect();
        return Err(BackupError::Failed(format!("Upload {}", text.trim())));
    }
    Err(BackupError::Failed(
        "The device did not confirm the restore.".into(),
    ))
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                out.push(' ');
            }
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `attachment; filename=Config_x_16.0.0.dmp` → the file name (path parts dropped).
fn filename_from_disposition(v: &str) -> Option<String> {
    let i = v.to_ascii_lowercase().find("filename=")?;
    let name = v[i + 9..].split(';').next()?.trim().trim_matches('"');
    let name = name.rsplit(['/', '\\']).next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Where the desktop app saves backups: the user's Downloads folder (else the home folder).
pub fn downloads_dir() -> Option<std::path::PathBuf> {
    let dirs = directories::UserDirs::new()?;
    Some(
        dirs.download_dir()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| dirs.home_dir().to_path_buf()),
    )
}

/// Saves a backup in `dir` without overwriting an older one (`name-1.dmp`, `name-2.dmp`…).
pub fn save(dir: &std::path::Path, b: &Backup) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = std::path::Path::new(&b.name);
    let stem = name
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Config");
    let ext = name.extension().and_then(|s| s.to_str()).unwrap_or("dmp");
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut i = 1;
    while path.exists() {
        path = dir.join(format!("{stem}-{i}.{ext}"));
        i += 1;
    }
    std::fs::write(&path, &b.data)?;
    Ok(path)
}

/// The `.dmp` files in `dir`, newest first.
pub fn list_dumps(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("dmp"))
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().map(|(_, p)| p).collect()
}

/// A file name for a backup when the device gives none: `Config_<topic>_<version>.dmp`.
pub fn default_name(topic: &str, version: Option<&str>) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let v = version
        .map(crate::device::short_version)
        .unwrap_or("unknown");
    format!("Config_{}_{}.dmp", clean(topic), clean(v))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(base64(b"admin:pw"), "YWRtaW46cHc=");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(
            filename_from_disposition("attachment; filename=Config_test-1234_16.0.0.dmp"),
            Some("Config_test-1234_16.0.0.dmp".into())
        );
        assert_eq!(
            filename_from_disposition("attachment; filename=\"../x/a.dmp\""),
            Some("a.dmp".into())
        );
        assert_eq!(
            default_name("my plug", Some("16.0.0(tasmota)")),
            "Config_my_plug_16.0.0.dmp"
        );
        assert!(upload_result("<b>Upload Successful</b>").is_ok());
        assert!(matches!(
            upload_result("<b>Upload Failed</b><br>Not enough space"),
            Err(BackupError::Failed(e)) if e.contains("Not enough space")
        ));
        assert_eq!(check_ip(""), Err(BackupError::NoAddress));
        assert_eq!(host_of("http://192.168.1.60/dl"), "192.168.1.60");
        assert!(check_ip("192.168.1.60/x?").is_err());
        assert_eq!(check_ip("192.168.1.60"), Ok("192.168.1.60"));
        assert_eq!(status_error(401, None), BackupError::NeedsPassword);
        let dir = std::env::temp_dir().join(format!("derust-backup-test-{}", std::process::id()));
        let b = Backup {
            name: "Config_x_1.dmp".into(),
            data: vec![1; 2048],
        };
        let p1 = save(&dir, &b).unwrap();
        let p2 = save(&dir, &b).unwrap();
        assert!(p1.ends_with("Config_x_1.dmp") && p2.ends_with("Config_x_1-1.dmp"));
        assert_eq!(list_dumps(&dir).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(status_error(401, Some("x")), BackupError::WrongPassword);
    }
}
