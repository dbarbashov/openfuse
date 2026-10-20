use openfuse_core::{
    audition,
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument, PresetParameter,
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
fn change(report: &mut Report64, index: usize, value: u8) {
    let mut b = *report.as_bytes();
    b[index] = value;
    *report = Report64::from_bytes(b);
}
fn replacement(doc: &mut PresetDocument, key: &str, id: u16) {
    let category = ModuleCategory::from_key(key).unwrap();
    let model = model_catalog::model(doc.product_id().unwrap(), category, id).unwrap();
    let parameters: Vec<_> = model
        .parameters
        .iter()
        .map(|p| PresetParameter {
            index: p.index,
            value: p.default_raw,
        })
        .collect();
    doc.replace_module_model(key, id, &parameters, Some(model.bypass_mode))
        .unwrap();
}
fn remove_aux(doc: &PresetDocument, keys: &[&str]) -> PresetDocument {
    let source = doc.to_xml().unwrap();
    let parsed = roxmltree::Document::parse(&source).unwrap();
    let mut ranges: Vec<_> = parsed
        .root_element()
        .children()
        .filter(|n| keys.contains(&n.tag_name().name()))
        .map(|n| n.range())
        .collect();
    ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut source = source;
    for range in ranges {
        source.replace_range(range, "");
    }
    PresetDocument::parse(&source).unwrap()
}

#[test]
fn all_profiles_build_exact_host_headers_and_never_emit_persistent_or_sync_commands() {
    for pid in [4, 5, 0xa, 0xe, 0x10, 0x12, 0x14, 0x16, 0x26] {
        let doc = document(pid);
        let raw = current(pid, &doc);
        let plan = audition::preset(pid, &doc, &raw).unwrap();
        let steps = plan.steps();
        let product = doc.product_id().unwrap();
        assert_eq!(
            steps.len(),
            if product == 11 {
                8
            } else if product == 7 {
                11
            } else {
                9
            }
        );
        assert_eq!(&steps[0].as_bytes()[..8], &[0x1c, 1, 4, 0, 7, 0, 1, 1]);
        for (offset, kind) in (5..=9).enumerate() {
            assert_eq!(
                &steps[offset + 1].as_bytes()[..8],
                &[0x1c, 3, kind, 0, 0, 0, 1, 1]
            );
        }
        let mut complete = [0; 64];
        complete[0] = 0x1c;
        complete[1] = 3;
        assert_eq!(steps.last().unwrap().as_bytes(), &complete);
        for report in steps {
            assert_eq!(report.message_id(), 0x1c);
            assert!(!matches!(report.as_bytes()[2], 1..=3));
            if report.as_bytes()[2] == 10 {
                assert_eq!(&report.as_bytes()[4..8], &[0, 0, 0, 0]);
                assert_eq!(&report.as_bytes()[17..19], &[4, 250]);
            }
        }
        plan.matches_readback(steps).unwrap();
    }
}
#[test]
fn preview_is_immutable_and_quantization_is_explained_before_send() {
    let mut doc = document(0x14);
    doc.set_parameter("Amplifier", 1, 0xabcd).unwrap();
    let raw = current(0x14, &document(0x14));
    let xml = doc.to_xml().unwrap();
    let original_raw = raw.clone();
    let plan = audition::preset(0x14, &doc, &raw).unwrap();
    assert_eq!(doc.to_xml().unwrap(), xml);
    assert_eq!(raw, original_raw);
    assert!(plan.warnings().iter().any(|s| s.contains("low 8 bits")));
    assert!(
        plan.warnings()
            .iter()
            .any(|s| s.starts_with("Only sound state is sent"))
    );
    assert!(!plan.warnings().iter().any(|s| s.contains("Rehearsal")));
    assert_eq!(plan.steps()[1].as_bytes()[33], 0xab);
}
#[test]
fn absent_file_auxiliary_inherits_device_state_and_preserves_global_endpoints() {
    let doc = document(0x12);
    let raw = current(0x12, &doc);
    let sparse = remove_aux(
        &doc,
        &[
            "FirstExpressionPedal",
            "SecondExpressionPedal",
            "UsbGain",
            "FxLoopBypass",
        ],
    );
    let plan = audition::preset(0x12, &sparse, &raw).unwrap();
    assert_eq!(
        plan.warnings()
            .iter()
            .filter(|s| s.contains("absent from the file"))
            .count(),
        4
    );
    for report in plan.steps().iter().filter(|r| r.as_bytes()[2] == 10) {
        assert_eq!(&report.as_bytes()[17..19], &[4, 250]);
    }
    assert_eq!(
        plan.steps()
            .iter()
            .find(|r| r.as_bytes()[2] == 13)
            .unwrap()
            .as_bytes()[16],
        9
    );
}
#[test]
fn last_received_global_pair_is_used_for_both_floor_pedals_with_notice() {
    let doc = document(0x12);
    let mut raw = current(0x12, &doc);
    let second = raw
        .iter_mut()
        .find(|r| r.as_bytes()[2] == 10 && r.as_bytes()[25] == 1)
        .unwrap();
    change(second, 17, 8);
    change(second, 18, 231);
    let plan = audition::preset(0x12, &doc, &raw).unwrap();
    for r in plan.steps().iter().filter(|r| r.as_bytes()[2] == 10) {
        assert_eq!(&r.as_bytes()[17..19], &[8, 231]);
    }
    assert!(
        plan.warnings()
            .iter()
            .any(|s| s.contains("last received pair"))
    );
}
#[test]
fn missing_globals_or_required_auxiliary_never_falls_back_to_zero() {
    let doc = document(0x14);
    let raw = current(0x14, &doc);
    let no_expression: Vec<_> = raw
        .iter()
        .copied()
        .filter(|r| r.as_bytes()[2] != 10)
        .collect();
    assert!(
        audition::preset(0x14, &doc, &no_expression)
            .unwrap_err()
            .to_string()
            .contains("global volume")
    );
    let sparse = remove_aux(&doc, &["UsbGain"]);
    let no_gain: Vec<_> = raw
        .iter()
        .copied()
        .filter(|r| r.as_bytes()[2] != 13)
        .collect();
    assert!(
        audition::preset(0x14, &sparse, &no_gain)
            .unwrap_err()
            .to_string()
            .contains("UsbGain")
    );
}
#[test]
fn readback_requires_models_values_names_slots_and_raw_global_volume_to_match() {
    let doc = document(0x14);
    let raw = current(0x14, &doc);
    let plan = audition::preset(0x14, &doc, &raw).unwrap();
    for (kind, index) in [
        (4, 16),
        (4, 4),
        (5, 16),
        (5, 33),
        (10, 17),
        (10, 18),
        (13, 16),
    ] {
        let mut readback = plan.steps().to_vec();
        let report = readback
            .iter_mut()
            .find(|r| r.as_bytes()[2] == kind)
            .unwrap();
        let new = report.as_bytes()[index] ^ 1;
        change(report, index, new);
        assert!(
            plan.matches_readback(&readback).is_err(),
            "kind{kind} byte{index}"
        );
    }
}
#[test]
fn slot_select_has_distinct_mustang_opcode_and_device_specific_bounds() {
    for pid in [4, 5, 0xa, 0xe, 0x10, 0x12, 0x14, 0x16, 0x26] {
        let count = audition::slot_count(pid).unwrap();
        assert_eq!(
            count,
            if pid == 0xe {
                0
            } else if [5, 0x12, 0x16].contains(&pid) {
                100
            } else {
                24
            }
        );
        assert!(audition::slot_select(pid, count).is_err());
        if count > 0 {
            for slot in [0, count - 1] {
                let plan = audition::slot_select(pid, slot).unwrap();
                assert_eq!(plan.steps().len(), 1);
                let bytes = plan.steps()[0].as_bytes();
                assert_eq!(&bytes[..4], &[0x1c, 0xc3, 1, 0]);
                assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), slot);
                assert_eq!(&bytes[6..8], &[1, 0]);
                assert!(bytes[8..].iter().all(|b| *b == 0));
                assert!(plan.expected().is_none());
            }
        }
    }
    assert!(audition::slot_count(2).is_err());
}
#[test]
fn slot_readback_cannot_accept_the_previous_slot() {
    let doc = document(0x14);
    let mut raw = current(0x14, &doc);
    let plan = audition::slot_select(0x14, 2).unwrap();
    assert!(plan.matches_readback(&raw).is_err());
    change(&mut raw[0], 4, 2);
    plan.matches_readback(&raw).unwrap();
}
#[test]
fn unknown_family_or_missing_current_snapshot_fails_before_a_plan_exists() {
    let doc = document(0x14);
    let raw = current(0x14, &doc);
    assert!(audition::preset(4, &doc, &raw).is_err());
    assert!(audition::preset(0x14, &doc, &[]).is_err());
}
#[test]
fn model_plan_changes_only_selected_module_and_uses_module_then_complete() {
    let original = document(0x14);
    let raw = current(0x14, &original);
    let mut desired = original.clone();
    replacement(&mut desired, "Delay", 43);
    desired.set_parameter("Amplifier", 1, 12345).unwrap();
    let plan = audition::module(0x14, &desired, &raw, "Delay").unwrap();
    assert_eq!(plan.steps().len(), 2);
    assert_eq!(&plan.steps()[0].as_bytes()[..3], &[0x1c, 3, 8]);
    assert_eq!(&plan.steps()[1].as_bytes()[..3], &[0x1c, 3, 0]);
    let before = wire_preset::decode(0x14, &raw).unwrap().document;
    let actual = plan.expected().unwrap();
    assert_eq!(before.modules()[0], actual.modules()[0]);
    assert_eq!(
        actual
            .modules()
            .iter()
            .find(|m| m.key == "Delay")
            .unwrap()
            .model_id,
        Some(43)
    );
}
#[test]
fn bypass_plan_uses_effect_state_command_and_preserves_other_state() {
    let doc = document(0x14);
    let raw = current(0x14, &doc);
    let plan = audition::bypass(0x14, &raw, "Amplifier", false).unwrap();
    assert_eq!(&plan.steps()[0].as_bytes()[..5], &[0x19, 0xc3, 2, 1, 0]);
    assert_eq!(plan.expected().unwrap().modules()[0].active, Some(false));
    assert!(audition::bypass(0x14, &raw, "Unknown", true).is_err());
}
#[test]
fn routing_plan_is_atomic_and_rejects_collisions_and_mode_dependent_positions() {
    let mut doc = document(0x14);
    for (key, id) in [
        ("Stompbox", 60),
        ("Modulation", 18),
        ("Delay", 43),
        ("Reverb", 33),
    ] {
        replacement(&mut doc, key, id);
    }
    let raw = current(0x14, &doc);
    let plan = audition::routing(0x14, &raw, [1, 0, 5, 4]).unwrap();
    assert_eq!(&plan.steps()[0].as_bytes()[..6], &[0x1b, 0xc3, 1, 0, 5, 4]);
    assert!(audition::routing(0x14, &raw, [0, 0, 4, 5]).is_err());
    assert!(audition::routing(0x14, &raw, [0, 1, 4, 8]).is_err());
    let sc = document(0xe);
    assert!(audition::routing(0xe, &current(0xe, &sc), [0, 1, 4, 5]).is_err());
}
#[test]
fn full_and_scoped_audition_reject_invalid_resulting_chains() {
    let mut doc = document(0x14);
    replacement(&mut doc, "Stompbox", 60);
    replacement(&mut doc, "Modulation", 18);
    let raw = current(0x14, &doc);
    doc.set_module_position("Modulation", 0).unwrap();
    assert!(audition::preset(0x14, &doc, &raw).is_err());
    assert!(audition::module(0x14, &doc, &raw, "Modulation").is_err());
    doc.set_module_position("Modulation", 8).unwrap();
    assert!(audition::preset(0x14, &doc, &raw).is_err());
    let mut sc = document(0xe);
    let current = current(0xe, &sc);
    sc.set_module_position("Modulation", 2).unwrap();
    assert!(
        audition::preset(0xe, &sc, &current)
            .unwrap_err()
            .to_string()
            .contains("Super Champ")
    );
}
#[test]
fn inherited_expression_cannot_target_empty_or_missing_parameters() {
    let doc = document(0x14);
    let mut raw = current(0x14, &doc);
    let first = raw.iter_mut().find(|r| r.as_bytes()[2] == 10).unwrap();
    change(first, 20, 2);
    let sparse = remove_aux(&doc, &["FirstExpressionPedal"]);
    assert!(
        audition::preset(0x14, &sparse, &raw)
            .unwrap_err()
            .to_string()
            .contains("empty module")
    );
    let mut raw = current(0x14, &doc);
    let first = raw.iter_mut().find(|r| r.as_bytes()[2] == 10).unwrap();
    change(first, 20, 1);
    let amp = raw.iter_mut().find(|r| r.as_bytes()[2] == 5).unwrap();
    change(amp, 53, 255);
    assert!(
        audition::preset(0x14, &sparse, &raw)
            .unwrap_err()
            .to_string()
            .contains("absent from")
    );
}
#[test]
fn removing_an_expression_target_requires_an_explicit_new_assignment() {
    let mut doc = document(0x14);
    replacement(&mut doc, "Delay", 43);
    let mut raw = current(0x14, &doc);
    let first = raw.iter_mut().find(|r| r.as_bytes()[2] == 10).unwrap();
    change(first, 20, 4);
    let mut desired = doc;
    replacement(&mut desired, "Delay", 0);
    assert!(audition::module(0x14, &desired, &raw, "Delay").is_err());
}
