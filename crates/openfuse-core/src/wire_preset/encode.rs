use super::*;
use crate::preset::expression::{self, PedalSlot};
use roxmltree::{Document, Node};

pub(super) fn frame(kind: u8) -> [u8; 64] {
    let mut b = [0; 64];
    b[0] = 0x1c;
    b[1] = 1;
    b[2] = kind;
    b[6] = 1;
    b[7] = 1;
    b
}
fn reject_extensions(node: Node<'_, '_>, attributes: &[&str], children: &[&str]) -> Result<()> {
    if node
        .attributes()
        .any(|a| a.namespace().is_some() || !attributes.contains(&a.name()))
        || node
            .children()
            .filter(|n| n.is_element())
            .any(|n| n.tag_name().namespace().is_some() || !children.contains(&n.tag_name().name()))
    {
        return Err(error(format!(
            "Unknown {} extensions cannot be represented in packed reports",
            node.tag_name().name()
        )));
    }
    Ok(())
}
fn module_node<'a>(root: Node<'a, 'a>, key: &str) -> Result<Node<'a, 'a>> {
    let container = if key == "Amplifier" {
        root
    } else {
        root.children()
            .find(|n| n.has_tag_name("FX"))
            .ok_or_else(|| error("Missing FX"))?
    };
    let category = container
        .children()
        .find(|n| n.has_tag_name(key))
        .ok_or_else(|| error(format!("Missing {key}")))?;
    reject_extensions(category, &["ID"], &["Module"])?;
    let expected_category = CATEGORIES
        .iter()
        .find(|(_, candidate, _)| *candidate == key)
        .expect("known key")
        .0
        - 5;
    let category_id = category
        .attribute("ID")
        .unwrap_or("0")
        .parse::<u8>()
        .map_err(|_| error("Invalid module category ID"))?;
    if category_id != expected_category {
        return Err(error("Module category ID does not match its XML element"));
    }

    let module = category
        .children()
        .find(|n| n.has_tag_name("Module"))
        .ok_or_else(|| error("Missing Module"))?;
    reject_extensions(
        module,
        &["ID", "POS", "BypassState", "BypassMode"],
        &["Param"],
    )?;
    for parameter in module.children().filter(|n| n.has_tag_name("Param")) {
        reject_extensions(parameter, &["ControlIndex"], &[])?;
    }
    Ok(module)
}

