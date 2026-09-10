//! Extracts a real, displayable PNG icon from a `.app` bundle, for the
//! result-card thumbnail — see http.rs's `/api/thumbnail` and
//! service.rs's `read_indexed_file`.
//!
//! macOS app icons are `.icns` files (Apple's own multi-resolution icon
//! format), which no browser can render directly via `<img>`. `Info.plist`
//! names the icon via `CFBundleIconFile` (or, for apps built against a
//! modern Xcode asset catalog instead of a standalone `.icns`,
//! `CFBundleIconName` — extracting from a compiled `Assets.car` catalog is
//! a much larger undertaking than this justifies, so that case is left
//! unsupported for now and simply fails this one thumbnail request rather
//! than the whole app).
//!
//! Uses `plutil` and `sips` — both are stock macOS command-line tools, not
//! new dependencies. This entire module is macOS-only, matching `.app`
//! bundles themselves.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

pub fn extract_icon_png(app_bundle: &Path) -> Result<Vec<u8>> {
    let info_plist = app_bundle.join("Contents/Info.plist");
    let icon_stem = read_icon_file_name(&info_plist)
        .with_context(|| format!("no CFBundleIconFile in {}", info_plist.display()))?;

    let resources = app_bundle.join("Contents/Resources");
    let icns_path = {
        let with_ext = resources.join(&icon_stem);
        if with_ext.extension().is_some() && with_ext.exists() {
            with_ext
        } else {
            let with_icns = resources.join(format!("{icon_stem}.icns"));
            if with_icns.exists() {
                with_icns
            } else {
                bail!("icon file {icon_stem} not found under {}", resources.display());
            }
        }
    };

    let tmp_out = std::env::temp_dir().join(format!("deepscan-appicon-{}.png", std::process::id()));
    let status = Command::new("sips")
        .args(["-s", "format", "png", icns_path.to_str().unwrap_or_default(), "--out"])
        .arg(&tmp_out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run sips to convert .icns to .png")?;
    if !status.success() {
        bail!("sips failed converting {}", icns_path.display());
    }

    let bytes = std::fs::read(&tmp_out)?;
    let _ = std::fs::remove_file(&tmp_out);
    Ok(bytes)
}

/// Runs `plutil -convert json -o -` (stock macOS tool) rather than parsing
/// Info.plist ourselves — it's just as often binary-encoded as XML, and
/// this avoids a dedicated plist-parsing dependency for one string field.
fn read_icon_file_name(info_plist: &Path) -> Result<String> {
    let output = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(info_plist)
        .output()
        .context("failed to run plutil")?;
    if !output.status.success() {
        bail!("plutil failed on {}", info_plist.display());
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let name = json
        .get("CFBundleIconFile")
        .and_then(|v| v.as_str())
        .or_else(|| json.get("CFBundleIconName").and_then(|v| v.as_str()))
        .context("Info.plist has neither CFBundleIconFile nor CFBundleIconName")?;
    // CFBundleIconFile conventionally omits the .icns extension, but isn't
    // required to — strip it here so the caller can uniformly re-append it.
    Ok(name.strip_suffix(".icns").unwrap_or(name).to_string())
}
