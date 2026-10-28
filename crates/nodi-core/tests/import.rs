mod support;

use std::fs;

use nodi_core::import::{self, Planned};
use support::TempDir;

#[test]
fn text_encodings_are_detected() {
    assert_eq!(import::decode_text(b"\xEF\xBB\xBFhalo \xC3\xA4"), "halo ä");
    assert_eq!(import::decode_text(b"\xFF\xFEh\0i\0"), "hi");
    assert_eq!(import::decode_text(b"\xFE\xFF\0h\0i"), "hi");
    assert_eq!(import::decode_text(b"h\0a\0l\0o\0"), "halo");
    assert_eq!(import::decode_text("café".as_bytes()), "café");
    // Not UTF-8: Windows-1252 ("ANSI").
    assert_eq!(import::decode_text(b"caf\xE9 \x93ok\x94"), "café “ok”");
}

#[test]
fn folder_plan_keeps_structure_and_reports_other_files() {
    let tmp = TempDir::new("import-plan");
    let root = tmp.path().join("Kuliah");
    fs::create_dir_all(root.join("Bab 1")).unwrap();
    fs::create_dir_all(root.join("kosong")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join("b.md"), "# B").unwrap();
    fs::write(root.join("a.txt"), "A").unwrap();
    fs::write(root.join("foto.jpg"), "x").unwrap();
    fs::write(root.join("Bab 1").join("isi.rtf"), "{\\rtf1 x}").unwrap();

    let plan = import::plan(std::slice::from_ref(&root));
    assert_eq!(plan.note_count(), 3);
    let Planned::Folder { title, children } = &plan.items[0] else {
        panic!("expected a folder: {:?}", plan.items);
    };
    assert_eq!(title, "Kuliah");
    let names: Vec<String> = children
        .iter()
        .map(|child| match child {
            Planned::Note { title, .. } | Planned::Folder { title, .. } => title.clone(),
        })
        .collect();
    assert_eq!(names, vec!["a", "b", "Bab 1"]);
    assert_eq!(plan.skipped.len(), 1, "{:?}", plan.skipped);
    assert!(plan.skipped[0].0.ends_with("foto.jpg"));
}

#[test]
fn markdown_pictures_are_embedded_only_from_inside_the_folder() {
    let tmp = TempDir::new("import-pictures");
    let dir = tmp.path().join("notes");
    fs::create_dir_all(dir.join("img")).unwrap();
    let png = nodi_core::math::encode_png(4, 2, &[0x88, 0xFF].repeat(8), "");
    fs::write(dir.join("img").join("a.png"), &png).unwrap();
    fs::write(tmp.path().join("secret.png"), &png).unwrap();
    fs::write(
        dir.join("note.md"),
        "# Judul\n\n![a](img/a.png) dan ![b](../secret.png) dan ![c](https://x.test/c.png)\n",
    )
    .unwrap();
    let (rtf, text) = import::convert_file(&dir.join("note.md"), 9000).unwrap();
    let rtf = String::from_utf8(rtf).unwrap();
    assert!(text.contains("# Judul"));
    assert_eq!(rtf.matches("\\pngblip").count(), 1, "{rtf}");
    assert!(rtf.contains("HYPERLINK \"https://x.test/c.png\""), "{rtf}");
    assert!(!rtf.contains('\u{E002}'));
}

#[test]
fn plain_text_becomes_paragraphs() {
    let rtf = import::text_to_rtf("satu\r\ndua {x}\\\tä");
    assert!(rtf.starts_with("{\\rtf1"));
    assert!(
        rtf.contains("satu\\par\ndua \\{x\\}\\\\\\tab \\u228?\\par}"),
        "{rtf}"
    );
}
