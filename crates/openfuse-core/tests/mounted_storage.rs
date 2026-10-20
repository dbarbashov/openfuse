#[cfg(unix)]
mod unix {
    use openfuse_core::mounted_storage::{
        FileKind, MAX_FILE_BYTES, MountedStorage, TransferPlan, Volume,
    };
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicBool, Ordering},
    };
    fn source(dir: &tempfile::TempDir, name: &str, data: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, data).unwrap();
        path
    }
    fn mounted(dir: &tempfile::TempDir) -> MountedStorage {
        MountedStorage::open(dir.path(), Volume::Sd).unwrap()
    }
    #[test]
    fn inventory_and_verified_download_preserve_arbitrary_phrase_and_mixed_case_audio() {
        let root = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        for folder in ["Audio", "Phrase", "Midi", "Presets"] {
            fs::create_dir(root.path().join(folder)).unwrap();
        }
        source(&root, "Audio/Track.WaV", b"audio");
        source(&root, "Midi/Song.MID", b"midi");
        source(&root, "Phrase/Recording", b"opaque phrase");
        source(&root, "Audio/vendor.dat", b"unknown");
        let storage = mounted(&root);
        let inventory = storage.scan().unwrap();
        assert_eq!(inventory.entries.len(), 3);
        assert_eq!(inventory.diagnostics.len(), 1);
        assert!(inventory.entries.iter().any(|e| e.kind == FileKind::Phrase));
        let plan = TransferPlan::download(&storage, &inventory.entries, out.path()).unwrap();
        assert_eq!(fs::read_dir(out.path()).unwrap().count(), 0);
        let receipt = plan.execute(&AtomicBool::new(false));
        assert!(receipt.error.is_none(), "{receipt:?}");
        assert_eq!(receipt.completed.len(), 3);
        assert_eq!(
            fs::read(out.path().join("Recording")).unwrap(),
            b"opaque phrase"
        );
        assert_eq!(storage.read(&inventory.entries[0]).unwrap(), b"audio");
        let kept =
            TransferPlan::download_keep_both(&storage, &inventory.entries, out.path()).unwrap();
        assert!(
            kept.items()
                .iter()
                .any(|i| i.destination.ends_with("Recording (2)"))
        );
    }
    #[test]
    fn upload_is_no_clobber_and_keep_both_accounts_for_batch_and_case() {
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("Audio")).unwrap();
        source(&root, "Audio/TRACK.WAV", b"old");
        let a = source(&local, "track.wav", b"one");
        let b = source(&other, "track.wav", b"two");
        let storage = mounted(&root);
        assert!(TransferPlan::upload(&storage, std::slice::from_ref(&a)).is_err());
        let plan = TransferPlan::upload_keep_both(&storage, &[a, b]).unwrap();
        assert!(plan.items()[0].destination.ends_with("track (2).wav"));
        assert!(plan.items()[1].destination.ends_with("track (3).wav"));
        assert!(plan.execute(&AtomicBool::new(false)).error.is_none());
        assert_eq!(
            fs::read(root.path().join("Audio/TRACK.WAV")).unwrap(),
            b"old"
        );
        assert_eq!(
            fs::read(root.path().join("Audio/track (3).wav")).unwrap(),
            b"two"
        );
    }
    #[test]
    fn preview_and_precancel_do_not_create_directories_or_files() {
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let path = source(&local, "track.mp3", b"bytes");
        let plan = TransferPlan::upload(&mounted(&root), &[path]).unwrap();
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        let receipt = plan.execute(&AtomicBool::new(true));
        assert!(receipt.cancelled);
        assert!(receipt.completed.is_empty());
        assert!(receipt.interrupted.is_none());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        let receipt = plan.execute(&AtomicBool::new(false));
        assert_eq!(receipt.completed.len(), 1);
    }
    #[test]
    fn full_batch_preflight_rejects_changed_later_source_before_any_write() {
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let a = source(&local, "a.wav", b"a");
        let b = source(&local, "b.wav", b"b");
        let plan = TransferPlan::upload(&mounted(&root), &[a, b.clone()]).unwrap();
        fs::write(b, b"changed").unwrap();
        let receipt = plan.execute(&AtomicBool::new(false));
        assert!(receipt.error.is_some());
        assert!(receipt.completed.is_empty());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn later_collision_preserves_completed_and_foreign_file_without_false_partial_receipt() {
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let plan = TransferPlan::upload(
            &mounted(&root),
            &[source(&local, "a.wav", b"a"), source(&local, "b.wav", b"b")],
        )
        .unwrap();
        let receipt = plan.execute_with_progress(&AtomicBool::new(false), |done, total| {
            assert_eq!((done, total), (1, 2));
            source(&root, "Audio/b.wav", b"foreign");
        });
        assert_eq!(receipt.completed.len(), 1);
        assert!(receipt.error.is_some());
        assert!(receipt.interrupted.is_none());
        assert_eq!(
            fs::read(root.path().join("Audio/b.wav")).unwrap(),
            b"foreign"
        );
    }
    #[test]
    fn cancel_after_verified_item_stops_next_item_and_reports_actual_progress() {
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let plan = TransferPlan::upload(
            &mounted(&root),
            &[source(&local, "a.wav", b"a"), source(&local, "b.wav", b"b")],
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let receipt = plan.execute_with_progress(&cancel, |done, total| {
            assert_eq!((done, total), (1, 2));
            cancel.store(true, Ordering::Release);
        });
        assert_eq!(receipt.completed.len(), 1);
        assert!(receipt.cancelled);
        assert!(!root.path().join("Audio/b.wav").exists());
    }
    #[test]
    fn preset_alias_is_changed_only_in_destination_clone_and_family_gate_is_strict() {
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let xml = r#"<Preset ProductId="10" amplifier="G-DEC 3.0"><Name>My &amp;amp; tone</Name><Vendor keep="yes"/></Preset>"#;
        let path = source(&local, "tone.fuse", xml.as_bytes());
        let plan = TransferPlan::upload(&mounted(&root), std::slice::from_ref(&path)).unwrap();
        assert_eq!(plan.items()[0].changes.len(), 2);
        assert!(plan.execute(&AtomicBool::new(false)).error.is_none());
        assert_eq!(fs::read_to_string(path).unwrap(), xml);
        let copy = fs::read_to_string(root.path().join("Presets/tone.fuse")).unwrap();
        assert!(copy.contains("G-DEC3"));
        assert!(copy.contains("<Vendor keep=\"yes\"/>"));
        let foreign = source(&local, "foreign.fuse", b"<Preset ProductId=\"13\"/>");
        assert!(TransferPlan::upload(&mounted(&root), &[foreign]).is_err());
        let internal = MountedStorage::open(root.path(), Volume::Internal).unwrap();
        assert!(
            TransferPlan::upload(&internal, &[source(&local, "other.fuse", xml.as_bytes())])
                .is_err()
        );
    }
    #[test]
    fn stale_inventory_and_mutated_public_metadata_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("Audio")).unwrap();
        source(&root, "Audio/a.wav", b"a");
        let storage = mounted(&root);
        let entry = storage.scan().unwrap().entries.remove(0);
        assert!(mounted(&other).read(&entry).is_err());
        let mut edited = entry.clone();
        edited.name = "../escape.wav".into();
        assert!(storage.read(&edited).is_err());
        edited = entry.clone();
        edited.size = 0;
        assert!(storage.read(&edited).is_err());
        source(&root, "Audio/a.wav", b"changed");
        assert!(storage.read(&entry).is_err());
    }
    #[test]
    fn oversized_files_and_nonportable_names_are_refused_without_copy() {
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let huge = local.path().join("huge.wav");
        fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(TransferPlan::upload(&mounted(&root), &[huge]).is_err());
        for name in ["CON.wav", "bad:track.wav", "LPT1.mid"] {
            // Validate names before opening them: Windows cannot create these
            // paths, and reserved device names must never be opened as fixtures.
            let error =
                TransferPlan::upload(&mounted(&root), &[local.path().join(name)]).unwrap_err();
            assert!(error.to_string().contains("Filename is not portable"));
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_and_replaced_root_or_child_cannot_redirect_frozen_plan() {
        use std::os::unix::fs::symlink;
        let parent = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = source(&local, "a.wav", b"a");
        symlink(&path, local.path().join("link.wav")).unwrap();
        let root = parent.path().join("sd");
        fs::create_dir(&root).unwrap();
        let storage = MountedStorage::open(&root, Volume::Sd).unwrap();
        assert!(TransferPlan::upload(&storage, &[local.path().join("link.wav")]).is_err());
        symlink(outside.path(), root.join("Audio")).unwrap();
        assert!(TransferPlan::upload(&storage, std::slice::from_ref(&path)).is_err());
        fs::remove_file(root.join("Audio")).unwrap();
        fs::create_dir(root.join("Audio")).unwrap();
        let plan = TransferPlan::upload(&storage, std::slice::from_ref(&path)).unwrap();
        fs::rename(root.join("Audio"), root.join("oldAudio")).unwrap();
        symlink(outside.path(), root.join("Audio")).unwrap();
        assert!(plan.execute(&AtomicBool::new(false)).error.is_some());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        fs::remove_file(root.join("Audio")).unwrap();
        let plan = TransferPlan::upload(&storage, &[path]).unwrap();
        fs::rename(&root, parent.path().join("oldsd")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(plan.execute(&AtomicBool::new(false)).error.is_some());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
    #[test]
    fn competing_plans_never_clobber_existing_winner() {
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("Audio")).unwrap();
        let plan =
            TransferPlan::upload(&mounted(&root), &[source(&local, "a.wav", b"winner")]).unwrap();
        let second = plan.clone();
        let handle = std::thread::spawn(move || second.execute(&AtomicBool::new(false)));
        let first = plan.execute(&AtomicBool::new(false));
        let second = handle.join().unwrap();
        assert_eq!(first.completed.len() + second.completed.len(), 1);
        assert_eq!(
            fs::read(root.path().join("Audio/a.wav")).unwrap(),
            b"winner"
        );
    }

    #[test]
    fn receipt_bound_media_import_refuses_external_changes_before_publication() {
        use openfuse_core::media_library::MediaLibrary;
        let root = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let catalog = tempfile::tempdir().unwrap();
        let plan =
            TransferPlan::upload(&mounted(&root), &[source(&local, "track.wav", b"verified")])
                .unwrap();
        let receipt = plan.execute(&AtomicBool::new(false));
        let item = &receipt.completed[0];
        let library = MediaLibrary::open(catalog.path()).unwrap();
        fs::write(&item.destination, b"external change").unwrap();
        assert!(
            library
                .import_file_checked(&item.destination, &item.output_sha256)
                .is_err()
        );
        assert_eq!(fs::read_dir(catalog.path()).unwrap().count(), 0);
        fs::write(&item.destination, b"verified").unwrap();
        let entry = library
            .import_file_checked(&item.destination, &item.output_sha256)
            .unwrap();
        assert_eq!(library.read(&entry).unwrap(), b"verified");
    }
}

#[cfg(not(unix))]
#[test]
fn mounted_execution_is_explicitly_read_only_without_stable_identity() {
    use openfuse_core::mounted_storage::{MountedStorage, TransferPlan, Volume};
    use std::sync::atomic::AtomicBool;
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let path = source.path().join("track.wav");
    std::fs::write(&path, b"unchanged").unwrap();
    let storage = MountedStorage::open(destination.path(), Volume::Sd).unwrap();
    let plan = TransferPlan::upload(&storage, &[path]).unwrap();
    let receipt = plan.execute(&AtomicBool::new(false));
    assert!(receipt.error.unwrap().contains("read-only"));
    assert!(receipt.completed.is_empty());
    assert!(receipt.interrupted.is_none());
    assert_eq!(std::fs::read_dir(destination.path()).unwrap().count(), 0);
}
