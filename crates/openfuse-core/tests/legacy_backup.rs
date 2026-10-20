use openfuse_core::legacy_backup::{EntryKind, LegacyBackup, MAX_LEGACY_FILES};
fn preset(name: &str) -> Vec<u8> {
    format!("<?xml version='1.0'?><Preset ProductId='6' amplifier='Mustang'><FUSE><Info name='{name}' author='Owner'>notes</Info></FUSE><Future opaque='yes'/></Preset>").into_bytes()
}
#[test]
fn preserves_full_tree_exact_bytes_and_original_slot_numbers() {
    let xml = preset("Embedded name");
    let rows = vec![
        ("MU_BackupName.fuse".into(), b"My archive\r\n".to_vec()),
        ("Presets/00_different name.fuse".into(), xml.clone()),
        ("Presets/23.fuse".into(), preset("Last")),
        (
            "FUSE/00.fuse".into(),
            b"<FUSE><Info name='sidecar'/></FUSE>".to_vec(),
        ),
        (
            "SystemSettings.fuse".into(),
            b"<System><QA>1</QA></System>".to_vec(),
        ),
        (
            "Effects/MOD/00_Chorus.fuse".into(),
            b"<FX Name='Chorus'/>".to_vec(),
        ),
        ("HD/Audio/backing.wav".into(), vec![0, 255, 23]),
        ("future/data.bin".into(), vec![12, 0, 128]),
    ];
    let backup = LegacyBackup::from_entries(rows.clone()).unwrap();
    assert_eq!(backup.entries().len(), rows.len());
    assert_eq!(backup.presets().len(), 2);
    assert_eq!(backup.description(), Some("My archive\r\n"));
    assert_eq!(backup.presets()[0].slot(), 0);
    assert_eq!(backup.presets()[1].slot(), 23);
    assert_eq!(
        backup.presets()[0].document().metadata().name,
        "Embedded name"
    );
    for (path, bytes) in rows {
        let entry = backup.entries().iter().find(|e| e.path() == path).unwrap();
        assert_eq!(entry.bytes(), bytes);
        assert_eq!(entry.sha256().len(), 64);
    }
    assert!(
        backup
            .entries()
            .iter()
            .any(|e| e.kind() == EntryKind::Unknown)
    );
    assert_eq!(
        backup.presets()[0].document().to_xml().unwrap().as_bytes(),
        xml
    );
}
#[test]
fn superchamp_auxiliary_only_and_gdec_are_preserved_without_fabricated_captures() {
    let backup = LegacyBackup::from_entries(vec![
        ("SC_BackupName.fuse".into(), b"Voice collection".to_vec()),
        (
            "VOICE/00_Tweed.fuse".into(),
            b"<VOICE Name='Tweed' amplifier='SuperChampX2'/>".to_vec(),
        ),
        (
            "FX/00_Reverb.fuse".into(),
            b"<FX Name='Reverb' amplifier='SuperChampX2'/>".to_vec(),
        ),
    ])
    .unwrap();
    assert!(backup.presets().is_empty());
    assert!(
        backup
            .warnings()
            .iter()
            .any(|s| s.contains("inspection only"))
    );
    let entry = backup
        .entries()
        .iter()
        .find(|e| e.kind() == EntryKind::VoiceBank)
        .unwrap();
    assert_eq!(entry.metadata().name.as_deref(), Some("Tweed"));
    let gd = LegacyBackup::from_entries(vec![(
        "Presets/99.fuse".into(),
        b"<Preset ProductId='10'><FUSE><Info name='GDEC'/></FUSE></Preset>".to_vec(),
    )])
    .unwrap();
    assert_eq!(gd.presets()[0].document().product_id(), Some(10));
}
#[test]
fn rejects_duplicate_old_new_slots_and_ambiguous_descriptions() {
    assert!(
        LegacyBackup::from_entries(vec![
            ("Presets/07.fuse".into(), preset("A")),
            ("Presets/07_B.fuse".into(), preset("B"))
        ])
        .is_err()
    );
    assert!(
        LegacyBackup::from_entries(vec![
            ("MU_BackupName.fuse".into(), b"A".to_vec()),
            ("M2_BackupName.fuse".into(), b"B".to_vec())
        ])
        .is_err()
    );
}
#[test]
fn rejects_traversal_case_collisions_and_file_directory_collision() {
    for path in [
        "/absolute",
        "../escape",
        "Presets/../evil",
        "C:/evil",
        "Presets\\00.fuse",
        "a//b",
        "./file",
        "a\0b",
    ] {
        assert!(
            LegacyBackup::from_entries(vec![(path.into(), preset("A"))]).is_err(),
            "{path}"
        );
    }
    assert!(
        LegacyBackup::from_entries(vec![
            ("MU_BackupName.fuse".into(), b"A".to_vec()),
            ("mu_backupname.fuse".into(), b"B".to_vec())
        ])
        .is_err()
    );
    assert!(
        LegacyBackup::from_entries(vec![
            ("MU_BackupName.fuse".into(), b"A".to_vec()),
            ("presets".into(), vec![]),
            ("presets/00.fuse".into(), preset("A"))
        ])
        .is_err()
    );
}
#[test]
fn malformed_presets_fail_but_unknown_and_malformed_auxiliary_stay_intact() {
    for xml in [
        "<Preset>",
        "<!DOCTYPE Preset [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><Preset>&x;</Preset>",
        "<FX/>",
    ] {
        assert!(
            LegacyBackup::from_entries(vec![("Presets/00.fuse".into(), xml.as_bytes().to_vec())])
                .is_err()
        );
    }
    let bytes = b"<System><bad".to_vec();
    let backup =
        LegacyBackup::from_entries(vec![("SystemSettings.fuse".into(), bytes.clone())]).unwrap();
    assert_eq!(backup.entries()[0].bytes(), bytes);
    assert!(!backup.entries()[0].metadata().warnings.is_empty());
}
#[test]
fn bounds_files_depth_and_description_decoding() {
    assert!(
        LegacyBackup::from_entries(
            (0..=MAX_LEGACY_FILES)
                .map(|i| (format!("x/{i}"), vec![]))
                .collect()
        )
        .is_err()
    );
    assert!(
        LegacyBackup::from_entries(vec![(format!("{}file", "a/".repeat(17)), vec![])]).is_err()
    );
    let backup =
        LegacyBackup::from_entries(vec![("BackupName.fuse".into(), vec![255, 254, 65, 0])])
            .unwrap();
    assert_eq!(backup.description(), Some("A"));
    assert_eq!(backup.entries()[0].bytes(), [255, 254, 65, 0]);
    assert!(
        LegacyBackup::from_entries(vec![("BackupName.fuse".into(), vec![255, 254, 65])]).is_err()
    );
}
#[test]
fn directory_read_is_read_only_and_keeps_origin_label() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("Presets")).unwrap();
    let xml = preset("A");
    std::fs::write(root.path().join("Presets/01_A.fuse"), &xml).unwrap();
    let backup = LegacyBackup::read_directory(root.path()).unwrap();
    assert_eq!(backup.presets()[0].slot(), 1);
    assert!(backup.source_name().is_some());
    assert_eq!(
        std::fs::read(root.path().join("Presets/01_A.fuse")).unwrap(),
        xml
    );
}
#[cfg(unix)]
#[test]
fn refuses_root_and_nested_symlinks_even_to_inside_files() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("MU_BackupName.fuse"), b"A").unwrap();
    symlink(
        root.path().join("MU_BackupName.fuse"),
        root.path().join("copy"),
    )
    .unwrap();
    assert!(LegacyBackup::read_directory(root.path()).is_err());
    let other = tempfile::tempdir().unwrap();
    symlink(root.path(), other.path().join("archive")).unwrap();
    assert!(LegacyBackup::read_directory(&other.path().join("archive")).is_err());
}

#[test]
fn refuses_oversized_sparse_file_and_oversized_preset_without_partial_import() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("MU_BackupName.fuse"), b"Archive").unwrap();
    let file = std::fs::File::create(root.path().join("huge.bin")).unwrap();
    file.set_len(openfuse_core::legacy_backup::MAX_LEGACY_BYTES as u64 + 1)
        .unwrap();
    assert!(
        LegacyBackup::read_directory(root.path())
            .unwrap_err()
            .to_string()
            .contains("64 MiB")
    );
    let oversized = vec![b' '; openfuse_core::preset::MAX_PRESET_BYTES + 1];
    assert!(
        LegacyBackup::from_entries(vec![("Presets/00.fuse".into(), oversized)])
            .unwrap_err()
            .to_string()
            .contains("2 MiB")
    );
}

#[cfg(unix)]
#[test]
fn rejects_unix_socket_without_attempting_to_read_it() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("MU_BackupName.fuse"), b"Archive").unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(root.path().join("socket")).unwrap();
    assert!(
        LegacyBackup::read_directory(root.path())
            .unwrap_err()
            .to_string()
            .contains("special file")
    );
}
