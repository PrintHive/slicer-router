//! Each installed slicer's own icon, read from its app when the window asks.
//!
//! The icons belong to the slicers' authors, so none ships in this repository.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// The icon of the app at `app` as a 128 px PNG data URL.
///
/// macOS only: the bundle's `CFBundleIconFile` is converted with `sips`. `None`
/// elsewhere, and when the bundle names no icon file or it cannot be converted,
/// so the window falls back to a monogram.
pub fn png_data_url(app: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let icns = app
        .join("Contents/Resources")
        .join(icon_file_name(&bundle_icon_file(app)?));
    let png = icns_to_png(&icns)?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

/// `CFBundleIconFile` is written with or without the `.icns` extension.
fn icon_file_name(value: &str) -> String {
    let value = value.trim();
    if value.to_ascii_lowercase().ends_with(".icns") {
        value.to_owned()
    } else {
        format!("{value}.icns")
    }
}

/// `plutil` reads XML and binary property lists alike.
fn bundle_icon_file(app: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleIconFile", "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .ok()?;
    let value = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !value.trim().is_empty()).then_some(value)
}

fn icns_to_png(icns: &Path) -> Option<Vec<u8>> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let out: PathBuf = std::env::temp_dir().join(format!(
        "slicer-router-icon-{}-{}.png",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let status = Command::new("/usr/bin/sips")
        .args(["-s", "format", "png", "-Z", "128"])
        .arg(icns)
        .arg("--out")
        .arg(&out)
        .output()
        .ok()?
        .status;
    let png = status.success().then(|| std::fs::read(&out).ok()).flatten();
    let _ = std::fs::remove_file(&out);
    png
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icns_extension_is_added_only_when_missing() {
        assert_eq!(icon_file_name("AppIcon"), "AppIcon.icns");
        assert_eq!(icon_file_name("Icon.icns\n"), "Icon.icns");
        assert_eq!(icon_file_name("Shouty.ICNS"), "Shouty.ICNS");
    }

    #[test]
    fn an_app_that_is_not_there_has_no_icon() {
        assert_eq!(
            png_data_url(Path::new("/slicer-router-test/none.app")),
            None
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_icon_comes_back_as_a_png_data_url() {
        let app = std::env::temp_dir().join(format!(
            "slicer-router-icon-test-{}.app",
            std::process::id()
        ));
        let resources = app.join("Contents/Resources");
        std::fs::create_dir_all(&resources).unwrap();
        let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/icon.icns");
        std::fs::copy(icon, resources.join("AppIcon.icns")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIconFile</key><string>AppIcon</string></dict></plist>
"#,
        )
        .unwrap();

        let url = png_data_url(&app).expect("the bundle has an icon");
        let encoded = url.strip_prefix("data:image/png;base64,").unwrap();
        assert!(STANDARD.decode(encoded).unwrap().starts_with(b"\x89PNG"));
        std::fs::remove_dir_all(app).unwrap();
    }
}
