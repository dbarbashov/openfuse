//! Independent filesystem scenarios; fixtures are original synthetic XML.
use openfuse_core::library::{PresetLibrary, ScanLimits};
use std::{fs, path::Path};
use tempfile::TempDir;

const XML: &str = r#"<?xml version="1.0"?><Preset ProductId="3"><FUSE><Info name="A &amp; B" author="Zoë" tags="clean, warm">Quiet &lt;tone&gt;</Info></FUSE><Unknown hello="preserve"/></Preset>"#;
fn write(path: impl AsRef<Path>) {
    fs::write(path, XML).unwrap();
}
fn setup() -> (TempDir, PresetLibrary) {
    let temp = TempDir::new().unwrap();
    let library = PresetLibrary::open(temp.path()).unwrap();
    (temp, library)
}

#[test]
fn scan_recurses_valid_formats_reports_failures_and_searches_metadata() {
    let (temp, library) = setup();
    fs::create_dir(temp.path().join("nested")).unwrap();
    write(temp.path().join("nested/preset.FUSE"));
    write(temp.path().join("second.xml"));
    fs::write(temp.path().join("not-a-preset.xml"), "<Settings/>").unwrap();
    fs::write(temp.path().join("broken.fuse"), "<Preset>").unwrap();
    fs::write(temp.path().join("not-utf8.xml"), [255]).unwrap();
    write(temp.path().join("ignored.txt"));
    let scan = library.scan().unwrap();
    assert_eq!(scan.entries.len(), 2);
    assert_eq!(scan.diagnostics.len(), 3);
    assert!(!scan.truncated);
    let entry = &scan.entries[0];
    assert_eq!(entry.product_id, Some(3));
    assert!(entry.matches("ZOË warm QUIET"));
    assert!(entry.matches("nested"));
    assert!(!entry.matches("distorted"));
    assert_eq!(entry.metadata.name, "A & B");
    assert_eq!(entry.fingerprint.len(), 64);
    assert_eq!(library.read(entry).unwrap().to_xml().unwrap(), XML);
}

#[test]
fn import_and_duplicate_preserve_xml_exactly_without_overwriting() {
    let (temp, library) = setup();
    let external = TempDir::new().unwrap();
    let source = external.path().join("A & B.fuse");
    write(&source);
    let first = library.import(&source).unwrap();
    let second = library.import(&source).unwrap();
    assert_ne!(first.path, second.path);
    let third = library.duplicate(&first).unwrap();
    let fourth = library.duplicate(&first).unwrap();
    assert_ne!(third.path, fourth.path);
    assert_eq!(first.metadata.name, "A & B");
    assert_eq!(third.metadata.name, "A & B");
    for entry in [first, second, third, fourth] {
        assert_eq!(fs::read_to_string(entry.path).unwrap(), XML);
    }
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 4);
    assert_eq!(fs::read_to_string(source).unwrap(), XML);
}

#[test]
fn archive_is_excluded_from_scan_and_survives_restart_with_collision_safe_restore() {
    let (temp, library) = setup();
    fs::create_dir(temp.path().join("nested")).unwrap();
    let source = temp.path().join("nested/preset.fuse");
    write(&source);
    let entry = library.scan().unwrap().entries.remove(0);
    let token = library.archive(&entry).unwrap();
    assert!(!source.exists());
    assert!(token.path.exists());
    assert!(library.scan().unwrap().entries.is_empty());
    drop(library);
    let library = PresetLibrary::open(temp.path()).unwrap();
    let persisted = library.archives().unwrap();
    assert_eq!(persisted.len(), 1);
    fs::write(&source, "external content must survive").unwrap();
    let restored = library.restore(&persisted[0]).unwrap();
    assert_ne!(restored.path, source);
    assert_eq!(restored.relative_path, Path::new("nested/preset-1.fuse"));
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "external content must survive"
    );
    assert_eq!(fs::read_to_string(restored.path).unwrap(), XML);
    assert!(token.path.exists()); // Recovery bytes are retained after restoration.
    assert!(library.archives().unwrap().is_empty());
    assert!(library.restore(&persisted[0]).is_err());
}

