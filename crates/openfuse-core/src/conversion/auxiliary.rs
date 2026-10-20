use super::*;
pub(super) fn convert(
    xml: &str,
    root: Node<'_, '_>,
    input: &PresetDocument,
    options: Options,
    edits: &mut Vec<Edit>,
    changes: &mut Vec<Change>,
) -> Result<()> {
    let target = options.target_product;
    let expression = matches!(target, 1 | 6 | 7 | 13 | 2259);
    for (key, supported, default) in [
        ("UsbGain", target != 10, "<UsbGain>0</UsbGain>"),
        (
            "FxLoopBypass",
            target == 7,
            "<FxLoopBypass>false</FxLoopBypass>",
        ),
    ] {
        match (child(root, key)?, supported) {
            (Some(node), false) => remove(xml, node, key, edits, changes),
            (None, true) => {
                preset::append_child(xml, root, default, edits).map_err(error)?;
                change(
                    changes,
                    ChangeKind::Normalization,
                    key,
                    None,
                    Some(default.into()),
                    "Original loader supplies missing target setting",
                );
            }
            (Some(node), true) => {
                let value = text(node);
                if key == "UsbGain" {
                    value.trim().parse::<u8>().map_err(error)?;
                } else if !matches!(value.trim(), "true" | "True" | "false" | "False") {
                    return Err(error("Invalid original FX loop boolean"));
                }
            }
            _ => {}
        }
    }
    for (key, slot) in [
        ("FirstExpressionPedal", preset::expression::PedalSlot::First),
        (
            "SecondExpressionPedal",
            preset::expression::PedalSlot::Second,
        ),
    ] {
        let supported = expression && (slot == preset::expression::PedalSlot::First || target == 7);
        let node = child(root, key)?;
        if !supported {
            if let Some(node) = node {
                remove(xml, node, key, edits, changes);
            }
            continue;
        }
        let Some(node) = node else {
            let second = slot == preset::expression::PedalSlot::Second;
            let first_attrs = if second {
                "PedalIndex=\"1\""
            } else {
                "VolumeModeBehavior=\"1\" PedalOverrideState=\"0\""
            };
            let content = format!(
                "<{key} {first_attrs} ExpressionModeBehavior=\"6\" ParameterIndex=\"0\" HeelSetting=\"0\" ToeSetting=\"65535\" PedalMode=\"0\" BypassEffectWhenVolumeMode=\"1\" VolumeSwitchRevert=\"0\" DefaultPedalState=\"0\"/>"
            );
            preset::append_child(xml, root, &content, edits).map_err(error)?;
            change(
                changes,
                ChangeKind::Normalization,
                key,
                None,
                Some(content),
                "Original missing-pedal defaults",
            );
            continue;
        };
        let pedal = input
            .expression_pedal(slot)
            .map_err(error)?
            .ok_or_else(|| error("Missing expression projection"))?;
        if pedal.pedal_index.unwrap_or(0) != slot.index() {
            return Err(error(
                "Expression PedalIndex differs from its element; conversion is ambiguous",
            ));
        }
        for name in [
            "ExpressionModeBehavior",
            "HeelSetting",
            "ToeSetting",
            "PedalMode",
            "BypassEffectWhenVolumeMode",
        ] {
            node.attribute(name)
                .ok_or_else(|| error(format!("Missing {key}/@{name}")))?
                .parse::<u16>()
                .map_err(error)?;
        }
        let mode = pedal.expression_mode_behavior.unwrap();
        let parameter = if (1..=5).contains(&mode) {
            pedal
                .parameter_index
                .ok_or_else(|| error("Expression assignment is missing ParameterIndex"))?
        } else {
            0
        };
        attr(
            xml,
            node,
            "ParameterIndex",
            Some(parameter.to_string()),
            &format!("{key}/@ParameterIndex"),
            ChangeKind::Normalization,
            "Original expression assignment serialization",
            edits,
            changes,
        )?;
        attr(
            xml,
            node,
            "BypassEffectWhenVolumeMode",
            Some(u8::from(pedal.bypass_effect_when_volume_mode.unwrap_or(0) != 0).to_string()),
            &format!("{key}/@BypassEffectWhenVolumeMode"),
            ChangeKind::Normalization,
            "Original boolean normalization",
            edits,
            changes,
        )?;
        for name in ["VolumeSwitchRevert", "DefaultPedalState"] {
            if slot == preset::expression::PedalSlot::Second && node.attribute(name).is_none() {
                return Err(error("Second expression pedal is missing required state"));
            }
            attr(
                xml,
                node,
                name,
                Some(node.attribute(name).unwrap_or("0").to_string()),
                &format!("{key}/@{name}"),
                ChangeKind::Normalization,
                "Original optional expression default",
                edits,
                changes,
            )?;
        }
        if slot == preset::expression::PedalSlot::First {
            for name in ["VolumeModeBehavior", "PedalOverrideState"] {
                node.attribute(name)
                    .ok_or_else(|| error(format!("Missing {key}/@{name}")))?
                    .parse::<u16>()
                    .map_err(error)?;
            }
        }
    }
    Ok(())
}
fn remove(
    xml: &str,
    node: Node<'_, '_>,
    key: &str,
    edits: &mut Vec<Edit>,
    changes: &mut Vec<Change>,
) {
    edits.push((node.range(), String::new()));
    change(
        changes,
        ChangeKind::Loss,
        key,
        Some(xml[node.range()].into()),
        None,
        "Target capabilities do not serialize this setting",
    );
}