/// Offline synthetic device-response representation. The returned flags and
/// sync-end marker are for rehearsal, never a preset-upload transaction.
/// Unknown XML extensions, unknown models, missing parameters, oversized enum
/// values, and unrepresentable names fail explicitly. Quantization is disclosed.
pub fn encode(pid: u16, document: &PresetDocument) -> Result<EncodedPreset> {
    let product_id = product(pid)?;
    if document.product_id() != Some(product_id) {
        return Err(error("Document family does not match selected device"));
    }
    let source = document.to_xml().map_err(|e| error(e.to_string()))?;
    let xml = Document::parse(&source).map_err(|e| error(e.to_string()))?;
    let root = xml.root_element();
    reject_extensions(
        root,
        &["ProductId", "amplifier"],
        &[
            "Amplifier",
            "FX",
            "FUSE",
            "Band",
            "FirstExpressionPedal",
            "SecondExpressionPedal",
            "UsbGain",
            "FxLoopBypass",
        ],
    )?;
    if let Some(fx) = root.children().find(|n| n.has_tag_name("FX")) {
        reject_extensions(fx, &[], &["Stompbox", "Modulation", "Delay", "Reverb"])?;
    }
    for key in ["FirstExpressionPedal", "SecondExpressionPedal"] {
        if let Some(node) = root.children().find(|n| n.has_tag_name(key)) {
            reject_extensions(
                node,
                &[
                    "PedalIndex",
                    "VolumeModeBehavior",
                    "ExpressionModeBehavior",
                    "ParameterIndex",
                    "HeelSetting",
                    "ToeSetting",
                    "PedalMode",
                    "BypassEffectWhenVolumeMode",
                    "VolumeSwitchRevert",
                    "DefaultPedalState",
                    "PedalOverrideState",
                ],
                &[],
            )?;
        }
    }
    let mut warnings=vec!["Rehearsal encodes sound state only; author, tags, rating, artwork colors, band references and other file metadata stay in the source XML".into()];
    let name = document.metadata().name;
    let name_bytes: Vec<_> = name
        .chars()
        .map(|c| {
            if c == '\0' || u32::from(c) > 255 {
                Err(error(
                    "Wire names require at most 48 non-NUL Latin-byte characters",
                ))
            } else {
                Ok(c as u8)
            }
        })
        .collect::<Result<_>>()?;
    if name_bytes.len() > 48 {
        return Err(error("Wire name exceeds 48 bytes; refusing truncation"));
    }
    if name.ends_with(' ') {
        warnings.push(
            "Trailing name spaces disappear when the original device name accessor is read".into(),
        );
    }
    let mut header = frame(4);
    header[16..16 + name_bytes.len()].copy_from_slice(&name_bytes);
    let mut reports = vec![Report64::from_bytes(header)];
    let modules = document.modules();
    let first = document
        .expression_pedal(PedalSlot::First)
        .map_err(|e| error(e.to_string()))?;
    for (kind, key, category) in CATEGORIES {
        module_node(root, key)?;
        let module = modules
            .iter()
            .find(|m| m.key == key)
            .ok_or_else(|| error(format!("Missing {key}")))?;
        let model = definition(
            product_id,
            category,
            module.model_id.ok_or_else(|| error("Missing model ID"))?,
        )?;
        if module.parameters.len() != model.parameters.len()
            || model
                .parameters
                .iter()
                .any(|p| !module.parameters.iter().any(|v| v.index == p.index))
        {
            return Err(error(format!(
                "{key} does not contain exactly the model's complete parameter set"
            )));
        }
        let mut b = frame(kind);
        b[16..18].copy_from_slice(&model.id.to_le_bytes());
        let mut quantized = 0;
        for parameter in &model.parameters {
            let value = module
                .parameters
                .iter()
                .find(|p| p.index == parameter.index)
                .expect("validated parameter")
                .value;
            b[32 + usize::from(parameter.index)] = if parameter.type_id < 128 {
                quantized += usize::from(value & 255 != 0);
                (value >> 8) as u8
            } else {
                u8::try_from(value).map_err(|_| {
                    error(format!(
                        "{key} parameter {} enum exceeds one byte",
                        parameter.index
                    ))
                })?
            };
        }
        if quantized != 0 {
            warnings.push(format!(
                "{key}: {quantized} continuous parameters lose their low 8 bits in packed reports"
            ));
        }
        let active = module
            .active
            .ok_or_else(|| error(format!("{key} has no activation state")))?;
        let expression_index = if first
            .as_ref()
            .and_then(|p| p.expression_mode_behavior)
            .and_then(expression::target_key)
            == Some(key)
        {
            first
                .as_ref()
                .and_then(|p| p.parameter_index)
                .ok_or_else(|| error("First expression binding is missing ParameterIndex"))?
        } else {
            model.expression_pedal_index
        };
        if kind == 5 {
            if module.position.unwrap_or(0) != 0 {
                return Err(error(
                    "Amplifier position is not representable by the packed amplifier message",
                ));
            }
            if module
                .bypass_mode
                .is_some_and(|mode| mode != model.bypass_mode)
            {
                return Err(error(
                    "Amplifier bypass mode has no packed field; refusing to drop it",
                ));
            }
            let before = (b[52], b[53]);
            if active {
                b[52] &= !0x40;
            } else {
                b[52] |= 0x40;
            }
            b[53] = expression_index;
            if model.parameters.len() > 20 && before != (b[52], b[53]) {
                warnings.push("Amplifier activation/expression metadata overlaps packed parameter bytes 20/21, as in the original encoder".into());
            }
        } else {
            b[18] = u8::try_from(
                module
                    .position
                    .ok_or_else(|| error(format!("{key} has no position")))?,
            )
            .map_err(|_| error("Module position exceeds one byte"))?;
            b[19] = expression_index;
            b[20] = model.tap_parameter_index;
            b[21] = module.bypass_mode.unwrap_or(model.bypass_mode);
            b[22] = u8::from(!active);
            if product_id == 2259 && b[18] == 4 {
                warnings.push(format!(
                    "{key}: Super Champ normalizes wire position 4 to position 5 when read"
                ));
            }
        }
        reports.push(Report64::from_bytes(b));
    }
    auxiliary::encode_auxiliary(product_id, document, root, &mut reports, &mut warnings)?;
    reports.push(Report64::from_bytes(frame(0)));
    let mut end = [0; 64];
    end[0] = 0xff;
    end[1] = 0xc1;
    reports.push(Report64::from_bytes(end));
    Ok(EncodedPreset { reports, warnings })
}
