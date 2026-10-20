//! Source-preserving preset expression settings. Global volume endpoints and
//! physical pedal detection are separate device settings.

use super::{
    PresetDocument, Result, append_child, attribute, attribute_node, child, edit_attributes, error,
    number, parse_xml,
};
use crate::model_catalog::{self, ModuleCategory, Parameter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PedalSlot {
    First,
    Second,
}
impl PedalSlot {
    pub fn key(self) -> &'static str {
        match self {
            Self::First => "FirstExpressionPedal",
            Self::Second => "SecondExpressionPedal",
        }
    }
    pub fn index(self) -> u16 {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpressionCapabilities {
    pub first: bool,
    pub second: bool,
}

/// Preset-file capability, not a claim that a physical pedal is connected.
/// Super Champ files permit this block even though the physical UI hides it.
pub fn capabilities(product_id: u32) -> Option<ExpressionCapabilities> {
    Some(match product_id {
        1 | 13 | 2259 | 6 => ExpressionCapabilities {
            first: true,
            second: false,
        },
        7 => ExpressionCapabilities {
            first: true,
            second: true,
        },
        10 | 11 => ExpressionCapabilities {
            first: false,
            second: false,
        },
        _ => return None,
    })
}

pub fn target_key(behavior: u16) -> Option<&'static str> {
    match behavior {
        1 => Some("Amplifier"),
        2 => Some("Stompbox"),
        3 => Some("Modulation"),
        4 => Some("Delay"),
        5 => Some("Reverb"),
        _ => None,
    }
}

/// Original FUSEUI's actual parameter-selection filter, not all model controls.
pub fn controllable_parameters(
    product_id: u32,
    module_key: &str,
    model_id: u16,
) -> Vec<&'static Parameter> {
    let Some(category) = ModuleCategory::from_key(module_key) else {
        return Vec::new();
    };
    let Some(model) = model_catalog::model(product_id, category, model_id) else {
        return Vec::new();
    };
    if model_id == 0 {
        return Vec::new();
    }
    model
        .parameters
        .iter()
        .filter(|parameter| {
            if category == ModuleCategory::Amplifier {
                matches!(parameter.index, 0 | 1 | 4 | 5 | 6 | 10)
                    || (parameter.index == 3 && matches!(model_id, 97 | 93 | 94 | 109 | 114))
            } else {
                parameter.type_id < 128
            }
        })
        .collect()
}