#[test]
fn stale_content_refuses_read_duplicate_and_archive() {
    let (temp, library) = setup();
    let source = temp.path().join("preset.xml");
    write(&source);
    let entry = library.scan().unwrap().entries.remove(0);
    fs::write(&source, XML.replace("Quiet", "Loud ")).unwrap();
    assert!(library.read(&entry).is_err());
    assert!(library.duplicate(&entry).is_err());
    assert!(library.archive(&entry).is_err());
    assert!(source.exists());
    assert_eq!(library.scan().unwrap().entries.len(), 1);
}

#[cfg(unix)]
#[test]
fn replacing_file_with_identical_bytes_invalidates_its_identity() {
    let (temp, library) = setup();
    let source = temp.path().join("preset.xml");
    write(&source);
    let entry = library.scan().unwrap().entries.remove(0);
    fs::rename(&source, temp.path().join("kept.txt")).unwrap();
    write(&source);
    assert!(library.archive(&entry).is_err());
    assert!(library.duplicate(&entry).is_err());
}

#[test]
fn scan_budgets_entry_count_depth_and_total_bytes() {
    let (temp, library) = setup();
    fs::create_dir(temp.path().join("nested")).unwrap();
    write(temp.path().join("nested/a.fuse"));
    write(temp.path().join("b.fuse"));
    write(temp.path().join("c.fuse"));
    let scan = library
        .scan_with_limits(ScanLimits {
            max_entries: 0,
            max_depth: 16,
            max_total_bytes: 99999,
        })
        .unwrap();
    assert!(scan.truncated);
    assert!(scan.entries.is_empty());
    let scan = library
        .scan_with_limits(ScanLimits {
            max_entries: 100,
            max_depth: 0,
            max_total_bytes: 99999,
        })
        .unwrap();
    assert!(scan.truncated);
    assert_eq!(scan.entries.len(), 2);
    let scan = library
        .scan_with_limits(ScanLimits {
            max_entries: 100,
            max_depth: 16,
            max_total_bytes: XML.len() as u64,
        })
        .unwrap();
    assert!(scan.truncated);
    assert_eq!(scan.entries.len(), 1);
}

#[test]
fn oversized_presets_do_not_abort_scan_or_publish_imports() {
    let (temp, library) = setup();
    let source = temp.path().join("huge.fuse");
    fs::write(
        &source,
        vec![b' '; openfuse_core::preset::MAX_PRESET_BYTES + 1],
    )
    .unwrap();
    write(temp.path().join("good.fuse"));
    let scan = library.scan().unwrap();
    assert_eq!(scan.entries.len(), 1);
    assert_eq!(scan.diagnostics.len(), 1);
    assert!(library.import(&source).is_err());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}

#[test]
fn invalid_imports_and_wrong_library_entries_do_not_mutate_destination() {
    let (_temp, library) = setup();
    let (other, second) = setup();
    let source = other.path().join("a.fuse");
    write(&source);
    let entry = second.scan().unwrap().entries.remove(0);
    assert!(library.read(&entry).is_err());
    assert!(library.duplicate(&entry).is_err());
    assert!(library.archive(&entry).is_err());
    fs::write(&source, "<NotPreset/>").unwrap();
    assert!(library.import(&source).is_err());
    assert!(library.scan().unwrap().entries.is_empty());
}

#[test]
fn archive_tampering_and_traversal_tokens_fail_closed() {
    let (temp, library) = setup();
    write(temp.path().join("a.fuse"));
    let entry = library.scan().unwrap().entries.remove(0);
    let token = library.archive(&entry).unwrap();
    let mut escaped = token.clone();
    escaped.original_relative_path = "../escape.fuse".into();
    assert!(library.restore(&escaped).is_err());
    let mut escaped = token.clone();
    escaped.path = temp.path().join(".confuse-trash/../a.fuse");
    assert!(library.restore(&escaped).is_err());
    fs::write(&token.path, XML.replace("Quiet", "Changed")).unwrap();
    assert!(library.restore(&token).is_err());
    assert!(token.path.exists());
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_scanned_imported_or_followed_after_snapshot() {
    use std::os::unix::fs::symlink;
    let (temp, library) = setup();
    let external = TempDir::new().unwrap();
    let outside = external.path().join("outside.fuse");
    write(&outside);
    symlink(external.path(), temp.path().join("linked-directory")).unwrap();
    symlink(&outside, temp.path().join("linked.fuse")).unwrap();
    assert!(library.scan().unwrap().entries.is_empty());
    assert!(library.import(temp.path().join("linked.fuse")).is_err());
    assert!(PresetLibrary::open(temp.path().join("linked-directory")).is_err());
    fs::create_dir(temp.path().join("nested")).unwrap();
    write(temp.path().join("nested/a.fuse"));
    let entry = library.scan().unwrap().entries.remove(0);
    fs::remove_dir_all(temp.path().join("nested")).unwrap();
    symlink(external.path(), temp.path().join("nested")).unwrap();
    assert!(library.read(&entry).is_err());
    assert!(library.archive(&entry).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), XML);
}

