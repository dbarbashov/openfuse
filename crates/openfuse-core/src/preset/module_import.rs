//! Selective, transactional import of known module state.
//! This API performs no device operations.
use std::collections::BTreeSet;

use super::{
    PresetDocument, PresetModule, Result, append_child, attribute, child, direct_text, edit_text,
    error, find_module, named, number, parse_xml,
};
use crate::model_catalog::{self, ModuleCategory};

pub const MODULE_KEYS: [&str; 5] = ["Amplifier", "Stompbox", "Modulation", "Delay", "Reverb"];

#[derive(Debug, Clone)]
pub struct ModuleImportOption {
    pub key: String,
    pub compatible: bool,
    pub reason: Option<String>,
    /// Includes every deliberate normalization/loss known before committing.
    pub notes: Vec<String>,
}
#[derive(Debug, Clone, Default)]
pub struct ModuleImportReport {
    pub imported: Vec<String>,
    pub changes: Vec<String>,
    pub losses: Vec<String>,
}

impl PresetDocument {
    pub fn preview_module_import(&self, source: &Self) -> Vec<ModuleImportOption> {
        MODULE_KEYS
            .iter()
            .map(|key| {
                match self.validate_import_module(source, key) {
                    Ok(module) => {
                        let notes = self.import_notes(source, &module);
                        let mut candidate = self.clone();
                        // Exercise the real source-preserving edits, including expression
                        // bindings and protected destination extensions. Chain collisions
                        // are selection-wide and checked only in the final transaction.
                        match candidate.import_one(source, &module) {
                            Ok(()) => ModuleImportOption {
                                key: (*key).into(),
                                compatible: true,
                                reason: None,
                                notes,
                            },
                            Err(reason) => ModuleImportOption {
                                key: (*key).into(),
                                compatible: false,
                                reason: Some(reason.to_string()),
                                notes,
                            },
                        }
                    }
                    Err(reason) => ModuleImportOption {
                        key: (*key).into(),
                        compatible: false,
                        reason: Some(reason.to_string()),
                        notes: Vec::new(),
                    },
                }
            })
            .collect()
    }

    /// Import a nonempty subset in canonical category order as one transaction.
    /// On any error the destination is byte-for-byte unchanged. Preview options
    /// must not be taken as proof that a combination has conflict-free routing.
    pub fn import_modules(&mut self, source: &Self, keys: &[&str]) -> Result<ModuleImportReport> {
        if keys.is_empty() {
            return Err(error("Choose at least one module to import"));
        }
        let selected: BTreeSet<_> = keys.iter().copied().collect();
        if selected.len() != keys.len() || selected.iter().any(|key| !MODULE_KEYS.contains(key)) {
            return Err(error(
                "Import selection contains an unknown or duplicate module",
            ));
        }
        let mut next = self.clone();
        let mut report = ModuleImportReport::default();
        for key in MODULE_KEYS.into_iter().filter(|key| selected.contains(key)) {
            let module = self.validate_import_module(source, key)?;
            let before = next.xml.clone();
            next.import_one(source, &module)?;
            report.imported.push(key.into());
            if next.xml != before {
                report.changes.push(format!(
                    "Imported {key} model, parameters, activation and stored routing"
                ));
            }
            report.losses.extend(self.import_notes(source, &module));
        }
        next.validate_import_routing(&selected)?;
        let mut seen = BTreeSet::new();
        report.losses.retain(|note| seen.insert(note.clone()));
        *self = next;
        Ok(report)
    }

