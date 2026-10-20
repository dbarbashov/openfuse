use openfuse_core::{
    model_catalog::{self, Model, ModuleCategory},
    preset::{PresetDocument, module_import::MODULE_KEYS},
};
use serde_json::json;

fn compatible_shape(a: &Model, b: &Model) -> bool {
    let shape = |m: &Model| {
        let mut s: Vec<_> = m
            .parameters
            .iter()
            .map(|p| (p.index, p.control_index, p.type_id))
            .collect();
        s.sort();
        s
    };
    shape(a) == shape(b)
}
fn preset(product: u32, source: bool, compatible_with: Option<u32>) -> PresetDocument {
    let mut xml = format!("<Preset ProductId=\"{product}\" future=\"keep\"><Amplifier ID=\"0\">");
    for (i, key) in MODULE_KEYS.iter().enumerate() {
        let category = ModuleCategory::from_key(key).unwrap();
        let model = model_catalog::models(product, category)
            .find(|m| {
                m.id != 0
                    && compatible_with.is_none_or(|other| {
                        model_catalog::model(other, category, m.id)
                            .is_some_and(|n| compatible_shape(m, n))
                    })
            })
            .unwrap();
        let position = if i == 0 {
            0
        } else if product == 10 {
            u16::from(i != 1)
        } else if product == 2259 {
            [0, 3, 0, 5, 6][i]
        } else {
            [0, 0, 1, 4, 5][i]
        };
        if i > 0 {
            xml.push_str(&format!("<{key} ID=\"{i}\">"));
        }
        xml.push_str(&format!(
            "<Module ID=\"{}\" POS=\"{position}\" BypassState=\"{}\" BypassMode=\"{}\">",
            model.id,
            u8::from(source),
            model.bypass_mode
        ));
        for parameter in &model.parameters {
            let value = if parameter.type_id == 1 {
                if source { 17000 } else { 8000 }
            } else {
                parameter.default_raw
            };
            xml.push_str(&format!(
                "<Param ControlIndex=\"{}\">{value}</Param>",
                parameter.index
            ));
        }
        xml.push_str("</Module>");
        if i == 0 {
            xml.push_str("</Amplifier><FX>");
        } else {
            xml.push_str(&format!("</{key}>"));
        }
    }
    xml.push_str(&format!("</FX><UsbGain>{}</UsbGain><FUSE><Info name=\"{}\" author=\"OpenFUSE &amp; contributors\" tags=\"import,test\" rating=\"0\" genre1=\"-1\" genre2=\"-1\" genre3=\"-1\" fenderid=\"0\">Keep this metadata</Info><PedalColors><Color ID=\"1\">14</Color><Color ID=\"2\">1</Color><Color ID=\"3\">2</Color><Color ID=\"4\">10</Color></PedalColors></FUSE><Future extension=\"untouched\"/></Preset>",if source {7}else{2},if source {"Source"}else{"Destination"}));
    PresetDocument::parse(&xml).unwrap()
}
fn entry(document: &PresetDocument, key: &str) -> openfuse_core::preset::PresetModule {
    document
        .modules()
        .into_iter()
        .find(|m| m.key == key)
        .unwrap()
}

#[test]
fn importing_one_effect_preserves_metadata_colors_and_all_unselected_state() {
    let mut destination = preset(13, false, None);
    let source = preset(13, true, None);
    let before = destination.clone();
    let report = destination.import_modules(&source, &["Delay"]).unwrap();
    assert_eq!(report.imported, vec!["Delay"]);
    assert_eq!(entry(&destination, "Delay"), entry(&source, "Delay"));
    for key in ["Amplifier", "Stompbox", "Modulation", "Reverb"] {
        assert_eq!(entry(&destination, key), entry(&before, key));
    }
    let xml = destination.to_xml().unwrap();
    assert!(xml.contains("<UsbGain>2</UsbGain>"));
    let old = before.to_xml().unwrap();
    assert_eq!(xml.split("<FUSE>").nth(1), old.split("<FUSE>").nth(1));
    assert_eq!(destination.metadata(), before.metadata());
}

