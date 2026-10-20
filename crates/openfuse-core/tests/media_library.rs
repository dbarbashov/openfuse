use openfuse_core::media_library::{MAX_MEDIA_BYTES, MediaFormat, MediaLibrary};
use std::fs;
#[test]
fn imported_copy_survives_original_changes_and_catalog_reopen() {
    let source = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = source.path().join("My Jam.WAV");
    fs::write(&path, b"original audio inventory").unwrap();
    let library = MediaLibrary::open(root.path()).unwrap();
    let entry = library.import_file(&path).unwrap();
    assert_eq!(entry.name, "My Jam.WAV");
    assert_eq!(entry.format, MediaFormat::Wav);
    assert!(entry.matches_query("jam wav"));
    assert_eq!(library.import_file(&path).unwrap().id, entry.id);
    fs::write(&path, b"changed").unwrap();
    let reopened = MediaLibrary::open(root.path()).unwrap();
    assert_eq!(reopened.read(&entry).unwrap(), b"original audio inventory");
    assert_eq!(reopened.scan().unwrap().entries.len(), 1);
    assert_ne!(reopened.import_file(&path).unwrap().id, entry.id);
}
#[test]
fn deletion_and_restore_keep_both_copies_and_refuse_stale_actions() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("jam.mp3");
    fs::write(&path, b"mp3 bytes").unwrap();
    let library = MediaLibrary::open(root.path()).unwrap();
    let entry = library.import_file(&path).unwrap();
    let deleted = library.delete(&entry).unwrap();
    assert!(deleted.trashed);
    assert!(library.scan().unwrap().entries.is_empty());
    assert_eq!(library.trash().unwrap().entries.len(), 1);
    assert!(library.delete(&entry).is_err());
    assert!(library.read(&entry).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"mp3 bytes");
    assert!(entry.path.exists());
    let restored = library.restore(&deleted).unwrap();
    assert!(!restored.trashed);
    assert_eq!(library.read(&restored).unwrap(), b"mp3 bytes");
    assert!(library.restore(&deleted).is_err());
}
#[test]
fn corruption_and_forged_entry_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("song.mid");
    fs::write(&path, b"MThd").unwrap();
    let library = MediaLibrary::open(root.path()).unwrap();
    let entry = library.import_file(&path).unwrap();
    assert_eq!(entry.format, MediaFormat::Midi);
    let mut forged = entry.clone();
    forged.id = "../escape".into();
    assert!(library.read(&forged).is_err());
    let other = tempfile::tempdir().unwrap();
    assert!(
        MediaLibrary::open(other.path())
            .unwrap()
            .read(&entry)
            .is_err()
    );
    fs::write(&entry.path, b"tampered").unwrap();
    assert!(library.read(&entry).is_err());
    assert!(library.import_file(&path).is_err());
    fs::write(
        root.path().join(format!("{}.media-deleted", entry.id)),
        b"bad marker",
    )
    .unwrap();
    let scan = library.scan().unwrap();
    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 1);
}
#[test]
fn source_limits_and_unknown_formats_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let library = MediaLibrary::open(root.path()).unwrap();
    let missing = source.path().join("missing/track.wav");
    assert!(library.import_file(&missing).is_err());
    assert!(!missing.parent().unwrap().exists());
    let unsupported = source.path().join("track.flac");
    fs::write(&unsupported, b"data").unwrap();
    assert!(library.import_file(&unsupported).is_err());
    let empty = source.path().join("empty.wav");
    fs::write(&empty, b"").unwrap();
    assert!(library.import_file(&empty).is_err());
    let big = source.path().join("large.wav");
    fs::File::create(&big)
        .unwrap()
        .set_len(MAX_MEDIA_BYTES + 1)
        .unwrap();
    assert!(library.import_file(&big).is_err());
    assert!(library.scan().unwrap().entries.is_empty());
}
#[cfg(unix)]
#[test]
fn symlinks_and_replaced_root_are_rejected() {
    use std::os::unix::fs::symlink;
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("library");
    fs::create_dir(&root).unwrap();
    let source = base.path().join("source.wav");
    fs::write(&source, b"audio").unwrap();
    let alias = base.path().join("alias.wav");
    symlink(&source, &alias).unwrap();
    let library = MediaLibrary::open(&root).unwrap();
    assert!(library.import_file(&alias).is_err());
    let entry = library.import_file(&source).unwrap();
    fs::remove_file(&entry.path).unwrap();
    symlink(&source, &entry.path).unwrap();
    assert!(library.read(&entry).is_err());
    fs::rename(&root, base.path().join("old")).unwrap();
    fs::create_dir(&root).unwrap();
    assert!(library.scan().is_err());
    assert!(library.import_file(&source).is_err());
}
#[test]
fn folder_import_is_flat_bounded_and_reports_partial_success() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir(source.path().join("nested")).unwrap();
    std::fs::write(source.path().join("nested/hidden.wav"), b"hidden").unwrap();
    std::fs::write(source.path().join("a.wav"), b"audio").unwrap();
    std::fs::write(source.path().join("b.mid"), b"midi").unwrap();
    std::fs::write(source.path().join("bad.mp3"), b"").unwrap();
    std::fs::write(source.path().join("ignored.txt"), b"text").unwrap();
    let library = MediaLibrary::open(root.path()).unwrap();
    let result = library.import_directory(source.path()).unwrap();
    assert_eq!(result.entries.len(), 2);
    assert_eq!(result.diagnostics.len(), 1);
    assert!(!result.truncated);
    assert_eq!(library.scan().unwrap().entries.len(), 2);
}
#[test]
fn repeated_import_of_catalog_folder_never_reimports_managed_copies() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("real-song.wav");
    fs::write(&source, b"audio").unwrap();
    let library = MediaLibrary::open(root.path()).unwrap();
    let first = library.import_directory(root.path()).unwrap();
    assert_eq!(first.entries.len(), 1);
    let id = first.entries[0].id.clone();
    let payload = first.entries[0].path.clone();
    for _ in 0..3 {
        let result = library.import_directory(root.path()).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].id, id);
        assert!(result.diagnostics.is_empty());
        assert_eq!(library.scan().unwrap().entries.len(), 1);
    }
    assert!(library.import_file(&payload).is_err());
    let deleted = library.delete(&first.entries[0]).unwrap();
    assert_eq!(
        library.import_directory(root.path()).unwrap().entries.len(),
        1
    );
    assert!(library.scan().unwrap().entries.is_empty());
    assert_eq!(library.trash().unwrap().entries.len(), 1);
    library.restore(&deleted).unwrap();
    assert_eq!(fs::read(source).unwrap(), b"audio");
}
#[test]
fn reserved_looking_unowned_and_external_filenames_remain_importable() {
    let root = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let filename = format!("{}.media.wav", "a".repeat(64));
    fs::write(root.path().join(&filename), b"unowned original").unwrap();
    let library = MediaLibrary::open(root.path()).unwrap();
    let entry = library.import_file(root.path().join(&filename)).unwrap();
    assert_eq!(entry.name, filename);
    let owned_name = entry.path.file_name().unwrap();
    fs::write(external.path().join(owned_name), b"independent external").unwrap();
    let other = library
        .import_file(external.path().join(owned_name))
        .unwrap();
    assert_ne!(entry.id, other.id);
    assert_eq!(library.read(&other).unwrap(), b"independent external");
}
