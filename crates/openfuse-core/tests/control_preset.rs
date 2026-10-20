use openfuse_core::{
    auxiliary::{self, Key, Snapshot},
    control_preset::ControlPreset,
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument,
        expression::{ExpressionPedal, PedalSlot},
    },
    protocol::Report64,
};
fn full(pid: u16) -> PresetDocument {
    let product = openfuse_core::wire_preset::product_for_pid(pid).unwrap();
    let mut xml = format!("<Preset ProductId=\"{product}\">");
    for (kind, key, category) in [
        (5u8, "Amplifier", ModuleCategory::Amplifier),
        (6, "Stompbox", ModuleCategory::Stompbox),
        (7, "Modulation", ModuleCategory::Modulation),
        (8, "Delay", ModuleCategory::Delay),
        (9, "Reverb", ModuleCategory::Reverb),
    ] {
        if kind == 6 {
            xml.push_str("<FX>");
        }
        let model = model_catalog::models(product, category)
            .find(|m| m.id != 0)
            .unwrap();
        xml.push_str(&format!(
            "<{key} ID=\"{}\"><Module ID=\"{}\" POS=\"{}\" BypassState=\"1\" BypassMode=\"{}\">",
            kind - 5,
            model.id,
            kind.saturating_sub(6),
            model.bypass_mode
        ));
        for p in &model.parameters {
            xml.push_str(&format!(
                "<Param ControlIndex=\"{}\">{}</Param>",
                p.index, p.default_raw
            ));
        }
        xml.push_str(&format!("</Module></{key}>"));
    }
    xml.push_str("</FX><UsbGain>19</UsbGain><FUSE><Info name=\"Current sound\"/></FUSE></Preset>");
    let mut result = PresetDocument::parse(&xml).unwrap();
    if pid != 10 {
        result
            .set_expression_pedal(
                PedalSlot::First,
                &ExpressionPedal::defaults(PedalSlot::First),
            )
            .unwrap();
    }
    result
}
#[test]
fn constructors_cover_every_evidenced_physical_bank() {
    for pid in [4, 20, 16, 10, 38, 14] {
        for (control, count) in auxiliary::capabilities(pid).unwrap().banks {
            let doc = ControlPreset::new(pid, control, "New sound").unwrap();
            assert_eq!(doc.pid(), pid);
            assert_eq!(doc.control(), control);
            assert_eq!(doc.name(), "New sound");
            assert!(doc.write_plan(0).is_ok());
            assert!(doc.write_plan(count).is_err());
        }
    }
    assert!(ControlPreset::new(5, 1, "No bank").is_err());
}
#[test]
fn mini_bronco_passport_current_fx_excludes_stomp_and_amp() {
    for pid in [16, 10, 38] {
        let doc = ControlPreset::from_preset(pid, 2, &full(pid)).unwrap();
        assert_eq!(
            doc.modules()
                .iter()
                .map(|m| m.key.as_str())
                .collect::<Vec<_>>(),
            vec!["Modulation", "Delay", "Reverb"]
        );
        assert!(
            doc.write_plan(0)
                .unwrap()
                .iter()
                .all(|r| r.as_bytes()[2] != 6)
        );
    }
}
#[test]
fn selected_projection_uses_model_zero_and_does_not_mutate_source() {
    let current = full(14);
    let before = current.to_xml().unwrap();
    let doc = ControlPreset::from_selected_preset(14, 2, None, &current, &["Delay"]).unwrap();
    assert_eq!(
        doc.modules()
            .iter()
            .find(|m| m.key == "Modulation")
            .unwrap()
            .model_id,
        Some(0)
    );
    assert_ne!(
        doc.modules()
            .iter()
            .find(|m| m.key == "Delay")
            .unwrap()
            .model_id,
        Some(0)
    );
    assert_eq!(before, current.to_xml().unwrap());
    assert!(ControlPreset::from_selected_preset(14, 2, None, &current, &["Stompbox"]).is_err());
    assert!(
        ControlPreset::from_selected_preset(14, 2, None, &current, &["Delay", "Delay"]).is_err()
    );
}
#[test]
fn clean_voice_is_structural_and_has_no_stomp_packets() {
    let clean = ControlPreset::new_for_slot(14, 1, 16, "Clean").unwrap();
    assert!(clean.is_clean_voice());
    assert_eq!(clean.modules().len(), 1);
    assert!(
        clean
            .write_plan(16)
            .unwrap()
            .iter()
            .all(|r| r.as_bytes()[2] != 6)
    );
    assert!(clean.write_plan(0).is_err());
    let source =
        ControlPreset::from_selected_preset(14, 1, Some(16), &full(14), &["Amplifier"]).unwrap();
    assert!(source.is_clean_voice());
}
#[test]
fn source_xml_extensions_and_comments_survive_known_edits() {
    let xml = "<?xml version=\"1.0\"?><FX Name='Old' vendor='opaque' xmlns:v='urn:test'><!--keep--><Modulation ID='2'><Module ID='0' POS='1' BypassState='1' vendor='stay'/></Modulation><v:Data>untouched</v:Data></FX>";
    let mut doc = ControlPreset::parse(20, 1, xml).unwrap();
    assert_eq!(doc.to_xml().unwrap(), xml);
    doc.set_name("New & bright").unwrap();
    doc.set_module_active("Modulation", false).unwrap();
    let after = doc.to_xml().unwrap();
    assert!(
        after.contains("vendor='opaque'")
            && after.contains("vendor='stay'")
            && after.contains("<!--keep-->")
            && after.contains("<v:Data>untouched</v:Data>")
    );
    assert!(doc.write_plan(0).is_err());
}
#[test]
fn model_and_parameter_edits_use_catalog_and_reject_atomic_errors() {
    let mut doc = ControlPreset::new(20, 1, "MOD").unwrap();
    let model = model_catalog::models(13, ModuleCategory::Modulation)
        .find(|m| m.id != 0)
        .unwrap();
    doc.replace_module_model("Modulation", model.id).unwrap();
    let p = &model.parameters[0];
    doc.set_parameter("Modulation", p.index, *p.raw_range().start())
        .unwrap();
    let before = doc.to_xml().unwrap();
    assert!(doc.set_parameter("Modulation", 255, 10).is_err());
    assert!(doc.replace_module_model("Amplifier", 0).is_err());
    assert_eq!(doc.to_xml().unwrap(), before);
    assert!(doc.write_plan(0).is_ok());
}
#[test]
fn unknown_parameter_extensions_block_destructive_model_replacement() {
    let model = model_catalog::models(13, ModuleCategory::Modulation)
        .find(|m| !m.parameters.is_empty())
        .unwrap();
    let mut doc = ControlPreset::new(20, 1, "MOD").unwrap();
    doc.replace_module_model("Modulation", model.id).unwrap();
    let xml = doc
        .to_xml()
        .unwrap()
        .replacen("<Param ", "<Param vendor=\"retain\" ", 1);
    let mut doc = ControlPreset::parse(20, 1, &xml).unwrap();
    assert!(doc.replace_module_model("Modulation", 0).is_err());
    assert_eq!(doc.to_xml().unwrap(), xml);
}
#[test]
fn gain_and_expression_are_profile_scoped() {
    let mut fx = ControlPreset::new(14, 2, "FX").unwrap();
    assert!(fx.set_usb_gain(1).is_err());
    let mut pedal = fx.expression_pedal().unwrap().unwrap();
    pedal.heel_setting = Some(321);
    fx.set_expression_pedal(&pedal).unwrap();
    assert_eq!(
        fx.expression_pedal().unwrap().unwrap().heel_setting,
        Some(321)
    );
    let mut voice = ControlPreset::new(14, 1, "Voice").unwrap();
    voice.set_usb_gain(255).unwrap();
    assert_eq!(voice.usb_gain().unwrap(), Some(255));
    assert!(voice.set_expression_pedal(&pedal).is_err());
    pedal.expression_mode_behavior = Some(1);
    assert!(fx.set_expression_pedal(&pedal).is_err());
}
#[test]
fn json_carries_exact_profile_and_detects_corruption() {
    let doc = ControlPreset::new(20, 1, "Save").unwrap();
    let json = doc.to_json().unwrap();
    let loaded = ControlPreset::parse_json(&json).unwrap();
    assert_eq!(loaded.to_xml().unwrap(), doc.to_xml().unwrap());
    assert!(ControlPreset::parse_json(&json.replace("Save", "Changed")).is_err());
    assert!(ControlPreset::parse(14, 1, &doc.to_xml().unwrap()).is_err());
}
fn snapshot() -> Snapshot {
    let mut reports = vec![];
    for (control, count) in [(1, 17), (2, 15)] {
        for slot in 0..count {
            reports.extend(
                ControlPreset::new_for_slot(14, control, slot, "Target")
                    .unwrap()
                    .write_plan(slot)
                    .unwrap(),
            );
        }
    }
    let mut end = [0; 64];
    end[0] = 0x14;
    end[1] = 1;
    end[3] = 2;
    end[4] = 1;
    reports.push(Report64::from_bytes(end));
    Snapshot::from_reports(14, &reports).unwrap()
}
#[test]
fn edited_document_uses_guarded_existing_selection_without_fake_capture() {
    let mut doc = ControlPreset::new(14, 2, "Edited").unwrap();
    doc.set_module_active("Delay", false).unwrap();
    let target = snapshot();
    let plan = doc.plan(&target, 7).unwrap();
    assert_eq!(
        plan.mappings()[0].target,
        Key::Bank {
            control: 2,
            slot: 7
        }
    );
    assert_eq!(plan.steps(), doc.write_plan(7).unwrap());
    assert!(plan.matches_readback(&target).is_err());
    plan.validate_destination(&target).unwrap();
    let serialized = doc.to_json().unwrap();
    assert!(!serialized.contains("reports"));
}
#[test]
fn raw_capture_unknown_fields_cannot_be_erased_by_editor_conversion() {
    let target = snapshot();
    let mut raw = target.reports().to_vec();
    let mut bytes = *raw[1].as_bytes();
    bytes[15] = 128;
    raw[1] = Report64::from_bytes(bytes);
    let capture = Snapshot::from_reports(14, &raw).unwrap();
    assert!(ControlPreset::from_entry(14, &capture.entries()[0]).is_err());
}