/// Optional fields reflect on-disk presence. Numeric enums are intentionally
/// open so unfamiliar values survive reading, saving and unrelated edits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExpressionPedal {
    pub pedal_index: Option<u16>,
    pub volume_mode_behavior: Option<u16>,
    pub expression_mode_behavior: Option<u16>,
    pub parameter_index: Option<u8>,
    pub heel_setting: Option<u16>,
    pub toe_setting: Option<u16>,
    pub pedal_mode: Option<u16>,
    /// Original loader interprets any nonzero ushort as true. Retain exact bits.
    pub bypass_effect_when_volume_mode: Option<u16>,
    pub volume_switch_revert: Option<u16>,
    pub default_pedal_state: Option<u16>,
    pub pedal_override_state: Option<u16>,
}
impl ExpressionPedal {
    pub fn defaults(slot: PedalSlot) -> Self {
        Self {
            pedal_index: (slot == PedalSlot::Second).then_some(1),
            volume_mode_behavior: (slot == PedalSlot::First).then_some(1),
            expression_mode_behavior: Some(6),
            parameter_index: Some(0),
            heel_setting: Some(0),
            toe_setting: Some(u16::MAX),
            pedal_mode: Some(0),
            bypass_effect_when_volume_mode: Some(1),
            volume_switch_revert: Some(0),
            default_pedal_state: Some(0),
            pedal_override_state: (slot == PedalSlot::First).then_some(0),
        }
    }
    fn fields(&self) -> [(&'static str, Option<u16>); 11] {
        [
            ("PedalIndex", self.pedal_index),
            ("VolumeModeBehavior", self.volume_mode_behavior),
            ("ExpressionModeBehavior", self.expression_mode_behavior),
            ("ParameterIndex", self.parameter_index.map(u16::from)),
            ("HeelSetting", self.heel_setting),
            ("ToeSetting", self.toe_setting),
            ("PedalMode", self.pedal_mode),
            (
                "BypassEffectWhenVolumeMode",
                self.bypass_effect_when_volume_mode,
            ),
            ("VolumeSwitchRevert", self.volume_switch_revert),
            ("DefaultPedalState", self.default_pedal_state),
            ("PedalOverrideState", self.pedal_override_state),
        ]
    }
}

impl PresetDocument {
    /// A missing pedal is None, not an implicit initialized block. Malformed
    /// known numeric values report an error without modifying the source XML.
    pub fn expression_pedal(&self, slot: PedalSlot) -> Result<Option<ExpressionPedal>> {
        let doc = parse_xml(&self.xml)?;
        let Some(node) = child(doc.root_element(), slot.key())? else {
            return Ok(None);
        };
        let field = |key| {
            attribute(node, key)
                .map(|value| number::<u16>(value, key))
                .transpose()
        };
        Ok(Some(ExpressionPedal {
            pedal_index: field("PedalIndex")?,
            volume_mode_behavior: field("VolumeModeBehavior")?,
            expression_mode_behavior: field("ExpressionModeBehavior")?,
            parameter_index: attribute(node, "ParameterIndex")
                .map(|value| number(value, "expression parameter index"))
                .transpose()?,
            heel_setting: field("HeelSetting")?,
            toe_setting: field("ToeSetting")?,
            pedal_mode: field("PedalMode")?,
            bypass_effect_when_volume_mode: field("BypassEffectWhenVolumeMode")?,
            volume_switch_revert: field("VolumeSwitchRevert")?,
            default_pedal_state: field("DefaultPedalState")?,
            pedal_override_state: field("PedalOverrideState")?,
        }))
    }

    /// Existing unknown values and unrelated XML remain intact. New assignments
    /// must be supported by the known family, present model and UI parameter set.
    /// Reversed heel/toe endpoints are allowed, as in the original independent sliders.
    pub fn set_expression_pedal(&mut self, slot: PedalSlot, value: &ExpressionPedal) -> Result<()> {
        let current = self.expression_pedal(slot)?;
        if current.as_ref() == Some(value) {
            return Ok(());
        }
        self.validate_expression_change(slot, current.as_ref(), value)?;
        let doc = parse_xml(&self.xml)?;
        let root = doc.root_element();
        let mut edits = Vec::new();
        if let Some(node) = child(root, slot.key())? {
            let old = current
                .as_ref()
                .expect("existing expression block was projected")
                .fields();
            let mut additions = Vec::new();
            for ((key, next), (_, previous)) in value.fields().into_iter().zip(old) {
                if next == previous {
                    continue;
                }
                if let Some(next) = next {
                    additions.push((key, next.to_string()));
                } else if let Some(attr) = attribute_node(node, key) {
                    edits.push((attr.range(), String::new()));
                }
            }
            edit_attributes(&self.xml, node, &additions, &mut edits)?;
        } else {
            let mut content = format!("<{}", slot.key());
            for (key, next) in value.fields() {
                if let Some(next) = next {
                    content.push_str(&format!(" {key}=\"{next}\""));
                }
            }
            content.push_str("/>");
            append_child(&self.xml, root, &content, &mut edits)?;
        }
        self.apply(edits)
    }

    /// Commit both pedal edits together, including binding validation.
    pub fn set_expression_pedals(&mut self, values: &[(PedalSlot, ExpressionPedal)]) -> Result<()> {
        if values.len() > 2 || (values.len() == 2 && values[0].0 == values[1].0) {
            return Err(error("Duplicate expression slot in update"));
        }
        let mut candidate = self.clone();
        for (slot, value) in values {
            candidate.set_expression_pedal(*slot, value)?;
        }
        *self = candidate;
        Ok(())
    }

