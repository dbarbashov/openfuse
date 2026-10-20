//! Host-only constructors corresponding to MustangDevice.SendPreset and the
//! individual SendAmp/SendFx methods. Do not substitute synthetic read frames.
use super::*;
use crate::preset::expression::{self, ExpressionPedal, PedalSlot};
const CATEGORIES: [(u8, &str, ModuleCategory); 5] = [
    (5, "Amplifier", ModuleCategory::Amplifier),
    (6, "Stompbox", ModuleCategory::Stompbox),
    (7, "Modulation", ModuleCategory::Modulation),
    (8, "Delay", ModuleCategory::Delay),
    (9, "Reverb", ModuleCategory::Reverb),
];
pub(super) fn categories() -> &'static [(u8, &'static str, ModuleCategory); 5] {
    &CATEGORIES
}
fn frame(kind: u8, current: bool) -> [u8; 64] {
    let mut b = [0; 64];
    b[0] = 0x1c;
    b[1] = 3;
    b[2] = kind;
    if current {
        b[6] = 1;
        b[7] = 1;
    }
    b
}
pub(super) fn complete() -> Report64 {
    Report64::from_bytes(frame(0, false))
}
fn byte(value: u16, field: &str) -> Result<u8> {
    u8::try_from(value).map_err(|_| error(format!("{field} exceeds one byte")))
}

pub(super) fn module(document: &PresetDocument, kind: u8, key: &str) -> Result<Report64> {
    let product = document
        .product_id()
        .ok_or_else(|| error("Missing family"))?;
    let category = CATEGORIES
        .iter()
        .find(|(id, _, _)| *id == kind)
        .map(|(_, _, c)| *c)
        .ok_or_else(|| error("Unknown module"))?;
    let modules = document.modules();
    let module = modules
        .iter()
        .find(|m| m.key == key)
        .ok_or_else(|| error("Missing module"))?;
    let model = model_catalog::model(
        product,
        category,
        module.model_id.ok_or_else(|| error("Missing model"))?,
    )
    .ok_or_else(|| error("Unknown model"))?;
    let mut b = frame(kind, true);
    b[16..18].copy_from_slice(&model.id.to_le_bytes());
    for parameter in &model.parameters {
        let value = module
            .parameters
            .iter()
            .find(|p| p.index == parameter.index)
            .ok_or_else(|| error("Missing model parameter"))?
            .value;
        let index = 32 + usize::from(parameter.index);
        if index >= 64 {
            return Err(error("Parameter exceeds packed message"));
        }
        b[index] = if parameter.type_id < 128 {
            (value >> 8) as u8
        } else {
            byte(value, "Parameter enum")?
        };
    }
    let first = document
        .expression_pedal(PedalSlot::First)
        .map_err(text_error)?;
    let expression = if first
        .as_ref()
        .and_then(|p| p.expression_mode_behavior)
        .and_then(expression::target_key)
        == Some(key)
    {
        first
            .as_ref()
            .and_then(|p| p.parameter_index)
            .ok_or_else(|| error("Missing expression binding"))?
    } else {
        model.expression_pedal_index
    };
    let active = module
        .active
        .ok_or_else(|| error("Missing activation state"))?;
    if kind == 5 {
        if active {
            b[52] &= !0x40
        } else {
            b[52] |= 0x40
        }
        b[53] = expression;
    } else {
        b[18] = byte(
            module
                .position
                .ok_or_else(|| error("Missing module position"))?,
            "Module position",
        )?;
        b[19] = expression;
        b[20] = model.tap_parameter_index;
        b[21] = module.bypass_mode.unwrap_or(model.bypass_mode);
        b[22] = u8::from(!active);
    }
    Ok(Report64::from_bytes(b))
}
fn required(value: Option<u16>, name: &str) -> Result<u16> {
    value.ok_or_else(|| error(format!("Missing expression field {name}")))
}
pub(super) fn expression(
    pedal: &ExpressionPedal,
    slot: PedalSlot,
    globals: (u8, u8),
) -> Result<Report64> {
    let mut b = frame(10, false);
    b[17] = globals.0;
    b[18] = globals.1;
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
            .ok_or_else(|| error("Missing second expression parameter"))?;
    }
    b[22] = (required(pedal.heel_setting, "HeelSetting")? >> 8) as u8;
    b[23] = (required(pedal.toe_setting, "ToeSetting")? >> 8) as u8;
    b[24] = byte(required(pedal.pedal_mode, "PedalMode")?, "PedalMode")?;
    b[25] = slot.index() as u8;
    b[26] = byte(
        pedal.volume_switch_revert.unwrap_or(0),
        "VolumeSwitchRevert",
    )?;
    b[27] = byte(pedal.default_pedal_state.unwrap_or(0), "DefaultPedalState")?;
    Ok(Report64::from_bytes(b))
}
pub(super) fn preset(
    document: &PresetDocument,
    slot: u16,
    globals: Option<(u8, u8)>,
) -> Result<Vec<Report64>> {
    let product = document
        .product_id()
        .ok_or_else(|| error("Missing family"))?;
    let mut name = frame(4, true);
    name[1] = 1;
    name[4..6].copy_from_slice(&slot.to_le_bytes());
    for (index, character) in document.metadata().name.chars().enumerate() {
        if index >= 48 || u32::from(character) > 255 || character == '\0' {
            return Err(error("Preset name is not representable in the device"));
        }
        name[16 + index] = character as u8;
    }
    let mut reports = vec![Report64::from_bytes(name)];
    for (kind, key, _) in CATEGORIES {
        reports.push(module(document, kind, key)?);
    }
    if product != 11 {
        let globals = globals.ok_or_else(|| error("Missing global expression volume endpoints"))?;
        for slot in [PedalSlot::First, PedalSlot::Second] {
            if slot == PedalSlot::Second && product != 7 {
                continue;
            }
            let pedal = document
                .expression_pedal(slot)
                .map_err(text_error)?
                .ok_or_else(|| error("Missing expression state"))?;
            reports.push(expression(&pedal, slot, globals)?);
        }
    }
    let gain = scalar(document, "UsbGain")?
        .ok_or_else(|| error("Missing USB gain"))?
        .parse::<u8>()
        .map_err(|_| error("Invalid USB gain"))?;
    let mut b = frame(13, true);
    b[16] = gain;
    reports.push(Report64::from_bytes(b));
    if product == 7 {
        let value =
            scalar(document, "FxLoopBypass")?.ok_or_else(|| error("Missing FX loop state"))?;
        let active = match value.as_str() {
            "true" | "True" | "1" => true,
            "false" | "False" | "0" => false,
            _ => return Err(error("Invalid FX loop boolean")),
        };
        let mut b = frame(14, true);
        b[16] = u8::from(active);
        reports.push(Report64::from_bytes(b));
    }
    reports.push(complete());
    Ok(reports)
}
