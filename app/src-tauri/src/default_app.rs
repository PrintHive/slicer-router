//! Whether Slicer Router opens `.3mf` files, and the platform's way to make it.
//!
//! macOS lets an app set the default for a file type, so the window offers a
//! button. Windows does not: the person picks it in Open with, so the window
//! selects a `.3mf` in Explorer for them.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use slicer_router_core::Slicer;
use slicer_router_macos::SetError;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// What opens `.3mf` files now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DefaultApp {
    pub is_default: bool,
    /// The app that opens them, which is Slicer Router when `is_default`.
    pub current_name: String,
}

const ROUTER_NAME: &str = "Slicer Router";

/// The type macOS gives `.3mf` once the app's Info.plist declares it.
const DECLARED_UTI: &str = "com.shapr3d.3d-manufacturing.3mf";

/// The file class the Windows installer registers for `.3mf`, from
/// `windows/hooks.nsh` and the file association name in `tauri.windows.conf.json`.
const WINDOWS_PROG_ID: &str = "PrintHive.SlicerRouter.3mf";

const EXAMPLE_FILE_NAME: &str = "Slicer Router example.3mf";

/// Who opens `.3mf` files, or `None` when the platform cannot say, so the window
/// shows nothing rather than a guess. `bundle_id` is this app's identifier.
pub fn status(bundle_id: &str) -> Option<DefaultApp> {
    mac_status(bundle_id)
        .or_else(|| windows_status(user_choice_prog_id().as_deref(), &own_exe_names()))
}

fn mac_status(bundle_id: &str) -> Option<DefaultApp> {
    let uti = slicer_router_macos::uti_for_extension("3mf")?;
    let handler = slicer_router_macos::default_handler(&uti)?;
    Some(mac_handler_status(
        bundle_id,
        &handler,
        slicer_router_macos::application_path,
    ))
}

/// Name the app with `handler` as its bundle id: Slicer Router, one of the
/// slicers, the name of the app LaunchServices finds, or the bundle id itself.
fn mac_handler_status(
    bundle_id: &str,
    handler: &str,
    app_path: impl Fn(&str) -> Option<PathBuf>,
) -> DefaultApp {
    if handler.trim().eq_ignore_ascii_case(bundle_id) {
        return default_app();
    }
    let name = Slicer::from_mac_bundle_id(handler)
        .map(|slicer| slicer.display_name().to_owned())
        .or_else(|| {
            let path = app_path(handler)?;
            Some(path.file_stem()?.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| handler.to_owned());
    DefaultApp {
        is_default: false,
        current_name: name,
    }
}

fn default_app() -> DefaultApp {
    DefaultApp {
        is_default: true,
        current_name: ROUTER_NAME.to_owned(),
    }
}

/// The file names this app's executable has: the one cargo builds, which the
/// installer ships, and the one actually running.
fn own_exe_names() -> Vec<String> {
    let mut names = vec![concat!(env!("CARGO_PKG_NAME"), ".exe").to_owned()];
    let running = std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.file_name()?.to_string_lossy().into_owned()));
    names.extend(running);
    names
}

/// The class Windows opens `.3mf` with, from `None` for no explicit choice.
///
/// Slicer Router is the default under the file class its installer registers, or
/// under `Applications\<exe>`, which is what Open with, Always writes when the
/// person picks the app from the list instead.
fn windows_status(prog_id: Option<&str>, own_exes: &[String]) -> Option<DefaultApp> {
    let prog_id = prog_id?;
    let picked_from_the_list = prog_id.split_once('\\').is_some_and(|(class, exe)| {
        class.eq_ignore_ascii_case("Applications")
            && own_exes.iter().any(|own| own.eq_ignore_ascii_case(exe))
    });
    if prog_id.eq_ignore_ascii_case(WINDOWS_PROG_ID) || picked_from_the_list {
        return Some(default_app());
    }
    Some(DefaultApp {
        is_default: false,
        current_name: windows_app_name(prog_id),
    })
}

/// A slicer's name when `prog_id` mentions it (`Applications\orca-slicer.exe`,
/// `BambuStudio.3mf`), otherwise "another app".
fn windows_app_name(prog_id: &str) -> String {
    let squash = |text: &str| {
        text.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
    };
    let prog_id = squash(prog_id);
    Slicer::ALL
        .iter()
        .find(|slicer| prog_id.contains(&squash(slicer.display_name())))
        .map_or("another app", |slicer| slicer.display_name())
        .to_owned()
}

