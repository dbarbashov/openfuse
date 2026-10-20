use openfuse_core::{
    archive::{Archive, ArchiveCatalog, ArchiveKind},
    control_library::{ControlLibrary, export_document, read_document},
    control_preset::ControlPreset,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "openfuse-control-library-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn library(&self) -> ControlLibrary {
        ControlLibrary::open(&self.0).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn document() -> ControlPreset {
    ControlPreset::new(20, 1, "My chorus").unwrap()
}

#[test]
fn immutable_versions_deduplicate_identical_content_and_recover_deleted_save() {
    let root = Scratch::new();
    let lib = root.library();
    let mut doc = document();
    let first = lib.save(&doc).unwrap();
    let again = lib.save(&doc).unwrap();
    assert_eq!(first.catalog.id, again.catalog.id);
    doc.set_name("Edited chorus").unwrap();
    let edited = lib.save(&doc).unwrap();
    assert_ne!(first.catalog.id, edited.catalog.id);
    assert_eq!(lib.scan().unwrap().entries.len(), 2);
    assert_eq!(lib.load(&first).unwrap().name(), "My chorus");
    let removed = lib.delete(&first).unwrap();
    assert!(removed.catalog.trashed);
    assert!(first.catalog.path.exists());
    assert_eq!(lib.scan().unwrap().entries.len(), 1);
    assert_eq!(lib.trash().unwrap().entries.len(), 1);
    assert!(lib.load(&first).is_err());
    let recovered = lib.save(&document()).unwrap();
    assert_eq!(recovered.catalog.id, first.catalog.id);
    assert!(!recovered.catalog.trashed);
    assert!(lib.trash().unwrap().entries.is_empty());
}
#[test]
fn duplicate_is_independent_even_when_xml_is_identical() {
    let root = Scratch::new();
    let lib = root.library();
    let first = lib.save(&document()).unwrap();
    let copy = lib.duplicate(&first).unwrap();
    assert_ne!(first.catalog.id, copy.catalog.id);
    assert_eq!(
        lib.load(&first).unwrap().to_xml().unwrap(),
        lib.load(&copy).unwrap().to_xml().unwrap()
    );
    let removed = lib.delete(&copy).unwrap();
    assert_eq!(lib.scan().unwrap().entries.len(), 1);
    let restored = lib.restore(&removed).unwrap();
    assert_eq!(restored.catalog.id, copy.catalog.id);
    assert_eq!(lib.scan().unwrap().entries.len(), 2);
    assert!(lib.restore(&removed).is_err());
}
#[test]
fn native_and_fuse_exports_are_no_clobber_and_profiles_are_explicit() {
    let root = Scratch::new();
    let doc = document();
    let native = root.0.join("one.json");
    let xml = root.0.join("one.fuse");
    export_document(&doc, &native).unwrap();
    export_document(&doc, &xml).unwrap();
    assert_eq!(
        read_document(&native, None).unwrap().to_xml().unwrap(),
        doc.to_xml().unwrap()
    );
    assert!(read_document(&native, Some((20, 1))).is_err());
    assert!(read_document(&xml, None).is_err());
    assert_eq!(
        read_document(&xml, Some((20, 1)))
            .unwrap()
            .to_xml()
            .unwrap(),
        doc.to_xml().unwrap()
    );
    assert!(
        read_document(&xml, Some((20, 2)))
            .unwrap()
            .write_plan(0)
            .is_err()
    );
    let original = fs::read(&xml).unwrap();
    assert!(export_document(&doc, &xml).is_err());
    assert_eq!(fs::read(&xml).unwrap(), original);
    assert!(export_document(&doc, root.0.join("bad.exe")).is_err());
    let lib = root.library();
    let imported = lib.import_file(&xml, Some((20, 1))).unwrap();
    assert_eq!(imported.pid, 20);
    assert_eq!(imported.control, 1);
    assert!(imported.matches("Mustang chorus MOD"));
    assert!(!imported.matches("VOICE"));
}
#[test]
fn catalog_bundle_reopens_with_profile_and_name_and_never_becomes_full_preset() {
    let root = Scratch::new();
    let lib = root.library();
    let entry = lib.save(&document()).unwrap();
    let archive = Archive::read_path(&entry.catalog.path).unwrap();
    assert!(archive.presets().is_empty());
    assert_eq!(archive.control_preset().unwrap().name(), "My chorus");
    assert_eq!(read_document(&entry.catalog.path, None).unwrap().pid(), 20);
    let catalog = ArchiveCatalog::open(&root.0).unwrap();
    let scan = catalog.scan().unwrap();
    assert_eq!(scan.entries[0].kind, ArchiveKind::ControlPreset);
    assert_eq!(scan.entries[0].pid, Some(20));
    assert_eq!(scan.entries[0].names, ["My chorus"]);
    assert!(archive.restore_selection(20, &[]).is_err());
}
#[test]
fn checksum_tampering_blocks_load_delete_and_duplicate_and_is_diagnosed() {
    let root = Scratch::new();
    let lib = root.library();
    let entry = lib.save(&document()).unwrap();
    let bytes = fs::read_to_string(&entry.catalog.path)
        .unwrap()
        .replace("My chorus", "Corrupted");
    fs::write(&entry.catalog.path, bytes).unwrap();
    assert!(lib.load(&entry).is_err());
    assert!(lib.delete(&entry).is_err());
    assert!(lib.duplicate(&entry).is_err());
    let scan = lib.scan().unwrap();
    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 1);
    assert!(entry.catalog.path.exists());
}
#[test]
fn replaced_root_and_foreign_entries_cannot_be_mutated() {
    let root = Scratch::new();
    let other = Scratch::new();
    let lib = root.library();
    let entry = lib.save(&document()).unwrap();
    assert!(other.library().delete(&entry).is_err());
    let old = root.0.with_extension("moved");
    let rename = fs::rename(&root.0, &old);
    #[cfg(windows)]
    {
        assert_eq!(rename.unwrap_err().raw_os_error(), Some(32));
        assert_eq!(lib.scan().unwrap().entries.len(), 1);
        assert!(lib.load(&entry).is_ok());
    }
    #[cfg(not(windows))]
    {
        rename.unwrap();
        fs::create_dir(&root.0).unwrap();
        assert!(lib.save(&document()).is_err());
        assert!(lib.delete(&entry).is_err());
        assert!(lib.scan().is_err());
        fs::remove_dir_all(old).unwrap();
    }
}
#[cfg(unix)]
#[test]
fn symlinks_cannot_import_or_overwrite_outside_selected_directory() {
    use std::os::unix::fs::symlink;
    let root = Scratch::new();
    let outside = Scratch::new();
    let doc = document();
    let real = outside.0.join("source.fuse");
    export_document(&doc, &real).unwrap();
    let link = root.0.join("link.fuse");
    symlink(&real, &link).unwrap();
    assert!(read_document(&link, Some((20, 1))).is_err());
    assert!(export_document(&doc, &link).is_err());
    assert_eq!(
        read_document(&real, Some((20, 1))).unwrap().name(),
        "My chorus"
    );
    let alias = root.0.join("alias");
    symlink(&outside.0, &alias).unwrap();
    assert!(ControlLibrary::open(alias).is_err());
}