#[test]
fn amplifier_import_copies_usb_gain_and_reports_it_before_commit() {
    let mut destination = preset(13, false, None);
    let source = preset(13, true, None);
    let preview = destination.preview_module_import(&source);
    assert!(preview[0].compatible);
    assert!(preview[0].notes.iter().any(|n| n.contains("USB gain")));
    destination.import_modules(&source, &["Amplifier"]).unwrap();
    assert!(
        destination
            .to_xml()
            .unwrap()
            .contains("<UsbGain>7</UsbGain>")
    );
    let source =
        PresetDocument::parse(&source.to_xml().unwrap().replace("<UsbGain>7</UsbGain>", ""))
            .unwrap();
    destination.import_modules(&source, &["Amplifier"]).unwrap();
    assert!(
        destination
            .to_xml()
            .unwrap()
            .contains("<UsbGain>0</UsbGain>")
    );
}

#[test]
fn compatible_cross_family_import_preserves_destination_product() {
    for (destination_id, source_id) in [(1, 13), (13, 1), (7, 6), (6, 7)] {
        let mut destination = preset(destination_id, false, Some(source_id));
        let source = preset(source_id, true, Some(destination_id));
        let preview = destination.preview_module_import(&source);
        assert!(
            preview.iter().all(|p| p.compatible),
            "{destination_id} <- {source_id}: {preview:?}"
        );
        destination.import_modules(&source, &MODULE_KEYS).unwrap();
        assert_eq!(destination.product_id(), Some(destination_id));
        for key in MODULE_KEYS {
            assert_eq!(entry(&destination, key), entry(&source, key));
        }
    }
}

#[test]
fn incompatible_families_and_models_have_individual_reasons_and_no_mutation() {
    let mut destination = preset(13, false, None);
    let before = destination.to_xml().unwrap();
    let source = preset(11, true, None);
    assert!(
        destination
            .preview_module_import(&source)
            .iter()
            .all(|p| !p.compatible && p.reason.is_some())
    );
    assert!(destination.import_modules(&source, &["Amplifier"]).is_err());
    assert_eq!(before, destination.to_xml().unwrap());
    let source = preset(13, true, None);
    let mut source = source.clone();
    source
        .replace_module_model("Delay", 65000, &[], Some(0))
        .unwrap();
    let preview = destination.preview_module_import(&source);
    assert!(
        !preview
            .iter()
            .find(|p| p.key == "Delay")
            .unwrap()
            .compatible
    );
    assert!(
        preview
            .iter()
            .find(|p| p.key == "Amplifier")
            .unwrap()
            .compatible
    );
    assert!(
        destination
            .import_modules(&source, &["Amplifier", "Delay"])
            .is_err()
    );
    assert_eq!(before, destination.to_xml().unwrap());
}

#[test]
fn selection_is_atomic_and_final_routing_can_be_resolved_by_importing_multiple_categories() {
    let mut destination = preset(13, false, None);
    let mut source = preset(13, true, None);
    source
        .set_module_positions(&[("Stompbox", 1), ("Modulation", 0)])
        .unwrap();
    let before = destination.to_xml().unwrap();
    assert!(
        destination
            .import_modules(&source, &["Modulation"])
            .is_err()
    );
    assert_eq!(before, destination.to_xml().unwrap());
    destination
        .import_modules(&source, &["Modulation", "Stompbox"])
        .unwrap();
    assert_eq!(entry(&destination, "Modulation").position, Some(0));
    assert_eq!(entry(&destination, "Stompbox").position, Some(1));
}

#[test]
fn unknown_source_extensions_and_missing_parameters_are_refused_but_dest_extensions_survive() {
    let original = preset(13, false, None);
    let mut destination = PresetDocument::parse(&original.to_xml().unwrap().replacen(
        "<Module ",
        "<Module retained=\"yes\" ",
        1,
    ))
    .unwrap();
    let source = preset(13, true, None);
    destination.import_modules(&source, &["Amplifier"]).unwrap();
    assert!(destination.to_xml().unwrap().contains("retained=\"yes\""));
    let with_unknown = PresetDocument::parse(&source.to_xml().unwrap().replacen(
        "<Module ",
        "<Module unknown=\"value\" ",
        1,
    ))
    .unwrap();
    assert!(!destination.preview_module_import(&with_unknown)[0].compatible);
    let before = destination.to_xml().unwrap();
    assert!(
        destination
            .import_modules(&with_unknown, &["Amplifier"])
            .is_err()
    );
    let mut missing = source.clone();
    let amp = entry(&missing, "Amplifier");
    missing
        .replace_module_model(
            "Amplifier",
            amp.model_id.unwrap(),
            &amp.parameters[1..],
            amp.bypass_mode,
        )
        .unwrap();
    assert!(
        destination
            .import_modules(&missing, &["Amplifier"])
            .is_err()
    );
    assert_eq!(before, destination.to_xml().unwrap());
}

