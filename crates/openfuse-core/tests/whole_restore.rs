use openfuse_core::{
    archive::Archive,
    audition,
    auxiliary::{self, Key, Snapshot, WholeRestore},
    control_preset::ControlPreset,
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument,
        expression::{ExpressionPedal, PedalSlot},
    },
    protocol::Report64,
    wire_preset,
};
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
fn snapshot(pid: u16) -> Snapshot {
    let doc = document(pid);
    let base = wire_preset::encode(pid, &doc).unwrap().reports;
    let mut reports = Vec::new();
    for slot in 0..audition::slot_count(pid).unwrap() {
        for report in base.iter().filter(|r| r.message_id() == 0x1c) {
            let mut b = *report.as_bytes();
            if b[2] == 4 {
                b[4..6].copy_from_slice(&slot.to_le_bytes());
            }
            if b[2] == 10 {
                b[17] = 4;
                b[18] = 250;
            }
            reports.push(Report64::from_bytes(b));
        }
    }
    let caps = auxiliary::capabilities(pid).unwrap();
    if caps.system {
        let mut b = [0; 64];
        b[0] = 0x1c;
        b[1] = 1;
        b[2] = 11;
        b[35] = 4;
        b[36] = 250;
        reports.push(Report64::from_bytes(b));
    }
    for (control, count) in caps.banks {
        let doc = ControlPreset::new(pid, control, "Whole bank fixture").unwrap();
        for slot in 0..count {
            reports.extend(
                auxiliary::plan_xml(pid, Key::Bank { control, slot }, &doc.to_xml().unwrap())
                    .unwrap(),
            );
        }
    }
    reports.push(terminal());
    Snapshot::from_reports(pid, &reports).unwrap()
}
fn terminal() -> Report64 {
    let mut b = [0; 64];
    b[0] = 0x14;
    b[1] = 1;
    b[3] = 2;
    b[4] = 1;
    Report64::from_bytes(b)
}
fn expected(plan: &WholeRestore) -> Snapshot {
    let mut reports = vec![];
    let mut saving = false;
    for step in plan.steps() {
        let b = step.report.as_bytes();
        if b[0] != 0x1c {
            continue;
        }
        if b[2] == 3 {
            saving = true;
            continue;
        }
        if saving && b[2] == 0 {
            saving = false;
            continue;
        }
        reports.push(step.report);
    }
    reports.push(terminal());
    Snapshot::from_reports(plan.pid(), &reports).unwrap()
}
#[test]
fn all_supported_profiles_have_full_envelope_and_exact_expected_readback() {
    for pid in [4, 5, 10, 14, 16, 18, 20, 22, 38] {
        let source = snapshot(pid);
        let before = source.to_json().unwrap();
        let plan = WholeRestore::new(&source, &source).unwrap_or_else(|e| panic!("PID {pid}: {e}"));
        assert_eq!(
            &plan.steps()[0].report.as_bytes()[..5],
            &[0x14, 0xc3, 0, 1, 1]
        );
        assert_eq!(plan.steps()[0].ack_timeout_ms, 2000);
        assert_eq!(
            &plan.steps().last().unwrap().report.as_bytes()[..5],
            &[0x14, 3, 0, 2, 1]
        );
        assert_eq!(plan.steps().last().unwrap().ack_timeout_ms, 2000);
        assert_eq!(plan.has_system(), pid != 14);
        assert_eq!(
            plan.preset_count(),
            usize::from(audition::slot_count(pid).unwrap())
        );
        plan.validate_destination(&source).unwrap();
        plan.matches_readback(&expected(&plan)).unwrap();
        assert_eq!(source.to_json().unwrap(), before);
        for step in plan
            .steps()
            .iter()
            .filter(|s| s.report.message_id() == 0x1c && s.report.as_bytes()[2] == 11)
        {
            assert_eq!(step.ack_timeout_ms, 0);
            assert_eq!(step.report.as_bytes()[1], 1);
        }
    }
}
#[test]
fn known_loader_changes_disclosed_and_global_mismatches_never_hidden() {
    let source = snapshot(20);
    let plan = WholeRestore::new(&source, &source).unwrap();
    assert!(
        plan.normalizations()
            .iter()
            .any(|c| c.kind == 10 && c.offset == 17 && c.before == 4 && c.after == 0)
    );
    assert!(!plan.warnings().is_empty());
    let good = expected(&plan);
    let mut reports = good.reports().to_vec();
    for r in &mut reports {
        let mut b = *r.as_bytes();
        if b[0] == 0x1c && b[2] == 10 && b[3] == 0 {
            b[17] = 4;
            b[18] = 250;
            *r = Report64::from_bytes(b);
        }
    }
    let reflected = Snapshot::from_reports(20, &reports).unwrap();
    assert!(plan.matches_readback(&reflected).is_err());
    assert!(plan.validate_destination(&good).is_err());
}
#[test]
fn unknown_system_preset_headers_and_reserved_fields_refuse_before_queue() {
    let source = snapshot(20);
    for (kind, offset, value) in [(11, 63, 1), (5, 63, 1), (5, 1, 9), (0, 63, 1)] {
        let mut reports = source.reports().to_vec();
        let r = reports
            .iter_mut()
            .find(|r| r.message_id() == 0x1c && r.as_bytes()[2] == kind && r.as_bytes()[3] == 0)
            .unwrap();
        let mut b = *r.as_bytes();
        b[offset] = value;
        *r = Report64::from_bytes(b);
        let changed = Snapshot::from_reports(20, &reports).unwrap();
        assert!(
            WholeRestore::new(&changed, &source).is_err(),
            "kind {kind} offset {offset}"
        );
        assert!(WholeRestore::new(&source, &changed).is_err());
    }
    assert!(WholeRestore::new(&source, &snapshot(4)).is_err());
}
fn legacy(source: &Snapshot, path: &std::path::Path) {
    std::fs::create_dir(path.join("Presets")).unwrap();
    for p in source.presets() {
        std::fs::write(
            path.join(format!("Presets/{:02}.fuse", p.slot())),
            p.document().to_xml().unwrap(),
        )
        .unwrap();
    }
    for e in source.entries() {
        let name = match e.key() {
            Key::System => "SystemSettings.fuse".into(),
            Key::Bank { control, slot } => format!(
                "Effects/{}/{slot:02}.fuse",
                if control == 1 { "Mod" } else { "DlyRvb" }
            ),
        };
        let file = path.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, e.xml().unwrap()).unwrap();
    }
}
#[test]
fn full_legacy_is_bound_to_all_original_files_and_unknown_or_incomplete_content_refused() {
    let source = snapshot(20);
    let root = tempfile::tempdir().unwrap();
    legacy(&source, root.path());
    let archive = Archive::read_path(root.path()).unwrap();
    let plan = WholeRestore::from_archive(&archive, 20, &source).unwrap();
    plan.matches_readback(&expected(&plan)).unwrap();
    std::fs::write(root.path().join("unrecognized.bin"), b"preserve").unwrap();
    assert!(
        WholeRestore::from_archive(&Archive::read_path(root.path()).unwrap(), 20, &source).is_err()
    );
    std::fs::remove_file(root.path().join("unrecognized.bin")).unwrap();
    std::fs::remove_file(root.path().join("Presets/03.fuse")).unwrap();
    assert!(
        WholeRestore::from_archive(&Archive::read_path(root.path()).unwrap(), 20, &source).is_err()
    );
}
#[test]
fn missing_clean_voice_stomp_and_extra_banks_are_rejected() {
    let source = snapshot(14);
    let mut reports = source.reports().to_vec();
    reports.retain(|r| {
        !(r.message_id() == 0x1c
            && r.as_bytes()[2] == 6
            && r.as_bytes()[3] == 1
            && r.as_bytes()[4] == 16)
    });
    let clean = Snapshot::from_reports(14, &reports).unwrap();
    assert!(WholeRestore::new(&clean, &source).is_err());
    let source = snapshot(20);
    let mut reports = source.reports().to_vec();
    let mut extra = source
        .entries()
        .iter()
        .find(|e| {
            matches!(
                e.key(),
                Key::Bank {
                    control: 1,
                    slot: 0
                }
            )
        })
        .unwrap()
        .raw_reports()
        .to_vec();
    for r in &mut extra {
        let mut b = *r.as_bytes();
        b[3] = 9;
        *r = Report64::from_bytes(b);
    }
    reports.pop();
    reports.extend(extra);
    reports.push(terminal());
    let extra = Snapshot::from_reports(20, &reports).unwrap();
    assert!(WholeRestore::new(&extra, &source).is_err());
}
#[test]
fn historical_unknown_scalar_attributes_mixed_text_and_duplicate_sound_containers_refuse() {
    let source = snapshot(20);
    let root = tempfile::tempdir().unwrap();
    legacy(&source, root.path());
    let file = root.path().join("Presets/00.fuse");
    let original = std::fs::read_to_string(&file).unwrap();
    for malformed in [
        original.replace("<UsbGain>", "<UsbGain Vendor=\"future\">"),
        original.replace("<UsbGain>9</UsbGain>", "<UsbGain>1<!--split-->2</UsbGain>"),
        original.replace("<FX>", "<FX>unrecognized text"),
        original.replace("</FX>", "</FX><FX/>"),
        original.replace("<Module ", "<Module Vendor=\"future\" "),
        original.replace("</Amplifier>", "</Amplifier><Amplifier ID=\"0\"/>"),
    ] {
        std::fs::write(&file, &malformed).unwrap();
        if let Ok(archive) = Archive::read_path(root.path()) {
            assert!(WholeRestore::from_archive(&archive, 20, &source).is_err());
        }
        assert_eq!(std::fs::read_to_string(&file).unwrap(), malformed);
    }
}
