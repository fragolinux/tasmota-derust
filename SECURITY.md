# Security

Please report vulnerabilities privately, through **Security > Report a vulnerability** on
this repository's GitHub page, not in a public issue.

Tasmota Derust controls real devices (relays, Wi-Fi settings, firmware upgrades), so reports
about anything that could send a command without the user's intent are especially welcome,
as are the usual ones about dependencies and the web server.

Every push is checked with Trivy (dependencies and committed secrets), and every release is
scanned again, on exactly the files that are shipped, before it is published: each archive
contains its own `TRIVY-REPORT.txt`, and `security-reports.zip` in each release has them all and "Releases and CI" in DEVELOPMENT.md.

## The web server: choices and their limits

The web server is the part of Tasmota Derust that other machines can reach, so these are the
decisions behind it, and what each one does **not** protect against. They followed
[issue #1](https://github.com/fragolinux/tasmota-derust/issues/1).

### Read-only unless you choose otherwise, every time

- The standalone server comes as **two programs**: `tasmota-derust-web` is always read-only,
  `tasmota-derust-web-rw` always sends commands. The mode is fixed by which one you run, so it
  is a visible choice at every launch, not a flag that is easy to forget (or to leave in a
  script). Up to 0.7.1 the single program started writable unless `--read-only` was given.
- The desktop app's **Start web server** asks every time: read-only or read-write. The web
  server's mode no longer follows the app's Read-only checkbox, so unlocking the app for a
  moment does not unlock every browser on the network.
- Read-write without a login is allowed (some networks are trusted), but never silently: the
  app's start dialog, the console and a red banner on the page say what it means.

### Other web pages cannot use it through your browser

- Any request that changes something must come from the server's own page: its `Origin` (or
  `Referer` when there is none) must name the same host. A form or script on another site,
  opened by someone on your network, is refused. Without this, a plain HTML form could switch
  a relay or start an upgrade.
- The `Host` must be an IP address or a local name (`localhost`, `nas`, `*.local`, `*.lan`,
  `*.home.arpa`, `*.internal`, `*.localhost`), which stops **DNS rebinding** (a public domain
  pointed at your server's address, where the Origin check alone would pass). Names behind a
  reverse proxy can be allowed with `DERUST_ALLOWED_HOSTS`.
- Requests without either header (curl, scripts) are not browser requests and are let through:
  whoever can run them on your network could also talk to the broker directly.

### Optional login, stored as a hash

- HTTP Basic: the browser asks, nothing to build or maintain, works on phones. It is the same
  login for the desktop app's server and both programs, because they all read the same config
  file (`DERUST_WEB_USER` / `DERUST_WEB_PASSWORD` override it on headless machines).
- The password is **never stored**: only a salted PBKDF2-HMAC-SHA256 hash (100,000 rounds, 16
  random bytes of salt). Anyone who reads the config file cannot log in with what they find,
  and a guess costs a PBKDF2 run. A plain password left by an earlier build is hashed and
  removed the first time the config is loaded.
- Checking a hash is slow on purpose, too slow for every request of a page that refreshes
  every 2 seconds. A login that passed is remembered **in memory only** (as the SHA-256 of the
  `Authorization` header, forgotten at restart, at most 64 kept), and unknown ones are checked
  one at a time, so wrong guesses cannot use more than one CPU core.
- Basic auth sends the password with every request: without HTTPS anyone who can see the
  traffic can read it. See below.

### Optional HTTPS, with a self-signed certificate

- Off by default, because a self-signed certificate means a browser warning, which would scare
  people who only use it at home. On: the **HTTPS** checkbox in the desktop app's Settings,
  `--https`, or `DERUST_WEB_HTTPS=1`. Port 8443 (8080 stays for HTTP).
- The certificate and its key are made once, next to the config file (`web-cert.pem`,
  `web-key.pem`), for `localhost`, `127.0.0.1`, `::1` and the computer's LAN address at that
  moment. Shared by the app and both programs, so a browser exception keeps working whichever
  one you start. Delete both files to make a new one, e.g. after the address changed.
- No authority signs it, so the browser cannot tell your server from an impostor: accept the
  warning only if the **SHA-256 fingerprint** matches the one the server prints at start (in
  the desktop app: hover the address). A real certificate (Let's Encrypt, your own CA) needs a
  domain name and renewal: out of scope for a home tool, a reverse proxy can do it instead.
- rustls with the *ring* crypto provider, already used by the app for its GitHub requests, so
  no OpenSSL or system library is needed and every platform builds the same way.
- Without HTTPS everything is plain HTTP: the login, every device's data and the Wi-Fi
  passwords you send to devices travel in clear text on your network. The app says so wherever
  the choice is made.

### What is stored, where, and how

| | Windows | macOS | Linux |
|---|---|---|---|
| Folder | `%APPDATA%\tasmota-derust\config\` | `~/Library/Application Support/tasmota-derust/` | `~/.config/tasmota-derust/` |

| File | Content | Protection |
|---|---|---|
| `config.json` | broker host, port, user, **password**; web login user and **password hash**; HTTPS on/off; theme, columns | broker password in **plain text**; web password only as a hash |
| `web-cert.pem` | the self-signed certificate (public) | none needed |
| `ota.json` | firmware server URL and highest version, the default and per device | plain text, no secrets |
| `http_devices.json` | addresses of the devices reached over HTTP only | plain text; their web passwords are never saved |
| `console.json` | console command history per device, the five favourite commands | plain text; commands with a password never enter the history, favourites are saved as typed |
| `web-key.pem` | its private key, not encrypted | file permissions: owner only on macOS/Linux (0600); on Windows the user profile's ACLs (the user, SYSTEM, administrators) |

- **Enable MQTT** sends the broker settings, password included, to devices over plain HTTP
  (Tasmota has no HTTPS), like Tasmota's own web page does; by default it uses the app's own
  broker password, which the web page never receives (the server fills it in). The network
  search only reads (`Status 0` to each address); web passwords given for it stay in memory.
- Nothing is encrypted on disk. Anything running as your user (or an administrator, or
  someone with your disk or a backup of it) can read these files. The web password hash does
  not let them log in; the **broker password** does let them connect to your broker.
- The broker password must be sent to the broker as typed, so it cannot be hashed. Keeping it
  in the system's secret store (Windows DPAPI / Credential Manager, macOS Keychain, Linux
  Secret Service) would protect it from copies of the file, not from software running as you,
  and many headless Linux machines running the web server have no secret store. Not done yet:
  see the roadmap in [DESIGN.md](DESIGN.md#roadmap).
- If the private key leaks, someone could impersonate your server to browsers that accepted
  the certificate: delete both files and accept the new fingerprint.
- The Android app keeps its broker settings in its private app storage, excluded from backups.

### Not covered

- No accounts or roles: whoever has the login has full control (in read-write mode).
- No rate limit or lockout beyond the slow, one-at-a-time password check.
- The MQTT connection itself is plain (no TLS to the broker yet); the broker's own users and
  ACLs are what protect the devices from anyone else on the network.

