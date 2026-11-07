//! Read the signals that identify the authoring slicer, and the preview image,
//! out of a `.3mf` archive.
//!
//! A `.3mf` is a zip file and can run past 100 MB, so this module opens only the
//! zip directory plus the few small entries it needs, and reads each of them
//! through a size cap.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zip::ZipArchive;

const MODEL_ENTRY: &str = "3D/3dmodel.model";
const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";
const PRUSA_CONFIG_ENTRY: &str = "Metadata/Slic3r_PE.config";
const PACKAGE_RELATIONSHIPS_ENTRY: &str = "_rels/.rels";
/// The Open Packaging Conventions relationship type for a package thumbnail.
const THUMBNAIL_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail";
/// Where slicers store the preview when the package names none, or names one it
/// left out: Bambu Studio and the Orca family, then PrusaSlicer.
const THUMBNAIL_FALLBACKS: [&str; 2] = ["Metadata/plate_1.png", "Metadata/thumbnail.png"];

/// The `<metadata>` block sits at the top of the model XML, ahead of any geometry.
const MODEL_HEAD_LIMIT: u64 = 1 << 20;
/// Project settings grow with the filament count; 16 MiB is far past any real file.
const CONFIG_LIMIT: u64 = 16 << 20;
/// `_rels/.rels` lists a handful of relationships.
const RELATIONSHIPS_LIMIT: u64 = 1 << 20;
/// Plate previews and MakerWorld covers are a few hundred KiB.
const THUMBNAIL_LIMIT: u64 = 4 << 20;

/// Why a `.3mf` could not be examined at all.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DetectError {
    #[error("cannot read {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    /// `source` is boxed so the zip library's types stay out of this crate's API.
    #[error("{} is not a valid .3mf (zip) archive: {source}", path.display())]
    NotAZip {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Printer identity recorded in the project's settings.
///
/// Each field is the trimmed, non-empty string from the config file. Config
/// values that are one-element arrays contribute their first element.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PrinterProfile {
    /// `printer_model`, for example `Snapmaker U1` or `MK4S`.
    pub model: Option<String>,
    /// `printer_settings_id`, the name of the printer preset.
    pub settings_id: Option<String>,
    /// `inherits`. Kept for debugging only: it often names a process or
    /// filament preset, so no detection rule reads it.
    pub inherits: Option<String>,
    /// `printer_vendor`, or the `PRINTER_VENDOR_<NAME>` marker in PrusaSlicer's
    /// `printer_notes`, when either is present.
    pub vendor: Option<String>,
}

/// Every signal read from a `.3mf`, before any rule interprets it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Evidence {
    /// `<metadata name="Application">` from `3D/3dmodel.model`, for example
    /// `BambuStudio-02.08.02.61`.
    pub application: Option<String>,
    /// Printer identity from `Metadata/project_settings.config` (Bambu Studio and
    /// the Orca family) or `Metadata/Slic3r_PE.config` (PrusaSlicer).
    pub printer: PrinterProfile,
    /// Whether the archive has PrusaSlicer's `Metadata/Slic3r_PE.config`.
    pub prusa_config_present: bool,
    /// Entries that were present but unreadable. They add no signal and do not
    /// fail detection.
    pub warnings: Vec<String>,
}

