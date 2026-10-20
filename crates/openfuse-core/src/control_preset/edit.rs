use super::*;
impl ControlPreset {
    fn check_module(&self, key: &str) -> Result<()> {
        if !self.allowed_modules().contains(&key) {
            return Err(error("Module does not belong to this physical bank"));
        }
        if !self.modules().iter().any(|m| m.key == key) {
            return Err(error("Module is absent from this document"));
        }
        Ok(())
    }
    pub fn set_name(&mut self, name: &str) -> Result<()> {
        if name.chars().count() > 48 || name.chars().any(|c| c < ' ' || u32::from(c) > 255) {
            return Err(error(
                "Physical bank names require at most 48 printable Latin-byte characters",
            ));
        }
        if self.name == name {
            return Ok(());
        }
        let doc = preset::parse_xml(&self.xml).map_err(error)?;
        let mut edits = vec![];
        preset::edit_attributes(
            &self.xml,
            doc.root_element(),
            &[("Name", name.into())],
            &mut edits,
        )
        .map_err(error)?;
        self.apply(edits)
    }
    pub fn set_parameter(&mut self, key: &str, index: u8, value: u16) -> Result<()> {
        self.check_module(key)?;
        let modules = self.modules();
        let module = modules.iter().find(|m| m.key == key).unwrap();
        let model = model_catalog::model(
            self.product_id(),
            ModuleCategory::from_key(key).unwrap(),
            module.model_id.ok_or_else(|| error("Missing model ID"))?,
        )
        .ok_or_else(|| error("Unknown model cannot be edited"))?;
        let parameter = model
            .parameters
            .iter()
            .find(|p| p.index == index)
            .ok_or_else(|| error("Unknown parameter"))?;
        if !parameter.raw_range().contains(&value) {
            return Err(error("Parameter value is outside its known domain"));
        }
        let current = module
            .parameters
            .iter()
            .map(|p| (p.index, p.value))
            .collect::<Vec<_>>();
        let values = model_catalog::cascade_parameters(
            self.product_id(),
            model.category,
            model.id,
            &current,
            index,
            value,
        )
        .into_iter()
        .map(|(index, value)| PresetParameter { index, value })
        .collect::<Vec<_>>();
        self.mutate_projection(|p| p.set_parameters(key, &values))
    }
    pub fn replace_module_model(&mut self, key: &str, id: u16) -> Result<()> {
        self.check_module(key)?;
        let model = model_catalog::model(
            self.product_id(),
            ModuleCategory::from_key(key).unwrap(),
            id,
        )
        .ok_or_else(|| error("Unknown model cannot be selected"))?;
        let parameters = model
            .parameters
            .iter()
            .map(|p| PresetParameter {
                index: p.index,
                value: p.default_raw,
            })
            .collect::<Vec<_>>();
        self.mutate_projection(|p| {
            p.replace_module_model(key, id, &parameters, Some(model.bypass_mode))
        })
    }
    pub fn set_module_active(&mut self, key: &str, active: bool) -> Result<()> {
        self.check_module(key)?;
        self.mutate_projection(|p| p.set_module_active(key, active))
    }
    pub fn set_module_position(&mut self, key: &str, position: u16) -> Result<()> {
        self.check_module(key)?;
        self.mutate_projection(|p| p.move_module(key, position))
    }
    pub fn set_expression_pedal(&mut self, pedal: &ExpressionPedal) -> Result<()> {
        if self.pid != 14 || self.control != 2 {
            return Err(error(
                "Expression settings belong only to SuperChamp FX banks",
            ));
        }
        if pedal
            .expression_mode_behavior
            .is_some_and(|v| matches!(v, 1 | 2))
        {
            return Err(error(
                "Voice modules cannot be expression targets of an FX bank",
            ));
        }
        self.mutate_projection(|p| p.set_expression_pedal(PedalSlot::First, pedal))
    }
    pub fn set_usb_gain(&mut self, gain: u8) -> Result<()> {
        if self.pid != 14 || self.control != 1 {
            return Err(error("USB gain belongs only to SuperChamp voice banks"));
        }
        let doc = preset::parse_xml(&self.xml).map_err(error)?;
        let mut edits = vec![];
        if let Some(node) = preset::child(doc.root_element(), "UsbGain").map_err(error)? {
            preset::edit_text(&self.xml, node, &gain.to_string(), &mut edits).map_err(error)?;
        } else {
            preset::append_child(
                &self.xml,
                doc.root_element(),
                &format!("<UsbGain>{gain}</UsbGain>"),
                &mut edits,
            )
            .map_err(error)?;
        }
        self.apply(edits)
    }
}
