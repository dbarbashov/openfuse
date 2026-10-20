use openfuse_core::{
    controls::{self, SettingsPatch, TunerControlState, TunerNote, TunerReading, VolumeEndpoints},
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

fn set(reports: &mut [Report64], kind: u8, slot: u8, offset: usize, value: u8) {
    let r = reports
        .iter_mut()
        .find(|r| {
            r.as_bytes()[0] == 0x1c
                && r.as_bytes()[2] == kind
                && (kind != 10 || r.as_bytes()[25] == slot)
        })
        .unwrap();
    let mut b = *r.as_bytes();
    b[offset] = value;
    *r = Report64::from_bytes(b);
}
#[test]
fn capabilities_distinguish_physical_pedals_from_xml_support() {
    for pid in [4, 5, 0xa, 0xe, 0x10, 0x12, 0x14, 0x16, 0x26] {
        let c = controls::capabilities(pid).unwrap();
        assert!(c.usb_gain);
        assert_eq!(c.tuner, pid != 0xe);
        assert_eq!(c.first_pedal, matches!(pid, 5 | 0x12 | 0x16));
        assert_eq!(c.global_volume, c.first_pedal);
        assert_eq!(c.second_pedal, pid == 0x12);
        assert_eq!(c.fx_loop, pid == 0x12);
    }
    assert!(controls::capabilities(0xffff).is_err());
}
#[test]
fn gain_uses_original_two_report_transaction_and_retains_sound() {
    for pid in [4, 5, 0xa, 0xe, 0x10, 0x12, 0x14, 0x16, 0x26] {
        let raw = current(pid, &document(pid));
        let saved = raw.clone();
        for gain in [0, 1, 128, 255] {
            let p = controls::settings_plan(
                pid,
                &raw,
                &SettingsPatch {
                    usb_gain: Some(gain),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(p.steps().len(), 2);
            assert_eq!(&p.steps()[0].as_bytes()[..8], &[0x1c, 3, 13, 0, 0, 0, 1, 1]);
            assert_eq!(p.steps()[0].as_bytes()[16], gain as u8);
            assert_eq!(&p.steps()[1].as_bytes()[..4], &[0x1c, 3, 0, 0]);
            let mut expected = raw.clone();
            set(&mut expected, 13, 0, 16, gain as u8);
            p.matches_readback(&expected).unwrap();
            if gain != 9 {
                assert!(p.matches_readback(&raw).is_err());
            }
        }
        assert_eq!(raw, saved);
    }
}
#[test]
fn loop_bypass_uses_fxkill_not_full_preset_auxiliary_upload() {
    let raw = current(0x12, &document(0x12));
    for bypass in [false, true] {
        let p = controls::settings_plan(
            0x12,
            &raw,
            &SettingsPatch {
                fx_loop_bypassed: Some(bypass),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.steps().len(), 1);
        assert_eq!(
            &p.steps()[0].as_bytes()[..5],
            &[25, 0xc3, 15, u8::from(!bypass), 0]
        );
        let mut expected = raw.clone();
        set(&mut expected, 14, 0, 16, u8::from(bypass));
        p.matches_readback(&expected).unwrap();
    }
}
#[test]
fn global_volume_sends_each_supported_pedal_and_compares_quantized_raw_endpoints() {
    for pid in [5, 0x16, 0x12] {
        let raw = current(pid, &document(pid));
        let patch = SettingsPatch {
            global_volume: Some(VolumeEndpoints {
                heel: 65535,
                toe: 1,
            }),
            ..Default::default()
        };
        let p = controls::settings_plan(pid, &raw, &patch).unwrap();
        let slots = if pid == 0x12 { 2 } else { 1 };
        assert_eq!(p.steps().len(), slots * 2);
        assert!(!p.warnings().is_empty());
        let mut expected = raw.clone();
        for slot in 0..slots {
            let b = p.steps()[slot * 2].as_bytes();
            assert_eq!(&b[..8], &[0x1c, 3, 10, 0, 0, 0, 0, 0]);
            assert_eq!(b[25], slot as u8);
            assert_eq!(&b[17..19], &[255, 0]);
            set(&mut expected, 10, slot as u8, 17, 255);
            set(&mut expected, 10, slot as u8, 18, 0);
        }
        p.matches_readback(&expected).unwrap();
        assert!(p.matches_readback(&raw).is_err());
    }
}
#[test]
fn pedal_modes_are_independent_and_unknown_values_are_read_without_invention() {
    let mut raw = current(0x12, &document(0x12));
    set(&mut raw, 10, 0, 24, 77);
    assert_eq!(
        controls::settings(0x12, &raw).unwrap().first_pedal_mode,
        Some(77)
    );
    for (first, second) in [(Some(0), None), (None, Some(1)), (Some(1), Some(0))] {
        let p = controls::settings_plan(
            0x12,
            &raw,
            &SettingsPatch {
                first_pedal_mode: first,
                second_pedal_mode: second,
                ..Default::default()
            },
        )
        .unwrap();
        let mut expected = raw.clone();
        if let Some(v) = first {
            set(&mut expected, 10, 0, 24, v)
        }
        if let Some(v) = second {
            set(&mut expected, 10, 1, 24, v)
        };
        p.matches_readback(&expected).unwrap();
    }
}
#[test]
fn rejects_missing_settings_bad_ranges_unsupported_profiles_and_unknown_models() {
    let raw = current(0x14, &document(0x14));
    for patch in [
        SettingsPatch::default(),
        SettingsPatch {
            usb_gain: Some(256),
            ..Default::default()
        },
        SettingsPatch {
            fx_loop_bypassed: Some(true),
            ..Default::default()
        },
        SettingsPatch {
            first_pedal_mode: Some(1),
            ..Default::default()
        },
        SettingsPatch {
            global_volume: Some(VolumeEndpoints { heel: 0, toe: 0 }),
            ..Default::default()
        },
    ] {
        assert!(controls::settings_plan(0x14, &raw, &patch).is_err());
    }
    let patch = SettingsPatch {
        usb_gain: Some(1),
        ..Default::default()
    };
    let mut missing = raw.clone();
    missing.retain(|r| r.as_bytes()[2] != 13 || r.as_bytes()[0] != 0x1c);
    assert_eq!(controls::settings(0x14, &missing).unwrap().usb_gain, None);
    assert!(controls::settings_plan(0x14, &missing, &patch).is_err());
    let mut unknown = raw;
    set(&mut unknown, 5, 0, 16, 255);
    set(&mut unknown, 5, 0, 17, 255);
    assert!(controls::settings_plan(0x14, &unknown, &patch).is_err());
    let floor = current(0x12, &document(0x12));
    assert!(
        controls::settings_plan(
            0x12,
            &floor,
            &SettingsPatch {
                second_pedal_mode: Some(2),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(controls::settings_plan(0xffff, &floor, &patch).is_err());
}
#[test]
fn unknown_bytes_are_retained_but_lossy_writes_are_rejected() {
    let mut raw = current(0x12, &document(0x12));
    set(&mut raw, 10, 0, 63, 0xa5);
    let saved = raw.clone();
    assert!(
        controls::settings_plan(
            0x12,
            &raw,
            &SettingsPatch {
                global_volume: Some(VolumeEndpoints {
                    heel: 0,
                    toe: 65535
                }),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert_eq!(raw, saved);
    // An unrelated scalar is still editable without resending expression bytes.
    controls::settings_plan(
        0x12,
        &raw,
        &SettingsPatch {
            usb_gain: Some(5),
            ..Default::default()
        },
    )
    .unwrap();
    set(&mut raw, 13, 0, 42, 0x80);
    assert!(
        controls::settings_plan(
            0x12,
            &raw,
            &SettingsPatch {
                usb_gain: Some(5),
                ..Default::default()
            }
        )
        .is_err()
    );
}
#[test]
fn mixed_patch_has_bounded_original_transactions_and_compares_all_changes() {
    let raw = current(0x12, &document(0x12));
    let p = controls::settings_plan(
        0x12,
        &raw,
        &SettingsPatch {
            usb_gain: Some(200),
            fx_loop_bypassed: Some(false),
            global_volume: Some(VolumeEndpoints {
                heel: 256,
                toe: 512,
            }),
            first_pedal_mode: Some(1),
            second_pedal_mode: Some(0),
        },
    )
    .unwrap();
    assert_eq!(p.steps().len(), 7);
    let mut expected = raw.clone();
    set(&mut expected, 13, 0, 16, 200);
    set(&mut expected, 14, 0, 16, 0);
    for slot in [0, 1] {
        set(&mut expected, 10, slot, 17, 1);
        set(&mut expected, 10, slot, 18, 2);
        set(&mut expected, 10, slot, 24, 1 - slot);
    }
    p.matches_readback(&expected).unwrap();
    set(&mut expected, 10, 1, 18, 3);
    assert!(p.matches_readback(&expected).is_err());
}
#[test]
fn tuner_has_no_ack_and_off_clears_mute_and_snap() {
    for enabled in [false, true] {
        for muted in [false, true] {
            let r = controls::tuner_control(0x14, enabled, muted).unwrap();
            assert_eq!(
                &r.as_bytes()[..5],
                &[
                    10,
                    1,
                    u8::from(enabled),
                    u8::from(enabled),
                    u8::from(enabled && muted)
                ]
            );
        }
    }
    assert!(controls::tuner_control(0xe, true, true).is_err());
    assert!(controls::tuner_control(0xffff, true, true).is_err());
}
#[test]
fn tuner_decode_preserves_sentinels_unknown_values_and_signed_extremes() {
    for tone in 0..=255u8 {
        for deviation in [0, 1, 127, 128, 255] {
            let mut b = [0; 64];
            b[0] = 11;
            b[2] = tone;
            b[3] = deviation;
            let r = TunerReading::decode(&Report64::from_bytes(b)).unwrap();
            assert_eq!(r.tone, tone);
            assert_eq!(r.offset, deviation as i8);
            assert_eq!(r.note.is_pitch(), tone < 12);
            if tone == 12 {
                assert_eq!(r.note, TunerNote::Invalid);
            }
            if tone > 12 {
                assert_eq!(r.note, TunerNote::Unknown(tone));
            }
        }
    }
    assert_eq!(TunerNote::from_raw(0).label(), "A");
    assert_eq!(TunerNote::from_raw(3).label(), "C");
    let mut b = [0; 64];
    b[0] = 10;
    b[2] = 2;
    b[3] = 7;
    b[4] = 255;
    let r = Report64::from_bytes(b);
    let s = TunerControlState::decode(&r).unwrap();
    assert_eq!(s.enabled_raw, 2);
    assert_eq!(s.snap_to_state_raw, 7);
    assert_eq!(s.muted_raw, 255);
    assert!(s.enabled() && s.muted());
    assert!(TunerReading::decode(&r).is_none());
}

#[test]
fn floor_second_hidden_fields_survive_isolated_edit_and_are_checked_on_readback() {
    let mut raw = current(0x12, &document(0x12));
    set(&mut raw, 10, 1, 16, 1);
    set(&mut raw, 10, 1, 28, 1);
    let plan = controls::settings_plan(
        0x12,
        &raw,
        &SettingsPatch {
            second_pedal_mode: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(plan.steps()[0].as_bytes()[16], 1);
    assert_eq!(plan.steps()[0].as_bytes()[28], 1);
    let mut expected = raw.clone();
    set(&mut expected, 10, 1, 24, 1);
    plan.matches_readback(&expected).unwrap();
    set(&mut expected, 10, 1, 28, 0);
    assert!(plan.matches_readback(&expected).is_err());
    // Unrelated scalar edits also verify these retained raw fields.
    let plan = controls::settings_plan(
        0x12,
        &raw,
        &SettingsPatch {
            usb_gain: Some(9),
            ..Default::default()
        },
    )
    .unwrap();
    let mut changed = raw.clone();
    set(&mut changed, 10, 1, 16, 0);
    assert!(plan.matches_readback(&changed).is_err());
}

#[test]
fn inconsistent_global_pairs_use_last_received_pair_with_explicit_warning() {
    let mut raw = current(0x12, &document(0x12));
    set(&mut raw, 10, 1, 17, 8);
    set(&mut raw, 10, 1, 18, 16);
    let state = controls::settings(0x12, &raw).unwrap();
    assert_eq!(
        state.global_volume,
        Some(VolumeEndpoints {
            heel: 2048,
            toe: 4096
        })
    );
    assert_eq!(state.warnings.len(), 1);
    let plan = controls::settings_plan(
        0x12,
        &raw,
        &SettingsPatch {
            global_volume: state.global_volume,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(plan.warnings().iter().any(|w| w.contains("disagree")));
    let mut expected = raw.clone();
    set(&mut expected, 10, 0, 17, 8);
    set(&mut expected, 10, 0, 18, 16);
    plan.matches_readback(&expected).unwrap();
}