    fn validate_expression_change(
        &self,
        slot: PedalSlot,
        current: Option<&ExpressionPedal>,
        value: &ExpressionPedal,
    ) -> Result<()> {
        let previous = current.cloned().unwrap_or_default();
        if current.is_none() {
            let caps = self
                .product_id
                .and_then(capabilities)
                .ok_or_else(|| error("Unknown family; cannot add expression settings"))?;
            if !(match slot {
                PedalSlot::First => caps.first,
                PedalSlot::Second => caps.second,
            }) {
                return Err(error(
                    "This preset family does not support this expression pedal slot",
                ));
            }
        }
        if (current.is_none() || value.pedal_index != previous.pedal_index)
            && value.pedal_index.unwrap_or(0) != slot.index()
        {
            return Err(error("PedalIndex does not match the expression slot"));
        }
        for ((key, next), (_, old)) in value.fields().into_iter().zip(previous.fields()) {
            if next == old && current.is_some() {
                continue;
            }
            let maximum = match key {
                "VolumeModeBehavior"
                | "PedalMode"
                | "BypassEffectWhenVolumeMode"
                | "VolumeSwitchRevert"
                | "DefaultPedalState"
                | "PedalOverrideState" => Some(1),
                "ExpressionModeBehavior" => Some(6),
                _ => None,
            };
            if let (Some(next), Some(maximum)) = (next, maximum)
                && next > maximum
            {
                return Err(error(format!("Unknown {key} cannot be newly assigned")));
            }
            let required = matches!(
                key,
                "ExpressionModeBehavior"
                    | "HeelSetting"
                    | "ToeSetting"
                    | "PedalMode"
                    | "BypassEffectWhenVolumeMode"
            ) || (slot == PedalSlot::First
                && matches!(key, "VolumeModeBehavior" | "PedalOverrideState"))
                || (slot == PedalSlot::Second
                    && matches!(
                        key,
                        "PedalIndex" | "VolumeSwitchRevert" | "DefaultPedalState"
                    ));
            if required && next.is_none() {
                return Err(error(format!("Missing required expression field {key}")));
            }
            if slot == PedalSlot::Second
                && matches!(key, "VolumeModeBehavior" | "PedalOverrideState")
                && next.is_some()
            {
                return Err(error(format!("{key} is not stored for the second pedal")));
            }
        }
        let binding_changed = current.is_none()
            || value.expression_mode_behavior != previous.expression_mode_behavior
            || value.parameter_index != previous.parameter_index;
        if binding_changed {
            let caps = self
                .product_id
                .and_then(capabilities)
                .ok_or_else(|| error("Unknown family; cannot change expression bindings"))?;
            if !(match slot {
                PedalSlot::First => caps.first,
                PedalSlot::Second => caps.second,
            }) {
                return Err(error(
                    "This family does not support expression bindings in this slot",
                ));
            }
            let behavior = value
                .expression_mode_behavior
                .ok_or_else(|| error("Missing expression target"))?;
            if let Some(key) = target_key(behavior) {
                let product = self
                    .product_id
                    .ok_or_else(|| error("Unknown preset family"))?;
                let module = self
                    .modules
                    .iter()
                    .find(|module| module.key == key)
                    .ok_or_else(|| error("Expression target module is absent"))?;
                let model = module
                    .model_id
                    .ok_or_else(|| error("Expression target model is absent"))?;
                let index = value
                    .parameter_index
                    .ok_or_else(|| error("Expression target requires ParameterIndex"))?;
                if !module
                    .parameters
                    .iter()
                    .any(|parameter| parameter.index == index)
                    || !controllable_parameters(product, key, model)
                        .iter()
                        .any(|parameter| parameter.index == index)
                {
                    return Err(error(
                        "Parameter is not an available expression target in this model",
                    ));
                }
            } else if behavior <= 6 {
                if value.parameter_index != Some(0) {
                    return Err(error(
                        "Off and volume expression targets require ParameterIndex 0",
                    ));
                }
            } else {
                return Err(error("Unknown expression target cannot be newly assigned"));
            }
        }
        Ok(())
    }
}