/// The class the person chose for `.3mf` in Open with, if they chose one.
///
/// Newer Windows 10 and 11 builds keep it under `UserChoiceLatest` and leave
/// `UserChoice` stale, so that one is read first.
fn user_choice_prog_id() -> Option<String> {
    first_prog_id(read_prog_id("UserChoiceLatest"), read_prog_id("UserChoice"))
}

fn first_prog_id(latest: Option<String>, legacy: Option<String>) -> Option<String> {
    [latest, legacy]
        .into_iter()
        .flatten()
        .find(|prog_id| !prog_id.trim().is_empty())
}

#[cfg(windows)]
fn read_prog_id(key: &str) -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.3mf\{key}"),
            KEY_READ,
        )
        .ok()?
        .get_value("ProgId")
        .ok()
}

#[cfg(not(windows))]
fn read_prog_id(_key: &str) -> Option<String> {
    None
}

/// Make this app open `.3mf` files, on macOS. `bundle_id` is this app's identifier.
///
/// Sets the type macOS gives `.3mf` and the type this app declares for it, which
/// are the same once the app is registered but differ before. Checks the result,
/// because LaunchServices can accept a handler it does not then use.
pub fn make_default(bundle_id: &str) -> Result<(), String> {
    let mut types = vec![DECLARED_UTI.to_owned()];
    types.extend(slicer_router_macos::uti_for_extension("3mf"));
    types.dedup();
    for uti in &types {
        slicer_router_macos::set_default_handler(uti, bundle_id).map_err(set_error_message)?;
    }
    match mac_status(bundle_id) {
        Some(DefaultApp {
            is_default: true, ..
        }) => Ok(()),
        _ => Err(
            "macOS did not switch it. In Finder, select a .3mf, press Cmd+I, \
                  choose Slicer Router under Open with, then click Change All."
                .to_owned(),
        ),
    }
}

fn set_error_message(err: SetError) -> String {
    match err {
        // kLSApplicationNotFoundErr: the app is not registered with LaunchServices yet.
        SetError::Status(-10814) => "macOS does not know Slicer Router yet. \
                                     Copy it to Applications, open it from there, and try again."
            .to_owned(),
        SetError::Status(code) => format!("macOS refused to switch it (error {code})."),
        SetError::Unsupported => "Only macOS can switch it for you.".to_owned(),
    }
}

/// Select a small `.3mf` in Explorer, on Windows, so the person can right-click
/// it and choose Open with. The file is made once and reused.
pub fn reveal_example() -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Explorer is only on Windows.".to_owned());
    }
    let path = std::env::temp_dir().join(EXAMPLE_FILE_NAME);
    if !path.is_file() {
        let bytes = example_3mf().map_err(|err| err.to_string())?;
        std::fs::write(&path, bytes)
            .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    }
    reveal_in_explorer(&path)
}

/// `/select,` and the path are separate arguments so a path with spaces is quoted.
fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    Command::new("explorer")
        .arg("/select,")
        .arg(path)
        .spawn()
        .map(drop)
        .map_err(|err| format!("cannot start Explorer: {err}"))
}

