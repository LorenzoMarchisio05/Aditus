//! Update check against GitHub releases, and (macOS) install of the release zip.
//!
//! Nothing is installed without a click. The install path re-queries GitHub itself, only accepts an
//! asset hosted under this repo, requires GitHub's own sha256 digest to match the download, and
//! requires a valid code signature and the same bundle id before swapping the app bundle.

use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, process::Command, time::Duration};

const REPO: &str = "LorenzoMarchisio05/Aditus";
const ASSET: &str = "Aditus-macos-arm64.zip";

/// Numeric `a.b.c` comparison; anything unparsable is "not newer".
pub fn is_newer(latest: &str, current: &str) -> bool {
    let n = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok()).collect::<Option<Vec<_>>>();
    matches!((n(latest), n(current)), (Some(l), Some(c)) if l > c)
}

struct Release {
    tag: String,
    page: String,
    asset: Option<(String, String)>, // (download url, sha256 hex)
}

/// Debug builds only: ADITUS_UPDATE_API points the check at a local fake release (see scripts/update-e2e.sh).
fn api_url() -> String {
    std::env::var("ADITUS_UPDATE_API").ok().filter(|_| cfg!(debug_assertions)).unwrap_or_else(|| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}

fn latest() -> Option<Release> {
    let body = ureq::get(&api_url())
        .set("User-Agent", "aditus")
        .timeout(Duration::from_secs(10))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let v: Value = serde_json::from_str(&body).ok()?;
    let asset = v["assets"].as_array()?.iter().find(|a| a["name"] == ASSET).and_then(|a| {
        let url = a["browser_download_url"].as_str()?;
        let sha = a["digest"].as_str()?.strip_prefix("sha256:")?;
        (url.starts_with(&format!("https://github.com/{REPO}/releases/download/")) || (cfg!(debug_assertions) && std::env::var_os("ADITUS_UPDATE_API").is_some())).then(|| (url.to_string(), sha.to_lowercase()))
    });
    Some(Release { tag: v["tag_name"].as_str()?.trim_start_matches('v').into(), page: v["html_url"].as_str()?.into(), asset })
}

/// The running `.app` bundle, when its parent directory is writable (else we can't swap it).
fn bundle() -> Option<PathBuf> {
    let app = std::env::current_exe().ok()?.ancestors().nth(3)?.to_path_buf();
    let probe = app.with_file_name(format!(".aditus-write-test-{}", std::process::id()));
    std::fs::write(&probe, "").ok()?;
    let _ = std::fs::remove_file(probe);
    app.extension().is_some_and(|e| e == "app").then_some(app)
}

pub fn check() -> Value {
    let current = env!("CARGO_PKG_VERSION");
    let Some(r) = latest() else { return json!({ "current": current, "latest": null, "url": null, "newer": false, "can_install": false }) };
    let newer = is_newer(&r.tag, current);
    let can_install = cfg!(target_os = "macos") && newer && r.asset.is_some() && bundle().is_some();
    json!({ "current": current, "latest": r.tag, "url": r.page, "newer": newer, "can_install": can_install })
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let o = cmd.output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
}

fn bundle_id(app: &Path) -> Result<String, String> {
    run(Command::new("/usr/libexec/PlistBuddy").args(["-c", "Print :CFBundleIdentifier"]).arg(app.join("Contents/Info.plist")))
}

/// Developer ID Team ID from `codesign -dv` output (it goes to stderr); `None` for ad-hoc/unsigned bundles.
fn team_id(codesign_out: &str) -> Option<String> {
    codesign_out.lines().find_map(|l| l.strip_prefix("TeamIdentifier=")).map(str::trim).filter(|t| !t.is_empty() && *t != "not set").map(String::from)
}

fn bundle_team(app: &Path) -> Option<String> {
    let o = Command::new("codesign").args(["-dv"]).arg(app).output().ok()?;
    team_id(&String::from_utf8_lossy(&o.stderr))
}

/// Download, verify and stage the new bundle, then start the swap that runs after this process exits.
/// Returns once the caller may quit.
pub fn install() -> Result<(), String> {
    let r = install_inner();
    if r.is_err() {
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("aditus-update-{}", std::process::id())));
    }
    r
}

