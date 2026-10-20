use openfuse_core::{
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument,
        expression::{
            ExpressionPedal, PedalSlot, capabilities, controllable_parameters, target_key,
        },
    },
};

fn fixture(product: u32) -> PresetDocument {
    let profile = model_catalog::product(product).unwrap();
    let amp =
        model_catalog::model(product, ModuleCategory::Amplifier, profile.default_amp_id).unwrap();
    let mut xml = format!(
        "<Preset ProductId=\"{product}\"><Amplifier ID=\"0\"><Module ID=\"{}\" POS=\"0\" BypassState=\"1\" BypassMode=\"{}\">",
        amp.id, amp.bypass_mode
    );
    for parameter in &amp.parameters {
        xml.push_str(&format!(
            "<Param ControlIndex=\"{}\">{}</Param>",
            parameter.index, parameter.default_raw
        ));
    }
    xml.push_str("</Module></Amplifier><FX>");
    for (i, key) in ["Stompbox", "Modulation", "Delay", "Reverb"]
        .iter()
        .enumerate()
    {
        let category = ModuleCategory::from_key(key).unwrap();
        let model = model_catalog::models(product, category)
            .find(|model| !controllable_parameters(product, key, model.id).is_empty())
            .unwrap();
        xml.push_str(&format!(
            "<{key} ID=\"{}\"><Module ID=\"{}\" POS=\"{i}\" BypassState=\"1\" BypassMode=\"{}\">",
            i + 1,
            model.id,
            model.bypass_mode
        ));
        for parameter in &model.parameters {
            xml.push_str(&format!(
                "<Param ControlIndex=\"{}\">{}</Param>",
                parameter.index, parameter.default_raw
            ));
        }
        xml.push_str(&format!("</Module></{key}>"));
    }
    xml.push_str("</FX><FUSE><Info name=\"Expression check\" author=\"OpenFUSE\" tags=\"test\" rating=\"0\" genre1=\"-1\" genre2=\"-1\" genre3=\"-1\" fenderid=\"0\">Independent synthetic expression fixture.</Info><PedalColors><Color ID=\"1\">14</Color><Color ID=\"2\">1</Color><Color ID=\"3\">2</Color><Color ID=\"4\">10</Color></PedalColors></FUSE></Preset>");
    PresetDocument::parse(&xml).unwrap()
}

fn binding(preset: &PresetDocument, slot: PedalSlot, target: u16) -> ExpressionPedal {
    let mut settings = ExpressionPedal::defaults(slot);
    settings.expression_mode_behavior = Some(target);
    if let Some(key) = target_key(target) {
        let module = preset
            .modules()
            .into_iter()
            .find(|module| module.key == key)
            .unwrap();
        settings.parameter_index = Some(
            controllable_parameters(preset.product_id().unwrap(), key, module.model_id.unwrap())[0]
                .index,
        );
    }
    settings
}

