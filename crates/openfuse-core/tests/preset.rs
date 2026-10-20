use openfuse_core::preset::{MAX_PRESET_BYTES, PresetDocument, PresetMetadata, PresetParameter};

const SAMPLE: &str = include_str!("fixtures/synthetic.fuse");

#[test]
fn untouched_roundtrip_and_typed_projection() {
    let preset = PresetDocument::parse(SAMPLE).unwrap();
    assert_eq!(preset.to_xml().unwrap(), SAMPLE);
    assert_eq!(preset.metadata().name, "Confuse demo");
    let modules = preset.modules();
    assert_eq!(modules.len(), 5);
    assert_eq!(modules[0].model_id, Some(65001));
    assert_eq!(modules[3].key, "Delay");
    assert_eq!(modules[3].parameters[0].value, 450);
}

#[test]
fn edits_parameter_only_and_preserves_every_other_byte() {
    let mut preset = PresetDocument::parse(SAMPLE).unwrap();
    preset.set_parameter("Delay", 0, 789).unwrap();
    assert_eq!(
        preset.to_xml().unwrap(),
        SAMPLE.replace(">450</Param>", ">789</Param>")
    );
    let original = preset.to_xml().unwrap();
    assert!(preset.set_parameter("Unknown", 0, 42).is_err());
    assert!(preset.set_parameter("Delay", 42, 42).is_err());
    assert_eq!(preset.to_xml().unwrap(), original);
}

#[test]
fn metadata_edit_preserves_unknown_fields_and_escapes_xml() {
    let mut preset = PresetDocument::parse(SAMPLE).unwrap();
    let metadata = PresetMetadata {
        name: "Amp <>&\"'\n\t".into(),
        author: "\u{410}\u{432}\u{442}\u{43e}\u{440}".into(),
        tags: "one; two".into(),
        description: "<description> &\r\nsecond line".into(),
    };
    preset.set_metadata(&metadata).unwrap();
    let result = preset.to_xml().unwrap();
    assert_eq!(PresetDocument::parse(&result).unwrap().metadata(), metadata);
    for untouched in [
        "future=\"untouched\"",
        "rating=\"0\"",
        "<ext:Future><![CDATA[unknown <content>]]></ext:Future>",
        "<!-- Independently authored",
        "<FirstExpressionPedal ParameterIndex=\"3\" FutureMode=\"123\"/>",
        "<Band Type=\"0\"",
    ] {
        assert!(result.contains(untouched), "missing: {untouched}");
    }
    assert_eq!(
        preset.modules(),
        PresetDocument::parse(SAMPLE).unwrap().modules()
    );
}

#[test]
fn unchanged_fields_do_not_normalize_original_encoding() {
    let source =
        "<Preset><FUSE><Info name='&#65;' author='me' tags='x'>description</Info></FUSE></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    let mut metadata = preset.metadata();
    metadata.author = "you".into();
    preset.set_metadata(&metadata).unwrap();
    assert_eq!(
        preset.to_xml().unwrap(),
        source.replace("author='me'", "author='you'")
    );
}

#[test]
fn creates_missing_metadata_including_self_closing_parents() {
    for source in [
        "<Preset/>",
        "<Preset></Preset>",
        "<Preset><FUSE/></Preset>",
        "<Preset><FUSE><Info/></FUSE></Preset>",
        "<Preset><FUSE><Info name='old'/></FUSE></Preset>",
    ] {
        let mut preset = PresetDocument::parse(source).unwrap();
        let metadata = PresetMetadata {
            name: "new".into(),
            description: "new description".into(),
            ..PresetMetadata::default()
        };
        preset.set_metadata(&metadata).unwrap();
        assert_eq!(
            PresetDocument::parse(&preset.to_xml().unwrap())
                .unwrap()
                .metadata(),
            metadata,
            "{source}"
        );
    }
}

#[test]
fn preserves_comments_extensions_and_pi_inside_edited_text() {
    let source = "<?xml version='1.0'?><?test keep?><Preset><FUSE><Info name='old'><![CDATA[hello <there>]]><!--note--><?inside keep?><extra/>second</Info></FUSE></Preset><!--after-->";
    let mut preset = PresetDocument::parse(source).unwrap();
    assert_eq!(preset.metadata().description, "hello <there>second");
    let mut metadata = preset.metadata();
    metadata.description = "new & description".into();
    preset.set_metadata(&metadata).unwrap();
    let output = preset.to_xml().unwrap();
    assert_eq!(PresetDocument::parse(&output).unwrap().metadata(), metadata);
    for preserved in [
        "<?test keep?>",
        "<!--note-->",
        "<?inside keep?>",
        "<extra/>",
        "<!--after-->",
    ] {
        assert!(output.contains(preserved));
    }
}

