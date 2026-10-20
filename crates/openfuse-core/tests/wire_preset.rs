use openfuse_core::{
    model_catalog::{self, ModuleCategory},
    preset::{PresetDocument, PresetMetadata, expression::PedalSlot},
    protocol::Report64,
    wire_preset::{self, decode, encode, parameter_change},
};

fn modify(report: &mut Report64, changes: &[(usize, u8)]) {
    let mut b = *report.as_bytes();
    for &(index, value) in changes {
        b[index] = value;
    }
    *report = Report64::from_bytes(b);
}
fn capture(pid: u16) -> Vec<Report64> {
    let product = wire_preset::product_for_pid(pid).unwrap();
    let mut b = [0; 64];
    b[0] = 0x1c;
    b[1] = 0x81;
    b[2] = 4;
    b[6] = 1;
    b[16..20].copy_from_slice(b"Tone");
    let mut reports = vec![Report64::from_bytes(b)];
    for (kind, category, position) in [
        (5, ModuleCategory::Amplifier, 0),
        (6, ModuleCategory::Stompbox, 0),
        (7, ModuleCategory::Modulation, 1),
        (8, ModuleCategory::Delay, 4),
        (9, ModuleCategory::Reverb, 5),
    ] {
        let model_id = if kind == 5 {
            model_catalog::product(product).unwrap().default_amp_id
        } else {
            0
        };
        let model = model_catalog::model(product, category, model_id).unwrap();
        let mut b = [0; 64];
        b[0] = 0x1c;
        b[1] = 0xad;
        b[2] = kind;
        b[6] = 1;
        b[16..18].copy_from_slice(&model_id.to_le_bytes());
        for parameter in &model.parameters {
            b[32 + usize::from(parameter.index)] = if parameter.type_id < 128 {
                (parameter.default_raw >> 8) as u8
            } else {
                parameter.default_raw as u8
            };
        }
        if kind == 5 {
            b[52] &= !0x40;
            b[53] = model.expression_pedal_index;
        } else {
            b[18] = position;
            b[19] = model.expression_pedal_index;
            b[20] = model.tap_parameter_index;
            b[21] = model.bypass_mode;
        }
        reports.push(Report64::from_bytes(b));
    }
    let mut b = [0; 64];
    b[0] = 0x1c;
    reports.push(Report64::from_bytes(b));
    reports
}
fn aux(kind: u8) -> Report64 {
    let mut b = [0; 64];
    b[0] = 0x1c;
    b[1] = 0x99;
    b[2] = kind;
    Report64::from_bytes(b)
}
fn insert(reports: &mut Vec<Report64>, report: Report64) {
    reports.insert(reports.len() - 1, report);
}
fn module(doc: &PresetDocument, key: &str) -> openfuse_core::preset::PresetModule {
    doc.modules().into_iter().find(|m| m.key == key).unwrap()
}