#[test]
fn all_supported_slots_and_targets_round_trip_and_export_probe_corpus() {
    let output =
        std::env::var_os("OPENFUSE_EXPORT_EXPRESSION_PRESETS").map(std::path::PathBuf::from);
    if let Some(path) = &output {
        assert!(path.is_absolute(), "Export path must be absolute");
        std::fs::create_dir_all(path).unwrap();
    }
    let mut count = 0;
    for product in [1, 13, 7, 2259, 6] {
        for slot in [PedalSlot::First, PedalSlot::Second] {
            if slot == PedalSlot::Second && product != 7 {
                continue;
            }
            for target in 0..=6 {
                let mut preset = fixture(product);
                let mut settings = binding(&preset, slot, target);
                if target == 4 {
                    settings.heel_setting = Some(65535);
                    settings.toe_setting = Some(0);
                }
                preset.set_expression_pedal(slot, &settings).unwrap();
                assert_eq!(preset.expression_pedal(slot).unwrap(), Some(settings));
                let xml = preset.to_xml().unwrap();
                if let Some(path) = &output {
                    std::fs::write(
                        path.join(format!(
                            "expression-{product}-{}-{target}.fuse",
                            slot.index()
                        )),
                        &xml,
                    )
                    .unwrap();
                }
                assert_eq!(PresetDocument::parse(&xml).unwrap().to_xml().unwrap(), xml);
                count += 1;
            }
        }
    }
    for slot in [PedalSlot::First, PedalSlot::Second] {
        let mut preset = fixture(7);
        preset
            .set_expression_pedal(slot, &ExpressionPedal::defaults(slot))
            .unwrap();
        let xml = preset
            .to_xml()
            .unwrap()
            .replace(
                "ExpressionModeBehavior=\"6\"",
                "ExpressionModeBehavior=\"65000\"",
            )
            .replace("PedalMode=\"0\"", "PedalMode=\"65001\"")
            .replace("VolumeSwitchRevert=\"0\"", "VolumeSwitchRevert=\"65002\"")
            .replace("DefaultPedalState=\"0\"", "DefaultPedalState=\"65003\"")
            .replace(
                "BypassEffectWhenVolumeMode=\"1\"",
                "BypassEffectWhenVolumeMode=\"2\"",
            );
        let mut preset = PresetDocument::parse(&xml).unwrap();
        let mut value = preset.expression_pedal(slot).unwrap().unwrap();
        value.heel_setting = Some(60000);
        value.toe_setting = Some(123);
        preset.set_expression_pedal(slot, &value).unwrap();
        assert_eq!(preset.expression_pedal(slot).unwrap(), Some(value));
        if let Some(path) = &output {
            std::fs::write(
                path.join(format!("unknown-expression-{}.fuse", slot.index())),
                preset.to_xml().unwrap(),
            )
            .unwrap();
        }
    }
    let mut preset = fixture(7);
    let mut legacy = ExpressionPedal::defaults(PedalSlot::First);
    legacy.volume_switch_revert = None;
    legacy.default_pedal_state = None;
    preset
        .set_expression_pedal(PedalSlot::First, &legacy)
        .unwrap();
    if let Some(path) = &output {
        std::fs::write(
            path.join("legacy-optional-expression.fuse"),
            preset.to_xml().unwrap(),
        )
        .unwrap();
    }
    let mut preset = fixture(7);
    let first = binding(&preset, PedalSlot::First, 1);
    let second = binding(&preset, PedalSlot::Second, 4);
    preset
        .set_expression_pedals(&[(PedalSlot::First, first), (PedalSlot::Second, second)])
        .unwrap();
    if let Some(path) = &output {
        std::fs::write(path.join("dual-expression.fuse"), preset.to_xml().unwrap()).unwrap();
    }
    assert_eq!(count, 42);
}

#[test]
fn unknown_values_and_xml_extensions_survive_unrelated_edits() {
    let source = "<Preset ProductId='7' xmlns:x='urn:future'><FirstExpressionPedal PedalIndex='99' ExpressionModeBehavior='65000' ParameterIndex='255' HeelSetting='&#54;' ToeSetting='65535' PedalMode='65001' BypassEffectWhenVolumeMode='2' x:HeelSetting='opaque' Future='value'><!--keep--><x:Future/></FirstExpressionPedal></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    let mut settings = preset.expression_pedal(PedalSlot::First).unwrap().unwrap();
    assert_eq!(settings.heel_setting, Some(6));
    assert_eq!(settings.volume_mode_behavior, None);
    preset
        .set_expression_pedal(PedalSlot::First, &settings)
        .unwrap();
    assert_eq!(preset.to_xml().unwrap(), source);
    settings.toe_setting = Some(12345);
    preset
        .set_expression_pedal(PedalSlot::First, &settings)
        .unwrap();
    assert_eq!(
        preset.to_xml().unwrap(),
        source.replace("ToeSetting='65535'", "ToeSetting='12345'")
    );
    let before = preset.to_xml().unwrap();
    settings.pedal_mode = Some(65002);
    assert!(
        preset
            .set_expression_pedal(PedalSlot::First, &settings)
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), before);
}

#[test]
fn adds_complete_defaults_without_inventing_global_volume_endpoints() {
    let mut preset = PresetDocument::parse("<Preset ProductId='7'/>").unwrap();
    assert_eq!(preset.expression_pedal(PedalSlot::First).unwrap(), None);
    let first = ExpressionPedal::defaults(PedalSlot::First);
    let second = ExpressionPedal::defaults(PedalSlot::Second);
    preset
        .set_expression_pedals(&[
            (PedalSlot::First, first.clone()),
            (PedalSlot::Second, second.clone()),
        ])
        .unwrap();
    assert_eq!(
        preset.expression_pedal(PedalSlot::First).unwrap(),
        Some(first)
    );
    assert_eq!(
        preset.expression_pedal(PedalSlot::Second).unwrap(),
        Some(second)
    );
    let xml = preset.to_xml().unwrap();
    assert!(!xml.contains("VolumeHeel"));
    assert!(!xml.contains("VolumeToe"));
    assert!(xml.contains("<SecondExpressionPedal PedalIndex=\"1\" ExpressionModeBehavior=\"6\""));
}