/// A 3MF package holding a 20 mm cube. It carries no slicer metadata, so Slicer
/// Router opens it in the default slicer, which is what the person is testing.
fn example_3mf() -> zip::result::ZipResult<Vec<u8>> {
    const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#;
    const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
    const MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="20" y="0" z="0"/><vertex x="20" y="20" z="0"/><vertex x="0" y="20" z="0"/><vertex x="0" y="0" z="20"/><vertex x="20" y="0" z="20"/><vertex x="20" y="20" z="20"/><vertex x="0" y="20" z="20"/></vertices><triangles><triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="3" v3="2"/><triangle v1="4" v2="5" v3="6"/><triangle v1="4" v2="6" v3="7"/><triangle v1="0" v2="1" v3="5"/><triangle v1="0" v2="5" v3="4"/><triangle v1="1" v2="2" v3="6"/><triangle v1="1" v2="6" v3="5"/><triangle v1="2" v2="3" v3="7"/><triangle v1="2" v2="7" v3="6"/><triangle v1="3" v2="0" v3="4"/><triangle v1="3" v2="4" v3="7"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>"#;

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", RELS),
        ("3D/3dmodel.model", MODEL),
    ] {
        zip.start_file(name, SimpleFileOptions::default())?;
        zip.write_all(body.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN_ID: &str = "com.printhive.slicer-router";

    fn no_app(_: &str) -> Option<PathBuf> {
        None
    }

    #[test]
    fn this_app_as_the_mac_handler_is_the_default_whatever_the_case() {
        let status = mac_handler_status(OWN_ID, "Com.PrintHive.Slicer-Router", no_app);
        assert_eq!(
            status,
            DefaultApp {
                is_default: true,
                current_name: "Slicer Router".into()
            }
        );
    }

    #[test]
    fn another_mac_handler_is_named_as_a_slicer_then_by_its_app_then_by_its_id() {
        let name = |handler: &str, path: Option<&str>| {
            let status = mac_handler_status(OWN_ID, handler, |_| path.map(PathBuf::from));
            assert!(!status.is_default);
            status.current_name
        };
        assert_eq!(name("com.bambulab.bambu-studio", None), "Bambu Studio");
        assert_eq!(name("com.prusa3d.slic3r/", None), "PrusaSlicer");
        assert_eq!(
            name(
                "com.autodesk.fusion",
                Some("/Applications/Autodesk Fusion.app")
            ),
            "Autodesk Fusion"
        );
        assert_eq!(name("com.example.mystery", None), "com.example.mystery");
    }

    fn exes() -> Vec<String> {
        vec![
            "slicer-router-app.exe".to_owned(),
            "Slicer Router.exe".to_owned(),
        ]
    }

    #[test]
    fn windows_knows_slicer_router_by_its_file_class() {
        assert_eq!(
            windows_status(Some("PrintHive.SlicerRouter.3mf"), &exes()),
            Some(default_app())
        );
    }

    #[test]
    fn windows_knows_slicer_router_picked_from_the_open_with_list() {
        for prog_id in [
            r"Applications\slicer-router-app.exe",
            r"applications\Slicer-Router-App.EXE",
            r"Applications\Slicer Router.exe",
        ] {
            assert_eq!(
                windows_status(Some(prog_id), &exes()),
                Some(default_app()),
                "{prog_id}"
            );
        }
    }

    #[test]
    fn another_app_picked_from_the_list_is_not_slicer_router() {
        let status = windows_status(Some(r"Applications\slicer-router-app2.exe"), &exes());
        assert_eq!(status.map(|s| s.is_default), Some(false));
        assert!(own_exe_names().contains(&"slicer-router-app.exe".to_owned()));
    }

    #[test]
    fn windows_names_a_slicer_from_its_class_and_otherwise_says_another_app() {
        let name = |prog_id| windows_status(Some(prog_id), &exes()).unwrap().current_name;
        assert_eq!(name(r"Applications\orca-slicer.exe"), "OrcaSlicer");
        assert_eq!(name(r"Applications\bambu-studio.exe"), "Bambu Studio");
        assert_eq!(name("Snapmaker_Orca.3mf"), "Snapmaker Orca");
        assert_eq!(name("PrusaSlicer.3mf"), "PrusaSlicer");
        assert_eq!(name("AppX7z4x3hw3nb0b0aypgvgdhsksh4z7q7qk"), "another app");
        assert!(
            !windows_status(Some("Applications\\cura.exe"), &exes())
                .unwrap()
                .is_default
        );
    }

    #[test]
    fn windows_with_no_explicit_choice_says_nothing() {
        assert_eq!(windows_status(None, &exes()), None);
    }

    #[test]
    fn the_newer_user_choice_key_wins_and_the_older_one_is_the_fallback() {
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(first_prog_id(some("new"), some("old")), some("new"));
        assert_eq!(first_prog_id(None, some("old")), some("old"));
        assert_eq!(first_prog_id(some("  "), some("old")), some("old"));
        assert_eq!(first_prog_id(None, None), None);
    }

    #[test]
    fn a_missing_app_registration_is_explained_not_numbered() {
        assert!(set_error_message(SetError::Status(-10814)).contains("Copy it to Applications"));
        assert!(set_error_message(SetError::Status(-50)).contains("-50"));
    }

    #[test]
    fn the_example_is_a_readable_3mf_with_no_slicer_metadata() {
        let path =
            std::env::temp_dir().join(format!("slicer-router-example-{}.3mf", std::process::id()));
        std::fs::write(&path, example_3mf().unwrap()).unwrap();

        let verdict = slicer_router_core::detect(&path).unwrap();
        assert_eq!(verdict.slicer, None);
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut names: Vec<_> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["3D/3dmodel.model", "[Content_Types].xml", "_rels/.rels"]
        );
        std::fs::remove_file(path).unwrap();
    }
}
