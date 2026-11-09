//! The C ABI that the macOS Finder thumbnail extension links.
//!
//! The Swift extension passes the file Finder asked about and the user's home
//! folder, under which the app keeps the routing snapshot. It gets back what to
//! draw: the preview the file carries and the app whose icon badges it. The
//! routing itself lives in `slicer-router-core`. `include/slicer_router.h`
//! declares these functions for Swift.

use std::ffi::{c_char, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;

use slicer_router_core::{detect, read_thumbnail, Snapshot, Verdict};

/// What to draw for one file. Release it with [`sr_thumbnail_free`].
#[repr(C)]
pub struct SrThumbnail {
    /// The `.app` of the slicer that will open the file, or null when Slicer
    /// Router would ask which slicer to use. Its icon is the badge.
    pub opener_app: *mut c_char,
    /// The preview image the file carries, PNG or JPEG, or null.
    pub image: *mut u8,
    pub image_len: usize,
}

impl SrThumbnail {
    fn new(opener_app: Option<PathBuf>, image: Option<Vec<u8>>) -> Self {
        let opener_app = opener_app
            .and_then(|path| CString::new(path.into_os_string().into_encoded_bytes()).ok())
            .map_or(ptr::null_mut(), CString::into_raw);
        let (image, image_len) = match image {
            Some(bytes) => {
                let bytes = Box::into_raw(bytes.into_boxed_slice());
                (bytes.cast::<u8>(), bytes.len())
            }
            None => (ptr::null_mut(), 0),
        };
        SrThumbnail {
            opener_app,
            image,
            image_len,
        }
    }
}

/// Decide what to draw for `file`, routing it with the snapshot the app saved
/// under `home`, the user's real home folder. A null `home`, or no snapshot,
/// gives no opener. A file that is not a zip routes like one with no slicer
/// metadata, as it does when opened.
///
/// # Safety
///
/// `file`, and `home` unless it is null, must point to NUL-terminated strings
/// that stay valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sr_thumbnail(file: *const c_char, home: *const c_char) -> SrThumbnail {
    // SAFETY: the caller guarantees both are null or valid C strings.
    let (file, home) = unsafe { (path_arg(file), path_arg(home)) };
    match file {
        Some(file) => {
            // A panic must not unwind into Swift; fall back to the plain card.
            let (opener_app, image) =
                std::panic::catch_unwind(|| thumbnail(file, home)).unwrap_or((None, None));
            SrThumbnail::new(opener_app, image)
        }
        None => SrThumbnail::new(None, None),
    }
}

/// Release what [`sr_thumbnail`] returned.
///
/// # Safety
///
/// `thumbnail` must come from [`sr_thumbnail`], unmodified, and be released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sr_thumbnail_free(thumbnail: SrThumbnail) {
    if !thumbnail.opener_app.is_null() {
        // SAFETY: made by `CString::into_raw` in `SrThumbnail::new`.
        drop(unsafe { CString::from_raw(thumbnail.opener_app) });
    }
    if !thumbnail.image.is_null() {
        let bytes = ptr::slice_from_raw_parts_mut(thumbnail.image, thumbnail.image_len);
        // SAFETY: made by `Box::into_raw` on a boxed slice of `image_len` bytes.
        drop(unsafe { Box::from_raw(bytes) });
    }
}

/// The opener's `.app` and the preview image for `file`.
fn thumbnail(file: &Path, home: Option<&Path>) -> (Option<PathBuf>, Option<Vec<u8>>) {
    let verdict = detect(file).unwrap_or_else(|_| Verdict::no_signal());
    let opener_app = home
        .and_then(|home| Snapshot::load(&Snapshot::dir_for_home(home)))
        .and_then(|snapshot| snapshot.opener(&verdict).map(|slicer| slicer.path.clone()));
    let image = read_thumbnail(file).ok().flatten();
    (opener_app, image)
}

