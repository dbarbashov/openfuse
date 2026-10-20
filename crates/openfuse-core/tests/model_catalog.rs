use openfuse_core::model_catalog::{self as catalog, ModuleCategory, cascade_parameters};
use std::collections::HashSet;

#[test]
fn complete_fact_table_has_unique_identifiers_and_total_formatters() {
    let mut count = 0;
    let mut params = 0;
    for product in catalog::products() {
        for category in [
            ModuleCategory::Amplifier,
            ModuleCategory::Stompbox,
            ModuleCategory::Modulation,
            ModuleCategory::Delay,
            ModuleCategory::Reverb,
        ] {
            let mut ids = HashSet::new();
            for model in catalog::models(product.id, category) {
                count += 1;
                assert!(ids.insert(model.id));
                let mut indices = HashSet::new();
                for p in &model.parameters {
                    params += 1;
                    assert!(indices.insert(p.index));
                    for value in [0, p.default_raw, 32768, 65535] {
                        assert!(!p.format_value(value).is_empty());
                    }
                }
            }
        }
    }
    assert_eq!(count, 382);
    assert_eq!(params, 3265);
    assert!(catalog::product(99999).is_none());
    assert!(catalog::model(13, ModuleCategory::Amplifier, 65000).is_none());
}

#[test]
fn xml_defaults_distinguish_parameter_and_physical_control_ids() {
    let model = catalog::model(13, ModuleCategory::Amplifier, 103).unwrap();
    assert!(model.name.contains("Deluxe"));
    let tremolo = catalog::model(10, ModuleCategory::Modulation, 14).unwrap();
    assert!(
        tremolo
            .parameters
            .iter()
            .any(|p| p.index == 2 && p.control_index == 5)
    );
    let delay = catalog::models(10, ModuleCategory::Delay)
        .flat_map(|m| &m.parameters)
        .find(|p| p.type_id == 136)
        .unwrap();
    assert!((30..=1110).contains(&delay.default_raw));
    assert_eq!(delay.format_value(1110), "1110 ms");
    assert_eq!(delay.raw_range(), 30..=1110);
}

#[test]
fn enum_identity_is_not_dictionary_insertion_order() {
    let cab = catalog::models(11, ModuleCategory::Amplifier)
        .flat_map(|m| &m.parameters)
        .find(|p| p.type_id == 147)
        .unwrap();
    let choices = cab.choices();
    assert_eq!(
        choices.iter().find(|c| c.value == 13).unwrap().label,
        "4x10 vintage"
    );
    assert_eq!(
        choices.iter().find(|c| c.value == 5).unwrap().label,
        "8x10 modern"
    );
    assert_eq!(cab.format_value(60000), "Custom (60000)");
}

#[test]
fn gate_macros_and_manual_edits_keep_custom_state_without_feedback() {
    let current = vec![(15, 0), (16, 9), (9, 123), (254, 50000)];
    let updated = cascade_parameters(13, ModuleCategory::Amplifier, 103, &current, 15, 2);
    assert_eq!(updated, vec![(15, 2), (16, 3), (9, 65535), (254, 50000)]);
    let manual = cascade_parameters(13, ModuleCategory::Amplifier, 103, &updated, 16, 4);
    assert_eq!(manual, vec![(15, 5), (16, 4), (9, 65535), (254, 50000)]);
    assert_eq!(
        cascade_parameters(13, ModuleCategory::Amplifier, 103, &current, 15, 0),
        current
    );
    assert_eq!(
        cascade_parameters(65000, ModuleCategory::Amplifier, 103, &current, 15, 2),
        vec![(15, 2), (16, 9), (9, 123), (254, 50000)]
    );
}

#[test]
fn every_shipped_parameter_accepts_cascades_and_preserves_unknown_fields() {
    for product in catalog::products() {
        for model in catalog::models(product.id, ModuleCategory::Amplifier) {
            let mut params: Vec<_> = model
                .parameters
                .iter()
                .map(|p| (p.index, p.default_raw))
                .collect();
            params.push((254, 60000));
            for p in &model.parameters {
                for value in p
                    .choices()
                    .iter()
                    .map(|c| c.value)
                    .chain([0, 1, 5, 13, 32768, 65535])
                {
                    let out = cascade_parameters(
                        product.id,
                        ModuleCategory::Amplifier,
                        model.id,
                        &params,
                        p.index,
                        value,
                    );
                    assert_eq!(out.len(), params.len());
                    assert_eq!(out.last(), Some(&(254, 60000)));
                }
            }
        }
    }
}

#[test]
fn cascading_results_match_1309_original_cil_executions() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/rule_vectors.json")).unwrap();
    for vector in fixture["vectors"].as_array().unwrap() {
        let pid = vector["product"].as_u64().unwrap() as u32;
        let baseline = &fixture["baselines"][pid.to_string()]["values"];
        let before: Vec<(u8, u16)> = serde_json::from_value(baseline.clone()).unwrap();
        let changes: Vec<(u8, u16)> = serde_json::from_value(vector["changes"].clone()).unwrap();
        let mut expected = before.clone();
        for (index, value) in changes {
            *expected.iter_mut().find(|(i, _)| *i == index).unwrap() = (index, value);
        }
        let actual = cascade_parameters(
            pid,
            ModuleCategory::Amplifier,
            vector["model"].as_u64().unwrap() as u16,
            &before,
            vector["index"].as_u64().unwrap() as u8,
            vector["value"].as_u64().unwrap() as u16,
        );
        assert_eq!(actual, expected, "original CIL vector {vector}");
    }
}