    fn validate_import_module(&self, source: &Self, key: &str) -> Result<PresetModule> {
        let destination_id = self
            .product_id
            .ok_or_else(|| error("Destination family is missing"))?;
        let source_id = source
            .product_id
            .ok_or_else(|| error("Source family is missing"))?;
        let destination = model_catalog::product(destination_id)
            .ok_or_else(|| error("Unknown destination family"))?;
        model_catalog::product(source_id)
            .ok_or_else(|| error("Unknown source family; legacy conversion is not inferred"))?;
        if !allowed_source(destination_id, source_id) {
            return Err(error(
                "Source family is outside this destination's recovered import compatibility list",
            ));
        }
        let category =
            ModuleCategory::from_key(key).ok_or_else(|| error("Unknown module category"))?;
        let source_module = source
            .modules
            .iter()
            .find(|module| module.key == key)
            .ok_or_else(|| error("Source has no module in this category"))?;
        if !self.modules.iter().any(|module| module.key == key) {
            return Err(error("Destination has no slot in this category"));
        }
        let id = source_module
            .model_id
            .ok_or_else(|| error("Source module has no model identifier"))?;
        let source_model = model_catalog::model(source_id, category, id).ok_or_else(|| {
            error("Source model is unknown; refusing to infer its parameter semantics")
        })?;
        let destination_model = model_catalog::model(destination_id, category, id).ok_or_else(|| error("Model is unavailable in destination family; automatic substitution/removal would lose the sound"))?;
        let shape = |model: &model_catalog::Model| {
            let mut result: Vec<_> = model
                .parameters
                .iter()
                .map(|p| (p.index, p.control_index, p.type_id))
                .collect();
            result.sort();
            result
        };
        if shape(source_model) != shape(destination_model) {
            return Err(error(
                "Matching model ID has different parameter semantics; conversion is not established",
            ));
        }
        let indices: BTreeSet<_> = source_module.parameters.iter().map(|p| p.index).collect();
        let expected: BTreeSet<_> = destination_model
            .parameters
            .iter()
            .map(|p| p.index)
            .collect();
        if indices != expected {
            return Err(error(
                "Source parameter set is incomplete or contains unknown indices; no defaults are silently inserted",
            ));
        }
        for parameter in &source_module.parameters {
            let kind = source_model
                .parameters
                .iter()
                .find(|p| p.index == parameter.index)
                .expect("validated index")
                .type_id;
            let milliseconds_max = match kind {
                136 => Some(1110),
                137 => Some(555),
                138 => Some(370),
                _ => None,
            };
            if let Some(maximum) = milliseconds_max
                && !(30..=maximum).contains(&parameter.value)
            {
                return Err(error(
                    "Legacy delay value is outside the canonical millisecond range; FUSE interprets it as normalized legacy data, so explicit conversion is required",
                ));
            }
        }
        if destination.store_bypass && source_module.active.is_none() {
            return Err(error("Source activation state is missing"));
        }
        if key != "Amplifier" {
            let position = source_module
                .position
                .ok_or_else(|| error("Source effect position is missing"))?;
            let maximum = if destination.orderable { 7 } else { 1 };
            if position > maximum {
                return Err(error(
                    "Source routing position is outside the destination's known range",
                ));
            }
            if destination_id == 10 && key == "Stompbox" && id != 0 && position != 0 {
                return Err(error("G-DEC stompbox must remain before the amplifier"));
            }
        }
        let parsed = parse_xml(&source.xml)?;
        let node = find_module(parsed.root_element(), key)?;
        let wrapper = node
            .parent()
            .ok_or_else(|| error("Source category wrapper is missing"))?;
        let expected_id = MODULE_KEYS
            .iter()
            .position(|candidate| *candidate == key)
            .expect("validated key") as u8;
        let declared_id = attribute(wrapper, "ID")
            .map(|value| number::<u8>(value, "category identifier"))
            .transpose()?
            .unwrap_or(0);
        if declared_id != expected_id {
            return Err(error(
                "Source category ID disagrees with its XML category; no reinterpretation is allowed",
            ));
        }
        if wrapper
            .attributes()
            .any(|a| a.namespace().is_some() || a.name() != "ID")
            || wrapper
                .children()
                .filter(|node| node.is_element())
                .any(|node| !named(node, "Module"))
        {
            return Err(error(
                "Source category wrapper has unknown content; import semantics are not established",
            ));
        }
        if node.attributes().any(|a| {
            a.namespace().is_some()
                || !["ID", "POS", "BypassState", "BypassMode"].contains(&a.name())
        }) || node
            .children()
            .filter(|n| n.is_element())
            .any(|n| !named(n, "Param"))
            || node.children().filter(|n| named(*n, "Param")).any(|n| {
                n.attributes()
                    .any(|a| a.namespace().is_some() || a.name() != "ControlIndex")
            })
        {
            return Err(error(
                "Source module contains unknown extensions; their import semantics are not established",
            ));
        }
        Ok(source_module.clone())
    }

