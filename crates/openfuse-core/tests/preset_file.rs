use openfuse_core::{preset::PresetDocument, preset_file};
use std::fs;
fn document() -> PresetDocument {
    PresetDocument::parse("<Preset ProductId=\"13\"><FUSE><Name>Copy</Name></FUSE></Preset>")
        .unwrap()
}

#[test]
fn source_reads_are_bounded_raw_and_do_not_create_missing_directories() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("legacy.fuse");
    fs::write(&source, "deliberately broken &amp; XML").unwrap();
    assert_eq!(
        preset_file::read(&source).unwrap(),
        "deliberately broken &amp; XML"
    );
    assert!(preset_file::read(root.path().join("missing/source.fuse")).is_err());
    assert!(!root.path().join("missing").exists());
    fs::write(&source, [255]).unwrap();
    assert!(preset_file::read(&source).is_err());
    fs::write(&source, vec![b' '; preset_file::MAX_BYTES as usize + 1]).unwrap();
    assert!(preset_file::read(&source).is_err());
}
#[test]
fn new_copy_publication_never_replaces_existing_file_or_hardlink() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("new.fuse");
    let doc = document();
    preset_file::write_new(&doc, &path).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(preset_file::read(&path).unwrap(), doc.to_xml().unwrap());
    assert!(preset_file::write_new(&doc, &path).is_err());
    let link = root.path().join("alias.xml");
    fs::hard_link(&path, &link).unwrap();
    assert!(preset_file::write_new(&doc, &link).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(preset_file::write_new(&doc, root.path().join("bad.json")).is_err());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
#[cfg(unix)]
#[test]
fn symbolic_input_output_and_parent_links_are_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let source = outside.path().join("original.fuse");
    preset_file::write_new(&document(), &source).unwrap();
    let alias = root.path().join("alias.fuse");
    symlink(&source, &alias).unwrap();
    assert!(preset_file::read(&alias).is_err());
    assert!(preset_file::write_new(&document(), &alias).is_err());
    let parent = root.path().join("directory");
    symlink(outside.path(), &parent).unwrap();
    assert!(preset_file::read(parent.join("original.fuse")).is_err());
    assert!(preset_file::write_new(&document(), parent.join("copy.fuse")).is_err());
    assert!(!outside.path().join("copy.fuse").exists());
}
