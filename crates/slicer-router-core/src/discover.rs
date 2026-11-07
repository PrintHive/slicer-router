//! Find installed slicers by reading the registry table.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::slicer::{Slicer, SlicerSpec};

/// One slicer found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledSlicer {
    pub slicer: Slicer,
    /// The `.app` bundle on macOS, the executable on Windows.
    pub path: PathBuf,
}

/// The installed slicers, at most one per [`Slicer`], in [`Slicer::ALL`] order.
///
/// macOS: apps in `/Applications`, `~/Applications` and one folder level below
/// them whose `CFBundleIdentifier` is in the registry. When several bundles share
/// an identifier (PrusaSlicer's stable and alpha builds do) the shortest bundle
/// name wins, which is the stock unversioned install.
///
/// Windows: the `App Paths` and `Uninstall` registry keys, then the usual
/// `Program Files` folders.
///
/// Other platforms: nothing is found.
pub fn discover_installed() -> Vec<InstalledSlicer> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut dirs = vec![PathBuf::from("/Applications")];
        dirs.extend(home.map(|home| home.join("Applications")));
        scan_mac_apps(&dirs)
    } else if cfg!(windows) {
        discover_windows(|name| std::env::var_os(name))
    } else {
        Vec::new()
    }
}

/// Scan `dirs` (and each folder directly inside them) for slicer `.app` bundles.
fn scan_mac_apps(dirs: &[PathBuf]) -> Vec<InstalledSlicer> {
    let mut found = Vec::new();
    for dir in dirs {
        for entry in sorted_entries(dir) {
            if is_app_bundle(&entry) {
                found.extend(identify_bundle(&entry));
            } else if entry.is_dir() {
                for nested in sorted_entries(&entry)
                    .into_iter()
                    .filter(|p| is_app_bundle(p))
                {
                    found.extend(identify_bundle(&nested));
                }
            }
        }
    }
    best_per_slicer(found)
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    entries
}

fn is_app_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "app") && path.is_dir()
}

fn identify_bundle(app: &Path) -> Option<InstalledSlicer> {
    let mut xml = String::new();
    File::open(app.join("Contents/Info.plist"))
        .ok()?
        .take(1 << 20)
        .read_to_string(&mut xml)
        .ok()?;
    let id = bundle_identifier(&xml)?;
    let id = id.trim().trim_end_matches('/');
    Slicer::ALL
        .into_iter()
        .find(|slicer| {
            slicer
                .spec()
                .mac_bundle_ids
                .iter()
                .any(|known| known.eq_ignore_ascii_case(id))
        })
        .map(|slicer| InstalledSlicer {
            slicer,
            path: app.to_owned(),
        })
}

/// `CFBundleIdentifier` from an XML property list.
fn bundle_identifier(xml: &str) -> Option<String> {
    #[derive(PartialEq)]
    enum In {
        Key,
        Value,
        Other,
    }
    let mut reader = Reader::from_str(xml);
    let mut inside = In::Other;
    let mut last_key = String::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(start) => {
                inside = match start.name().as_ref() {
                    "key" => In::Key,
                    "string" => In::Value,
                    _ => In::Other,
                }
            }
            Event::Text(text) => {
                let text = text.xml10_content();
                match inside {
                    In::Key => last_key = text.trim().to_owned(),
                    In::Value if last_key == "CFBundleIdentifier" => {
                        return Some(text.trim().to_owned())
                    }
                    _ => {}
                }
            }
            Event::End(_) => inside = In::Other,
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// Keep one install per slicer: the shortest file name, then the first path.
fn best_per_slicer(mut found: Vec<InstalledSlicer>) -> Vec<InstalledSlicer> {
    found.sort_by_key(|f| {
        let name_len = f.path.file_name().map_or(0, |n| n.len());
        (name_len, f.path.clone())
    });
    Slicer::ALL
        .into_iter()
        .filter_map(|slicer| found.iter().find(|f| f.slicer == slicer).cloned())
        .collect()
}

/// Find slicers on Windows from `env` (for `Program Files`), the registry and disk.
fn discover_windows(env: impl Fn(&str) -> Option<OsString>) -> Vec<InstalledSlicer> {
    let roots = program_roots(&env);
    let found = Slicer::ALL
        .into_iter()
        .filter_map(|slicer| {
            let spec = slicer.spec();
            let mut candidates = registry_candidates(spec);
            candidates.extend(program_dirs(spec, &roots));
            find_exe(&candidates, spec.windows.exe_names)
                .map(|path| InstalledSlicer { slicer, path })
        })
        .collect();
    best_per_slicer(found)
}

/// Folders that hold per-machine and per-user installs.
fn program_roots(env: &impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(env)
        .map(PathBuf::from)
        .collect();
    roots.extend(env("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs")));
    roots.dedup();
    roots
}

fn program_dirs(spec: &SlicerSpec, roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .flat_map(|root| {
            spec.windows
                .install_dirs
                .iter()
                .map(move |dir| root.join(dir))
        })
        .collect()
}

/// The first executable among `candidates`. A candidate that is a file is the
/// executable itself; a folder is searched for each of `exe_names`.
fn find_exe(candidates: &[PathBuf], exe_names: &[&str]) -> Option<PathBuf> {
    candidates.iter().find_map(|candidate| {
        if candidate.is_file() {
            return Some(candidate.clone());
        }
        exe_names
            .iter()
            .map(|exe| candidate.join(exe))
            .find(|path| path.is_file())
    })
}

