//! Mustang-family current-preset wire/XML bridge, with retained raw captures.
//! Retains raw parameter precision when translating between wire and XML forms.
//! Encoding is an offline rehearsal artifact, not authorization to send a preset.
use crate::{
    messages::ParameterChange,
    model_catalog::{self, Model, ModuleCategory},
    preset::PresetDocument,
    protocol::Report64,
};
use std::{collections::BTreeMap, fmt};
mod auxiliary;
mod encode;
pub use encode::encode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirePresetError(String);
impl fmt::Display for WirePresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for WirePresetError {}
type Result<T> = std::result::Result<T, WirePresetError>;
fn error(message: impl Into<String>) -> WirePresetError {
    WirePresetError(message.into())
}

#[derive(Debug, Clone)]
pub struct DecodedPreset {
    pub document: PresetDocument,
    pub warnings: Vec<String>,
    /// Exact original input, including unknown bits and unrepresented fields.
    pub raw_reports: Vec<Report64>,
}
#[derive(Debug, Clone)]
pub struct EncodedPreset {
    /// Synthetic current-preset response for local rehearsal, NOT upload commands.
    /// Name, five modules, optional auxiliary fields, Complete, and sync-end.
    pub reports: Vec<Report64>,
    pub warnings: Vec<String>,
}

/// Normal PID → preset family; recognition does not establish hardware support.
pub fn product_for_pid(pid: u16) -> Option<u32> {
    match pid {
        4 | 5 | 0x10 => Some(1),
        0x14 | 0x16 => Some(13),
        0x12 => Some(7),
        0xa => Some(11),
        0xe => Some(2259),
        0x26 => Some(6),
        _ => None,
    }
}
fn product(pid: u16) -> Result<u32> {
    product_for_pid(pid).ok_or_else(|| {
        error(format!(
            "PID {pid:04x} has no evidenced Mustang preset bridge"
        ))
    })
}
pub(crate) const CATEGORIES: [(u8, &str, ModuleCategory); 5] = [
    (5, "Amplifier", ModuleCategory::Amplifier),
    (6, "Stompbox", ModuleCategory::Stompbox),
    (7, "Modulation", ModuleCategory::Modulation),
    (8, "Delay", ModuleCategory::Delay),
    (9, "Reverb", ModuleCategory::Reverb),
];
fn definition(product: u32, category: ModuleCategory, id: u16) -> Result<&'static Model> {
    let model = model_catalog::model(product, category, id).ok_or_else(|| {
        error(format!(
            "Unknown model {id} in {category:?} for preset family {product}"
        ))
    })?;
    if model.parameters.len() > 32
        || model
            .parameters
            .iter()
            .enumerate()
            .any(|(i, p)| usize::from(p.index) != i)
    {
        return Err(error(
            "Catalog parameter shape cannot be represented by the packed wire layout",
        ));
    }
    Ok(model)
}
fn word(bytes: &[u8; 64], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
fn escape(value: &str) -> Result<String> {
    let mut text = String::new();
    for character in value.chars() {
        match character {
            '&' => text.push_str("&amp;"),
            '<' => text.push_str("&lt;"),
            '>' => text.push_str("&gt;"),
            '"' => text.push_str("&quot;"),
            '\'' => text.push_str("&apos;"),
            '\t' => text.push_str("&#9;"),
            '\n' => text.push_str("&#10;"),
            '\r' => text.push_str("&#13;"),
            c if c < ' ' => {
                return Err(error(
                    "Preset name contains a control character not representable in XML 1.0",
                ));
            }
            c => text.push(c),
        }
    }
    Ok(text)
}

/// Decode a single complete current-preset block. The session engine's snapshot
/// omits sync-end, so both Name…Complete and Name…Complete,ff are accepted.
/// Missing modules and unknown model/parameter schemas are errors, not defaults.
pub fn decode(pid: u16, reports: &[Report64]) -> Result<DecodedPreset> {
    let product_id = product(pid)?;
    if reports.is_empty() || reports.len() > 512 {
        return Err(error("Preset capture must contain 1..512 reports"));
    }
    let mut name = None;
    let mut modules = BTreeMap::new();
    let mut auxiliary = Vec::new();
    let mut complete = false;
    let mut warnings = Vec::new();
    for &report in reports {
        let b = report.as_bytes();
        if b[0] == 0xff {
            if !complete {
                return Err(error("Sync ended before the current preset completed"));
            }
            continue;
        }
        if b[0] != 0x1c || b[3] != 0 {
            warnings.push(format!(
                "Unrepresented message {:02x}/control {} retained in the raw capture",
                b[0], b[3]
            ));
            continue;
        }
        if complete {
            return Err(error("Another preset data report follows Complete"));
        }
        if b[2] == 4 {
            if name.is_some() || b[6] == 0 {
                return Err(error("Expected exactly one current-preset Name report"));
            }
            name = Some(report);
            continue;
        }
        if name.is_none() {
            return Err(error("Preset payload appeared before Name"));
        }
        match b[2] {
            0 => complete = true,
            5..=9 => {
                if modules.insert(b[2], report).is_some() {
                    return Err(error("Duplicate module in current preset"));
                }
            }
            10 | 13 | 14 => auxiliary.push(report),
            kind => warnings.push(format!(
                "Preset data type {kind} is retained only in the raw capture"
            )),
        }
    }
    if !complete || modules.len() != 5 {
        return Err(error(
            "A complete preset requires Name, all five modules, and Complete",
        ));
    }
    let name_report = name.ok_or_else(|| error("Missing current preset Name"))?;
    let name: String = name_report.as_bytes()[16..]
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| char::from(c))
        .collect();
    let mut xml = format!("<Preset ProductId=\"{product_id}\">");
    for (kind, key, category) in CATEGORIES {
        if kind == 6 {
            xml.push_str("<FX>");
        }
        let b = modules[&kind].as_bytes();
        let model = definition(product_id, category, word(b, 16))?;
        let defaults: Vec<_> = model
            .parameters
            .iter()
            .map(|p| (p.index, p.default_raw))
            .collect();
        let values: Vec<_> = model
            .parameters
            .iter()
            .map(|p| {
                (
                    p.index,
                    if p.type_id < 128 {
                        u16::from(b[32 + usize::from(p.index)]) * 256
                    } else {
                        u16::from(b[32 + usize::from(p.index)])
                    },
                )
            })
            .collect();
        let values = model_catalog::load_wire_parameters(product_id, category, &defaults, &values);
        let active = if kind == 5 {
            b[52] & 0x40 == 0
        } else {
            b[22] == 0
        };
        let position = if kind == 5 {
            0
        } else if product_id == 2259 && b[18] == 4 {
            5
        } else {
            b[18]
        };
        let bypass = if kind == 5 { model.bypass_mode } else { b[21] };
        xml.push_str(&format!("<{key} ID=\"{}\"><Module ID=\"{}\" POS=\"{position}\" BypassState=\"{}\" BypassMode=\"{bypass}\">", kind-5, model.id, u8::from(active)));
        for (index, value) in values {
            xml.push_str(&format!("<Param ControlIndex=\"{index}\">{value}</Param>"));
        }
        xml.push_str(&format!("</Module></{key}>"));
    }
    xml.push_str("</FX>");
    auxiliary::decode_auxiliary(product_id, &auxiliary, &modules, &mut xml, &mut warnings)?;
    xml.push_str(&format!("<FUSE><Info name=\"{}\" author=\"\" tags=\"\" rating=\"0\" genre1=\"-1\" genre2=\"-1\" genre3=\"-1\" fenderid=\"0\"/></FUSE></Preset>",escape(name.trim_end_matches(' '))?));
    let document = PresetDocument::parse(&xml).map_err(|e| error(e.to_string()))?;
    warnings.push("The raw capture retains wire-only fields, reserved bytes, and precision; the editable XML is not a complete device backup".into());
    Ok(DecodedPreset {
        document,
        warnings,
        raw_reports: reports.to_vec(),
    })
}