#[test]
fn parameter_cdata_and_comments_are_edited_safely() {
    let source = "<Preset><Amplifier><Module ID='65535'><Param ControlIndex='255'><![CDATA[12]]><!--keep-->34</Param></Module></Amplifier></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    assert_eq!(preset.modules()[0].parameters[0].value, 1234);
    preset.set_parameter("Amplifier", 255, 65535).unwrap();
    assert_eq!(preset.modules()[0].parameters[0].value, 65535);
    assert!(preset.to_xml().unwrap().contains("<!--keep-->"));
}

#[test]
fn rejects_unsafe_malformed_ambiguous_and_out_of_range_inputs() {
    let invalid = [
        "",
        "<Wrong/>",
        "<Preset>",
        "<Preset/><Preset/>",
        "<Preset xmlns='urn:other'/>",
        "<!DOCTYPE Preset><Preset/>",
        "<!DOCTYPE Preset SYSTEM 'file:///etc/passwd'><Preset/>",
        "<!DOCTYPE Preset [<!ENTITY x 'expanded'>]><Preset>&x;</Preset>",
        "<Preset>&unknown;</Preset>",
        "<Preset><FUSE/><FUSE/></Preset>",
        "<Preset><FUSE><Info/><Info/></FUSE></Preset>",
        "<Preset><Amplifier><Module/><Module/></Amplifier></Preset>",
        "<Preset><Amplifier><Module ID='65536'/></Amplifier></Preset>",
        "<Preset><Amplifier><Module POS='-1'/></Amplifier></Preset>",
        "<Preset><Amplifier><Module><Param ControlIndex='256'>0</Param></Module></Amplifier></Preset>",
        "<Preset><Amplifier><Module><Param ControlIndex='0'>65536</Param></Module></Amplifier></Preset>",
        "<Preset><Amplifier><Module><Param ControlIndex='0'>+1</Param></Module></Amplifier></Preset>",
        "<Preset><Amplifier><Module><Param ControlIndex='0'>-1</Param></Module></Amplifier></Preset>",
        "<Preset><Amplifier><Module><Param ControlIndex='0'/></Module></Amplifier></Preset>",
        "<Preset><Amplifier><Module><Param>1</Param></Module></Amplifier></Preset>",
        "<Preset><Amplifier><Module><Param ControlIndex='0'>0</Param><Param ControlIndex='0'>1</Param></Module></Amplifier></Preset>",
        "<Preset><Amplifier><Module><Param ControlIndex='0'><Unknown>1</Unknown></Param></Module></Amplifier></Preset>",
    ];
    for xml in invalid {
        assert!(PresetDocument::parse(xml).is_err(), "accepted {xml}");
    }
}

#[test]
fn bounded_input_and_edits_are_transactional() {
    let oversized = format!("<Preset>{}</Preset>", " ".repeat(MAX_PRESET_BYTES));
    assert!(PresetDocument::parse(&oversized).is_err());
    let deep = format!("<Preset>{}{}</Preset>", "<x>".repeat(65), "</x>".repeat(65));
    assert!(PresetDocument::parse(&deep).is_err());
    let mut preset = PresetDocument::parse(SAMPLE).unwrap();
    let mut metadata = preset.metadata();
    metadata.description = "\u{0}".into();
    assert!(preset.set_metadata(&metadata).is_err());
    assert_eq!(preset.to_xml().unwrap(), SAMPLE);
    metadata.description = "x".repeat(MAX_PRESET_BYTES);
    assert!(preset.set_metadata(&metadata).is_err());
    assert_eq!(preset.to_xml().unwrap(), SAMPLE);
}

#[test]
fn namespaces_do_not_collide_with_recognized_fields() {
    let source = "<Preset xmlns:x='urn:unknown'><x:Amplifier/><Amplifier><Module ID='1' x:ID='strange'><x:Param ControlIndex='unknown'/><Param ControlIndex='0'>1</Param></Module></Amplifier><FUSE><x:Info/><Info x:name='unchanged' name='old'/></FUSE></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    let mut metadata = preset.metadata();
    metadata.name = "new".into();
    preset.set_metadata(&metadata).unwrap();
    assert_eq!(preset.modules()[0].model_id, Some(1));
    assert!(preset.to_xml().unwrap().contains("x:name='unchanged'"));
    assert!(preset.to_xml().unwrap().contains("x:ID='strange'"));
}