#[test]
fn corrected_mini_profile_collects_three_effect_modules_without_inventing_stomp() {
    let pid = 16;
    let mut reports = vec![];
    let current = openfuse_core::wire_preset::encode(pid, &full(pid))
        .unwrap()
        .reports;
    for slot in 0u16..24 {
        for report in current.iter().filter(|r| r.message_id() == 0x1c) {
            let mut b = *report.as_bytes();
            if b[2] == 4 {
                b[4..6].copy_from_slice(&slot.to_le_bytes());
                b[6] = 0;
            }
            reports.push(Report64::from_bytes(b));
        }
    }
    let system = format!(
        "<System>{}<LCDContrast>0</LCDContrast><MemoryLock>0</MemoryLock><Foot2BtnMode>0</Foot2BtnMode><ExpPedalVolumeHeel>0</ExpPedalVolumeHeel><ExpPedalVolumeToe>255</ExpPedalVolumeToe></System>",
        "<QA>0</QA>".repeat(16)
    );
    reports.push(auxiliary::decode_system_xml(&system).unwrap());
    for slot in 0..12 {
        reports.extend(
            ControlPreset::new(pid, 2, "Three FX")
                .unwrap()
                .write_plan(slot)
                .unwrap(),
        );
    }
    let mut end = [0; 64];
    end[0] = 0x14;
    end[1] = 1;
    end[3] = 2;
    end[4] = 1;
    reports.push(Report64::from_bytes(end));
    let capture = Snapshot::from_reports(pid, &reports).unwrap();
    assert_eq!(capture.presets().len(), 24);
    assert_eq!(capture.entries().len(), 13);
    assert!(
        capture
            .entries()
            .iter()
            .filter(|e| matches!(e.key(), Key::Bank { .. }))
            .all(|e| e.restore_error().is_none())
    );
}

#[test]
fn voice_amp_parameter_rules_commit_as_one_atomic_edit() {
    let mut doc = ControlPreset::new(14, 1, "Voice rules").unwrap();
    doc.set_parameter("Amplifier", 15, 1).unwrap();
    let modules = doc.modules();
    let amp = modules.iter().find(|m| m.key == "Amplifier").unwrap();
    assert_eq!(
        amp.parameters.iter().find(|p| p.index == 16).unwrap().value,
        2
    );
    assert_eq!(
        amp.parameters.iter().find(|p| p.index == 9).unwrap().value,
        0
    );
}