#[cfg(unix)]
#[test]
fn symlink_collisions_are_not_overwritten_and_archive_symlink_is_refused() {
    use std::os::unix::fs::symlink;
    let (temp, library) = setup();
    let external = TempDir::new().unwrap();
    let outside = external.path().join("a.fuse");
    write(&outside);
    symlink(&outside, temp.path().join("a.fuse")).unwrap();
    let imported = library.import(&outside).unwrap();
    assert_eq!(imported.relative_path, Path::new("a-1.fuse"));
    symlink(external.path(), temp.path().join(".confuse-trash")).unwrap();
    assert!(library.archive(&imported).is_err());
    assert!(imported.path.exists());
    assert!(
        fs::symlink_metadata(temp.path().join("a.fuse"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn concurrent_imports_never_clobber_or_expose_partial_xml() {
    let (temp, library) = setup();
    let external = TempDir::new().unwrap();
    let source = external.path().join("a.fuse");
    write(&source);
    let handles: Vec<_> = (0..12)
        .map(|_| {
            let library = library.clone();
            let source = source.clone();
            std::thread::spawn(move || library.import(source).unwrap())
        })
        .collect();
    for handle in handles {
        assert_eq!(
            fs::read_to_string(handle.join().unwrap().path).unwrap(),
            XML
        );
    }
    assert_eq!(library.scan().unwrap().entries.len(), 12);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 12);
}

#[test]
fn moved_or_replaced_root_cannot_return_misleading_entry_paths() {
    let outer = TempDir::new().unwrap();
    let root = outer.path().join("library");
    fs::create_dir(&root).unwrap();
    write(root.join("a.fuse"));
    let library = PresetLibrary::open(&root).unwrap();
    let entry = library.scan().unwrap().entries.remove(0);
    fs::rename(&root, outer.path().join("moved")).unwrap();
    fs::create_dir(&root).unwrap();
    write(root.join("a.fuse"));
    assert!(library.scan().is_err());
    assert!(library.read(&entry).is_err());
    assert!(library.duplicate(&entry).is_err());
    assert!(library.archive(&entry).is_err());
    assert!(library.archives().is_err());
}

#[test]
fn archived_presets_remain_restorable_after_moving_the_library() {
    let outer = TempDir::new().unwrap();
    let first = outer.path().join("first");
    fs::create_dir(&first).unwrap();
    write(first.join("a.fuse"));
    let library = PresetLibrary::open(&first).unwrap();
    let entry = library.scan().unwrap().entries.remove(0);
    library.archive(&entry).unwrap();
    drop(library);
    let second = outer.path().join("second");
    fs::rename(first, &second).unwrap();
    let library = PresetLibrary::open(&second).unwrap();
    let token = library.archives().unwrap().remove(0);
    assert!(token.path.starts_with(library.root()));
    let restored = library.restore(&token).unwrap();
    assert_eq!(fs::read_to_string(restored.path).unwrap(), XML);
}

#[cfg(unix)]
#[test]
fn archive_manifest_symlinks_and_non_regular_files_are_refused() {
    use std::os::unix::fs::symlink;
    let (temp, library) = setup();
    write(temp.path().join("a.fuse"));
    let entry = library.scan().unwrap().entries.remove(0);
    let token = library.archive(&entry).unwrap();
    let manifest = token.path.parent().unwrap().join("record.json");
    let original = fs::read(&manifest).unwrap();
    fs::remove_file(&manifest).unwrap();
    fs::create_dir(&manifest).unwrap();
    assert!(library.archives().is_err());
    fs::remove_dir(&manifest).unwrap();
    fs::write(temp.path().join("outside.json"), original).unwrap();
    symlink(temp.path().join("outside.json"), &manifest).unwrap();
    assert!(library.archives().is_err());
}