/// Read the detection signals from the `.3mf` at `path`.
///
/// Fails only when the file is not a readable zip archive. A zip with no model
/// or no settings yields an empty [`Evidence`].
pub fn read_evidence(path: &Path) -> Result<Evidence, DetectError> {
    let mut archive = open_archive(path)?;
    let mut evidence = Evidence::default();

    if let Some(name) = find_entry(&archive, MODEL_ENTRY) {
        match archive.by_name(&name) {
            Ok(entry) => match read_application(BufReader::new(entry.take(MODEL_HEAD_LIMIT))) {
                Ok(application) => evidence.application = application,
                Err(err) => evidence
                    .warnings
                    .push(format!("{MODEL_ENTRY}: unreadable XML ({err})")),
            },
            Err(err) => evidence
                .warnings
                .push(format!("{MODEL_ENTRY}: cannot open ({err})")),
        }
    }

    if let Some(name) = find_entry(&archive, PROJECT_SETTINGS_ENTRY) {
        match archive.by_name(&name) {
            Ok(entry) => match read_json_printer(BufReader::new(entry.take(CONFIG_LIMIT))) {
                Ok(printer) => evidence.printer = printer,
                Err(err) => evidence
                    .warnings
                    .push(format!("{PROJECT_SETTINGS_ENTRY}: unreadable JSON ({err})")),
            },
            Err(err) => evidence
                .warnings
                .push(format!("{PROJECT_SETTINGS_ENTRY}: cannot open ({err})")),
        }
    }

    if let Some(name) = find_entry(&archive, PRUSA_CONFIG_ENTRY) {
        evidence.prusa_config_present = true;
        if evidence.printer == PrinterProfile::default() {
            match archive.by_name(&name) {
                Ok(entry) => {
                    evidence.printer = read_ini_printer(BufReader::new(entry.take(CONFIG_LIMIT)));
                }
                Err(err) => evidence
                    .warnings
                    .push(format!("{PRUSA_CONFIG_ENTRY}: cannot open ({err})")),
            }
        }
    }

    Ok(evidence)
}

/// The preview image the `.3mf` at `path` carries: PNG or JPEG bytes, as stored.
///
/// The package thumbnail named in `_rels/.rels` comes first. Slicers point it at
/// a plate preview, and MakerWorld downloads at the model's cover. Then come
/// `Metadata/plate_1.png` and `Metadata/thumbnail.png`, because some exports name
/// a thumbnail they never wrote. An entry over 4 MiB, or one that is not a PNG
/// or JPEG, is skipped.
///
/// Fails only when the file is not a readable zip archive. A zip without a
/// preview yields `None`.
pub fn read_thumbnail(path: &Path) -> Result<Option<Vec<u8>>, DetectError> {
    let mut archive = open_archive(path)?;
    let named = read_capped(
        &mut archive,
        PACKAGE_RELATIONSHIPS_ENTRY,
        RELATIONSHIPS_LIMIT,
    )
    .and_then(|rels| thumbnail_target(&rels));
    let candidates = named
        .into_iter()
        .chain(THUMBNAIL_FALLBACKS.map(str::to_owned));
    Ok(candidates
        .filter_map(|name| read_capped(&mut archive, &name, THUMBNAIL_LIMIT))
        .find(|bytes| is_png_or_jpeg(bytes)))
}

fn open_archive(path: &Path) -> Result<ZipArchive<BufReader<File>>, DetectError> {
    let io_err = |source| DetectError::Io {
        path: path.to_owned(),
        source,
    };
    let file = File::open(path).map_err(io_err)?;
    ZipArchive::new(BufReader::new(file)).map_err(|source| match source {
        zip::result::ZipError::Io(source) => io_err(source),
        source => DetectError::NotAZip {
            path: path.to_owned(),
            source: Box::new(source),
        },
    })
}

