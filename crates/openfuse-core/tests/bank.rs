use openfuse_core::{
    audition,
    bank::{self, Backup, BackupCollector, Names, SlotCapture, StoredRead},
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument,
        expression::{ExpressionPedal, PedalSlot},
    },
    protocol::Report64,
    wire_preset,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
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
fn finish() -> Report64 {
    let mut b = [0; 64];
    b[0] = 0x14;
    b[1] = 1;
    b[3] = 2;
    Report64::from_bytes(b)
}
fn backup(pid: u16) -> Backup {
    Backup::new(
        pid,
        (0..audition::slot_count(pid).unwrap())
            .map(|slot| SlotCapture::new(pid, slot, stored(pid, slot)).unwrap())
            .collect(),
    )
    .unwrap()
}
fn resign(value: &mut Value) {
    for entry in value["slots"].as_array_mut().unwrap() {
        let input = serde_json::to_vec(&(
            entry["slot"].as_u64().unwrap(),
            &entry["raw_reports"],
            &entry["xml"],
        ))
        .unwrap();
        entry["sha256"] = format!("{:x}", Sha256::digest(input)).into();
    }
    let input = serde_json::to_vec(&(
        &value["format"],
        &value["version"],
        &value["pid"],
        &value["family"],
        &value["slot_count"],
        &value["slots"],
    ))
    .unwrap();
    value["sha256"] = format!("{:x}", Sha256::digest(input)).into();
}
#[test]
fn requests_match_original_read_only_headers_and_boundaries() {
    for pid in [4, 5, 0xa, 0x10, 0x12, 0x14, 0x16, 0x26] {
        assert_eq!(
            &bank::names_request(pid).unwrap().as_bytes()[..2],
            &[0xff, 0xc1]
        );
        assert_eq!(
            &bank::backup_request(pid, None).unwrap().as_bytes()[..5],
            &[0x14, 1, 0, 0, 1]
        );
        let count = audition::slot_count(pid).unwrap();
        assert_eq!(
            &bank::backup_request(pid, Some(count - 1))
                .unwrap()
                .as_bytes()[..5],
            &[0x14, 1, (count - 1) as u8, 0, 0]
        );
        assert!(bank::stored_request(pid, count).is_err());
    }
    for pid in [0xe, 0xffff] {
        assert!(bank::names_request(pid).is_err());
        assert!(bank::backup_request(pid, None).is_err());
    }
}
#[test]
fn full_sync_names_require_exact_stored_list_and_terminal() {
    let mut names = Names::new(0x14).unwrap();
    assert!(!names.observe(current(0x14, &document(0x14))[0]).unwrap());
    for slot in (0..24).rev() {
        assert!(!names.observe(stored(0x14, slot)[0]).unwrap());
    }
    assert!(!names.is_complete());
    assert!(
        names
            .observe(openfuse_core::messages::sync_request(false))
            .unwrap()
    );
    assert_eq!(names.entries().len(), 24);
    assert!(names.is_complete());
    assert!(names.observe(stored(0x14, 0)[0]).is_err());
}
#[test]
fn names_reject_duplicates_missing_slots_and_out_of_range() {
    let mut names = Names::new(0x14).unwrap();
    assert!(
        names
            .observe(openfuse_core::messages::sync_request(false))
            .is_err()
    );
    let mut names = Names::new(0x14).unwrap();
    names.observe(stored(0x14, 0)[0]).unwrap();
    assert!(names.observe(stored(0x14, 0)[0]).is_err());
    let mut names = Names::new(0x14).unwrap();
    assert!(names.observe(stored(0x14, 24)[0]).is_err());
}
#[test]
fn stored_read_waits_for_backup_termination_and_checks_slot() {
    let raw = stored(0x14, 3);
    let mut read = StoredRead::new(0x14, 3).unwrap();
    for &r in &raw {
        assert!(read.observe(r).unwrap().is_none());
    }
    let entry = read.observe(finish()).unwrap().unwrap();
    assert_eq!(entry.slot(), 3);
    assert_eq!(entry.raw_reports(), raw);
    assert!(read.observe(finish()).is_err());
    let mut read = StoredRead::new(0x14, 2).unwrap();
    assert!(read.observe(raw[0]).is_err());
    let mut read = StoredRead::new(0x14, 3).unwrap();
    read.observe(raw[0]).unwrap();
    assert!(read.observe(finish()).is_err());
}
#[test]
fn full_backup_collects_every_slot_without_selecting_and_never_returns_partial() {
    let mut collect = BackupCollector::new(0x14).unwrap();
    for slot in (0..24).rev() {
        for r in stored(0x14, slot) {
            assert!(collect.observe(r).unwrap().is_none());
        }
    }
    assert_eq!(collect.completed_slots(), 24);
    let bank = collect.observe(finish()).unwrap().unwrap();
    assert_eq!(bank.slots().first().unwrap().slot(), 0);
    assert_eq!(bank.slots().last().unwrap().slot(), 23);
    let mut collect = BackupCollector::new(0x14).unwrap();
    for r in stored(0x14, 0) {
        collect.observe(r).unwrap();
    }
    assert!(collect.observe(finish()).is_err());
}
#[test]
fn malformed_streams_fail_before_exposing_a_backup() {
    let raw = stored(0x14, 0);
    let mut stream = BackupCollector::new(0x14).unwrap();
    assert!(stream.observe(raw[1]).is_err());
    let mut stream = BackupCollector::new(0x14).unwrap();
    stream.observe(raw[0]).unwrap();
    assert!(stream.observe(raw[0]).is_err());
    let mut stream = BackupCollector::new(0x14).unwrap();
    for &r in &raw {
        stream.observe(r).unwrap();
    }
    assert!(stream.observe(raw[0]).is_err());
    let mut stream = BackupCollector::new(0x14).unwrap();
    stream.observe(raw[0]).unwrap();
    let mut b = [0; 64];
    b[0] = 0xab;
    for _ in 0..63 {
        stream.observe(Report64::from_bytes(b)).unwrap();
    }
    assert!(stream.observe(Report64::from_bytes(b)).is_err());
}
#[test]
fn backup_round_trip_preserves_every_raw_byte_and_decoded_xml() {
    let mut entries = Vec::new();
    for slot in 0..24 {
        let mut raw = stored(0x14, slot);
        let mut b = *raw[1].as_bytes();
        b[60] = slot as u8;
        b[61] = 0xa5;
        raw[1] = Report64::from_bytes(b);
        entries.push(SlotCapture::new(0x14, slot, raw).unwrap());
    }
    let original = Backup::new(0x14, entries).unwrap();
    let text = original.to_json().unwrap();
    let loaded = Backup::parse(&text).unwrap();
    assert_eq!(loaded.to_json().unwrap(), text);
    assert_eq!(loaded.family(), 13);
    assert_eq!(loaded.pid(), 0x14);
    for (a, b) in original.slots().iter().zip(loaded.slots()) {
        assert_eq!(a.raw_reports(), b.raw_reports());
        assert_eq!(
            a.document().to_xml().unwrap(),
            b.document().to_xml().unwrap()
        );
    }
    assert!(loaded.validate_for_pid(0x16).is_err());
    assert!(loaded.validate_for_pid(0x14).is_ok());
    assert!(serde_json::to_value(&loaded).unwrap().is_object());
}
#[test]
fn backup_checksums_catch_corruption_and_structural_validation_survives_resigning() {
    let text = backup(0x14).to_json().unwrap();
    let original: Value = serde_json::from_str(&text).unwrap();
    let mut value = original.clone();
    value["slots"][0]["xml"] = "corrupt".into();
    assert!(Backup::parse(&value.to_string()).is_err());
    for mutate in [0, 1, 2, 3, 4, 5, 6] {
        let mut value = original.clone();
        match mutate {
            0 => value["slots"][1]["slot"] = 0.into(),
            1 => value["slots"][0]["xml"] = "<Preset ProductId=\"1\"/>".into(),
            2 => value["slots"][0]["raw_reports"][0] = "00".into(),
            3 => value["version"] = 99.into(),
            4 => value["family"] = 1.into(),
            5 => {
                value["slots"].as_array_mut().unwrap().pop();
            }
            _ => value["extra"] = "field".into(),
        }
        resign(&mut value);
        assert!(
            Backup::parse(&value.to_string()).is_err(),
            "mutation {mutate}"
        );
    }
    assert!(Backup::parse(&" ".repeat(bank::MAX_BACKUP_BYTES + 1)).is_err());
}
#[test]
fn incomplete_wrong_pid_and_wrong_slot_captures_cannot_form_backup() {
    assert!(Backup::new(0x14, vec![]).is_err());
    let mut entries = backup(0x14).slots().to_vec();
    entries[1] = entries[0].clone();
    assert!(Backup::new(0x14, entries).is_err());
    assert!(SlotCapture::new(0x14, 1, stored(0x14, 0)).is_err());
    assert!(Backup::new(4, backup(0x14).slots().to_vec()).is_err());
    let mut raw = stored(0x14, 0);
    raw.remove(1);
    assert!(SlotCapture::new(0x14, 0, raw).is_err());
}
#[test]
fn save_current_is_exact_two_packet_persistent_transaction_with_stored_readback() {
    let raw = current(0x14, &document(0x14));
    let plan = bank::save_current(0x14, &raw, 23).unwrap();
    assert_eq!(plan.steps().len(), 2);
    let name = plan.steps()[0].as_bytes();
    assert_eq!(&name[..8], &[0x1c, 1, 3, 0, 23, 0, 1, 0]);
    assert_eq!(&name[16..], &raw[0].as_bytes()[16..]);
    assert_eq!(&plan.steps()[1].as_bytes()[..3], &[0x1c, 3, 0]);
    assert!(plan.steps()[1].as_bytes()[3..].iter().all(|&b| b == 0));
    let expected = stored(0x14, 23);
    assert!(plan.matches_stored_readback(&expected).is_ok());
    assert!(plan.matches_readback(&expected).is_err());
    assert!(plan.matches_stored_readback(&stored(0x14, 22)).is_err());
    assert!(bank::save_current(0x14, &raw, 24).is_err());
    let mut changed = expected.clone();
    let mut b = *changed[1].as_bytes();
    b[32] ^= 1;
    changed[1] = Report64::from_bytes(b);
    assert!(plan.matches_stored_readback(&changed).is_err());
}
#[test]
fn restore_reconstructs_host_packets_and_preserves_current_global_endpoints() {
    let raw = current(0x14, &document(0x14));
    let mut desired = document(0x14);
    desired.set_parameter("Amplifier", 0, 32768).unwrap();
    let plan = bank::restore_slot(0x14, &desired, &raw, 23).unwrap();
    let steps = plan.steps();
    assert_eq!(steps[0].as_bytes()[2], 4);
    assert_eq!(steps[0].as_bytes()[4], 23);
    assert_eq!(steps[steps.len() - 2].as_bytes()[2], 3);
    assert_eq!(steps[steps.len() - 2].as_bytes()[4], 23);
    assert!(steps.iter().all(|r| r.message_id() == 0x1c));
    let expression = steps.iter().find(|r| r.as_bytes()[2] == 10).unwrap();
    assert_eq!(&expression.as_bytes()[17..19], &[4, 250]);
    assert!(
        plan.matches_stored_readback(&steps[..steps.len() - 2])
            .is_ok()
    );
    assert_eq!(raw, current(0x14, &document(0x14)));
    assert!(plan.warnings().iter().any(|w| w.contains("unknown bytes")));
}

