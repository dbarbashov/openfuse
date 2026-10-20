use openfuse_core::{
    archive::{Archive, ArchiveCatalog, ArchiveKind},
    auxiliary::{self, Collector, Key, Mapping, Selection, Snapshot},
    legacy_backup::LegacyBackup,
    model_catalog::{self, ModuleCategory},
    protocol::Report64,
};
fn module(product: u32, key: &str, category: ModuleCategory, kind: u8) -> String {
    let model = model_catalog::models(product, category).next().unwrap();
    let mut xml = format!(
        "<{key} ID=\"{}\"><Module ID=\"{}\" POS=\"0\" BypassState=\"1\" BypassMode=\"{}\">",
        kind - 5,
        model.id,
        model.bypass_mode
    );
    for p in &model.parameters {
        xml.push_str(&format!(
            "<Param ControlIndex=\"{}\">{}</Param>",
            p.index, p.default_raw
        ));
    }
    xml.push_str(&format!("</Module></{key}>"));
    xml
}
fn xml(control: u8, name: &str) -> String {
    if control == 1 {
        format!(
            "<VOICE Name=\"{name}\" amplifier=\"SuperChampX2\">{}{}<UsbGain>0</UsbGain></VOICE>",
            module(2259, "Amplifier", ModuleCategory::Amplifier, 5),
            module(2259, "Stompbox", ModuleCategory::Stompbox, 6)
        )
    } else {
        format!(
            "<FX Name=\"{name}\" amplifier=\"SuperChampX2\">{}{}{}</FX>",
            module(2259, "Modulation", ModuleCategory::Modulation, 7),
            module(2259, "Delay", ModuleCategory::Delay, 8),
            module(2259, "Reverb", ModuleCategory::Reverb, 9)
        )
    }
}
fn frame(fields: &[(usize, u8)]) -> Report64 {
    let mut b = [0; 64];
    for &(i, v) in fields {
        b[i] = v;
    }
    Report64::from_bytes(b)
}
fn reports() -> Vec<Report64> {
    let mut reports = vec![];
    for (control, count) in [(1, 17), (2, 15)] {
        for slot in 0..count {
            reports.extend(
                auxiliary::plan_xml(
                    14,
                    Key::Bank { control, slot },
                    &xml(control, &format!("Entry {control} {slot}")),
                )
                .unwrap(),
            );
        }
    }
    reports.push(frame(&[(0, 0x14), (1, 1), (3, 2), (4, 1)]));
    reports
}
fn snapshot() -> Snapshot {
    Snapshot::from_reports(14, &reports()).unwrap()
}
fn legacy(control: u8, slot: u16, text: String) -> LegacyBackup {
    LegacyBackup::from_entries(vec![(
        format!(
            "{}/{slot:02}_entry.fuse",
            if control == 1 { "VOICE" } else { "FX" }
        ),
        text.into_bytes(),
    )])
    .unwrap()
}
#[test]
fn complete_superchamp_has_32_physical_entries_and_no_full_presets() {
    let s = snapshot();
    assert_eq!(s.entries().len(), 32);
    assert!(s.presets().is_empty());
    assert!(
        s.entries().iter().all(|e| e.restore_error().is_none()),
        "{:?}",
        s.warnings()
    );
}
#[test]
fn capture_checksum_roundtrip_and_tamper() {
    let s = snapshot();
    let text = s.to_json().unwrap();
    assert_eq!(
        Snapshot::parse(&text).unwrap().fingerprint(),
        s.fingerprint()
    );
    assert!(Snapshot::parse(&text.replace("\"pid\": 14", "\"pid\": 20")).is_err());
}
#[test]
fn incomplete_bank_cannot_be_published() {
    let mut r = reports();
    r.drain(..4);
    assert!(Snapshot::from_reports(14, &r).is_err());
    let mut r = reports();
    r.pop();
    assert!(Snapshot::from_reports(14, &r).is_err());
}
#[test]
fn stream_rejects_duplicate_interleaving_late_payload_and_resource_overflow() {
    let r = reports();
    let mut c = Collector::new(14).unwrap();
    c.observe(r[0]).unwrap();
    assert!(c.observe(r[0]).is_err());
    let mut r = reports();
    r.push(frame(&[]));
    assert!(Snapshot::from_reports(14, &r).is_err());
    let mut c = Collector::new(14).unwrap();
    for _ in 0..4096 {
        c.observe(frame(&[])).unwrap();
    }
    assert!(c.observe(frame(&[])).is_err());
}
#[test]
fn unknown_record_is_preserved_but_not_restorable() {
    let mut r = reports();
    r.insert(1, frame(&[(0, 0x1c), (1, 1), (2, 99), (3, 1)]));
    let s = Snapshot::from_reports(14, &r).unwrap();
    assert_eq!(s.reports(), r);
    assert!(s.entries()[0].restore_error().is_some());
}
#[test]
fn reserved_raw_bits_cannot_be_silently_written() {
    let mut r = reports();
    let mut b = *r[1].as_bytes();
    b[15] = 128;
    r[1] = Report64::from_bytes(b);
    let s = Snapshot::from_reports(14, &r).unwrap();
    assert!(s.entries()[0].restore_error().is_some());
    assert_eq!(s.entries()[0].raw_reports()[1].as_bytes()[15], 128);
}
#[test]
fn legacy_has_exact_xml_and_no_fabricated_raw() {
    let text = xml(1, "Legacy");
    let s = Snapshot::from_legacy(14, &legacy(1, 2, text.clone())).unwrap();
    assert!(s.is_legacy());
    assert!(s.reports().is_empty());
    assert!(s.entries()[0].raw_reports().is_empty());
    assert_eq!(s.entries()[0].xml(), Some(text.as_str()));
    assert!(s.to_json().is_err());
}
#[test]
fn selective_plan_is_frozen_remapped_and_checks_untouched_state() {
    let target = snapshot();
    let source = Snapshot::from_legacy(14, &legacy(1, 2, xml(1, "Restored"))).unwrap();
    let mapping = Mapping {
        source: Key::Bank {
            control: 1,
            slot: 2,
        },
        target: Key::Bank {
            control: 1,
            slot: 5,
        },
    };
    let plan = Selection::new(&source, &target, &[mapping]).unwrap();
    assert_eq!(&plan.steps()[0].as_bytes()[3..8], &[1, 5, 0, 1, 1]);
    assert_eq!(
        &plan.steps().last().unwrap().as_bytes()[3..8],
        &[1, 0, 0, 0, 0]
    );
    assert!(plan.matches_readback(&target).is_err());
    let mut r = target.reports().to_vec();
    let start = r
        .iter()
        .position(|v| v.as_bytes()[2] == 4 && v.as_bytes()[3] == 1 && v.as_bytes()[4] == 5)
        .unwrap();
    let end = start
        + r[start..]
            .iter()
            .position(|v| v.as_bytes()[2] == 0)
            .unwrap()
        + 1;
    r.splice(start..end, plan.steps());
    let readback = Snapshot::from_reports(14, &r).unwrap();
    plan.matches_readback(&readback).unwrap();
    assert!(plan.validate_destination(&readback).is_err());
    let mut b = *r[1].as_bytes();
    b[32] ^= 1;
    r[1] = Report64::from_bytes(b);
    assert!(
        plan.matches_readback(&Snapshot::from_reports(14, &r).unwrap())
            .is_err()
    );
}
#[test]
fn mapping_rejects_empty_duplicates_crossbank_and_legacy_destination() {
    let s = snapshot();
    let key = Key::Bank {
        control: 1,
        slot: 2,
    };
    let m = Mapping {
        source: key,
        target: key,
    };
    assert!(Selection::new(&s, &s, &[]).is_err());
    assert!(Selection::new(&s, &s, &[m, m]).is_err());
    assert!(
        Selection::new(
            &s,
            &s,
            &[Mapping {
                source: key,
                target: Key::Bank {
                    control: 2,
                    slot: 2
                }
            }]
        )
        .is_err()
    );
    let l = Snapshot::from_legacy(14, &legacy(1, 2, xml(1, "Legacy"))).unwrap();
    assert!(Selection::new(&s, &l, &[m]).is_err());
}
#[test]
fn xml_extensions_duplicates_and_bad_names_fail_before_plan() {
    let base = xml(2, "Valid");
    let key = Key::Bank {
        control: 2,
        slot: 0,
    };
    for text in [
        base.replace("Name=", "Unknown=\"1\" Name="),
        base.replace("</FX>", "<UsbGain>2</UsbGain></FX>"),
        base.replace(
            "Valid",
            "\u{41a}\u{438}\u{440}\u{438}\u{43b}\u{43b}\u{438}\u{446}\u{430}",
        ),
        base.replace("Valid", &"a".repeat(49)),
        base.replace(
            "</FX>",
            &format!("{}</FX>", module(2259, "Delay", ModuleCategory::Delay, 8)),
        ),
    ] {
        assert!(auxiliary::plan_xml(14, key, &text).is_err(), "{text}");
    }
}
#[test]
fn system_xml_is_offline_only_and_optional_foot4_defaults_zero() {
    let qa = (0..16).map(|i| format!("<QA>{i}</QA>")).collect::<String>();
    let text = format!(
        "<System>{qa}<LCDContrast>255</LCDContrast><MemoryLock>1</MemoryLock><Foot2BtnMode>2</Foot2BtnMode><ExpPedalVolumeHeel>3</ExpPedalVolumeHeel><ExpPedalVolumeToe>4</ExpPedalVolumeToe></System>"
    );
    let r = auxiliary::decode_system_xml(&text).unwrap();
    assert_eq!(&r.as_bytes()[32..38], &[255, 1, 2, 3, 4, 0]);
    assert!(auxiliary::plan_xml(14, Key::System, &text).is_err());
    assert!(auxiliary::decode_system_xml(&text.replace("<QA>0</QA>", "")).is_err());
    assert!(auxiliary::decode_system_xml(&text.replace("255", "256")).is_err());
}
#[test]
fn expression_fallback_matches_original_live_mode() {
    let plan = auxiliary::plan_xml(
        14,
        Key::Bank {
            control: 2,
            slot: 0,
        },
        &xml(2, "FX"),
    )
    .unwrap();
    let b = plan
        .iter()
        .find(|r| r.as_bytes()[2] == 10)
        .unwrap()
        .as_bytes();
    assert_eq!(&b[16..25], &[0, 0, 255, 0, 6, 0, 0, 255, 1]);
}
#[test]
fn auxiliary_catalog_roundtrip_trash_and_direct_import() {
    let dir = tempfile::tempdir().unwrap();
    let cat = ArchiveCatalog::open(dir.path()).unwrap();
    let s = snapshot();
    let entry = cat.import_auxiliary(&s, "Full capture").unwrap();
    assert_eq!(entry.kind, ArchiveKind::Auxiliary);
    assert_eq!(entry.pid, Some(14));
    let loaded = cat.load(&entry).unwrap();
    assert_eq!(loaded.auxiliary().unwrap().fingerprint(), s.fingerprint());
    assert!(loaded.auxiliary_for_pid(20).is_err());
    let deleted = cat.delete(&entry).unwrap();
    assert_eq!(cat.trash().unwrap().entries.len(), 1);
    cat.restore(&deleted).unwrap();
    let file = dir.path().join("capture.json");
    std::fs::write(&file, s.to_json().unwrap()).unwrap();
    assert!(Archive::read_path(&file).unwrap().auxiliary().is_some());
    assert_eq!(cat.import_file(&file).unwrap().kind, ArchiveKind::Auxiliary);
}
#[test]
fn incompatible_legacy_bank_paths_do_not_infer_profile() {
    assert!(Snapshot::from_legacy(20, &legacy(1, 2, xml(1, "Legacy"))).is_err());
    assert!(auxiliary::capabilities(0xffff).is_err());
}