fn install_inner() -> Result<(), String> {
    let r = latest().filter(|r| is_newer(&r.tag, env!("CARGO_PKG_VERSION"))).ok_or("No newer release")?;
    let (url, sha) = r.asset.ok_or("Release has no verifiable download")?;
    let cur = bundle().ok_or("Aditus isn't in a writable location; download it from the release page")?;

    let dir = std::env::temp_dir().join(format!("aditus-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join(ASSET);
    let res = ureq::get(&url).set("User-Agent", "aditus").timeout(Duration::from_secs(900)).call().map_err(|e| e.to_string())?;
    std::io::copy(&mut res.into_reader(), &mut std::fs::File::create(&zip).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let got = run(Command::new("shasum").args(["-a", "256"]).arg(&zip))?;
    if got.split_whitespace().next() != Some(sha.as_str()) {
        return Err("Download doesn't match the published checksum".into());
    }
    let stage = dir.join("new");
    run(Command::new("ditto").args(["-x", "-k"]).arg(&zip).arg(&stage))?;
    let new = stage.join("Aditus.app");
    // Our Info.plist has LSFileQuarantineEnabled, so everything we write is quarantined; Gatekeeper would
    // then refuse the relaunch of an unnotarized build. The download was checked against GitHub's digest.
    let _ = crate::applog::check("update xattr", run(Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(&new)));
    run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(&new)).map_err(|e| format!("Bad signature: {e}"))?;
    if bundle_id(&new)? != bundle_id(&cur)? {
        return Err("Downloaded app has a different identity".into());
    }
    // A Developer-ID-signed install only accepts updates from the same team, so a forged release can't swap in
    // another signer. Ad-hoc builds have no Team ID to pin (the bundle id above is all they can check).
    if let Some(team) = bundle_team(&cur) {
        if bundle_team(&new) != Some(team) {
            return Err("Downloaded app is signed by a different team".into());
        }
    }

    // Wait for us to exit, swap with a backup, relaunch; roll back if the move fails.
    let script = r#"while kill -0 "$1" 2>/dev/null; do sleep 0.3; done
rm -rf "$2.old"; mv "$2" "$2.old" && mv "$3" "$2" && { rm -rf "$2.old" "$4"; open "$2"; } || { [ -d "$2.old" ] && { rm -rf "$2"; mv "$2.old" "$2"; }; rm -rf "$4"; open "$2"; }"#;
    // `open` drops our environment; debug runs pass their sandbox through so the relaunch can't touch real data.
    let env = std::env::var("ADITUS_DATA_DIR").ok().filter(|_| cfg!(debug_assertions)).map(|d| format!("--env ADITUS_DATA_DIR={d} --env ADITUS_DEBUG_PORT={}", std::env::var("ADITUS_DEBUG_PORT").unwrap_or_default())).unwrap_or_default();
    Command::new("sh").args(["-c", &script.replace("open ", &format!("open {env} ")), "sh", &std::process::id().to_string()]).arg(&cur).arg(&new).arg(&dir).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn team_ids() {
        assert_eq!(super::team_id("Identifier=x\nTeamIdentifier=AB12CD34EF\n").as_deref(), Some("AB12CD34EF"));
        assert_eq!(super::team_id("TeamIdentifier=not set\n"), None);
        assert_eq!(super::team_id("Signature=adhoc\n"), None);
    }

    #[test]
    fn newer() {
        assert!(super::is_newer("0.2.0", "0.1.0"));
        assert!(super::is_newer("v0.10.0", "0.9.9"));
        assert!(!super::is_newer("0.1.0", "0.1.0"));
        assert!(!super::is_newer("0.0.9", "0.1.0"));
        assert!(!super::is_newer("nightly", "0.1.0"));
    }
}