#[test]
fn wrong_family_slots_and_wrong_identity_are_transactional_errors() {
    for product in [10, 11, 65000] {
        let source = format!("<Preset ProductId='{product}'/>");
        let mut preset = PresetDocument::parse(&source).unwrap();
        assert!(
            preset
                .set_expression_pedal(
                    PedalSlot::First,
                    &ExpressionPedal::defaults(PedalSlot::First)
                )
                .is_err()
        );
        assert_eq!(preset.to_xml().unwrap(), source);
    }
    let mut preset = fixture(13);
    let before = preset.to_xml().unwrap();
    assert!(
        preset
            .set_expression_pedal(
                PedalSlot::Second,
                &ExpressionPedal::defaults(PedalSlot::Second)
            )
            .is_err()
    );
    let mut settings = ExpressionPedal::defaults(PedalSlot::First);
    settings.pedal_index = Some(1);
    assert!(
        preset
            .set_expression_pedal(PedalSlot::First, &settings)
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), before);
}

#[test]
fn rejects_missing_or_disallowed_target_without_partial_changes() {
    let mut preset = fixture(7);
    let before = preset.to_xml().unwrap();
    for target in 1..=5 {
        let mut settings = binding(&preset, PedalSlot::First, target);
        settings.parameter_index = Some(255);
        assert!(
            preset
                .set_expression_pedal(PedalSlot::First, &settings)
                .is_err()
        );
        settings.parameter_index = None;
        assert!(
            preset
                .set_expression_pedal(PedalSlot::First, &settings)
                .is_err()
        );
        assert_eq!(preset.to_xml().unwrap(), before);
    }
    let mut settings = binding(&preset, PedalSlot::First, 1);
    settings.parameter_index = Some(2); // Amp volume is excluded by original UI.
    assert!(
        preset
            .set_expression_pedal(PedalSlot::First, &settings)
            .is_err()
    );
    settings = ExpressionPedal::defaults(PedalSlot::First);
    settings.parameter_index = Some(1);
    assert!(
        preset
            .set_expression_pedal(PedalSlot::First, &settings)
            .is_err()
    );
    let mut no_modules = PresetDocument::parse("<Preset ProductId='7'/>").unwrap();
    settings.expression_mode_behavior = Some(4);
    settings.parameter_index = Some(0);
    assert!(
        no_modules
            .set_expression_pedal(PedalSlot::First, &settings)
            .is_err()
    );
}

#[test]
fn dual_update_fails_atomically_and_keeps_independent_bindings() {
    let mut preset = fixture(7);
    let first = binding(&preset, PedalSlot::First, 1);
    let mut second = binding(&preset, PedalSlot::Second, 4);
    second.parameter_index = Some(255);
    let before = preset.to_xml().unwrap();
    assert!(
        preset
            .set_expression_pedals(&[
                (PedalSlot::First, first.clone()),
                (PedalSlot::Second, second)
            ])
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), before);
    assert!(
        preset
            .set_expression_pedals(&[
                (PedalSlot::First, first.clone()),
                (PedalSlot::First, first.clone())
            ])
            .is_err()
    );
    let second = binding(&preset, PedalSlot::Second, 4);
    preset
        .set_expression_pedals(&[
            (PedalSlot::First, first.clone()),
            (PedalSlot::Second, second.clone()),
        ])
        .unwrap();
    assert_eq!(
        preset.expression_pedal(PedalSlot::First).unwrap(),
        Some(first)
    );
    assert_eq!(
        preset.expression_pedal(PedalSlot::Second).unwrap(),
        Some(second)
    );
}