/// Executables and install folders that the Windows registry records for `spec`.
#[cfg(windows)]
fn registry_candidates(spec: &SlicerSpec) -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    const UNINSTALL: [&str; 2] = [
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ];

    let mut candidates = Vec::new();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let hive = RegKey::predef(hive);
        for exe in spec.windows.exe_names {
            let key = hive.open_subkey_with_flags(format!(r"{APP_PATHS}\{exe}"), KEY_READ);
            if let Ok(path) = key.and_then(|key| key.get_value::<String, _>("")) {
                candidates.push(PathBuf::from(path.trim_matches('"')));
            }
        }
        for uninstall in UNINSTALL {
            let Ok(root) = hive.open_subkey_with_flags(uninstall, KEY_READ) else {
                continue;
            };
            for name in root.enum_keys().flatten() {
                let Ok(app) = root.open_subkey_with_flags(&name, KEY_READ) else {
                    continue;
                };
                let display: String = app.get_value("DisplayName").unwrap_or_default();
                let matches = spec
                    .windows
                    .uninstall_display_names
                    .iter()
                    .any(|prefix| display.to_lowercase().starts_with(&prefix.to_lowercase()));
                if let (true, Ok(dir)) = (matches, app.get_value::<String, _>("InstallLocation")) {
                    candidates.push(PathBuf::from(dir.trim_matches('"')));
                }
            }
        }
    }
    candidates
}

#[cfg(not(windows))]
fn registry_candidates(_spec: &SlicerSpec) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fake_app(dir: &Path, name: &str, bundle_id: &str) -> PathBuf {
        let app = dir.join(name);
        fs::create_dir_all(app.join("Contents")).unwrap();
        fs::write(
            app.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\"><dict>\n<key>CFBundleName</key><string>x</string>\n\
                 <key>CFBundleIdentifier</key><string>{bundle_id}</string>\n</dict></plist>\n"
            ),
        )
        .unwrap();
        app
    }

    #[test]
    fn bundle_ids_match_trimmed_and_ignoring_case() {
        let dir = TempDir::new().unwrap();
        fake_app(dir.path(), "Bambu.app", "com.bambulab.bambu-studio");
        fake_app(dir.path(), "Prusa.app", "com.prusa3d.slic3r/");
        fake_app(dir.path(), "Snap.app", "COM.Snapmaker.Snapmaker-Orca");
        fake_app(dir.path(), "Other.app", "com.example.other");
        let found = scan_mac_apps(&[dir.path().to_owned()]);
        let slicers: Vec<_> = found.iter().map(|f| f.slicer).collect();
        assert_eq!(
            slicers,
            [
                Slicer::BambuStudio,
                Slicer::SnapmakerOrca,
                Slicer::PrusaSlicer
            ]
        );
    }

    #[test]
    fn shortest_bundle_name_wins_among_duplicates() {
        let dir = TempDir::new().unwrap();
        fake_app(
            dir.path(),
            "PrusaSlicer-3.0.0-alpha11.app",
            "com.prusa3d.slic3r/",
        );
        let stock = fake_app(dir.path(), "PrusaSlicer.app", "com.prusa3d.slic3r/");
        let found = scan_mac_apps(&[dir.path().to_owned()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, stock);
    }

    #[test]
    fn apps_one_folder_down_are_found() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("3D Printing");
        fs::create_dir(&nested).unwrap();
        let app = fake_app(&nested, "OrcaSlicer.app", "com.softfever3d.orca-slicer");
        let found = scan_mac_apps(&[dir.path().to_owned(), dir.path().join("missing")]);
        assert_eq!(
            found,
            [InstalledSlicer {
                slicer: Slicer::OrcaSlicer,
                path: app
            }]
        );
    }

    #[test]
    fn windows_finds_executables_in_program_files_folders() {
        let dir = TempDir::new().unwrap();
        let pf = dir.path().join("Program Files");
        let orca = pf.join("OrcaSlicer");
        fs::create_dir_all(&orca).unwrap();
        fs::write(orca.join("orca-slicer.exe"), b"").unwrap();
        let prusa = pf.join("Prusa3D").join("PrusaSlicer");
        fs::create_dir_all(&prusa).unwrap();
        fs::write(prusa.join("prusa-slicer.exe"), b"").unwrap();
        // A folder without the executable is not an install.
        fs::create_dir_all(pf.join("Bambu Studio")).unwrap();

        let pf_value: OsString = pf.clone().into();
        let found = discover_windows(|name| (name == "ProgramFiles").then(|| pf_value.clone()));
        assert_eq!(
            found,
            [
                InstalledSlicer {
                    slicer: Slicer::OrcaSlicer,
                    path: orca.join("orca-slicer.exe")
                },
                InstalledSlicer {
                    slicer: Slicer::PrusaSlicer,
                    path: prusa.join("prusa-slicer.exe")
                },
            ]
        );
    }

    #[test]
    fn a_registry_candidate_that_is_a_file_is_the_executable() {
        let dir = TempDir::new().unwrap();
        let exe = dir.path().join("custom-name.exe");
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            find_exe(std::slice::from_ref(&exe), &["bambu-studio.exe"]),
            Some(exe)
        );
    }
}