/// Construct one evidenced live parameter message from exact document state.
/// No quantization is needed for 0x05: unlike packed preset blocks it carries a
/// full 16-bit value. Callers still require a matching current device model.
pub fn parameter_change(
    pid: u16,
    document: &PresetDocument,
    module_key: &str,
    index: u8,
) -> Result<ParameterChange> {
    let product_id = product(pid)?;
    if document.product_id() != Some(product_id) {
        return Err(error("Document family does not match selected device"));
    }
    let (kind, _, category) = CATEGORIES
        .iter()
        .find(|(_, key, _)| *key == module_key)
        .ok_or_else(|| error("Unknown module category"))?;
    let modules = document.modules();
    let module = modules
        .iter()
        .find(|m| m.key == module_key)
        .ok_or_else(|| error("Missing module"))?;
    let model_id = module
        .model_id
        .ok_or_else(|| error("Missing model identifier"))?;
    let model = definition(product_id, *category, model_id)?;
    let parameter = model
        .parameters
        .iter()
        .find(|p| p.index == index)
        .ok_or_else(|| error("Unknown model parameter index"))?;
    let value = module
        .parameters
        .iter()
        .find(|p| p.index == index)
        .ok_or_else(|| error("Missing document parameter"))?
        .value;
    Ok(ParameterChange {
        module_type: kind - 3,
        model_id,
        parameter_index: index,
        control_index: parameter.control_index,
        parameter_type: parameter.type_id,
        wire_value: value,
    })
}