/// The whole entry `wanted`, or `None` when it is missing, unreadable or over
/// `limit` bytes. The size is checked before reading and again while reading,
/// because the zip directory can understate it.
fn read_capped<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    wanted: &str,
    limit: u64,
) -> Option<Vec<u8>> {
    let name = find_entry(archive, wanted)?;
    let entry = archive.by_name(&name).ok()?;
    if entry.size() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    entry.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// The entry named by the package thumbnail relationship in `_rels/.rels`.
fn thumbnail_target(rels: &[u8]) -> Option<String> {
    let mut xml = Reader::from_reader(rels);
    let mut buf = Vec::new();
    loop {
        match xml.read_event_into(&mut buf).ok()? {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == "Relationship" =>
            {
                let attribute = |name: &str| {
                    let value = element.try_get_attribute(name).ok()??;
                    value
                        .normalized_value(XmlVersion::Implicit1_0)
                        .ok()
                        .map(Cow::into_owned)
                };
                let is_thumbnail = attribute("Type")
                    .is_some_and(|kind| kind.trim().eq_ignore_ascii_case(THUMBNAIL_RELATIONSHIP));
                if let Some(target) = attribute("Target").filter(|_| is_thumbnail) {
                    return non_empty(target.trim_start_matches('/'));
                }
            }
            Event::Eof => return None,
            _ => {}
        }
        buf.clear();
    }
}

fn is_png_or_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

/// Exact entry name for `wanted`, tolerating case and a leading slash.
fn find_entry<R: Read + io::Seek>(archive: &ZipArchive<R>, wanted: &str) -> Option<String> {
    archive
        .file_names()
        .flatten()
        .find(|name| name.trim_start_matches('/').eq_ignore_ascii_case(wanted))
        .map(Cow::into_owned)
}

/// The `Application` metadata value, scanning only until the geometry starts.
///
/// The value arrives as several events when it holds an entity (`PrusaSlicer&#45;2.9.0`
/// is text, a reference, then more text), so pieces are joined until the end tag.
fn read_application(reader: impl BufRead) -> Result<Option<String>, quick_xml::Error> {
    let mut xml = Reader::from_reader(reader);
    let mut buf = Vec::new();
    let mut value: Option<String> = None;
    loop {
        match xml.read_event_into(&mut buf)? {
            Event::Start(start) if start.local_name().as_ref() == "metadata" => {
                let is_application = start
                    .attributes()
                    .flatten()
                    .any(|a| a.key.as_ref() == "name" && a.value == "Application");
                value = is_application.then(String::new);
            }
            Event::Start(start) | Event::Empty(start)
                if matches!(start.local_name().as_ref(), "resources" | "build") =>
            {
                return Ok(None);
            }
            Event::Text(text) => {
                if let Some(value) = &mut value {
                    value.push_str(&text.xml10_content());
                }
            }
            Event::CData(data) => {
                if let Some(value) = &mut value {
                    value.push_str(&data.xml10_content());
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(value) = &mut value {
                    if let Ok(Some(c)) = reference.resolve_char_ref() {
                        value.push(c);
                    } else if let Some(text) = resolve_predefined_entity(&reference) {
                        value.push_str(text);
                    }
                }
            }
            Event::End(_) => {
                if let Some(value) = value.take() {
                    return Ok(non_empty(&value));
                }
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
        buf.clear();
    }
}

/// Only the keys detection reads; serde skips everything else in the file.
#[derive(Deserialize)]
struct ProjectSettings {
    printer_model: Option<Value>,
    printer_settings_id: Option<Value>,
    inherits: Option<Value>,
    printer_vendor: Option<Value>,
}

fn read_json_printer(reader: impl Read) -> Result<PrinterProfile, serde_json::Error> {
    let settings: ProjectSettings = serde_json::from_reader(reader)?;
    let scalar = |value: Option<Value>| match value? {
        Value::String(s) => non_empty(&s),
        Value::Array(items) => items.first().and_then(Value::as_str).and_then(non_empty),
        _ => None,
    };
    Ok(PrinterProfile {
        model: scalar(settings.printer_model),
        settings_id: scalar(settings.printer_settings_id),
        inherits: scalar(settings.inherits),
        vendor: scalar(settings.printer_vendor),
    })
}

/// PrusaSlicer stores its config as `; key = value` comment lines.
///
/// Its `printer_vendor` is usually empty, so the vendor falls back to the
/// `PRINTER_VENDOR_<NAME>` marker that Prusa printer presets put in `printer_notes`.
fn read_ini_printer(reader: impl BufRead) -> PrinterProfile {
    let mut printer = PrinterProfile::default();
    let mut notes = None;
    for line in reader.split(b'\n').map_while(Result::ok) {
        let line = String::from_utf8_lossy(&line);
        let Some((key, value)) = line
            .trim()
            .strip_prefix(';')
            .and_then(|l| l.split_once('='))
        else {
            continue;
        };
        let slot = match key.trim() {
            "printer_model" => &mut printer.model,
            "printer_settings_id" => &mut printer.settings_id,
            "printer_vendor" => &mut printer.vendor,
            "printer_notes" => &mut notes,
            _ => continue,
        };
        if slot.is_none() {
            *slot = non_empty(value);
        }
    }
    if printer.vendor.is_none() {
        printer.vendor = notes.as_deref().and_then(vendor_marker);
    }
    printer
}

fn vendor_marker(notes: &str) -> Option<String> {
    const MARKER: &str = "PRINTER_VENDOR_";
    let start = notes.find(MARKER)? + MARKER.len();
    let name: String = notes[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn application_is_read_before_geometry() {
        let xml = r#"<?xml version="1.0"?>
<model unit="millimeter"><metadata name="Title">x</metadata>
<metadata name="Application">BambuStudio-02.08.02.61</metadata>
<resources><object id="1"/></resources></model>"#;
        assert_eq!(
            read_application(Cursor::new(xml)).unwrap().as_deref(),
            Some("BambuStudio-02.08.02.61")
        );
    }

    #[test]
    fn application_split_by_entities_is_joined() {
        let xml = r#"<model><metadata name="Application">PrusaSlicer&#45;2.9.0</metadata><resources/></model>"#;
        assert_eq!(
            read_application(Cursor::new(xml)).unwrap().as_deref(),
            Some("PrusaSlicer-2.9.0")
        );
        let xml = r#"<model><metadata name="Application">A&amp;B &#x2D; <![CDATA[C]]></metadata><resources/></model>"#;
        assert_eq!(
            read_application(Cursor::new(xml)).unwrap().as_deref(),
            Some("A&B - C")
        );
    }

    #[test]
    fn empty_application_tag_has_no_value() {
        let xml = r#"<model><metadata name="Application"></metadata><resources/></model>"#;
        assert_eq!(read_application(Cursor::new(xml)).unwrap(), None);
    }

    #[test]
    fn application_after_geometry_is_ignored() {
        let xml = r#"<model><resources/><metadata name="Application">Late-1</metadata></model>"#;
        assert_eq!(read_application(Cursor::new(xml)).unwrap(), None);
    }

    #[test]
    fn ini_takes_first_value_of_each_key() {
        let ini = "; generated by PrusaSlicer\n; printer_model = MK4S\n; printer_settings_id = Original Prusa MK4S 0.4 nozzle\n; printer_model = XL\n; printer_vendor =\n";
        let printer = read_ini_printer(Cursor::new(ini));
        assert_eq!(printer.model.as_deref(), Some("MK4S"));
        assert_eq!(
            printer.settings_id.as_deref(),
            Some("Original Prusa MK4S 0.4 nozzle")
        );
        assert_eq!(printer.vendor, None);
    }

    #[test]
    fn ini_vendor_falls_back_to_the_notes_marker() {
        let ini = "; printer_notes = Don't remove these keywords.\\nPRINTER_VENDOR_PRUSA3D\\nPRINTER_MODEL_MK3\\n\n; printer_vendor =\n";
        let printer = read_ini_printer(Cursor::new(ini));
        assert_eq!(printer.vendor.as_deref(), Some("PRUSA3D"));
    }

    #[test]
    fn json_accepts_strings_and_one_element_arrays() {
        let json = r#"{"printer_model":["Snapmaker J1"],"printer_settings_id":"  ","inherits":"x","big":[1,2,3]}"#;
        let printer = read_json_printer(Cursor::new(json)).unwrap();
        assert_eq!(printer.model.as_deref(), Some("Snapmaker J1"));
        assert_eq!(printer.settings_id, None);
        assert_eq!(printer.inherits.as_deref(), Some("x"));
    }
}