    fn import_one(&mut self, source: &Self, module: &PresetModule) -> Result<()> {
        let id = self.product_id.expect("validated family");
        let category = ModuleCategory::from_key(&module.key).expect("validated category");
        let model = model_catalog::model(id, category, module.model_id.expect("validated model"))
            .expect("validated catalog model");
        self.replace_module_model(
            &module.key,
            model.id,
            &module.parameters,
            Some(model.bypass_mode),
        )?;
        let active = if model_catalog::product(id)
            .expect("validated product")
            .store_bypass
        {
            module.active.expect("validated activation")
        } else {
            true
        };
        self.set_module_active(&module.key, active)?;
        if module.key != "Amplifier" {
            let mut position = module.position.expect("validated position");
            if id == 2259 {
                position = match (module.key.as_str(), position) {
                    ("Stompbox", _) => 3,
                    (_, 4) => 5,
                    (_, 3) => 2,
                    _ => position,
                };
            }
            self.set_module_position(&module.key, position)?;
        } else if id != 10 {
            self.import_usb_gain(source)?;
        }
        Ok(())
    }

    fn import_usb_gain(&mut self, source: &Self) -> Result<()> {
        let source_xml = parse_xml(&source.xml)?;
        let value = child(source_xml.root_element(), "UsbGain")?
            .map(|node| {
                if node.attributes().len() != 0 || node.children().any(|n| n.is_element()) {
                    return Err(error("Source USB gain has unknown extensions"));
                }
                number::<u8>(&direct_text(node), "USB gain")
            })
            .transpose()?
            .unwrap_or(0);
        let document = parse_xml(&self.xml)?;
        let mut edits = Vec::new();
        if let Some(node) = child(document.root_element(), "UsbGain")? {
            if direct_text(node).trim() != value.to_string() {
                edit_text(&self.xml, node, &value.to_string(), &mut edits)?;
            }
        } else {
            append_child(
                &self.xml,
                document.root_element(),
                &format!("<UsbGain>{value}</UsbGain>"),
                &mut edits,
            )?;
        }
        self.apply(edits)
    }

    fn import_notes(&self, source: &Self, module: &PresetModule) -> Vec<String> {
        let mut notes = Vec::new();
        let id = self.product_id.expect("validated family");
        let category = ModuleCategory::from_key(&module.key).expect("validated category");
        let model = model_catalog::model(id, category, module.model_id.unwrap())
            .expect("validated catalog");
        if module.bypass_mode != Some(model.bypass_mode) {
            notes.push(format!(
                "{}: FUSE loading resets bypass mode to catalog default {}",
                module.key, model.bypass_mode
            ));
        }
        if id == 10 && module.active != Some(true) {
            notes.push(format!(
                "{}: G-DEC imports effects as active; saved bypass state is not supported",
                module.key
            ));
        }
        if module.key == "Amplifier" && id != 10 {
            notes.push(
                "Amplifier import also copies USB gain; absent source gain becomes 0, as in FUSE"
                    .into(),
            );
        }
        if id == 2259 && module.key != "Amplifier" {
            notes.push(format!("{}: Super Champ routing reserves stomp slot 3 and skips post-amp slot 4; conflicting selections are refused", module.key));
        }
        if self.product_id != source.product_id {
            notes.push(format!("{}: compatible model and parameter encoding verified; destination family stays unchanged", module.key));
        }
        if self.xml.contains("ExpressionPedal") {
            notes.push(format!("{}: destination expression bindings are retained when valid; first-pedal parameter follows imported model default, removed effects disable associated bindings", module.key));
        }
        notes.push("Source preset name, tags, colors, expression configuration and band settings are not imported".into());
        notes
    }

    fn validate_import_routing(&self, selected: &BTreeSet<&str>) -> Result<()> {
        if !selected.iter().any(|key| *key != "Amplifier") {
            return Ok(());
        }
        let product = model_catalog::product(self.product_id.unwrap()).expect("validated product");
        if !product.orderable {
            return Ok(());
        }
        let mut positions = BTreeSet::new();
        for module in self
            .modules
            .iter()
            .filter(|m| m.key != "Amplifier" && m.model_id != Some(0))
        {
            let position = module.position.filter(|value| *value <= 7).ok_or_else(|| error("Existing active chain has unknown routing; resolve it before importing effects"))?;
            if !positions.insert(position) {
                return Err(error(
                    "Imported effect positions collide with the retained chain; include the conflicting category or choose a differently routed source",
                ));
            }
        }
        Ok(())
    }
}

fn allowed_source(destination: u32, source: u32) -> bool {
    match destination {
        1 | 6 | 7 | 13 => [1, 2, 6, 7, 13].contains(&source),
        2259 => [2259, 1, 13].contains(&source),
        10 => source == 10,
        11 => source == 11,
        _ => false,
    }
}
