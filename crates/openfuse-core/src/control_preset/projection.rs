use super::*;
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}
pub(super) fn to_preset(pid: u16, source: &str) -> Result<PresetDocument> {
    let doc = preset::parse_xml(source).map_err(error)?;
    let root = doc.root_element();
    let mut xml = format!(
        "<Preset ProductId=\"{}\"",
        wire_preset::product_for_pid(pid).unwrap()
    );
    for ns in root.namespaces() {
        if let Some(name) = ns.name() {
            xml.push_str(&format!(" xmlns:{name}=\"{}\"", escape(ns.uri())));
        }
    }
    xml.push('>');
    if let Some(amp) = preset::child(root, "Amplifier").map_err(error)? {
        xml.push_str(&source[amp.range()]);
    }
    xml.push_str("<FX>");
    for key in ["Stompbox", "Modulation", "Delay", "Reverb"] {
        if let Some(node) = preset::child(root, key).map_err(error)? {
            xml.push_str(&source[node.range()]);
        }
    }
    xml.push_str("</FX>");
    for key in ["FirstExpressionPedal", "UsbGain"] {
        if let Some(node) = preset::child(root, key).map_err(error)? {
            xml.push_str(&source[node.range()]);
        }
    }
    xml.push_str("</Preset>");
    PresetDocument::parse(&xml).map_err(error)
}
fn category<'a, 'i>(root: Node<'a, 'i>, key: &str) -> Result<Option<Node<'a, 'i>>> {
    let parent = if matches!(key, "Stompbox" | "Modulation" | "Delay" | "Reverb") {
        preset::child(root, "FX")
            .map_err(error)?
            .ok_or_else(|| error("Missing FX projection"))?
    } else {
        root
    };
    preset::child(parent, key).map_err(error)
}
impl ControlPreset {
    pub fn from_preset(pid: u16, control: u8, document: &PresetDocument) -> Result<Self> {
        let template = Self::new(pid, control, &document.metadata().name)?;
        Self::from_selected_preset(pid, control, None, document, template.allowed_modules())
    }
    pub fn from_reports(pid: u16, control: u8, reports: &[Report64]) -> Result<Self> {
        let decoded = wire_preset::decode(pid, reports).map_err(error)?;
        Self::from_preset(pid, control, &decoded.document)
    }
    pub fn from_selected_preset(
        pid: u16,
        control: u8,
        slot: Option<u16>,
        document: &PresetDocument,
        selected: &[&str],
    ) -> Result<Self> {
        if document.product_id() != wire_preset::product_for_pid(pid) {
            return Err(error(
                "Preset family does not match chosen physical bank profile",
            ));
        }
        let mut result = Self::new(pid, control, &document.metadata().name)?;
        if selected.is_empty() {
            return Err(error("Choose at least one module"));
        }
        let clean = pid == 14 && control == 1 && slot == Some(16);
        let mut seen = std::collections::BTreeSet::new();
        for key in selected {
            if !seen.insert(*key)
                || !result.allowed_modules().contains(key)
                || clean && *key != "Amplifier"
            {
                return Err(error(
                    "Invalid or duplicate selected module for physical bank",
                ));
            }
        }
        if let Some(slot) = slot {
            let count = auxiliary::capabilities(pid).map_err(error)?.banks[&control];
            if slot >= count {
                return Err(error("Physical bank slot out of range"));
            }
        }
        let source = document.to_xml().map_err(error)?;
        let src = preset::parse_xml(&source).map_err(error)?;
        let dst = preset::parse_xml(&result.xml).map_err(error)?;
        let mut edits = vec![];
        for &key in result.allowed_modules() {
            let node = preset::child(dst.root_element(), key)
                .map_err(error)?
                .unwrap();
            if clean && key == "Stompbox" {
                edits.push((node.range(), String::new()));
            } else if selected.contains(&key) {
                let replacement = category(src.root_element(), key)?
                    .ok_or_else(|| error(format!("Current preset has no {key}")))?;
                edits.push((node.range(), source[replacement.range()].to_string()));
            } else {
                let model = model_catalog::model(
                    result.product_id(),
                    ModuleCategory::from_key(key).unwrap(),
                    0,
                )
                .ok_or_else(|| error("No disabled model for this module"))?;
                let category_id = wire_preset::CATEGORIES
                    .iter()
                    .find(|(_, k, _)| *k == key)
                    .unwrap()
                    .0
                    - 5;
                let id = if key == "Amplifier" {
                    String::new()
                } else {
                    format!(" ID=\"{category_id}\"")
                };
                let bypass = if pid == 14 {
                    format!(" BypassMode=\"{}\"", model.bypass_mode)
                } else {
                    String::new()
                };
                edits.push((
                    node.range(),
                    format!(
                        "<{key}{id}><Module ID=\"0\" POS=\"0\" BypassState=\"1\"{bypass}/></{key}>"
                    ),
                ));
            }
        }
        for key in if pid == 14 && control == 1 {
            &["UsbGain"][..]
        } else if pid == 14 && control == 2 {
            &["FirstExpressionPedal"][..]
        } else {
            &[][..]
        } {
            if let Some(replacement) = category(src.root_element(), key)? {
                let node = preset::child(dst.root_element(), key)
                    .map_err(error)?
                    .unwrap();
                edits.push((node.range(), source[replacement.range()].to_string()));
            }
        }
        result.apply(edits)?;
        if pid == 14 && control == 2 {
            let pedal = result.expression_pedal()?;
            let mode = pedal
                .as_ref()
                .and_then(|p| p.expression_mode_behavior)
                .unwrap_or(0);
            let key = preset::expression::target_key(mode);
            let assigned = key.is_some_and(|key| {
                result
                    .modules()
                    .iter()
                    .any(|m| m.key == key && m.model_id.is_some_and(|id| id != 0))
            });
            if !assigned {
                let doc = preset::parse_xml(&result.xml).map_err(error)?;
                if let Some(node) =
                    preset::child(doc.root_element(), "FirstExpressionPedal").map_err(error)?
                {
                    let mut edits = vec![];
                    preset::edit_attributes(
                        &result.xml,
                        node,
                        &[("ParameterIndex", "0".into())],
                        &mut edits,
                    )
                    .map_err(error)?;
                    result.apply(edits)?;
                }
            }
        }
        Ok(result)
    }
    pub fn is_clean_voice(&self) -> bool {
        self.pid == 14 && self.control == 1 && !self.modules().iter().any(|m| m.key == "Stompbox")
    }
    pub(super) fn mutate_projection(
        &mut self,
        operation: impl FnOnce(&mut PresetDocument) -> std::result::Result<(), preset::PresetError>,
    ) -> Result<()> {
        let mut projected = self.projected.clone();
        operation(&mut projected).map_err(error)?;
        let next = projected.to_xml().map_err(error)?;
        let parsed = preset::parse_xml(&next).map_err(error)?;
        let original = preset::parse_xml(&self.xml).map_err(error)?;
        let mut edits = vec![];
        for key in [
            "Amplifier",
            "Stompbox",
            "Modulation",
            "Delay",
            "Reverb",
            "FirstExpressionPedal",
            "UsbGain",
        ] {
            if let Some(new) = category(parsed.root_element(), key)? {
                let text = &next[new.range()];
                if let Some(old) = preset::child(original.root_element(), key).map_err(error)? {
                    if &self.xml[old.range()] != text {
                        edits.push((old.range(), text.to_string()));
                    }
                } else {
                    preset::append_child(&self.xml, original.root_element(), text, &mut edits)
                        .map_err(error)?;
                }
            }
        }
        self.apply(edits)
    }
}
