"""Reports of the Trivy JSON results written by package.sh.

  trivy_summary.py <report dir>
      Markdown summary of all reports (reads <report dir>/trivy-version.json, the output of
      `trivy version --format json`).
  trivy_summary.py --report <trivy json> <trivy-version.json>
      Plain-text report of one release file (TRIVY-REPORT.txt inside each archive).

Summary mode: exit status 1 when the release should be blocked: a HIGH or CRITICAL
vulnerability that has a fixed version, or a scanned binary without dependency data.
"""

import json
import pathlib
import sys

SEVERITIES = ["CRITICAL", "HIGH", "MEDIUM", "LOW", "UNKNOWN"]
BLOCKING = {"CRITICAL", "HIGH"}


def main() -> int:
    report_dir = pathlib.Path(sys.argv[1])
    info = json.loads((report_dir / "trivy-version.json").read_text())
    db = (info.get("VulnerabilityDB") or {}).get("UpdatedAt", "?")[:10]

    rows, findings, problems = [], [], []
    for path in sorted(report_dir.glob("*.trivy.json")):
        label = path.name.removesuffix(".trivy.json")
        report = json.loads(path.read_text())
        results = report.get("Results") or []
        packages = sum(len(r.get("Packages") or []) for r in results)
        counts = dict.fromkeys(SEVERITIES, 0)
        for r in results:
            for v in r.get("Vulnerabilities") or []:
                sev = v.get("Severity", "UNKNOWN")
                counts[sev] = counts.get(sev, 0) + 1
                findings.append((label, v))
                if sev in BLOCKING and v.get("FixedVersion"):
                    problems.append(f"{label}: {v['VulnerabilityID']} in {v['PkgName']} "
                                    f"(fixed in {v['FixedVersion']})")
        if packages == 0:
            problems.append(f"{label}: no dependency data found (built without cargo-auditable?)")
        rows.append((label, packages, counts))

    out = [
        "## Security scan",
        "",
        f"Scanned with Trivy {info.get('Version', '?')}, vulnerability database of {db}. "
        "Rust binaries are checked through the dependency list embedded by cargo-auditable; "
        "the APK's Java/Kotlin libraries through the Gradle lockfile of the release build.",
        "",
        "| Artifact | Packages | Critical | High | Medium | Low |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for label, packages, c in rows:
        out.append(f"| `{label}` | {packages} | {c['CRITICAL']} | {c['HIGH']} | {c['MEDIUM']} | {c['LOW']} |")
    out.append("")

    if findings:
        out += ["### Findings", "", "| Artifact | ID | Package | Installed | Fixed in | Severity |",
                "|---|---|---|---|---|---|"]
        order = {s: i for i, s in enumerate(SEVERITIES)}
        for label, v in sorted(findings, key=lambda f: order.get(f[1].get("Severity"), 9)):
            vid = v["VulnerabilityID"]
            link = v.get("PrimaryURL")
            vid = f"[{vid}]({link})" if link else vid
            out.append(f"| `{label}` | {vid} | {v['PkgName']} | {v.get('InstalledVersion', '')} | "
                       f"{v.get('FixedVersion') or '–'} | {v.get('Severity', '')} |")
        out.append("")
    else:
        out += ["No known vulnerabilities.", ""]

    out.append("Each archive and disk image contains its own `TRIVY-REPORT.txt` / `.json` and "
               "`SHA256SUMS`. All reports and CycloneDX SBOMs: `security-reports.zip`.")
    if problems:
        out += ["", "### Blocking", ""] + [f"- {p}" for p in problems]
    print("\n".join(out))
    return 1 if problems else 0


def report(json_path: str, info_path: str) -> int:
    """Plain-text report of one Trivy JSON result (TRIVY-REPORT.txt inside each archive)."""
    rep = json.loads(pathlib.Path(json_path).read_text())
    info = json.loads(pathlib.Path(info_path).read_text())
    db = (info.get("VulnerabilityDB") or {}).get("UpdatedAt", "?")[:10]
    results = rep.get("Results") or []
    vulns = [(r.get("Target", ""), v) for r in results for v in r.get("Vulnerabilities") or []]
    out = [
        f"Trivy {info.get('Version', '?')} scan, vulnerability database of {db}, "
        f"made {rep.get('CreatedAt', '?')[:19].replace('T', ' ')} UTC.",
        "Severities checked: CRITICAL, HIGH, MEDIUM, LOW, UNKNOWN.",
        "",
    ]
    if vulns:
        out.append(f"Vulnerabilities: {len(vulns)}")
        for target, v in vulns:
            out.append(f"  {v.get('Severity', '?'):8} {v['VulnerabilityID']}  {v['PkgName']} "
                       f"{v.get('InstalledVersion', '')} -> {v.get('FixedVersion') or 'no fix'}  "
                       f"({target})")
    else:
        out.append("No known vulnerabilities.")
    for r in results:
        pkgs = r.get("Packages") or []
        out += ["", f"{r.get('Target', '?')}: {len(pkgs)} packages"]
        out += [f"  {p.get('Name', '?')} {p.get('Version', '')}"
                for p in sorted(pkgs, key=lambda p: p.get("Name", ""))]
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "--report":
        sys.exit(report(sys.argv[2], sys.argv[3]))
    sys.exit(main())
