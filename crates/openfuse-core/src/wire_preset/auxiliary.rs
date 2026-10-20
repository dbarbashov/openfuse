use super::*;
use crate::preset::expression::{self, PedalSlot};

pub(super) fn decode_auxiliary(
    product: u32,
    reports: &[Report64],
    modules: &BTreeMap<u8, Report64>,
    xml: &mut String,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let mut seen = Vec::new();
    for report in reports {
        let b = report.as_bytes();
        let identity = (b[2], if b[2] == 10 { b[25] } else { 0 });
        if seen.contains(&identity) {
            return Err(error("Duplicate auxiliary state in capture"));
        }
        seen.push(identity);
        match b[2] {
            13 => xml.push_str(&format!("<UsbGain>{}</UsbGain>", b[16])),
            14 => {
                if product != 7 {
                    return Err(error(
                        "FX loop capture is unsupported for this preset family",
                    ));
                }
                xml.push_str(&format!("<FxLoopBypass>{}</FxLoopBypass>", b[16] != 0));
            }
            10 => {
                let slot = match b[25] {
                    0 => PedalSlot::First,
                    1 => PedalSlot::Second,
                    _ => {
                        return Err(error(
                            "Unknown expression pedal index retained only by raw capture; cannot reconstruct preset",
                        ));
                    }
                };
                let caps = expression::capabilities(product).expect("known product");
                if !caps.first || (slot == PedalSlot::Second && !caps.second) {
                    return Err(error(
                        "Expression pedal slot is unavailable for this preset family",
                    ));
                }
                let parameter = if (1..=5).contains(&b[20]) {
                    if slot == PedalSlot::Second {
                        b[21]
                    } else {
                        let module = modules[&(b[20] + 4)].as_bytes();
                        if b[20] == 1 { module[53] } else { module[19] }
                    }
                } else {
                    0
                };
                xml.push_str(&format!("<{} ExpressionModeBehavior=\"{}\" ParameterIndex=\"{parameter}\" HeelSetting=\"{}\" ToeSetting=\"{}\" PedalMode=\"{}\" BypassEffectWhenVolumeMode=\"{}\" VolumeSwitchRevert=\"{}\" DefaultPedalState=\"{}\"",slot.key(),b[20],u16::from(b[22])*256,u16::from(b[23])*256,b[24],u8::from(b[19]!=0),b[26],b[27]));
                if slot == PedalSlot::First {
                    xml.push_str(&format!(
                        " VolumeModeBehavior=\"{}\" PedalOverrideState=\"{}\"",
                        b[16], b[28]
                    ));
                } else {
                    xml.push_str(" PedalIndex=\"1\"");
                }
                xml.push_str("/>");
                warnings.push(format!("{} global volume endpoints ({}, {}) and wire-only assignment fields remain in the raw capture",slot.key(),u16::from(b[17])*256,u16::from(b[18])*256));
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

pub(super) fn encode_auxiliary(
    product: u32,
    document: &PresetDocument,
    root: roxmltree::Node<'_, '_>,
    reports: &mut Vec<Report64>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    for slot in [PedalSlot::First, PedalSlot::Second] {
        let Some(pedal) = document
            .expression_pedal(slot)
            .map_err(|e| error(e.to_string()))?
        else {
            continue;
        };
        let caps = expression::capabilities(product).expect("known product");
        if !caps.first || (slot == PedalSlot::Second && !caps.second) {
            return Err(error("Unsupported expression slot in document"));
        }
        let mut b = encode::frame(10);
        let required = |value: Option<u16>, label: &str| {
            value.ok_or_else(|| error(format!("{} missing {label}", slot.key())))
        };
        let byte = |value: u16, label: &str| {
            u8::try_from(value)
                .map_err(|_| error(format!("{label} exceeds the 8-bit expression field")))
        };
        if pedal.pedal_index.unwrap_or(0) != slot.index() {
            return Err(error("Expression pedal index does not match its XML slot"));
        }
        if slot == PedalSlot::First {
            b[16] = byte(
                required(pedal.volume_mode_behavior, "VolumeModeBehavior")?,
                "VolumeModeBehavior",
            )?;
            b[28] = byte(
                required(pedal.pedal_override_state, "PedalOverrideState")?,
                "PedalOverrideState",
            )?;
        }
        b[19] = u8::from(
            required(
                pedal.bypass_effect_when_volume_mode,
                "BypassEffectWhenVolumeMode",
            )? != 0,
        );
        b[20] = byte(
            required(pedal.expression_mode_behavior, "ExpressionModeBehavior")?,
            "ExpressionModeBehavior",
        )?;
        if slot == PedalSlot::Second && (1..=5).contains(&b[20]) {
            b[21] = pedal
                .parameter_index
                .ok_or_else(|| error("Second expression parameter is missing"))?;
        }
        let heel = required(pedal.heel_setting, "HeelSetting")?;
        let toe = required(pedal.toe_setting, "ToeSetting")?;
        b[22] = (heel >> 8) as u8;
        b[23] = (toe >> 8) as u8;
        b[24] = byte(required(pedal.pedal_mode, "PedalMode")?, "PedalMode")?;
        b[25] = slot.index() as u8;
        let (revert, default_state) = if slot == PedalSlot::Second {
            if pedal.volume_mode_behavior.is_some() || pedal.pedal_override_state.is_some() {
                return Err(error(
                    "Second pedal XML has first-pedal-only fields that cannot be preserved by its original file semantics",
                ));
            }
            (
                required(pedal.volume_switch_revert, "VolumeSwitchRevert")?,
                required(pedal.default_pedal_state, "DefaultPedalState")?,
            )
        } else {
            (
                pedal.volume_switch_revert.unwrap_or(0),
                pedal.default_pedal_state.unwrap_or(0),
            )
        };
        b[26] = byte(revert, "VolumeSwitchRevert")?;
        b[27] = byte(default_state, "DefaultPedalState")?;
        if heel & 255 != 0 || toe & 255 != 0 {
            warnings.push(format!(
                "{} heel/toe endpoints lose their low 8 bits in packed reports",
                slot.key()
            ));
        }
        warnings.push(format!("{} global volume endpoints are absent from preset XML; rehearsal bytes17/18 use zero and must not be sent to a device",slot.key()));
        reports.push(Report64::from_bytes(b));
    }
    for (key, kind) in [("UsbGain", 13), ("FxLoopBypass", 14)] {
        let nodes: Vec<_> = root.children().filter(|n| n.has_tag_name(key)).collect();
        if nodes.len() > 1 {
            return Err(error(format!("Duplicate {key}")));
        }
        if let Some(node) = nodes.first() {
            if node.attributes().len() != 0 || node.children().any(|n| n.is_element()) {
                return Err(error(format!("Unknown {key} extensions cannot be encoded")));
            }
            let value = node.text().unwrap_or("").trim();
            let mut b = encode::frame(kind);
            b[16] = if kind == 13 {
                value
                    .parse()
                    .map_err(|_| error("USB gain must fit one byte"))?
            } else {
                if product != 7 {
                    return Err(error("FX loop is unavailable for this family"));
                }
                match value {
                    "true" | "True" | "1" => 1,
                    "false" | "False" | "0" => 0,
                    _ => return Err(error("Invalid FX loop boolean")),
                }
            };
            reports.push(Report64::from_bytes(b));
        }
    }
    Ok(())
}
