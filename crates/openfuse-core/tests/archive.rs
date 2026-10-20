use openfuse_core::{
    archive::{Archive, ArchiveCatalog, ArchiveKind},
    bank::{Backup, RestoreSelection, SlotCapture, SlotMapping},
    library::PresetLibrary,
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument,
        expression::{ExpressionPedal, PedalSlot},
    },
    protocol::Report64,
    wire_preset,
};
use std::fs;
use tempfile::tempdir;
fn document(pid: u16) -> PresetDocument {
    let product = wire_preset::product_for_pid(pid).unwrap();
    let mut xml = format!("<Preset ProductId=\"{product}\">");
    for (key, kind, category, position) in [
        ("Amplifier", 0, ModuleCategory::Amplifier, 0),
        ("Stompbox", 1, ModuleCategory::Stompbox, 0),
        ("Modulation", 2, ModuleCategory::Modulation, 1),
        ("Delay", 3, ModuleCategory::Delay, 4),
        ("Reverb", 4, ModuleCategory::Reverb, 5),
    ] {
        if kind == 1 {
            xml.push_str("<FX>");
        }
        let id = if kind == 0 {
            model_catalog::product(product).unwrap().default_amp_id
        } else {
            0
        };
        let model = model_catalog::model(product, category, id).unwrap();
        xml.push_str(&format!("<{key} ID=\"{kind}\"><Module ID=\"{id}\" POS=\"{position}\" BypassState=\"1\" BypassMode=\"{}\">",model.bypass_mode));
        for p in &model.parameters {
            xml.push_str(&format!(
                "<Param ControlIndex=\"{}\">{}</Param>",
                p.index, p.default_raw
            ));
        }
        xml.push_str(&format!("</Module></{key}>"));
    }
    xml.push_str("</FX><UsbGain>9</UsbGain><FUSE><Info name=\"Independent fixture\" author=\"Local author\" tags=\"keep\">Local description</Info></FUSE>");
    if product == 7 {
        xml.push_str("<FxLoopBypass>true</FxLoopBypass>");
    }
    xml.push_str("</Preset>");
    let mut doc = PresetDocument::parse(&xml).unwrap();
    if product != 11 {
        doc.set_expression_pedal(
            PedalSlot::First,
            &ExpressionPedal::defaults(PedalSlot::First),
        )
        .unwrap();
    }
    if product == 7 {
        doc.set_expression_pedal(
            PedalSlot::Second,
            &ExpressionPedal::defaults(PedalSlot::Second),
        )
        .unwrap();
    }
    doc
}
fn current(pid: u16, doc: &PresetDocument) -> Vec<Report64> {
    let mut reports = wire_preset::encode(pid, doc).unwrap().reports;
    for report in &mut reports {
        let mut b = *report.as_bytes();
        if b[0] == 0x1c && b[2] == 4 {
            b[4] = 7;
        }
        if b[0] == 0x1c && b[2] == 10 {
            b[17] = 4;
            b[18] = 250;
        }
        *report = Report64::from_bytes(b);
    }
    reports
}

fn stored(pid: u16, slot: u16) -> Vec<Report64> {
    let mut raw = current(pid, &document(pid));
    raw.retain(|r| r.message_id() != 0xff);
    for report in &mut raw {
        let mut b = *report.as_bytes();
        if b[0] == 0x1c && b[2] == 4 {
            b[4..6].copy_from_slice(&slot.to_le_bytes());
            b[6] = 0;
        }
        *report = Report64::from_bytes(b);
    }
    raw
}