#[test]
fn activation_and_positions_preserve_unrelated_bytes() {
    let mut preset = PresetDocument::parse(SAMPLE).unwrap();
    assert_eq!(preset.product_id(), Some(65000));
    assert_eq!(preset.modules()[1].active, Some(true));
    assert_eq!(preset.modules()[2].active, None);
    preset.set_module_active("Stompbox", false).unwrap();
    assert_eq!(
        preset.to_xml().unwrap(),
        SAMPLE.replace("BypassState=\"1\"", "BypassState=\"0\"")
    );
    preset
        .set_module_positions(&[("Delay", 7), ("Reverb", 6)])
        .unwrap();
    assert_eq!(preset.modules()[3].position, Some(7));
    assert_eq!(preset.modules()[4].position, Some(6));
    let before = preset.to_xml().unwrap();
    assert!(
        preset
            .set_module_positions(&[("Delay", 3), ("Missing", 2)])
            .is_err()
    );
    assert!(
        preset
            .set_module_positions(&[("Delay", 3), ("Delay", 2)])
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), before);
}

#[test]
fn model_change_preserves_extensions_and_replaces_only_known_state() {
    let source = "<Preset xmlns:x='urn:extension' ProductId='3'><FX><Delay><Module ID='99' POS='7' BypassState='0' x:future='retained'><Param ControlIndex='0' x:unit='opaque'>1<!--retained--></Param><Param ControlIndex='1'>2</Param><x:Param ControlIndex='200'>unknown</x:Param><x:Future/></Module></Delay></FX></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    let params = [
        PresetParameter {
            index: 0,
            value: 30,
        },
        PresetParameter {
            index: 3,
            value: 50,
        },
    ];
    preset
        .replace_module_model("Delay", 10, &params, Some(2))
        .unwrap();
    let module = &preset.modules()[0];
    assert_eq!(module.model_id, Some(10));
    assert_eq!(module.active, Some(false));
    assert_eq!(module.position, Some(7));
    assert_eq!(module.bypass_mode, Some(2));
    assert_eq!(module.parameters, params);
    let xml = preset.to_xml().unwrap();
    for fragment in [
        "x:future='retained'",
        "x:unit='opaque'",
        "<!--retained-->",
        "<x:Param ControlIndex='200'>unknown</x:Param>",
        "<x:Future/>",
    ] {
        assert!(xml.contains(fragment), "lost {fragment}");
    }
    assert!(!xml.contains("<Param ControlIndex='1'>"));
}

#[test]
fn model_replacement_can_expand_an_empty_module_and_empty_a_known_model() {
    let mut preset =
        PresetDocument::parse("<Preset><Amplifier><Module/></Amplifier></Preset>").unwrap();
    preset
        .replace_module_model(
            "Amplifier",
            42,
            &[PresetParameter {
                index: 0,
                value: 32,
            }],
            Some(1),
        )
        .unwrap();
    assert_eq!(preset.modules()[0].parameters[0].value, 32);
    assert_eq!(preset.modules()[0].bypass_mode, Some(1));
    preset
        .replace_module_model("Amplifier", 0, &[], None)
        .unwrap();
    assert!(preset.modules()[0].parameters.is_empty());
    preset.set_module_active("Amplifier", true).unwrap();
    assert_eq!(preset.modules()[0].active, Some(true));
}

#[test]
fn model_replacement_refuses_unknown_parameter_content_and_is_transactional() {
    for content in [
        "<Param ControlIndex='0' future='x'>1</Param>",
        "<Param ControlIndex='0'>1<!--preserve--></Param>",
    ] {
        let source =
            format!("<Preset><Amplifier><Module ID='1'>{content}</Module></Amplifier></Preset>");
        let mut preset = PresetDocument::parse(&source).unwrap();
        assert!(
            preset
                .replace_module_model("Amplifier", 2, &[], Some(1))
                .is_err()
        );
        assert_eq!(preset.to_xml().unwrap(), source);
    }
    let mut preset = PresetDocument::parse(SAMPLE).unwrap();
    let duplicate = [
        PresetParameter { index: 0, value: 1 },
        PresetParameter { index: 0, value: 2 },
    ];
    assert!(
        preset
            .replace_module_model("Amplifier", 2, &duplicate, None)
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), SAMPLE);
}