#[test]
fn deleting_a_destination_parameter_extension_is_refused_atomically() {
    let original = preset(13, false, None);
    let mut destination = PresetDocument::parse(&original.to_xml().unwrap().replacen(
        "<Param ",
        "<Param retained=\"yes\" ",
        1,
    ))
    .unwrap();
    let mut source = preset(13, true, None);
    source
        .replace_module_model("Amplifier", 0, &[], Some(0))
        .unwrap();
    let before = destination.to_xml().unwrap();
    assert!(!destination.preview_module_import(&source)[0].compatible);
    assert!(destination.import_modules(&source, &["Amplifier"]).is_err());
    assert_eq!(before, destination.to_xml().unwrap());
}

#[test]
fn bypass_normalization_is_disclosed_and_gdec_activates_imported_effects() {
    let mut destination = preset(10, false, None);
    let mut source = preset(10, false, None);
    source
        .replace_module_model(
            "Delay",
            entry(&source, "Delay").model_id.unwrap(),
            &entry(&source, "Delay").parameters,
            Some(255),
        )
        .unwrap();
    let preview = destination.preview_module_import(&source);
    let option = preview.iter().find(|p| p.key == "Delay").unwrap();
    assert!(option.notes.iter().any(|s| s.contains("bypass mode")));
    assert!(option.notes.iter().any(|s| s.contains("active")));
    let report = destination.import_modules(&source, &["Delay"]).unwrap();
    assert!(!report.losses.is_empty());
    assert_eq!(entry(&destination, "Delay").active, Some(true));
    assert_ne!(entry(&destination, "Delay").bypass_mode, Some(255));
}

#[test]
fn selected_removal_disables_its_associated_expression_only() {
    let base = preset(7, false, None);
    let xml=base.to_xml().unwrap().replace("</Preset>","<FirstExpressionPedal ExpressionModeBehavior=\"4\" ParameterIndex=\"0\"/><SecondExpressionPedal ExpressionModeBehavior=\"0\" ParameterIndex=\"0\"/></Preset>");
    let mut destination = PresetDocument::parse(&xml).unwrap();
    let mut source = preset(7, true, None);
    source
        .replace_module_model("Delay", 0, &[], Some(0))
        .unwrap();
    destination.import_modules(&source, &["Delay"]).unwrap();
    assert!(
        destination
            .to_xml()
            .unwrap()
            .contains("<FirstExpressionPedal ExpressionModeBehavior=\"0\" ParameterIndex=\"0\"/>")
    );
    assert!(
        destination
            .to_xml()
            .unwrap()
            .contains("<SecondExpressionPedal ExpressionModeBehavior=\"0\" ParameterIndex=\"0\"/>")
    );
}

