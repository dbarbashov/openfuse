use openfuse_core::{
    conversion::{self, ChangeKind, Options, Plan},
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument,
        expression::{ExpressionPedal, PedalSlot},
    },
};
fn fixture(product: u32) -> String {
    let mut xml = format!(
        "<Preset ProductId=\"{product}\" amplifier=\"Original name\" vendor=\"retained\"><!--source-->"
    );
    for (kind, key, category) in [
        (0, "Amplifier", ModuleCategory::Amplifier),
        (1, "Stompbox", ModuleCategory::Stompbox),
        (2, "Modulation", ModuleCategory::Modulation),
        (3, "Delay", ModuleCategory::Delay),
        (4, "Reverb", ModuleCategory::Reverb),
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
        xml.push_str(&format!("<{key} ID=\"{kind}\"><Module ID=\"{id}\" POS=\"0\" BypassState=\"1\" BypassMode=\"{}\">",model.bypass_mode));
        for p in &model.parameters {
            xml.push_str(&format!(
                "<Param ControlIndex=\"{}\">{}</Param>",
                p.index, p.default_raw
            ));
        }
        xml.push_str(&format!("</Module></{key}>"));
    }
    xml.push_str("</FX><Band/><FUSE><Info name=\"Original\" author=\"Keep\">Description</Info></FUSE><Future>unchanged</Future></Preset>");
    let mut doc = PresetDocument::parse(&xml).unwrap();
    if !matches!(product, 10 | 11) {
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
    doc.to_xml().unwrap()
}
fn options(target: u32) -> Options {
    Options {
        target_product: target,
        repair_double_entities: false,
        normalize_legacy_delays: false,
        assume_missing_gdec: false,
    }
}
#[test]
fn conversion_is_source_bound_and_preserves_unrelated_xml() {
    let original = fixture(13);
    let copy = original.clone();
    let plan = Plan::preview(&original, options(1)).unwrap();
    let candidate = plan.result().to_xml().unwrap();
    assert_eq!(original, copy);
    assert!(
        candidate.contains("vendor=\"retained\"")
            && candidate.contains("<!--source-->")
            && candidate.contains("<Future>unchanged</Future>")
    );
    assert_eq!(plan.source_product(), Some(13));
    assert_eq!(plan.target_product(), 1);
    assert_eq!(plan.apply(&original).unwrap().product_id(), Some(1));
    assert!(plan.apply(&(original + " ")).is_err());
}
#[test]
fn original_compatibility_routes_are_explicit() {
    assert_eq!(conversion::supported_targets(13), vec![1, 13, 7, 2259, 6]);
    assert!(Plan::preview(&fixture(11), options(1)).is_err());
    assert!(Plan::preview(&fixture(10), options(1)).is_err());
    assert!(Plan::preview(&fixture(1), options(11)).is_err());
    assert!(
        Plan::preview(
            &fixture(1).replace("ProductId=\"1\"", "ProductId=\"2\""),
            options(1)
        )
        .is_err()
    );
    assert!(Plan::preview(&fixture(1).replace("ProductId=\"1\"", ""), options(10)).is_err());
}
#[test]
fn double_entity_repair_is_single_pass_and_opt_in() {
    let raw = "&amp;amp;|&amp;quot;|&amp;apos;|&amp;lt;|&amp;gt;|&amp;#65;|&amp;#x41;|&amp;#X41;|&amp;other;";
    assert_eq!(
        conversion::repair_double_encoded_xml(raw).unwrap(),
        "&amp;|&quot;|'|&lt;|&gt;|&#65;|&#x41;|&amp;#X41;|&amp;other;"
    );
    let original = fixture(1).replace("name=\"Original\"", "name=\"A &amp;amp; B\"");
    let normal = Plan::preview(&original, options(1)).unwrap();
    assert_eq!(normal.result().metadata().name, "A &amp; B");
    let mut opts = options(1);
    opts.repair_double_entities = true;
    let repaired = Plan::preview(&original, opts).unwrap();
    assert_eq!(repaired.result().metadata().name, "A & B");
    assert!(
        repaired
            .changes()
            .iter()
            .any(|c| c.kind == ChangeKind::Repair)
    );
}
#[test]
fn repair_reports_failure_without_damaging_single_quoted_input() {
    let original = fixture(1).replace("name=\"Original\"", "name='A &amp;apos; B'");
    assert!(PresetDocument::parse(&original).is_ok());
    let mut opts = options(1);
    opts.repair_double_entities = true;
    assert!(Plan::preview(&original, opts).is_err());
    assert!(original.contains("&amp;apos;"));
}
#[test]
fn legacy_repair_budget_is_enforced_before_large_report_allocation() {
    assert!(conversion::repair_double_encoded_xml(&"&amp;amp;".repeat(4096)).is_ok());
    assert!(conversion::repair_double_encoded_xml(&"&amp;amp;".repeat(4097)).is_err());
    assert!(Plan::preview(&"x".repeat(2 * 1024 * 1024 + 1), options(1)).is_err());
}
#[test]
fn known_unavailable_amp_is_replaced_with_target_defaults_and_inactive() {
    let mut doc = PresetDocument::parse(&fixture(13)).unwrap();
    let model = model_catalog::model(13, ModuleCategory::Amplifier, 249).unwrap();
    doc.replace_module_model(
        "Amplifier",
        249,
        &model
            .parameters
            .iter()
            .map(|p| openfuse_core::preset::PresetParameter {
                index: p.index,
                value: p.default_raw,
            })
            .collect::<Vec<_>>(),
        Some(model.bypass_mode),
    )
    .unwrap();
    let plan = Plan::preview(&doc.to_xml().unwrap(), options(1)).unwrap();
    let amp = plan
        .result()
        .modules()
        .into_iter()
        .find(|m| m.key == "Amplifier")
        .unwrap();
    assert_eq!(amp.model_id, Some(124));
    assert_eq!(amp.active, Some(false));
    assert!(
        plan.changes()
            .iter()
            .any(|c| c.kind == ChangeKind::Replacement)
    );
    assert!(
        plan.changes()
            .iter()
            .any(|c| c.kind == ChangeKind::Loss && c.path.contains("Param"))
    );
}
#[test]
fn unavailable_effect_becomes_original_off_model() {
    let base = fixture(13);
    let model = model_catalog::models(13, ModuleCategory::Stompbox)
        .find(|m| model_catalog::model(1, ModuleCategory::Stompbox, m.id).is_none())
        .unwrap();
    let mut doc = PresetDocument::parse(&base).unwrap();
    doc.replace_module_model(
        "Stompbox",
        model.id,
        &model
            .parameters
            .iter()
            .map(|p| openfuse_core::preset::PresetParameter {
                index: p.index,
                value: p.default_raw,
            })
            .collect::<Vec<_>>(),
        Some(model.bypass_mode),
    )
    .unwrap();
    let plan = Plan::preview(&doc.to_xml().unwrap(), options(1)).unwrap();
    let module = plan
        .result()
        .modules()
        .into_iter()
        .find(|m| m.key == "Stompbox")
        .unwrap();
    assert_eq!(module.model_id, Some(0));
    assert_eq!(module.active, Some(false));
    assert!(module.parameters.is_empty());
}
#[test]
fn floor_auxiliary_losses_and_target_defaults_are_visible() {
    let input = fixture(7).replace("</Preset>", "<FxLoopBypass>true</FxLoopBypass></Preset>");
    let plan = Plan::preview(&input, options(1)).unwrap();
    let result = plan.result().to_xml().unwrap();
    assert!(!result.contains("SecondExpressionPedal") && !result.contains("FxLoopBypass"));
    for key in ["SecondExpressionPedal", "FxLoopBypass"] {
        assert!(
            plan.changes()
                .iter()
                .any(|c| c.kind == ChangeKind::Loss && c.path == key)
        );
    }
    let plan = Plan::preview(&fixture(1), options(7)).unwrap();
    let result = plan.result().to_xml().unwrap();
    assert!(
        result.contains("SecondExpressionPedal")
            && result.contains("<FxLoopBypass>false</FxLoopBypass>")
    );
}
#[test]
fn expression_parameter_binding_survives_model_substitution() {
    let input = fixture(13)
        .replace(
            "ExpressionModeBehavior=\"6\"",
            "ExpressionModeBehavior=\"1\"",
        )
        .replace("ParameterIndex=\"0\"", "ParameterIndex=\"4\"");
    let result = Plan::preview(&input, options(1)).unwrap();
    assert_eq!(
        result
            .result()
            .expression_pedal(PedalSlot::First)
            .unwrap()
            .unwrap()
            .parameter_index,
        Some(4)
    );
}
#[test]
fn original_load_does_not_apply_editor_cascading_rules() {
    let input = fixture(13).replace(
        "<Param ControlIndex=\"15\">0</Param>",
        "<Param ControlIndex=\"15\">1</Param>",
    );
    let before = PresetDocument::parse(&input).unwrap().modules()[0]
        .parameters
        .clone();
    let result = Plan::preview(&input, options(13)).unwrap();
    assert_eq!(result.result().modules()[0].parameters, before);
}
#[test]
fn delay_boundaries_retain_original_unchecked_semantics() {
    for (kind, max, length) in [(136, 1110, 1111), (137, 555, 556), (138, 370, 371)] {
        let min = conversion::legacy_delay_value(kind, 30).unwrap();
        assert!(!min.was_dirty);
        assert_eq!(min.milliseconds, 30);
        let end = conversion::legacy_delay_value(kind, max).unwrap();
        assert_eq!(end.normalized, 65535);
        assert_eq!(end.milliseconds, max);
        let overflow = conversion::legacy_delay_value(kind, length).unwrap();
        assert!(!overflow.was_dirty);
        assert!(overflow.normalized < 200);
        assert_eq!(overflow.milliseconds, 30);
        let old = conversion::legacy_delay_value(kind, 65535).unwrap();
        assert!(old.was_dirty);
        assert_eq!(old.milliseconds, max);
    }
}
#[test]
fn ambiguous_gdec_delay_requires_explicit_normalization() {
    let product = 10;
    let model = model_catalog::models(product, ModuleCategory::Delay)
        .find(|m| m.parameters.iter().any(|p| matches!(p.type_id, 136..=138)))
        .unwrap();
    let mut doc = PresetDocument::parse(&fixture(product)).unwrap();
    let parameters = model
        .parameters
        .iter()
        .map(|p| openfuse_core::preset::PresetParameter {
            index: p.index,
            value: if matches!(p.type_id, 136..=138) {
                65535
            } else {
                p.default_raw
            },
        })
        .collect::<Vec<_>>();
    doc.replace_module_model("Delay", model.id, &parameters, None)
        .unwrap();
    let original = doc.to_xml().unwrap();
    assert!(Plan::preview(&original, options(product)).is_err());
    let mut opts = options(product);
    opts.normalize_legacy_delays = true;
    let result = Plan::preview(&original, opts).unwrap();
    assert!(
        result
            .changes()
            .iter()
            .any(|c| c.kind == ChangeKind::Normalization && c.path.starts_with("Delay/"))
    );
    assert_eq!(doc.to_xml().unwrap(), original);
}
#[test]
fn malformed_and_unsupported_structures_fail_without_guessing() {
    let original = fixture(13);
    assert!(
        Plan::preview(
            &original.replace("<Modulation ID=\"2\">", "<Modulation ID=\"3\">"),
            options(1)
        )
        .is_err()
    );
    assert!(
        Plan::preview(
            &original.replacen("POS=\"0\"", "POS=\"999\"", 2),
            options(1)
        )
        .is_err()
    );
    assert!(
        Plan::preview(
            "<!DOCTYPE Preset [<!ENTITY x 'value'>]><Preset ProductId=\"1\">&x;</Preset>",
            options(1)
        )
        .is_err()
    );
}

#[test]
fn missing_and_invalid_legacy_ids_require_explicit_gdec_assumption() {
    let base = fixture(10);
    for raw in [
        "",
        "ProductId=\"not-a-number\"",
        "ProductId=\"65536\"",
        "ProductId=\"-1\"",
    ] {
        let source = base.replace("ProductId=\"10\"", raw);
        assert!(Plan::preview(&source, options(10)).is_err());
        let mut opts = options(10);
        opts.assume_missing_gdec = true;
        let plan = Plan::preview(&source, opts).unwrap();
        assert_eq!(plan.source_product(), None);
        assert_eq!(plan.result().product_id(), Some(10));
        assert!(
            plan.changes()
                .iter()
                .any(|c| c.kind == ChangeKind::Repair && c.path == "Preset/@ProductId")
        );
        opts.target_product = 1;
        assert!(Plan::preview(&source, opts).is_err());
    }
    for valid_unknown in [2, 9999] {
        let source = base.replace(
            "ProductId=\"10\"",
            &format!("ProductId=\"{valid_unknown}\""),
        );
        let mut opts = options(10);
        opts.assume_missing_gdec = true;
        assert!(Plan::preview(&source, opts).is_err());
    }
}

#[test]
fn original_fallback_to_unregistered_target_models_is_explicitly_refused() {
    for product in [10, 11] {
        let original = fixture(product);
        let id = model_catalog::product(product).unwrap().default_amp_id;
        for unsupported in [249, 252, 255] {
            let source = original.replacen(
                &format!("<Module ID=\"{id}\""),
                &format!("<Module ID=\"{unsupported}\""),
                1,
            );
            let failure = Plan::preview(&source, options(product)).unwrap_err();
            assert!(
                failure
                    .to_string()
                    .contains("absent from the target catalog")
            );
            assert!(source.contains(&format!("<Module ID=\"{unsupported}\"")));
        }
    }
}