#[test]
fn dependent_parameter_updates_are_atomic() {
    let mut preset = PresetDocument::parse(SAMPLE).unwrap();
    let updates = [
        PresetParameter { index: 0, value: 1 },
        PresetParameter { index: 1, value: 2 },
    ];
    preset.set_parameters("Delay", &updates).unwrap();
    assert_eq!(preset.modules()[3].parameters, updates);
    let before = preset.to_xml().unwrap();
    assert!(
        preset
            .set_parameters(
                "Delay",
                &[
                    PresetParameter { index: 0, value: 7 },
                    PresetParameter {
                        index: 255,
                        value: 8
                    }
                ]
            )
            .is_err()
    );
    assert!(
        preset
            .set_parameters(
                "Delay",
                &[
                    PresetParameter { index: 0, value: 7 },
                    PresetParameter { index: 0, value: 8 }
                ]
            )
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), before);
}

#[test]
fn rejects_malformed_known_module_attributes_without_guessing() {
    for attribute in [
        "BypassState='false'",
        "BypassState='256'",
        "BypassMode='256'",
    ] {
        assert!(
            PresetDocument::parse(&format!(
                "<Preset><Amplifier><Module {attribute}/></Amplifier></Preset>"
            ))
            .is_err()
        );
    }
    assert!(PresetDocument::parse("<Preset ProductId='oops'/>").is_err());
    let preset = PresetDocument::parse("<Preset><Amplifier><Module BypassState='2' BypassMode='255' POS='65535'/></Amplifier></Preset>").unwrap();
    assert_eq!(preset.modules()[0].active, Some(true));
    assert_eq!(preset.modules()[0].bypass_mode, Some(255));
    assert_eq!(preset.modules()[0].position, Some(65535));
}

#[test]
fn chain_insert_shifts_occupants_within_bank_instead_of_swapping() {
    let xml = SAMPLE.replace("ProductId=\"65000\"", "ProductId=\"1\"");
    let mut preset = PresetDocument::parse(&xml).unwrap();
    preset.move_module("Reverb", 0).unwrap();
    let positions: Vec<_> = preset
        .modules()
        .iter()
        .skip(1)
        .map(|module| module.position.unwrap())
        .collect();
    assert_eq!(positions, [1, 2, 3, 0]);
    preset.move_module("Reverb", 3).unwrap();
    let positions: Vec<_> = preset
        .modules()
        .iter()
        .skip(1)
        .map(|module| module.position.unwrap())
        .collect();
    assert_eq!(positions, [0, 1, 2, 3]);
    preset.move_module("Delay", 5).unwrap();
    preset.move_module("Reverb", 5).unwrap();
    assert_eq!(preset.modules()[3].position, Some(6));
    assert_eq!(preset.modules()[4].position, Some(5));
    // No move crosses the amplifier boundary except the requested source.
    assert_eq!(preset.modules()[1].position, Some(0));
    assert_eq!(preset.modules()[2].position, Some(1));
}

#[test]
fn chain_family_constraints_and_unknown_shapes_fail_transactionally() {
    for product in [65000, 2259] {
        let source = SAMPLE.replace("ProductId=\"65000\"", &format!("ProductId=\"{product}\""));
        let mut preset = PresetDocument::parse(&source).unwrap();
        assert!(preset.move_module("Delay", 4).is_err());
        assert_eq!(preset.to_xml().unwrap(), source);
    }
    let source = SAMPLE.replace("ProductId=\"65000\"", "ProductId=\"10\"");
    let mut preset = PresetDocument::parse(&source).unwrap();
    assert!(preset.move_module("Stompbox", 1).is_err());
    assert!(preset.move_module("Delay", 4).is_err());
    assert_eq!(preset.to_xml().unwrap(), source);
    preset.move_module("Delay", 1).unwrap();
    assert_eq!(preset.modules()[3].position, Some(1));
    assert_eq!(preset.modules()[2].position, Some(1)); // G-DEC category bays may share pre/post state.
    let source = SAMPLE.replace("ProductId=\"65000\"", "ProductId=\"1\"");
    let mut preset = PresetDocument::parse(&source).unwrap();
    preset.set_module_position("Delay", 0).unwrap();
    let before = preset.to_xml().unwrap();
    assert!(preset.move_module("Reverb", 4).is_err());
    assert_eq!(preset.to_xml().unwrap(), before);
}