/// # Safety
///
/// `arg` must be null or a NUL-terminated string that stays valid for `'a`.
unsafe fn path_arg<'a>(arg: *const c_char) -> Option<&'a Path> {
    if arg.is_null() {
        return None;
    }
    // SAFETY: not null, and the caller guarantees the rest.
    let text = unsafe { CStr::from_ptr(arg) }.to_str().ok()?;
    Some(Path::new(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slicer_router_core::{Config, InstalledSlicer, Slicer};
    use tempfile::TempDir;

    fn c_path(path: &Path) -> CString {
        CString::new(path.to_str().unwrap()).unwrap()
    }

    fn fixture(name: &str) -> CString {
        c_path(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures")
                .join(name),
        )
    }

    /// A home folder holding a snapshot as the app saves it, with each slicer at
    /// `/Apps/<id>.app`.
    fn home_with_snapshot(installed: &[Slicer], config: &Config) -> TempDir {
        let home = TempDir::new().unwrap();
        let apps = installed
            .iter()
            .map(|&slicer| {
                InstalledSlicer::new(slicer, format!("/Apps/{}.app", slicer.id()).into())
            })
            .collect();
        Snapshot::new(apps, config)
            .save(&Snapshot::dir_for_home(home.path()))
            .unwrap();
        home
    }

    fn not_a_zip(dir: &TempDir) -> CString {
        let file = dir.path().join("junk.3mf");
        std::fs::write(&file, b"not a zip").unwrap();
        c_path(&file)
    }

    /// Call the C ABI and copy out what it returned, releasing it.
    fn draw(file: &CString, home: Option<&TempDir>) -> (Option<String>, Option<Vec<u8>>) {
        let home = home.map(|home| c_path(home.path()));
        let home_ptr = home.as_ref().map_or(ptr::null(), |home| home.as_ptr());
        // SAFETY: both arguments are live C strings or null.
        let thumbnail = unsafe { sr_thumbnail(file.as_ptr(), home_ptr) };
        let opener = (!thumbnail.opener_app.is_null()).then(|| {
            // SAFETY: a C string owned by `thumbnail` until it is freed below.
            let opener = unsafe { CStr::from_ptr(thumbnail.opener_app) };
            opener.to_str().unwrap().to_owned()
        });
        let image = (!thumbnail.image.is_null()).then(|| {
            // SAFETY: `image_len` bytes owned by `thumbnail` until it is freed below.
            unsafe { std::slice::from_raw_parts(thumbnail.image, thumbnail.image_len) }.to_vec()
        });
        // SAFETY: returned by `sr_thumbnail` above and freed once.
        unsafe { sr_thumbnail_free(thumbnail) };
        (opener, image)
    }

    #[test]
    fn a_bambu_project_is_badged_with_bambu_studio_and_keeps_its_preview() {
        let home = home_with_snapshot(
            &[Slicer::BambuStudio, Slicer::PrusaSlicer],
            &Config::default(),
        );
        let (opener, image) = draw(&fixture("real-bambu-studio-cli.3mf"), Some(&home));
        assert_eq!(opener.as_deref(), Some("/Apps/bambu-studio.app"));
        assert!(image.unwrap().starts_with(b"\x89PNG"));
    }

    #[test]
    fn without_a_snapshot_there_is_no_opener_but_still_the_preview() {
        let empty_home = TempDir::new().unwrap();
        for home in [None, Some(&empty_home)] {
            let (opener, image) = draw(&fixture("real-bambu-studio-cli.3mf"), home);
            assert_eq!(opener, None);
            assert!(image.is_some());
        }
    }

    #[test]
    fn a_file_the_router_would_ask_about_has_no_opener() {
        let home = home_with_snapshot(
            &[Slicer::OrcaSlicer, Slicer::PrusaSlicer],
            &Config::default(),
        );
        assert_eq!(draw(&not_a_zip(&home), Some(&home)), (None, None));
    }

    #[test]
    fn the_default_slicer_badges_a_file_with_no_signal() {
        let mut config = Config::default();
        config.default_slicer = Some(Slicer::SnapmakerOrca);
        let home = home_with_snapshot(&[Slicer::BambuStudio, Slicer::SnapmakerOrca], &config);
        let (opener, _) = draw(&not_a_zip(&home), Some(&home));
        assert_eq!(opener.as_deref(), Some("/Apps/snapmaker-orca.app"));
    }

    #[test]
    fn the_extension_is_entitled_to_read_the_snapshot_folder() {
        let entitlements = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../app/macos/ThumbnailExtension/SlicerRouterThumbnail.entitlements");
        let text = std::fs::read_to_string(entitlements).unwrap();
        let folder = Snapshot::dir_for_home(Path::new("/"));
        let folder = format!("<string>{}/</string>", folder.display());
        assert!(
            text.contains("home-relative-path.read-only") && text.contains(&folder),
            "the extension's entitlements must grant read access to {folder}"
        );
    }
}