#[test]
fn malformed_legacy_filenames_remain_inventory_only() {
    for path in [
        "Effects/MOD/nested/00_name.fuse",
        "Effects/MOD/00bad.fuse",
        "Effects/MOD/00_name.txt",
    ] {
        let backup = LegacyBackup::from_entries(vec![
            ("MU_BackupName.fuse".into(), b"Test".to_vec()),
            (path.into(), b"<FX Name=\"x\"/>".to_vec()),
        ])
        .unwrap();
        assert!(Snapshot::from_legacy(20, &backup).is_err());
    }
}
#[test]
fn unknown_expression_values_do_not_overflow_or_get_written_from_capture() {
    let mut r = reports();
    let index = r.iter().position(|v| v.as_bytes()[2] == 10).unwrap();
    for mode in 252..=255 {
        let mut b = *r[index].as_bytes();
        b[20] = mode;
        r[index] = Report64::from_bytes(b);
        let s = Snapshot::from_reports(14, &r).unwrap();
        assert!(s.entries().iter().any(|e| e.restore_error().is_some()));
    }
    let source = xml(2, "Extreme").replace(
        "</FX>",
        "<FirstExpressionPedal ExpressionModeBehavior=\"65535\"/></FX>",
    );
    let plan = auxiliary::plan_xml(
        14,
        Key::Bank {
            control: 2,
            slot: 0,
        },
        &source,
    )
    .unwrap();
    assert_eq!(
        plan.iter()
            .find(|r| r.as_bytes()[2] == 10)
            .unwrap()
            .as_bytes()[20],
        6
    );
}
#[test]
fn mixed_xml_text_is_rejected_and_scalar_text_nodes_are_joined() {
    let source = xml(1, "Text");
    assert!(
        auxiliary::plan_xml(
            14,
            Key::Bank {
                control: 1,
                slot: 0
            },
            &source.replace("<UsbGain>", "unrepresented<UsbGain>")
        )
        .is_err()
    );
    assert!(
        auxiliary::plan_xml(
            14,
            Key::Bank {
                control: 1,
                slot: 0
            },
            &source.replace(
                "<UsbGain>0</UsbGain>",
                "<UsbGain>2<!-- split -->56</UsbGain>"
            )
        )
        .is_err()
    );
}
#[test]
fn unknown_header_and_complete_payload_bits_are_preserved_and_blocked() {
    for (index, byte) in [(0, 1), (0, 6), (4, 63)] {
        let mut r = reports();
        let index = if index == 4 {
            r.iter().position(|v| v.as_bytes()[2] == 0).unwrap()
        } else {
            index
        };
        let mut b = *r[index].as_bytes();
        b[byte] |= 32;
        r[index] = Report64::from_bytes(b);
        let s = Snapshot::from_reports(14, &r).unwrap();
        assert_eq!(s.reports(), r);
        assert!(s.entries()[0].restore_error().is_some());
    }
}