#[test]
fn root_product_identifier_respects_namespace_and_duplicate_rejection() {
    let preset =
        PresetDocument::parse("<Preset xmlns:x='urn:future' x:ProductId='opaque' ProductId='13'/>")
            .unwrap();
    assert_eq!(preset.product_id(), Some(13));
    let preset = PresetDocument::parse("<Preset xmlns:x='urn:future' x:ProductId='13'/>").unwrap();
    assert_eq!(preset.product_id(), None);
    assert!(PresetDocument::parse("<Preset ProductId='1' ProductId='13'/>").is_err());
}

fn delay_defaults() -> (u16, u8, Vec<PresetParameter>) {
    use openfuse_core::model_catalog::{self, ModuleCategory};
    let model = model_catalog::model(13, ModuleCategory::Delay, 43).unwrap();
    (
        model.id,
        model.expression_pedal_index,
        model
            .parameters
            .iter()
            .map(|p| PresetParameter {
                index: p.index,
                value: p.default_raw,
            })
            .collect(),
    )
}

#[test]
fn model_change_updates_first_expression_default_and_preserves_second_binding() {
    let (id, default_index, params) = delay_defaults();
    let second_index = params.last().unwrap().index;
    let source = format!(
        "<Preset ProductId='13' xmlns:x='urn:future'><FX><Delay ID='3'><Module ID='43' POS='4'><Param ControlIndex='0'>1</Param></Module></Delay></FX><FirstExpressionPedal ExpressionModeBehavior='4' ParameterIndex='254' x:future='keep'><x:Extension/></FirstExpressionPedal><SecondExpressionPedal PedalIndex='1' ExpressionModeBehavior='4' ParameterIndex='{second_index}' FutureValue='keep'/></Preset>"
    );
    let mut preset = PresetDocument::parse(&source).unwrap();
    preset
        .replace_module_model("Delay", id, &params, None)
        .unwrap();
    let xml = preset.to_xml().unwrap();
    assert!(xml.contains(&format!(
        "ExpressionModeBehavior='4' ParameterIndex='{default_index}' x:future='keep'"
    )));
    assert!(xml.contains("<x:Extension/>"));
    assert!(xml.contains(&format!("<SecondExpressionPedal PedalIndex='1' ExpressionModeBehavior='4' ParameterIndex='{second_index}' FutureValue='keep'/>")));
}

#[test]
fn removal_disables_only_expression_pedals_associated_with_removed_effect() {
    let source = "<Preset ProductId='7'><FX><Delay ID='3'><Module ID='43' POS='4'><Param ControlIndex='0'>1</Param></Module></Delay></FX><FirstExpressionPedal ExpressionModeBehavior='4' ParameterIndex='3' Future='keep'/><SecondExpressionPedal ExpressionModeBehavior='2' ParameterIndex='7'/></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    preset.replace_module_model("Delay", 0, &[], None).unwrap();
    let xml = preset.to_xml().unwrap();
    assert!(xml.contains(
        "<FirstExpressionPedal ExpressionModeBehavior='0' ParameterIndex='0' Future='keep'/>"
    ));
    assert!(xml.contains("<SecondExpressionPedal ExpressionModeBehavior='2' ParameterIndex='7'/>"));
}

#[test]
fn invalid_or_unknown_expression_bindings_block_model_change_atomically() {
    let (id, _, params) = delay_defaults();
    for pedal in [
        "<FirstExpressionPedal ParameterIndex='1'/>",
        "<FirstExpressionPedal ExpressionModeBehavior='100' ParameterIndex='1'/>",
        "<FirstExpressionPedal ExpressionModeBehavior='4'/><FirstExpressionPedal ExpressionModeBehavior='4'/>",
        "<SecondExpressionPedal ExpressionModeBehavior='4' ParameterIndex='254'/>",
        "<SecondExpressionPedal ExpressionModeBehavior='4'/>",
    ] {
        let xml = format!(
            "<Preset ProductId='13'><FX><Delay><Module ID='43' POS='4'/></Delay></FX>{pedal}</Preset>"
        );
        let mut preset = PresetDocument::parse(&xml).unwrap();
        assert!(
            preset
                .replace_module_model("Delay", id, &params, None)
                .is_err(),
            "accepted {pedal}"
        );
        assert_eq!(preset.to_xml().unwrap(), xml);
    }
    let source = "<Preset ProductId='65000'><FX><Delay><Module ID='43'/></Delay></FX><FirstExpressionPedal ExpressionModeBehavior='4' ParameterIndex='1'/></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    assert!(
        preset
            .replace_module_model("Delay", id, &params, None)
            .is_err()
    );
    assert_eq!(preset.to_xml().unwrap(), source);
}