#[test]
fn all_nine_pid_profiles_decode_complete_editable_presets_and_rehearse() {
    for pid in [4, 5, 0xa, 0xe, 0x10, 0x12, 0x14, 0x16, 0x26] {
        let reports = capture(pid);
        let result = decode(pid, &reports).unwrap();
        assert_eq!(
            result.document.product_id(),
            wire_preset::product_for_pid(pid)
        );
        assert_eq!(result.document.metadata().name, "Tone");
        assert_eq!(result.raw_reports, reports);
        assert_eq!(result.document.modules().len(), 5);
        let encoded = encode(pid, &result.document).unwrap();
        assert_eq!(encoded.reports.len(), 8);
        assert_eq!(encoded.reports.last().unwrap().message_id(), 0xff);
        assert!(decode(pid, &encoded.reports).is_ok());
    }
}
#[test]
fn unknown_pid_or_wrong_document_family_is_rejected() {
    let reports = capture(0x14);
    for pid in [2, 3, 6, 0xc, 0xffff] {
        assert!(decode(pid, &reports).is_err());
    }
    let doc = decode(0x14, &reports).unwrap().document;
    assert!(encode(4, &doc).is_err());
    assert!(parameter_change(4, &doc, "Amplifier", 0).is_err());
}
#[test]
fn partial_duplicate_and_interleaved_current_blocks_fail_without_defaults() {
    let base = capture(0x14);
    for missing in 0..base.len() {
        let mut reports = base.clone();
        reports.remove(missing);
        assert!(decode(0x14, &reports).is_err(), "missing {missing}");
    }
    let mut duplicate = base.clone();
    duplicate.insert(2, base[1]);
    assert!(decode(0x14, &duplicate).is_err());
    let mut interleaved = base.clone();
    let mut name = base[0];
    modify(&mut name, &[(6, 0)]);
    interleaved.insert(2, name);
    assert!(decode(0x14, &interleaved).is_err());
    let mut after_complete = base.clone();
    after_complete.push(base[1]);
    assert!(decode(0x14, &after_complete).is_err());
}
#[test]
fn unknown_model_is_not_replaced_with_catalog_defaults() {
    let mut reports = capture(0x14);
    modify(&mut reports[1], &[(16, 255), (17, 255)]);
    assert!(
        decode(0x14, &reports)
            .unwrap_err()
            .to_string()
            .contains("Unknown model")
    );
}
#[test]
fn raw_unknowns_and_reserved_bytes_survive_without_mutating_input() {
    let mut reports = capture(0x14);
    modify(&mut reports[3], &[(1, 0xff), (63, 0xd7)]);
    let unknown = aux(0xee);
    insert(&mut reports, unknown);
    let before = reports.clone();
    let decoded = decode(0x14, &reports).unwrap();
    assert_eq!(decoded.raw_reports, before);
    assert_eq!(reports, before);
    assert!(decoded.warnings.iter().any(|s| s.contains("238")));
}
#[test]
fn names_follow_latin_byte_nul_and_space_rules_with_xml_escaping() {
    let mut reports = capture(0x14);
    modify(
        &mut reports[0],
        &[
            (16, b'&'),
            (17, 0xe9),
            (18, b'\"'),
            (19, b' '),
            (20, 0),
            (21, b'X'),
        ],
    );
    let doc = decode(0x14, &reports).unwrap().document;
    assert_eq!(doc.metadata().name, "&é\"");
    let encoded = encode(0x14, &doc).unwrap();
    assert_eq!(&encoded.reports[0].as_bytes()[16..19], &[b'&', 0xe9, b'\"']);
    modify(&mut reports[0], &[(16, 1)]);
    assert!(decode(0x14, &reports).is_err());
}
#[test]
fn names_that_need_truncation_or_unicode_loss_are_rejected() {
    for name in [
        "x".repeat(49),
        "\u{413}\u{438}\u{442}\u{430}\u{440}\u{430}".into(),
    ] {
        let mut doc = decode(0x14, &capture(0x14)).unwrap().document;
        doc.set_metadata(&PresetMetadata {
            name,
            ..doc.metadata()
        })
        .unwrap();
        assert!(encode(0x14, &doc).is_err());
    }
}
#[test]
fn continuous_packed_values_quantize_but_live_parameter_keeps_full_sixteen_bits() {
    let mut doc = decode(0x14, &capture(0x14)).unwrap().document;
    doc.set_parameter("Amplifier", 1, 0xabcd).unwrap();
    let command = parameter_change(0x14, &doc, "Amplifier", 1).unwrap();
    assert_eq!(command.wire_value, 0xabcd);
    assert_eq!(command.module_type, 2);
    let encoded = encode(0x14, &doc).unwrap();
    assert_eq!(encoded.reports[1].as_bytes()[33], 0xab);
    assert!(
        encoded
            .warnings
            .iter()
            .any(|s| s.contains("lose their low 8 bits"))
    );
    let read = decode(0x14, &encoded.reports).unwrap().document;
    assert_eq!(
        module(&read, "Amplifier")
            .parameters
            .iter()
            .find(|p| p.index == 1)
            .unwrap()
            .value,
        0xab00
    );
}
#[test]
fn effect_and_amplifier_activation_have_distinct_wire_polarities() {
    let mut reports = capture(0x14);
    modify(&mut reports[1], &[(52, 0x40)]);
    modify(&mut reports[2], &[(22, 2)]);
    let doc = decode(0x14, &reports).unwrap().document;
    assert_eq!(module(&doc, "Amplifier").active, Some(false));
    assert_eq!(module(&doc, "Stompbox").active, Some(false));
    let encoded = encode(0x14, &doc).unwrap();
    assert_ne!(encoded.reports[1].as_bytes()[52] & 0x40, 0);
    assert_eq!(encoded.reports[2].as_bytes()[22], 1);
}
#[test]
fn superchamp_position_normalization_is_isolated_and_does_not_change_raw_capture() {
    let mut reports = capture(0xe);
    modify(&mut reports[3], &[(18, 4)]);
    let raw = reports.clone();
    let decoded = decode(0xe, &reports).unwrap();
    assert_eq!(module(&decoded.document, "Modulation").position, Some(5));
    assert_eq!(decoded.raw_reports, raw);
    assert_eq!(
        module(&decode(0x14, &capture(0x14)).unwrap().document, "Delay").position,
        Some(4)
    );
}
#[test]
fn first_expression_binding_comes_from_associated_module_not_generic_index() {
    let mut reports = capture(0x14);
    modify(&mut reports[1], &[(53, 6)]);
    let mut pedal = aux(10);
    modify(
        &mut pedal,
        &[
            (16, 1),
            (17, 5),
            (18, 240),
            (19, 255),
            (20, 1),
            (21, 17),
            (22, 3),
            (23, 253),
            (24, 1),
            (26, 1),
            (27, 1),
            (28, 1),
        ],
    );
    insert(&mut reports, pedal);
    let decoded = decode(0x14, &reports).unwrap();
    let state = decoded
        .document
        .expression_pedal(PedalSlot::First)
        .unwrap()
        .unwrap();
    assert_eq!(state.parameter_index, Some(6));
    assert_eq!(state.heel_setting, Some(768));
    assert_eq!(state.toe_setting, Some(64768));
    assert_eq!(state.bypass_effect_when_volume_mode, Some(1));
    let encoded = encode(0x14, &decoded.document).unwrap();
    let pedal = encoded
        .reports
        .iter()
        .find(|r| r.as_bytes()[2] == 10)
        .unwrap();
    assert_eq!(pedal.as_bytes()[21], 0);
    assert_eq!(&pedal.as_bytes()[17..19], &[0, 0]);
    assert!(
        encoded
            .warnings
            .iter()
            .any(|s| s.contains("must not be sent"))
    );
}
#[test]
fn dual_expression_usb_gain_and_loop_preserve_known_state() {
    let mut reports = capture(0x12);
    let mut first = aux(10);
    modify(&mut first, &[(16, 1), (20, 6), (23, 255)]);
    insert(&mut reports, first);
    let mut second = aux(10);
    modify(
        &mut second,
        &[
            (16, 255),
            (20, 4),
            (21, 7),
            (22, 20),
            (23, 250),
            (25, 1),
            (28, 255),
        ],
    );
    insert(&mut reports, second);
    let mut gain = aux(13);
    modify(&mut gain, &[(16, 231)]);
    insert(&mut reports, gain);
    let mut fxloop = aux(14);
    modify(&mut fxloop, &[(16, 3)]);
    insert(&mut reports, fxloop);
    let decoded = decode(0x12, &reports).unwrap();
    let second = decoded
        .document
        .expression_pedal(PedalSlot::Second)
        .unwrap()
        .unwrap();
    assert_eq!(second.parameter_index, Some(7));
    assert_eq!(second.volume_mode_behavior, None);
    let xml = decoded.document.to_xml().unwrap();
    assert!(xml.contains("<UsbGain>231</UsbGain>"));
    assert!(xml.contains("<FxLoopBypass>true</FxLoopBypass>"));
    let encoded = encode(0x12, &decoded.document).unwrap();
    let second = encoded
        .reports
        .iter()
        .find(|r| r.as_bytes()[2] == 10 && r.as_bytes()[25] == 1)
        .unwrap();
    assert_eq!(second.as_bytes()[16], 0);
    assert_eq!(second.as_bytes()[28], 0);
    assert_eq!(second.as_bytes()[21], 7);
}
#[test]
fn missing_auxiliary_packets_do_not_invent_optional_xml_state() {
    let doc = decode(0x12, &capture(0x12)).unwrap().document;
    assert!(doc.expression_pedal(PedalSlot::First).unwrap().is_none());
    assert!(!doc.to_xml().unwrap().contains("UsbGain"));
    assert!(!doc.to_xml().unwrap().contains("FxLoopBypass"));
}
#[test]
fn unknown_or_duplicate_auxiliary_slot_is_rejected() {
    let mut reports = capture(0x14);
    let mut pedal = aux(10);
    modify(&mut pedal, &[(25, 2)]);
    insert(&mut reports, pedal);
    assert!(decode(0x14, &reports).is_err());
    let mut reports = capture(0x14);
    insert(&mut reports, aux(13));
    insert(&mut reports, aux(13));
    assert!(decode(0x14, &reports).is_err());
    let mut reports = capture(0xa);
    insert(&mut reports, aux(10));
    assert!(decode(0xa, &reports).is_err());
}
#[test]
fn missing_unknown_or_oversized_xml_parameters_do_not_silently_disappear() {
    let xml = decode(0x14, &capture(0x14))
        .unwrap()
        .document
        .to_xml()
        .unwrap();
    let param_start = xml.find("<Param").unwrap();
    let param_end = param_start + xml[param_start..].find("</Param>").unwrap() + 8;
    let missing = format!("{}{}", &xml[..param_start], &xml[param_end..]);
    assert!(encode(0x14, &PresetDocument::parse(&missing).unwrap()).is_err());
    let extra = xml.replacen(
        "</Module>",
        "<Param ControlIndex=\"99\">7</Param></Module>",
        1,
    );
    assert!(encode(0x14, &PresetDocument::parse(&extra).unwrap()).is_err());
    let mut doc = PresetDocument::parse(&xml).unwrap();
    let model = model_catalog::model(
        13,
        ModuleCategory::Amplifier,
        module(&doc, "Amplifier").model_id.unwrap(),
    )
    .unwrap();
    let index = model
        .parameters
        .iter()
        .find(|p| p.type_id >= 128)
        .unwrap()
        .index;
    doc.set_parameter("Amplifier", index, 256).unwrap();
    assert!(encode(0x14, &doc).is_err());
}
#[test]
fn sound_extensions_cannot_be_silently_dropped_by_encode() {
    let xml = decode(0x14, &capture(0x14))
        .unwrap()
        .document
        .to_xml()
        .unwrap();
    for edited in [
        xml.replacen("<Module ", "<Module Mystery=\"1\" ", 1),
        xml.replacen("</Module>", "<Unknown/></Module>", 1),
        xml.replacen("</Preset>", "<Unknown/></Preset>", 1),
        xml.replacen("<Param ", "<Param Mystery=\"1\" ", 1),
    ] {
        assert!(encode(0x14, &PresetDocument::parse(&edited).unwrap()).is_err());
    }
}
#[test]
fn live_parameter_identity_comes_from_catalog_and_unknown_index_is_rejected() {
    let doc = decode(0x14, &capture(0x14)).unwrap().document;
    let model = model_catalog::model(
        13,
        ModuleCategory::Amplifier,
        module(&doc, "Amplifier").model_id.unwrap(),
    )
    .unwrap();
    for p in &model.parameters {
        let command = parameter_change(0x14, &doc, "Amplifier", p.index).unwrap();
        assert_eq!(command.model_id, model.id);
        assert_eq!(command.control_index, p.control_index);
        assert_eq!(command.parameter_type, p.type_id);
    }
    assert!(parameter_change(0x14, &doc, "Amplifier", 255).is_err());
    assert!(parameter_change(0x14, &doc, "Arbitrary", 0).is_err());
    assert!(parameter_change(0x14, &doc, "Stompbox", 0).is_err());
}
#[test]
fn bounded_capture_rejects_unbounded_input_before_parsing() {
    assert!(decode(0x14, &vec![aux(99); 513]).is_err());
}

#[test]
fn mismatched_category_ids_and_first_only_second_pedal_fields_are_not_dropped() {
    let xml = decode(0x14, &capture(0x14))
        .unwrap()
        .document
        .to_xml()
        .unwrap();
    let changed = xml.replace("<Delay ID=\"3\">", "<Delay ID=\"2\">");
    assert!(encode(0x14, &PresetDocument::parse(&changed).unwrap()).is_err());
    let mut reports = capture(0x12);
    let mut pedal = aux(10);
    modify(&mut pedal, &[(25, 1), (20, 6)]);
    insert(&mut reports, pedal);
    let xml = decode(0x12, &reports).unwrap().document.to_xml().unwrap();
    let changed = xml.replace(
        "<SecondExpressionPedal ",
        "<SecondExpressionPedal VolumeModeBehavior=\"1\" ",
    );
    assert!(encode(0x12, &PresetDocument::parse(&changed).unwrap()).is_err());
    let missing = xml.replace("VolumeSwitchRevert=\"0\"", "");
    assert!(encode(0x12, &PresetDocument::parse(&missing).unwrap()).is_err());
}