#[test]
fn malformed_projection_is_an_error_without_rewriting_raw_document() {
    for fields in [
        "HeelSetting='65536'",
        "ParameterIndex='256'",
        "ToeSetting='-1'",
        "BypassEffectWhenVolumeMode='true'",
        "PedalMode=''",
        "ExpressionModeBehavior='hello'",
    ] {
        let source = format!("<Preset ProductId='7'><FirstExpressionPedal {fields}/></Preset>");
        let mut preset = PresetDocument::parse(&source).unwrap();
        assert!(preset.expression_pedal(PedalSlot::First).is_err());
        assert!(
            preset
                .set_expression_pedal(
                    PedalSlot::First,
                    &ExpressionPedal::defaults(PedalSlot::First)
                )
                .is_err()
        );
        assert_eq!(preset.to_xml().unwrap(), source);
    }
    let preset =
        PresetDocument::parse("<Preset><FirstExpressionPedal/><FirstExpressionPedal/></Preset>")
            .unwrap();
    assert!(preset.expression_pedal(PedalSlot::First).is_err());
}

#[test]
fn first_optional_fields_may_be_absent_but_required_removal_is_rejected() {
    let mut preset = fixture(7);
    let mut first = ExpressionPedal::defaults(PedalSlot::First);
    first.volume_switch_revert = None;
    first.default_pedal_state = None;
    preset
        .set_expression_pedal(PedalSlot::First, &first)
        .unwrap();
    let before = preset.to_xml().unwrap();
    first.heel_setting = None;
    assert!(
        preset
            .set_expression_pedal(PedalSlot::First, &first)
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), before);
    let mut second = ExpressionPedal::defaults(PedalSlot::Second);
    second.volume_mode_behavior = Some(1);
    assert!(
        preset
            .set_expression_pedal(PedalSlot::Second, &second)
            .is_err()
    );
}

#[test]
fn original_parameter_filter_respects_amplifier_exceptions_and_effect_enums() {
    let regular = controllable_parameters(13, "Amplifier", 241);
    assert!(regular.iter().any(|p| p.index == 0));
    assert!(!regular.iter().any(|p| p.index == 2 || p.index == 3));
    for product in model_catalog::products() {
        for category in [
            ModuleCategory::Stompbox,
            ModuleCategory::Modulation,
            ModuleCategory::Delay,
            ModuleCategory::Reverb,
        ] {
            let key = match category {
                ModuleCategory::Stompbox => "Stompbox",
                ModuleCategory::Modulation => "Modulation",
                ModuleCategory::Delay => "Delay",
                _ => "Reverb",
            };
            for model in model_catalog::models(product.id, category) {
                assert!(
                    controllable_parameters(product.id, key, model.id)
                        .iter()
                        .all(|parameter| parameter.type_id < 128)
                );
            }
        }
    }
    assert!(controllable_parameters(65000, "Amplifier", 241).is_empty());
    assert!(controllable_parameters(13, "Delay", 0).is_empty());
    assert!(capabilities(7).unwrap().second);
    assert!(capabilities(65000).is_none());
}

#[test]
fn amplifier_index_three_is_available_only_for_the_recovered_model_exceptions() {
    let mut observed_exceptions = std::collections::BTreeSet::new();
    for product in model_catalog::products() {
        for model in model_catalog::models(product.id, ModuleCategory::Amplifier) {
            let has_index_three = model
                .parameters
                .iter()
                .any(|parameter| parameter.index == 3);
            let returned = controllable_parameters(product.id, "Amplifier", model.id)
                .iter()
                .any(|parameter| parameter.index == 3);
            let exception = matches!(model.id, 97 | 93 | 94 | 109 | 114);
            assert_eq!(
                returned,
                has_index_three && exception,
                "product {} model {}",
                product.id,
                model.id
            );
            if returned {
                observed_exceptions.insert(model.id);
            }
        }
    }
    assert_eq!(
        observed_exceptions.into_iter().collect::<Vec<_>>(),
        [93, 94, 97, 109, 114]
    );
}

#[test]
fn unsupported_existing_block_is_preserved_but_cannot_be_newly_rebound() {
    let source = "<Preset ProductId='11'><FirstExpressionPedal ExpressionModeBehavior='6' ParameterIndex='0' HeelSetting='0' ToeSetting='65535'/></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    let mut settings = preset.expression_pedal(PedalSlot::First).unwrap().unwrap();
    settings.heel_setting = Some(100);
    preset
        .set_expression_pedal(PedalSlot::First, &settings)
        .unwrap();
    let before = preset.to_xml().unwrap();
    settings.expression_mode_behavior = Some(0);
    assert!(
        preset
            .set_expression_pedal(PedalSlot::First, &settings)
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), before);
}
