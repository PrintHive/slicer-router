// Each test crate compiles its own copy of the builder and uses only part of it.
#[allow(dead_code)]
mod common;

use common::ThreeMf;
use slicer_router_core::{read_thumbnail, DetectError};
use tempfile::TempDir;

fn png(tag: &str) -> Vec<u8> {
    [b"\x89PNG\r\n\x1a\n".as_slice(), tag.as_bytes()].concat()
}

fn relationships(thumbnail: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n \
         <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel-1\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n \
         <Relationship Target=\"{thumbnail}\" Id=\"rel-2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail\"/>\n\
         </Relationships>\n"
    )
    .into_bytes()
}

#[test]
fn the_plate_preview_wins_over_the_cover_the_package_names() {
    let dir = TempDir::new().unwrap();
    let file = ThreeMf::new()
        .model(None)
        .entry(
            "_rels/.rels",
            relationships("/Auxiliaries/.thumbnails/thumbnail_3mf.png"),
        )
        .entry("Metadata/plate_1.png", png("plate"))
        .entry("Auxiliaries/.thumbnails/thumbnail_3mf.png", png("cover"))
        .write(dir.path(), "makerworld.3mf");
    assert_eq!(read_thumbnail(&file).unwrap(), Some(png("plate")));
}

#[test]
fn prusaslicer_thumbnail_is_found_without_relationships() {
    let dir = TempDir::new().unwrap();
    let file = ThreeMf::new()
        .model(Some("PrusaSlicer-2.9.0"))
        .entry("Metadata/thumbnail.png", png("prusa"))
        .write(dir.path(), "prusa.3mf");
    assert_eq!(read_thumbnail(&file).unwrap(), Some(png("prusa")));
}

#[test]
fn the_package_thumbnail_is_used_when_no_known_preview_exists() {
    let dir = TempDir::new().unwrap();
    let file = ThreeMf::new()
        .model(None)
        .entry("_rels/.rels", relationships("/Metadata/plate_7.png"))
        .entry("Metadata/plate_7.png", png("plate 7"))
        .write(dir.path(), "plate-7.3mf");
    assert_eq!(read_thumbnail(&file).unwrap(), Some(png("plate 7")));

    let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    let file = ThreeMf::new()
        .entry("_rels/.rels", relationships("/Metadata/cover.jpg"))
        .entry("Metadata/cover.jpg", jpeg.clone())
        .write(dir.path(), "jpeg.3mf");
    assert_eq!(read_thumbnail(&file).unwrap(), Some(jpeg));
}

#[test]
fn an_entry_that_is_not_an_image_is_skipped() {
    let dir = TempDir::new().unwrap();
    let file = ThreeMf::new()
        .entry("Metadata/plate_1.png", b"not an image".to_vec())
        .entry("Metadata/thumbnail.png", png("prusa"))
        .write(dir.path(), "junk-plate.3mf");
    assert_eq!(read_thumbnail(&file).unwrap(), Some(png("prusa")));

    let file = ThreeMf::new()
        .entry("_rels/.rels", relationships("/Metadata/notes.txt"))
        .entry("Metadata/notes.txt", b"not an image".to_vec())
        .write(dir.path(), "text.3mf");
    assert_eq!(read_thumbnail(&file).unwrap(), None);
}

#[test]
fn a_preview_over_4_mib_is_skipped() {
    let dir = TempDir::new().unwrap();
    let mut huge = png("huge");
    huge.resize((4 << 20) + 1, 0);
    let file = ThreeMf::new()
        .stored_entry("Metadata/plate_1.png", huge)
        .entry("Metadata/thumbnail.png", png("small"))
        .write(dir.path(), "huge.3mf");
    assert_eq!(read_thumbnail(&file).unwrap(), Some(png("small")));
}

#[test]
fn a_file_without_a_preview_has_none() {
    let dir = TempDir::new().unwrap();
    let file = ThreeMf::new()
        .model(None)
        .entry("_rels/.rels", relationships("/Metadata/plate_1.png"))
        .write(dir.path(), "plain.3mf");
    assert_eq!(read_thumbnail(&file).unwrap(), None);
}

#[test]
fn a_file_that_is_not_a_zip_is_an_error() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("junk.3mf");
    std::fs::write(&file, b"junk").unwrap();
    assert!(matches!(
        read_thumbnail(&file),
        Err(DetectError::NotAZip { .. })
    ));
}
