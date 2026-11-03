//! Update detection against the GitHub releases API, plus a self-replacing
//! install for the macOS app bundle.

use serde::Deserialize;
use std::path::PathBuf;

const REPO: &str = "drosendo/convertx-native";
const USER_AGENT: &str = "ConvertX-native-updater";

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    /// Without the leading `v`, e.g. `0.2.0`.
    pub version: String,
    pub title: String,
    pub notes: String,
    pub page_url: String,
    pub download_url: Option<String>,
    pub download_name: Option<String>,
}

impl ReleaseInfo {
    /// "Download 11 MB app" style label for the button.
    pub fn action_label(&self) -> &'static str {
        if can_self_update() && self.download_url.is_some() {
            "Install and restart"
        } else {
            "Open download page"
        }
    }
}

/// Parses `1.2.3` into a comparable tuple, ignoring any suffix (`-beta`, `+sha`).
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let trimmed = text.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next().unwrap_or(trimmed);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// True when `candidate` is a newer version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn platform_asset_name(assets: &[Asset]) -> Option<&Asset> {
    let matcher: fn(&str) -> bool = if cfg!(target_os = "macos") {
        |name| name.contains("macos-arm64") && name.ends_with(".zip")
    } else if cfg!(target_os = "windows") {
        |name| name.contains("windows") && name.ends_with(".zip")
    } else if cfg!(target_os = "android") {
        |name| name.ends_with(".apk")
    } else {
        |name| name.contains("linux") && name.ends_with(".tar.gz")
    };
    assets.iter().find(|asset| matcher(&asset.name))
}

fn parse_release(json: &str) -> Result<ReleaseInfo, String> {
    let release: Release =
        serde_json::from_str(json).map_err(|err| format!("Unexpected API response: {err}"))?;
    let asset = platform_asset_name(&release.assets);

    Ok(ReleaseInfo {
        version: release.tag_name.trim_start_matches('v').to_string(),
        title: if release.name.is_empty() {
            release.tag_name.clone()
        } else {
            release.name
        },
        notes: release.body,
        page_url: if release.html_url.is_empty() {
            format!("https://github.com/{REPO}/releases")
        } else {
            release.html_url
        },
        download_url: asset.map(|asset| asset.browser_download_url.clone()),
        download_name: asset.map(|asset| asset.name.clone()),
    })
}

/// Asks GitHub for the latest release. `Ok(None)` means "already up to date".
pub fn check() -> Result<Option<ReleaseInfo>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let response = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(12)))
        .build()
        .call()
        .map_err(|err| match err {
            ureq::Error::StatusCode(404) => "No published release yet".to_string(),
            other => format!("Could not reach GitHub: {other}"),
        })?;

    let body = response
        .into_body()
        .read_to_string()
        .map_err(|err| format!("Could not read the response: {err}"))?;

    let release = parse_release(&body)?;
    if is_newer(&release.version, CURRENT_VERSION) {
        Ok(Some(release))
    } else {
        Ok(None)
    }
}

/// The bundle this process runs from, on macOS: `…/ConvertX.app`.
pub fn app_bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos_dir = exe.parent()?; // …/Contents/MacOS
    let contents = macos_dir.parent()?; // …/Contents
    if macos_dir.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    if bundle.extension()? != "app" {
        return None;
    }
    Some(bundle.to_path_buf())
}

/// Only the macOS bundle can replace itself; other platforms open the page.
pub fn can_self_update() -> bool {
    cfg!(target_os = "macos") && app_bundle_path().is_some()
}

/// Downloads the release asset and hands it to a helper script that swaps the
/// bundle once this process exits. The caller must quit afterwards.
pub fn install(release: &ReleaseInfo) -> Result<(), String> {
    let Some(url) = release.download_url.clone() else {
        return Err("This release has no download for your platform".into());
    };
    let bundle = app_bundle_path().ok_or("Self-update only works from the ConvertX.app bundle")?;

    let staging = std::env::temp_dir().join(format!("convertx-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|err| format!("No temp folder: {err}"))?;
    let archive = staging.join("update.zip");

    let response = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .build()
        .call()
        .map_err(|err| format!("Download failed: {err}"))?;
    let mut file = std::fs::File::create(&archive).map_err(|err| format!("Cannot write: {err}"))?;
    std::io::copy(&mut response.into_body().into_reader(), &mut file)
        .map_err(|err| format!("Download interrupted: {err}"))?;
    drop(file);

    // Unpack first: if the archive is broken, nothing has been replaced yet.
    let unpacked = staging.join("unpacked");
    let status = std::process::Command::new("ditto")
        .args(["-x", "-k"])
        .arg(&archive)
        .arg(&unpacked)
        .status()
        .map_err(|err| format!("Cannot unpack the update: {err}"))?;
    if !status.success() || !unpacked.join("ConvertX.app").exists() {
        return Err("The downloaded update did not contain ConvertX.app".into());
    }

    let script = staging.join("apply-update.sh");
    let pid = std::process::id();
    let body = format!(
        r#"#!/bin/bash
set -e
# Wait for the running app to exit before touching the bundle.
while kill -0 {pid} 2>/dev/null; do sleep 0.4; done
rm -rf "{bundle}.old"
if [ -d "{bundle}" ]; then mv "{bundle}" "{bundle}.old"; fi
cp -R "{source}" "{bundle}"
xattr -dr com.apple.quarantine "{bundle}" 2>/dev/null || true
rm -rf "{bundle}.old"
open "{bundle}"
rm -rf "{staging}"
"#,
        bundle = bundle.display(),
        source = unpacked.join("ConvertX.app").display(),
        staging = staging.display(),
    );
    std::fs::write(&script, body).map_err(|err| format!("Cannot write the updater: {err}"))?;

    std::process::Command::new("/bin/bash")
        .arg(&script)
        .spawn()
        .map_err(|err| format!("Cannot start the updater: {err}"))?;

    Ok(())
}

/// Opens the release page in the user's browser.
pub fn open_page(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert!(is_newer("0.1.1-beta.1", "0.1.0"));
    }

    #[test]
    fn parses_a_github_release_payload() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "name": "ConvertX 0.2.0",
            "body": "- faster",
            "html_url": "https://github.com/drosendo/convertx-native/releases/tag/v0.2.0",
            "assets": [
                {"name": "convertx-0.2.0-linux-x64.tar.gz", "browser_download_url": "https://example.org/linux"},
                {"name": "ConvertX-0.2.0-macos-arm64.zip", "browser_download_url": "https://example.org/mac"},
                {"name": "convertx-0.2.0-windows-x64.zip", "browser_download_url": "https://example.org/win"}
            ]
        }"#;
        let release = parse_release(json).expect("parse");
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.title, "ConvertX 0.2.0");
        assert!(release.notes.contains("faster"));
        // The chosen asset depends on the platform running the test.
        assert!(release.download_url.is_some());
        assert!(release.download_name.is_some());
    }

    #[test]
    fn missing_optional_fields_do_not_break_parsing() {
        let release = parse_release(r#"{"tag_name": "v9.9.9"}"#).expect("parse");
        assert_eq!(release.version, "9.9.9");
        assert_eq!(
            release.page_url,
            "https://github.com/drosendo/convertx-native/releases"
        );
        assert!(release.download_url.is_none());
    }
}
