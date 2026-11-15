# Security

Please report vulnerabilities privately, through **Security > Report a vulnerability** on
this repository's GitHub page, not in a public issue.

Tasmota Derust controls real devices (relays, Wi-Fi settings, firmware upgrades), so reports
about anything that could send a command without the user's intent are especially welcome,
as are the usual ones about dependencies and the web server.

Every push is checked with Trivy (dependencies and committed secrets), and every release is
scanned again, on exactly the files that are shipped, before it is published: each archive
contains its own `TRIVY-REPORT.txt`, and `security-reports.zip` in each release has them all and "Releases and CI" in DEVELOPMENT.md.