fn partial(slots: &[u16]) -> Backup {
    Backup::selected(
        0x14,
        slots
            .iter()
            .map(|s| SlotCapture::new(0x14, *s, stored(0x14, *s)).unwrap())
            .collect(),
    )
    .unwrap()
}
fn legacy(root: &std::path::Path) {
    fs::create_dir(root.join("Presets")).unwrap();
    fs::write(
        root.join("Presets/02_Fixture.fuse"),
        document(0x14).to_xml().unwrap(),
    )
    .unwrap();
    fs::write(root.join("BackupName.fuse"), b"Archive description").unwrap();
    fs::write(root.join("opaque.bin"), [0, 255, 42]).unwrap();
    fs::write(root.join("empty.bin"), []).unwrap();
}
#[test]
fn selected_archive_roundtrips_version_two_and_complete_version_one_unchanged() {
    let selected = partial(&[9, 2]);
    assert!(!selected.is_complete());
    assert_eq!(
        selected
            .slots()
            .iter()
            .map(|s| s.slot())
            .collect::<Vec<_>>(),
        [2, 9]
    );
    let json = selected.to_json().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["version"],
        2
    );
    let round = Backup::parse(&json).unwrap();
    assert_eq!(round.slots().len(), 2);
    assert!(round.slot(9).is_some());
    assert!(round.slot(0).is_none());
    let complete = Backup::new(
        0x14,
        (0..24)
            .map(|s| SlotCapture::new(0x14, s, stored(0x14, s)).unwrap())
            .collect(),
    )
    .unwrap();
    assert!(complete.is_complete());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&complete.to_json().unwrap()).unwrap()["version"],
        1
    );
    assert_eq!(
        Backup::parse(&complete.to_json().unwrap())
            .unwrap()
            .slots()
            .len(),
        24
    );
    assert_eq!(complete.selection(&[23]).unwrap().slots()[0].slot(), 23);
    assert!(complete.selection(&[2, 2]).is_err());
    assert!(complete.selection(&[24]).is_err());
    assert!(complete.selection(&[]).is_err());
    assert!(Backup::new(0x14, selected.slots().to_vec()).is_err());
    assert!(Backup::selected(0x14, vec![]).is_err());
}
#[test]
fn version_one_incomplete_and_corrupt_version_two_are_not_accepted() {
    let json = partial(&[2]).to_json().unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["version"] = 1.into();
    assert!(Backup::parse(&value.to_string()).is_err());
    value["version"] = 2.into();
    value["slots"][0]["slot"] = 3.into();
    assert!(Backup::parse(&value.to_string()).is_err());
}
#[test]
fn selection_freezes_mapping_preflights_all_presets_and_uses_expected_destination_readback() {
    let backup = partial(&[2, 9]);
    let mappings = [
        SlotMapping {
            source: 9,
            target: 4,
        },
        SlotMapping {
            source: 2,
            target: 8,
        },
    ];
    let selection = RestoreSelection::new(&backup, 0x14, &mappings).unwrap();
    assert_eq!(selection.entries()[0].source(), 9);
    assert_eq!(selection.entries()[0].target(), 4);
    let plans = selection.plans(&current(0x14, &document(0x14))).unwrap();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0].expected_slot(), 4);
    assert_eq!(plans[1].expected_slot(), 8);
    let desired = plans[0].expected().unwrap();
    let mut readback = current(0x14, desired);
    for report in &mut readback {
        let mut b = *report.as_bytes();
        if b[0] == 0x1c && b[2] == 4 {
            b[4] = 4;
            b[6] = 0;
        }
        *report = Report64::from_bytes(b);
    }
    plans[0].matches_stored_readback(&readback).unwrap();
    assert!(plans[1].matches_stored_readback(&readback).is_err());
}
#[test]
fn selection_rejects_duplicate_absent_and_incompatible_inputs_before_any_plans() {
    let backup = partial(&[2, 9]);
    for map in [
        vec![],
        vec![SlotMapping {
            source: 1,
            target: 2,
        }],
        vec![SlotMapping {
            source: 2,
            target: 24,
        }],
        vec![
            SlotMapping {
                source: 2,
                target: 1,
            },
            SlotMapping {
                source: 9,
                target: 1,
            },
        ],
        vec![
            SlotMapping {
                source: 2,
                target: 1,
            },
            SlotMapping {
                source: 2,
                target: 2,
            },
        ],
    ] {
        assert!(RestoreSelection::new(&backup, 0x14, &map).is_err());
    }
    assert!(
        RestoreSelection::new(
            &backup,
            0x16,
            &[SlotMapping {
                source: 2,
                target: 2
            }]
        )
        .is_err()
    );
    assert!(
        RestoreSelection::from_documents(
            0x14,
            vec![(2, document(4))],
            0x14,
            &[SlotMapping {
                source: 2,
                target: 2
            }]
        )
        .is_err()
    );
    let mut broken = document(0x14);
    broken
        .replace_module_model("Amplifier", 65535, &[], None)
        .unwrap();
    let selection = RestoreSelection::from_documents(
        0x14,
        vec![(2, document(0x14)), (9, broken)],
        0x14,
        &[
            SlotMapping {
                source: 2,
                target: 2,
            },
            SlotMapping {
                source: 9,
                target: 9,
            },
        ],
    )
    .unwrap();
    assert!(selection.plans(&current(0x14, &document(0x14))).is_err());
}
#[test]
fn catalog_import_is_no_clobber_searchable_and_retains_exact_original_file() {
    let root = tempdir().unwrap();
    let input = tempdir().unwrap();
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let json = format!("  {}  ", partial(&[2, 9]).to_json().unwrap());
    let source = input.path().join("Tour bank.json");
    fs::write(&source, &json).unwrap();
    let entry = catalog.import_file(&source).unwrap();
    assert_eq!(entry.original_bytes, json.len() as u64);
    assert_eq!(entry.kind, ArchiveKind::Native);
    assert_eq!(entry.pid, Some(0x14));
    assert_eq!(entry.preset_count, 2);
    assert!(entry.matches("tour fixture"));
    assert!(!entry.matches("absent"));
    let saved = fs::read(&entry.path).unwrap();
    let twice = catalog.import_file(&source).unwrap();
    assert_eq!(entry.id, twice.id);
    assert_eq!(saved, fs::read(&entry.path).unwrap());
    assert_eq!(catalog.scan().unwrap().entries.len(), 1);
    let value: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    let bytes = openfuse_core::hex::decode(value["files"][0]["hex"].as_str().unwrap()).unwrap();
    assert_eq!(bytes, json.as_bytes());
    let loaded = catalog.load(&entry).unwrap();
    assert_eq!(loaded.presets().len(), 2);
    assert!(loaded.native().is_some());
    assert_eq!(Archive::read_path(&entry.path).unwrap().presets().len(), 2);
    assert_eq!(Archive::read_path(&source).unwrap().presets().len(), 2);
}
#[test]
fn historical_import_retains_unknown_empty_and_description_bytes_without_fabricating_raw_reports() {
    let root = tempdir().unwrap();
    let input = tempdir().unwrap();
    legacy(input.path());
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let entry = catalog.import_legacy(input.path()).unwrap();
    assert_eq!(entry.pid, None);
    assert_eq!(entry.kind, ArchiveKind::Legacy);
    assert_eq!(entry.preset_count, 1);
    let archive = catalog.load(&entry).unwrap();
    assert!(archive.native().is_none());
    let legacy = archive.legacy().unwrap();
    assert_eq!(legacy.entries().len(), 4);
    assert_eq!(
        legacy
            .entries()
            .iter()
            .find(|e| e.path() == "opaque.bin")
            .unwrap()
            .bytes(),
        [0, 255, 42]
    );
    assert!(
        legacy
            .entries()
            .iter()
            .find(|e| e.path() == "empty.bin")
            .unwrap()
            .bytes()
            .is_empty()
    );
    assert_eq!(legacy.description(), Some("Archive description"));
    assert!(archive.warnings().iter().any(|w| w.contains("PID")));
    assert!(
        archive
            .restore_selection(
                0x14,
                &[SlotMapping {
                    source: 2,
                    target: 8
                }]
            )
            .is_ok()
    );
    assert!(
        archive
            .restore_selection(
                4,
                &[SlotMapping {
                    source: 2,
                    target: 8
                }]
            )
            .is_err()
    );
    assert_eq!(Archive::read_path(input.path()).unwrap().presets().len(), 1);
}
#[test]
fn delete_and_restore_are_recoverable_and_do_not_modify_original_bundle() {
    let root = tempdir().unwrap();
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let entry = catalog.import_backup(&partial(&[2]), "Session").unwrap();
    let bytes = fs::read(&entry.path).unwrap();
    let deleted = catalog.delete(&entry).unwrap();
    assert!(deleted.trashed);
    assert!(catalog.scan().unwrap().entries.is_empty());
    assert_eq!(catalog.trash().unwrap().entries.len(), 1);
    assert_eq!(bytes, fs::read(&entry.path).unwrap());
    assert!(catalog.load(&entry).is_err());
    let duplicate = catalog
        .import_backup(&partial(&[2]), "New label ignored")
        .unwrap();
    assert!(duplicate.trashed);
    assert_eq!(duplicate.label, "Session");
    let restored = catalog.restore(&deleted).unwrap();
    assert!(!restored.trashed);
    assert_eq!(catalog.scan().unwrap().entries.len(), 1);
    assert!(catalog.trash().unwrap().entries.is_empty());
    assert_eq!(bytes, fs::read(&entry.path).unwrap());
    assert!(catalog.restore(&deleted).is_err());
}
#[test]
fn corruption_and_unrelated_catalog_entries_cannot_be_deleted_or_restored() {
    let root = tempdir().unwrap();
    let other = tempdir().unwrap();
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let entry = catalog.import_backup(&partial(&[2]), "Session").unwrap();
    assert!(
        ArchiveCatalog::open(other.path())
            .unwrap()
            .delete(&entry)
            .is_err()
    );
    fs::write(&entry.path, b"broken").unwrap();
    assert!(catalog.load(&entry).is_err());
    assert!(catalog.delete(&entry).is_err());
    let scan = catalog.scan().unwrap();
    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 1);
}
#[test]
fn extraction_preflights_slots_and_uses_unique_safe_library_filenames() {
    let root = tempdir().unwrap();
    let out = tempdir().unwrap();
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let library = PresetLibrary::open(out.path()).unwrap();
    let entry = catalog.import_backup(&partial(&[2, 9]), "Session").unwrap();
    assert!(catalog.extract(&entry, &[2, 7], &library).is_err());
    assert!(library.scan().unwrap().entries.is_empty());
    assert!(catalog.extract(&entry, &[2, 2], &library).is_err());
    let first = catalog.extract(&entry, &[2, 9], &library).unwrap();
    let again = catalog.extract(&entry, &[2], &library).unwrap();
    assert_ne!(first[0].path, again[0].path);
    assert_eq!(library.scan().unwrap().entries.len(), 3);
    let document = document(0x14);
    assert!(
        library
            .import_document(&document, "../outside.fuse")
            .is_err()
    );
    assert!(library.import_document(&document, "plain.bin").is_err());
}
#[cfg(unix)]
#[test]
fn symlink_sources_roots_entries_and_recovery_markers_are_refused() {
    use std::os::unix::fs::symlink;
    let root = tempdir().unwrap();
    let input = tempdir().unwrap();
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let source = input.path().join("bank.json");
    fs::write(&source, partial(&[2]).to_json().unwrap()).unwrap();
    let alias = input.path().join("alias.json");
    symlink(&source, &alias).unwrap();
    assert!(catalog.import_file(&alias).is_err());
    assert!(Archive::read_path(&alias).is_err());
    let root_alias = input.path().join("catalog");
    symlink(root.path(), &root_alias).unwrap();
    assert!(ArchiveCatalog::open(root_alias).is_err());
    let entry = catalog.import_file(&source).unwrap();
    let original = fs::read(&entry.path).unwrap();
    fs::remove_file(&entry.path).unwrap();
    symlink(&source, &entry.path).unwrap();
    assert!(catalog.load(&entry).is_err());
    assert!(catalog.delete(&entry).is_err());
    fs::remove_file(&entry.path).unwrap();
    fs::write(&entry.path, original).unwrap();
    symlink(&source, root.path().join(format!("{}.deleted", entry.id))).unwrap();
    assert!(catalog.load(&entry).is_err());
}
#[test]
fn bounded_reads_and_replaced_roots_fail_without_catalog_mutation() {
    let root = tempdir().unwrap();
    let input = tempdir().unwrap();
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let file = input.path().join("oversized.json");
    let out = fs::File::create(&file).unwrap();
    out.set_len(4 * 1024 * 1024 + 1).unwrap();
    assert!(catalog.import_file(&file).is_err());
    assert!(catalog.scan().unwrap().entries.is_empty());
    let parent = tempdir().unwrap();
    let path = parent.path().join("catalog");
    fs::create_dir(&path).unwrap();
    let moved = ArchiveCatalog::open(&path).unwrap();
    fs::rename(&path, parent.path().join("old")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(moved.import_backup(&partial(&[2]), "stale").is_err());
    assert!(fs::read_dir(&path).unwrap().next().is_none());
}

#[test]
fn selective_legacy_restore_does_not_require_unselected_family_compatibility() {
    let selection = RestoreSelection::from_documents(
        0x14,
        vec![(2, document(0x14)), (99, document(4))],
        0x14,
        &[SlotMapping {
            source: 2,
            target: 3,
        }],
    )
    .unwrap();
    assert_eq!(
        selection
            .plans(&current(0x14, &document(0x14)))
            .unwrap()
            .len(),
        1
    );
    assert!(
        RestoreSelection::from_documents(
            0x14,
            vec![(2, document(0x14)), (99, document(4))],
            0x14,
            &[SlotMapping {
                source: 99,
                target: 3
            }]
        )
        .is_err()
    );
}

#[test]
fn concurrent_identical_imports_publish_one_complete_bundle_without_overwrite() {
    let root = tempdir().unwrap();
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let backup = partial(&[2, 9]);
    let results = std::thread::scope(|scope| {
        (0..4)
            .map(|_| {
                let catalog = &catalog;
                let backup = &backup;
                scope.spawn(move || catalog.import_backup(backup, "Concurrent").unwrap())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(results.iter().all(|entry| entry.id == results[0].id));
    assert_eq!(catalog.scan().unwrap().entries.len(), 1);
    assert_eq!(catalog.load(&results[0]).unwrap().presets().len(), 2);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn corrupt_recovery_marker_never_releases_or_discards_original_bytes() {
    let root = tempdir().unwrap();
    let catalog = ArchiveCatalog::open(root.path()).unwrap();
    let entry = catalog.import_backup(&partial(&[2]), "Recovery").unwrap();
    let original = fs::read(&entry.path).unwrap();
    let deleted = catalog.delete(&entry).unwrap();
    let marker = root.path().join(format!("{}.deleted", entry.id));
    fs::write(&marker, b"broken").unwrap();
    assert!(catalog.restore(&deleted).is_err());
    assert_eq!(fs::read(&entry.path).unwrap(), original);
    assert_eq!(fs::read(&marker).unwrap(), b"broken");
}
