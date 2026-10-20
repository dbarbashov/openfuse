use super::*;
pub(super) fn convert(
    xml: &str,
    root: Node<'_, '_>,
    input: &PresetDocument,
    options: Options,
    edits: &mut Vec<Edit>,
    changes: &mut Vec<Change>,
) -> Result<()> {
    let target = model_catalog::product(options.target_product).unwrap();
    let fx = child(root, "FX")?.ok_or_else(|| error("Missing FX container"))?;
    for (kind, key, category) in crate::wire_preset::CATEGORIES {
        let wrapper = child(if key == "Amplifier" { root } else { fx }, key)?
            .ok_or_else(|| error(format!("Missing {key}")))?;
        if wrapper
            .attribute("ID")
            .map(str::parse::<u8>)
            .transpose()
            .map_err(error)?
            .unwrap_or(0)
            != kind - 5
        {
            return Err(error(format!(
                "{key} category ID differs from original loader semantics"
            )));
        }
        let node =
            child(wrapper, "Module")?.ok_or_else(|| error(format!("Missing {key} Module")))?;
        let module = input.modules().into_iter().find(|m| m.key == key).unwrap();
        let old_id = module.model_id.ok_or_else(|| error("Missing model ID"))?;
        let direct = model_catalog::model(target.id, category, old_id);
        let id = if direct.is_some() {
            old_id
        } else if category == ModuleCategory::Amplifier {
            match old_id {
                249 => 124,
                252 => 109,
                255 => 94,
                _ => target.default_amp_id,
            }
        } else {
            0
        };
        let model = model_catalog::model(target.id, category, id)
            .ok_or_else(|| error("Original substitute is absent from the target catalog"))?;
        let replaced = direct.is_none();
        let base = format!("{key}/Module");
        attr(
            xml,
            node,
            "ID",
            Some(id.to_string()),
            &format!("{base}/@ID"),
            ChangeKind::Replacement,
            "Original unavailable-model substitution",
            edits,
            changes,
        )?;
        let position = if key == "Amplifier" {
            0
        } else {
            module
                .position
                .ok_or_else(|| error("Missing module position"))?
        };
        if position > 255 {
            return Err(error(
                "Original output cannot represent module position above 255",
            ));
        }
        attr(
            xml,
            node,
            "POS",
            Some(position.to_string()),
            &format!("{base}/@POS"),
            ChangeKind::Normalization,
            "Original stored module position",
            edits,
            changes,
        )?;
        let active = if replaced {
            false
        } else if target.store_bypass {
            module
                .active
                .ok_or_else(|| error("Target requires a declared BypassState"))?
        } else {
            true
        };
        attr(
            xml,
            node,
            "BypassState",
            target.store_bypass.then(|| u8::from(active).to_string()),
            &format!("{base}/@BypassState"),
            if target.store_bypass {
                ChangeKind::Normalization
            } else {
                ChangeKind::Loss
            },
            "Original target activation semantics",
            edits,
            changes,
        )?;
        attr(
            xml,
            node,
            "BypassMode",
            (target.id == 2259).then(|| model.bypass_mode.to_string()),
            &format!("{base}/@BypassMode"),
            if target.id == 2259 {
                ChangeKind::Normalization
            } else {
                ChangeKind::Loss
            },
            "Original loader resets bypass mode to catalog default; only Super Champ serializes it",
            edits,
            changes,
        )?;
        let params = node
            .children()
            .filter(|n| n.has_tag_name("Param"))
            .collect::<Vec<_>>();
        if replaced {
            for parameter in params {
                let index = parameter.attribute("ControlIndex").unwrap_or("?");
                change(
                    changes,
                    ChangeKind::Loss,
                    format!("{base}/Param[{index}]"),
                    Some(xml[parameter.range()].into()),
                    None,
                    "Source model parameter removed by original replacement",
                );
                edits.push((parameter.range(), String::new()));
            }
            let mut defaults = String::new();
            for p in &model.parameters {
                defaults.push_str(&format!(
                    "<Param ControlIndex=\"{}\">{}</Param>",
                    p.index, p.default_raw
                ));
            }
            if !defaults.is_empty() {
                preset::append_child(xml, node, &defaults, edits).map_err(error)?;
                change(
                    changes,
                    ChangeKind::Normalization,
                    format!("{base}/Param"),
                    None,
                    Some(defaults),
                    "Original replacement installs target catalog defaults",
                );
            }
        } else {
            if params.len() < model.parameters.len() {
                return Err(error(format!(
                    "{key}: original target loader requires {} input parameters",
                    model.parameters.len()
                )));
            }
            for (offset, parameter) in params.into_iter().enumerate() {
                let index = parameter.attribute("ControlIndex").unwrap_or("?");
                if offset >= model.parameters.len() {
                    change(
                        changes,
                        ChangeKind::Loss,
                        format!("{base}/Param[{index}]"),
                        Some(xml[parameter.range()].into()),
                        None,
                        "Original target loader ignores excess input parameters",
                    );
                    edits.push((parameter.range(), String::new()));
                    continue;
                }
                let index = index.parse::<u8>().map_err(error)?;
                let definition = model
                    .parameters
                    .iter()
                    .find(|p| p.index == index)
                    .ok_or_else(|| error("Input parameter index is unknown in target model"))?;
                let value = text(parameter).trim().parse::<u16>().map_err(error)?;
                if matches!(definition.type_id, 136..=138) {
                    let transformed = legacy_delay_value(definition.type_id, value)?;
                    let max = match definition.type_id {
                        136 => 1110,
                        137 => 555,
                        _ => 370,
                    };
                    if !options.normalize_legacy_delays && !(30..=max).contains(&value) {
                        return Err(error(
                            "Legacy G-DEC delay requires explicit normalize_legacy_delays",
                        ));
                    }
                    if options.normalize_legacy_delays && transformed.milliseconds != value {
                        preset::edit_text(
                            xml,
                            parameter,
                            &transformed.milliseconds.to_string(),
                            edits,
                        )
                        .map_err(error)?;
                        change(
                            changes,
                            ChangeKind::Normalization,
                            format!("{base}/Param[{index}]"),
                            Some(value.to_string()),
                            Some(transformed.milliseconds.to_string()),
                            format!(
                                "Original delay type {} load/save: internal normalized {}, legacy dirty {}",
                                definition.type_id, transformed.normalized, transformed.was_dirty
                            ),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