#[test]
fn names_accept_current_only_coverage_and_prefer_separate_stored_name() {
    let mut names = Names::new(0x14).unwrap();
    for slot in 0..24 {
        let mut b = *stored(0x14, slot)[0].as_bytes();
        if slot == 7 {
            b[6] = 1;
            b[16] = b'C';
        }
        names.observe(Report64::from_bytes(b)).unwrap();
    }
    assert!(
        names
            .observe(openfuse_core::messages::sync_request(false))
            .unwrap()
    );
    assert!(names.entries()[&7].starts_with('C'));
    for reverse in [false, true] {
        let mut names = Names::new(0x14).unwrap();
        let stored = stored(0x14, 7)[0];
        let mut b = *stored.as_bytes();
        b[6] = 1;
        b[16] = b'C';
        let current = Report64::from_bytes(b);
        let pair = if reverse {
            [stored, current]
        } else {
            [current, stored]
        };
        for report in pair {
            names.observe(report).unwrap();
        }
        assert_eq!(names.entries()[&7], "Independent fixture");
    }
}

#[test]
fn optional_correlated_backup_start_does_not_replace_complete_or_terminal() {
    for start in [false, true] {
        let mut read = StoredRead::new(0x14, 3).unwrap();
        if start {
            assert!(
                read.observe(bank::stored_request(0x14, 3).unwrap())
                    .unwrap()
                    .is_none()
            );
        }
        for r in stored(0x14, 3) {
            assert!(read.observe(r).unwrap().is_none());
        }
        assert_eq!(read.observe(finish()).unwrap().unwrap().slot(), 3);
    }
    let mut all = BackupCollector::new(0x14).unwrap();
    assert!(
        all.observe(bank::backup_request(0x14, None).unwrap())
            .unwrap()
            .is_none()
    );
    for slot in 0..24 {
        for report in stored(0x14, slot) {
            assert!(all.observe(report).unwrap().is_none());
        }
    }
    assert_eq!(all.observe(finish()).unwrap().unwrap().slots().len(), 24);
    let mut read = StoredRead::new(0x14, 3).unwrap();
    read.observe(bank::stored_request(0x14, 3).unwrap())
        .unwrap();
    assert!(read.observe(finish()).is_err());
}
#[test]
fn backup_start_rejects_duplicates_late_targets_and_restore_operations() {
    let start = bank::stored_request(0x14, 3).unwrap();
    let mut read = StoredRead::new(0x14, 3).unwrap();
    read.observe(start).unwrap();
    assert!(read.observe(start).is_err());
    let mut read = StoredRead::new(0x14, 3).unwrap();
    read.observe(stored(0x14, 3)[0]).unwrap();
    assert!(read.observe(start).is_err());
    let mut read = StoredRead::new(0x14, 3).unwrap();
    for r in stored(0x14, 3) {
        read.observe(r).unwrap();
    }
    assert!(read.observe(start).is_err());
    for (index, value) in [(2, 4), (3, 1), (3, 3), (3, 255), (4, 1)] {
        let mut b = *start.as_bytes();
        b[index] = value;
        assert!(
            StoredRead::new(0x14, 3)
                .unwrap()
                .observe(Report64::from_bytes(b))
                .is_err()
        );
    }
    assert!(BackupCollector::new(0x14).unwrap().observe(start).is_err());
}