#[test]
fn all_profiles_support_same_family_selective_import_and_export_original_probe_cases() {
    let output =
        std::env::var_os("OPENFUSE_EXPORT_MODULE_IMPORT_CASES").map(std::path::PathBuf::from);
    if let Some(folder) = &output {
        std::fs::create_dir_all(folder).unwrap();
    }
    let mut cases = Vec::new();
    for (destination_id, source_id) in [
        (10, 10),
        (1, 1),
        (13, 13),
        (7, 7),
        (11, 11),
        (2259, 2259),
        (6, 6),
        (1, 13),
        (13, 1),
        (7, 6),
        (6, 7),
    ] {
        let before = preset(destination_id, false, Some(source_id));
        let source = preset(source_id, true, Some(destination_id));
        let mut after = before.clone();
        after.import_modules(&source, &MODULE_KEYS).unwrap();
        let name = format!("{destination_id}_from_{source_id}");
        if let Some(folder) = &output {
            let before_name = format!("{name}-before.fuse");
            let source_name = format!("{name}-source.fuse");
            let after_name = format!("{name}-after.fuse");
            std::fs::write(folder.join(&before_name), before.to_xml().unwrap()).unwrap();
            std::fs::write(folder.join(&source_name), source.to_xml().unwrap()).unwrap();
            std::fs::write(folder.join(&after_name), after.to_xml().unwrap()).unwrap();
            cases.push(json!({"name":name,"before":before_name,"source":source_name,"after":after_name,"categories":MODULE_KEYS}));
        }
    }
    if let Some(folder) = &output {
        std::fs::write(
            folder.join("cases.json"),
            serde_json::to_vec_pretty(&json!({"cases":cases})).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn empty_duplicate_or_unknown_selections_and_bad_gain_fail_without_changes() {
    let mut destination = preset(13, false, None);
    let source = preset(13, true, None);
    let before = destination.to_xml().unwrap();
    for keys in [&[][..], &["Delay", "Delay"][..], &["NoModule"][..]] {
        assert!(destination.import_modules(&source, keys).is_err());
    }
    let source = PresetDocument::parse(
        &source
            .to_xml()
            .unwrap()
            .replace("<UsbGain>7</UsbGain>", "<UsbGain>256</UsbGain>"),
    )
    .unwrap();
    assert!(!destination.preview_module_import(&source)[0].compatible);
    assert!(destination.import_modules(&source, &["Amplifier"]).is_err());
    assert_eq!(before, destination.to_xml().unwrap());
}

#[test]
fn legacy_gdec_delay_encodings_are_refused_with_a_reason_instead_of_reinterpreted() {
    let mut destination = preset(10, false, None);
    let source = preset(10, true, None);
    let delay = entry(&source, "Delay");
    let model = model_catalog::model(10, ModuleCategory::Delay, delay.model_id.unwrap()).unwrap();
    let parameter = model
        .parameters
        .iter()
        .find(|p| [136, 137, 138].contains(&p.type_id))
        .unwrap();
    let maximum = match parameter.type_id {
        136 => 1110,
        137 => 555,
        _ => 370,
    };
    let before = destination.to_xml().unwrap();
    for value in [0, 29, maximum + 1, 65535] {
        let mut bad = source.clone();
        bad.set_parameter("Delay", parameter.index, value).unwrap();
        let option = destination
            .preview_module_import(&bad)
            .into_iter()
            .find(|p| p.key == "Delay")
            .unwrap();
        assert!(!option.compatible);
        assert!(option.reason.unwrap().contains("Legacy delay"));
        assert!(destination.import_modules(&bad, &["Delay"]).is_err());
        assert_eq!(before, destination.to_xml().unwrap());
    }
    for value in [30, maximum] {
        let mut valid = source.clone();
        valid
            .set_parameter("Delay", parameter.index, value)
            .unwrap();
        destination.import_modules(&valid, &["Delay"]).unwrap();
    }
}

#[test]
fn category_id_mismatch_cannot_reinterpret_a_module_in_the_original_loader() {
    let mut destination = preset(13, false, None);
    let source = preset(13, true, None);
    let invalid = PresetDocument::parse(
        &source
            .to_xml()
            .unwrap()
            .replace("<Delay ID=\"3\">", "<Delay ID=\"2\">"),
    )
    .unwrap();
    assert!(
        !destination
            .preview_module_import(&invalid)
            .into_iter()
            .find(|p| p.key == "Delay")
            .unwrap()
            .compatible
    );
    let before = destination.to_xml().unwrap();
    assert!(destination.import_modules(&invalid, &["Delay"]).is_err());
    assert_eq!(before, destination.to_xml().unwrap());
}

#[test]
fn module_import_retains_opaque_destination_band_data_without_interpreting_it() {
    let base = preset(13, false, None);
    let xml = base.to_xml().unwrap().replace(
        "</Preset>",
        "<Band future=\"untouched\"><Unknown/></Band></Preset>",
    );
    let mut destination = PresetDocument::parse(&xml).unwrap();
    destination
        .import_modules(&preset(13, true, None), &MODULE_KEYS)
        .unwrap();
    assert!(
        destination
            .to_xml()
            .unwrap()
            .contains("<Band future=\"untouched\"><Unknown/></Band>")
    );
}

#[test]
fn source_nested_parameter_payload_is_rejected_before_import_and_wrapper_payload_is_not_discarded()
{
    let mut destination = preset(13, false, None);
    let source = preset(13, true, None);
    let nested = source.to_xml().unwrap().replacen(
        "</Param>",
        "<future:Payload xmlns:future=\"urn:future\"/></Param>",
        1,
    );
    assert!(PresetDocument::parse(&nested).is_err());
    let wrapper = source.to_xml().unwrap().replacen(
        "</Amplifier>",
        "<future:Payload xmlns:future=\"urn:future\"/></Amplifier>",
        1,
    );
    let parsed = PresetDocument::parse(&wrapper).unwrap();
    let before = destination.to_xml().unwrap();
    assert!(!destination.preview_module_import(&parsed)[0].compatible);
    assert!(destination.import_modules(&parsed, &["Amplifier"]).is_err());
    assert_eq!(destination.to_xml().unwrap(), before);
}
