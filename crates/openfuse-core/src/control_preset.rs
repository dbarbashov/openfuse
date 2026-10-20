//! Standalone physical-knob FX/VOICE documents. Exact USB profile context is
//! separate from FUSE XML, which does not encode a unique product ID.
//! Edits preserve unknown XML; the stricter USB planner rejects unrepresented
//! fields. These documents never pretend to be captured device reports.
use crate::{
    auxiliary::{self, Entry, Key, Selection, Snapshot},
    model_catalog::{self, ModuleCategory},
    preset::{
        self, PresetDocument, PresetModule, PresetParameter,
        expression::{ExpressionPedal, PedalSlot},
    },
    protocol::Report64,
    wire_preset,
};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, ops::Range};
mod edit;
mod projection;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPresetError(String);
impl fmt::Display for ControlPresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ControlPresetError {}
pub type Result<T> = std::result::Result<T, ControlPresetError>;
fn error(value: impl fmt::Display) -> ControlPresetError {
    ControlPresetError(value.to_string())
}
#[derive(Debug, Clone)]
pub struct ControlPreset {
    pid: u16,
    control: u8,
    xml: String,
    name: String,
    projected: PresetDocument,
}
impl ControlPreset {
    pub fn parse(pid: u16, control: u8, xml: &str) -> Result<Self> {
        let caps = auxiliary::capabilities(pid).map_err(error)?;
        if !caps.banks.contains_key(&control) {
            return Err(error("Profile has no such physical bank"));
        }
        let document = preset::parse_xml(xml).map_err(error)?;
        let root = document.root_element();
        let expected = if pid == 14 && control == 1 {
            "VOICE"
        } else {
            "FX"
        };
        if !root.has_tag_name(expected) {
            return Err(error("Expected matching unnamespaced FX/VOICE root"));
        }
        if root
            .attribute("amplifier")
            .is_some_and(|v| v != "SuperChampX2" || pid != 14)
        {
            return Err(error("Document amplifier family differs from profile"));
        }
        let name = root.attribute("Name").unwrap_or("").to_string();
        let projected = projection::to_preset(pid, xml)?;
        Ok(Self {
            pid,
            control,
            xml: xml.to_string(),
            name,
            projected,
        })
    }
    pub fn from_entry(pid: u16, entry: &Entry) -> Result<Self> {
        if !entry.raw_reports().is_empty() && entry.restore_error().is_some() {
            return Err(error(
                "Captured raw state cannot be projected losslessly into an editable control preset",
            ));
        }
        let Key::Bank { control, .. } = entry.key() else {
            return Err(error("System settings are not a control preset"));
        };
        Self::parse(
            pid,
            control,
            entry
                .xml()
                .ok_or_else(|| error("Entry has no editable XML projection"))?,
        )
    }
    pub fn new(pid: u16, control: u8, name: &str) -> Result<Self> {
        let mut result = Self::parse(
            pid,
            control,
            &format!(
                "<{} Name=\"\"{}/>",
                if pid == 14 && control == 1 {
                    "VOICE"
                } else {
                    "FX"
                },
                if pid == 14 {
                    " amplifier=\"SuperChampX2\""
                } else {
                    ""
                }
            ),
        )?;
        let mut content = String::new();
        for &key in result.allowed_modules() {
            let category = ModuleCategory::from_key(key).unwrap();
            let model_id = if key == "Amplifier" {
                model_catalog::product(result.product_id())
                    .unwrap()
                    .default_amp_id
            } else {
                0
            };
            let model = model_catalog::model(result.product_id(), category, model_id)
                .ok_or_else(|| error("Missing catalog default model"))?;
            let kind = wire_preset::CATEGORIES
                .iter()
                .find(|(_, k, _)| *k == key)
                .unwrap()
                .0;
            let bypass = if pid == 14 {
                format!(" BypassMode=\"{}\"", model.bypass_mode)
            } else {
                String::new()
            };
            let category_id = if key == "Amplifier" {
                String::new()
            } else {
                format!(" ID=\"{}\"", kind - 5)
            };
            content.push_str(&format!(
                "<{key}{category_id}><Module ID=\"{}\" POS=\"{}\" BypassState=\"1\"{bypass}>",
                model.id,
                kind.saturating_sub(6)
            ));
            for p in &model.parameters {
                content.push_str(&format!(
                    "<Param ControlIndex=\"{}\">{}</Param>",
                    p.index, p.default_raw
                ));
            }
            content.push_str(&format!("</Module></{key}>"));
        }
        if pid == 14 && control == 1 {
            content.push_str("<UsbGain>0</UsbGain>");
        }
        if pid == 14 && control == 2 {
            content.push_str("<FirstExpressionPedal VolumeModeBehavior=\"0\" ExpressionModeBehavior=\"6\" ParameterIndex=\"0\" HeelSetting=\"0\" ToeSetting=\"65535\" PedalMode=\"1\" BypassEffectWhenVolumeMode=\"0\" VolumeSwitchRevert=\"0\" DefaultPedalState=\"0\" PedalOverrideState=\"0\"/>");
        }
        let doc = preset::parse_xml(&result.xml).map_err(error)?;
        let mut edits = vec![];
        preset::append_child(&result.xml, doc.root_element(), &content, &mut edits)
            .map_err(error)?;
        result.apply(edits)?;
        result.set_name(name)?;
        Ok(result)
    }
    pub fn new_for_slot(pid: u16, control: u8, slot: u16, name: &str) -> Result<Self> {
        let mut value = Self::new(pid, control, name)?;
        if slot >= auxiliary::capabilities(pid).map_err(error)?.banks[&control] {
            return Err(error("Physical bank slot out of range"));
        }
        if pid == 14 && control == 1 && slot == 16 {
            let doc = preset::parse_xml(&value.xml).map_err(error)?;
            let node = preset::child(doc.root_element(), "Stompbox")
                .map_err(error)?
                .unwrap();
            value.apply(vec![(node.range(), String::new())])?;
        }
        Ok(value)
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }
    pub fn control(&self) -> u8 {
        self.control
    }
    pub fn product_id(&self) -> u32 {
        wire_preset::product_for_pid(self.pid).expect("validated profile")
    }
    pub fn name(&self) -> String {
        self.name.clone()
    }
    /// Read-only partial view for existing catalog/expression controls. It has
    /// only this physical bank's modules and is not a complete amplifier preset.
    pub fn preset_projection(&self) -> &PresetDocument {
        &self.projected
    }
    pub fn modules(&self) -> Vec<PresetModule> {
        self.projected.modules()
    }
    pub fn to_xml(&self) -> Result<String> {
        Ok(self.xml.clone())
    }
    pub fn allowed_modules(&self) -> &'static [&'static str] {
        match (self.pid, self.control) {
            (4 | 20, 1) => &["Modulation"],
            (4 | 20, 2) => &["Delay", "Reverb"],
            (14, 1) => &["Amplifier", "Stompbox"],
            _ => &["Modulation", "Delay", "Reverb"],
        }
    }
    pub fn expression_pedal(&self) -> Result<Option<ExpressionPedal>> {
        self.projected
            .expression_pedal(PedalSlot::First)
            .map_err(error)
    }
    pub fn usb_gain(&self) -> Result<Option<u8>> {
        let doc = preset::parse_xml(&self.xml).map_err(error)?;
        let node = preset::child(doc.root_element(), "UsbGain").map_err(error)?;
        node.map(|n| {
            n.children()
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect::<String>()
                .trim()
                .parse()
                .map_err(error)
        })
        .transpose()
    }
    pub fn plan(&self, destination: &Snapshot, slot: u16) -> Result<Selection> {
        Selection::from_control_preset(self, destination, slot).map_err(error)
    }
    pub fn write_plan(&self, slot: u16) -> Result<Vec<Report64>> {
        auxiliary::plan_xml(
            self.pid,
            Key::Bank {
                control: self.control,
                slot,
            },
            &self.xml,
        )
        .map_err(error)
    }
    fn apply(&mut self, mut edits: Vec<(Range<usize>, String)>) -> Result<()> {
        edits.sort_by_key(|e| e.0.start);
        if edits.windows(2).any(|w| w[0].0.end > w[1].0.start) {
            return Err(error("Overlapping control preset edits"));
        }
        let mut xml = self.xml.clone();
        for (range, value) in edits.into_iter().rev() {
            xml.replace_range(range, &value);
        }
        *self = Self::parse(self.pid, self.control, &xml)?;
        Ok(())
    }
    pub fn to_json(&self) -> Result<String> {
        let mut value = File {
            format: "confuse-control-preset".into(),
            version: 1,
            pid: self.pid,
            control: self.control,
            xml: self.xml.clone(),
            sha256: String::new(),
        };
        value.sha256 = value.checksum()?;
        serde_json::to_string_pretty(&value).map_err(error)
    }
    pub fn parse_json(json: &str) -> Result<Self> {
        if json.len() > 4 * preset::MAX_PRESET_BYTES {
            return Err(error("Control preset file exceeds 8 MiB"));
        }
        let value: File = serde_json::from_str(json).map_err(error)?;
        if value.format != "confuse-control-preset"
            || value.version != 1
            || value.sha256 != value.checksum()?
        {
            return Err(error("Unsupported or corrupt control preset file"));
        }
        Self::parse(value.pid, value.control, &value.xml)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    format: String,
    version: u32,
    pid: u16,
    control: u8,
    xml: String,
    sha256: String,
}
impl File {
    fn checksum(&self) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    &self.format,
                    self.version,
                    self.pid,
                    self.control,
                    &self.xml
                ))
                .map_err(error)?
            )
        ))
    }
}
