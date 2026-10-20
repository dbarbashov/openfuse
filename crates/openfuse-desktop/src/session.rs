//! File editing state; deliberately independent of windows and USB access.
use openfuse_core::preset::{
    MAX_PRESET_BYTES, PresetAnnotations, PresetDocument, PresetMetadata, PresetParameter,
};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[cfg(test)]
const DEMO: &str = include_str!("../../openfuse-core/tests/fixtures/synthetic.fuse");

const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HISTORY_ENTRIES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEdit {
    Parameters { module: String, indices: Vec<u8> },
    Structure,
}

pub struct Session {
    pub document: PresetDocument,
    pub path: Option<PathBuf>,
    pub source_label: Option<String>,
    conversion_copy: bool,
    storage_copy: bool,
    saved: String,
    current: String,
    undo: Vec<String>,
    redo: Vec<String>,
    device_edits: Vec<DeviceEdit>,
}

impl Session {
    pub fn new_preset(product_id: u32) -> Result<Self, String> {
        use openfuse_core::model_catalog::{self, ModuleCategory};
        let product = model_catalog::product(product_id).ok_or("Unknown preset product")?;
        let amp = model_catalog::model(
            product_id,
            ModuleCategory::Amplifier,
            product.default_amp_id,
        )
        .ok_or("Missing default amplifier")?;
        let mut xml = format!(
            "<Preset ProductId=\"{product_id}\"><Amplifier ID=\"0\"><Module ID=\"{}\" POS=\"0\" BypassState=\"1\" BypassMode=\"{}\">",
            amp.id, amp.bypass_mode
        );
        for parameter in &amp.parameters {
            xml.push_str(&format!(
                "<Param ControlIndex=\"{}\">{}</Param>",
                parameter.index, parameter.default_raw
            ));
        }
        xml.push_str("</Module></Amplifier><FX>");
        for (key, position) in [
            ("Stompbox", 0),
            ("Modulation", 1),
            ("Delay", 4),
            ("Reverb", 5),
        ] {
            let position = if product.orderable {
                position
            } else {
                u8::from(key != "Stompbox")
            };
            let category_id = match key {
                "Stompbox" => 1,
                "Modulation" => 2,
                "Delay" => 3,
                _ => 4,
            };
            xml.push_str(&format!("<{key} ID=\"{category_id}\"><Module ID=\"0\" POS=\"{position}\" BypassState=\"0\" BypassMode=\"0\"/></{key}>"));
        }
        xml.push_str("</FX><FUSE><Info name=\"Untitled preset\" author=\"\" tags=\"\" rating=\"0\" genre1=\"-1\" genre2=\"-1\" genre3=\"-1\" fenderid=\"0\"/><PedalColors><Color ID=\"1\">14</Color><Color ID=\"2\">1</Color><Color ID=\"3\">2</Color><Color ID=\"4\">10</Color></PedalColors></FUSE></Preset>");
        let mut session = Self::parse(&xml, None)?;
        session.saved.clear();
        Ok(session)
    }

    /// Independently authored example composed from catalog defaults, never a
    /// factory preset. It remains an offline example without hardware validation.
    pub fn example() -> Result<Self, String> {
        use openfuse_core::model_catalog::{self, ModuleCategory};
        let mut xml = String::from(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><Preset amplifier=\"Mustang V2\" ProductId=\"13\">",
        );
        for (key, id, position, active) in [
            ("Amplifier", 103, 0, true),
            ("Stompbox", 60, 0, false),
            ("Modulation", 18, 1, false),
            ("Delay", 43, 4, true),
            ("Reverb", 33, 5, true),
        ] {
            let category = ModuleCategory::from_key(key).ok_or("Unknown example category")?;
            let model = model_catalog::model(13, category, id).ok_or("Missing example model")?;
            if key == "Stompbox" {
                xml.push_str("<FX>");
            }
            let category_id = match key {
                "Amplifier" => 0,
                "Stompbox" => 1,
                "Modulation" => 2,
                "Delay" => 3,
                _ => 4,
            };
            xml.push_str(&format!("<{key} ID=\"{category_id}\"><Module ID=\"{id}\" POS=\"{position}\" BypassState=\"{}\" BypassMode=\"{}\">", u8::from(active), model.bypass_mode));
            for parameter in &model.parameters {
                xml.push_str(&format!(
                    "<Param ControlIndex=\"{}\">{}</Param>",
                    parameter.index, parameter.default_raw
                ));
            }
            xml.push_str(&format!("</Module></{key}>"));
        }
        xml.push_str("</FX><FUSE><Info name=\"Warm room\" author=\"OpenFUSE contributors\" tags=\"example, clean, offline\" rating=\"0\" genre1=\"-1\" genre2=\"-1\" genre3=\"-1\" fenderid=\"0\">An original example using catalog defaults. Explore the controls, then save your own variation. Not validated on an amplifier.</Info><PedalColors><Color ID=\"1\">14</Color><Color ID=\"2\">1</Color><Color ID=\"3\">2</Color><Color ID=\"4\">10</Color></PedalColors></FUSE></Preset>");
        Self::parse(&xml, None)
    }

    pub fn parse(xml: &str, path: Option<PathBuf>) -> Result<Self, String> {
        let document = PresetDocument::parse(xml).map_err(|e| e.to_string())?;
        let current = document.to_xml().map_err(|e| e.to_string())?;
        Ok(Self {
            document,
            path,
            source_label: None,
            conversion_copy: false,
            storage_copy: false,
            saved: current.clone(),
            current,
            undo: Vec::new(),
            redo: Vec::new(),
            device_edits: Vec::new(),
        })
    }

    pub fn from_device(document: &PresetDocument, source_label: String) -> Result<Self, String> {
        let mut session =
            Self::parse(&document.to_xml().map_err(|error| error.to_string())?, None)?;
        session.saved.clear();
        session.source_label = Some(source_label);
        Ok(session)
    }

    pub fn take_device_edits(&mut self) -> Vec<DeviceEdit> {
        std::mem::take(&mut self.device_edits)
    }

    fn record_device_edit(&mut self, edit: DeviceEdit) {
        if self.device_edits.len() >= 256 {
            self.device_edits.clear();
            self.device_edits.push(DeviceEdit::Structure);
        } else {
            self.device_edits.push(edit);
        }
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
        let mut bytes = Vec::new();
        file.take((MAX_PRESET_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_PRESET_BYTES {
            return Err(format!("Preset exceeds {MAX_PRESET_BYTES} bytes"));
        }
        let xml = String::from_utf8(bytes).map_err(|e| format!("Preset is not UTF-8: {e}"))?;
        Self::parse(&xml, Some(path.to_owned()))
    }

    pub fn dirty(&self) -> bool {
        self.current != self.saved
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn edit<T>(
        &mut self,
        change: impl FnOnce(&mut PresetDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        self.edit_recorded(change, true)
    }

    fn edit_local<T>(
        &mut self,
        change: impl FnOnce(&mut PresetDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        self.edit_recorded(change, false)
    }

    fn edit_recorded<T>(
        &mut self,
        change: impl FnOnce(&mut PresetDocument) -> Result<T, String>,
        structural: bool,
    ) -> Result<T, String> {
        // Apply to a candidate so a rejected edit cannot partly mutate the live preset.
        let mut candidate = PresetDocument::parse(&self.current).map_err(|e| e.to_string())?;
        let result = change(&mut candidate)?;
        let next = candidate.to_xml().map_err(|e| e.to_string())?;
        if next != self.current {
            self.undo.push(std::mem::replace(&mut self.current, next));
            self.redo.clear();
            self.trim_history();
            self.document = candidate;
            if structural {
                self.record_device_edit(DeviceEdit::Structure);
            }
        }
        Ok(result)
    }

    pub fn expression(
        &mut self,
        slot: openfuse_core::preset::expression::PedalSlot,
        values: &openfuse_core::preset::expression::ExpressionPedal,
    ) -> Result<(), String> {
        self.edit(|document| {
            document
                .set_expression_pedal(slot, values)
                .map_err(|error| error.to_string())
        })
    }

    pub fn import_modules(
        &mut self,
        source: &PresetDocument,
        keys: &[&str],
    ) -> Result<openfuse_core::preset::module_import::ModuleImportReport, String> {
        self.edit(|document| {
            document
                .import_modules(source, keys)
                .map_err(|error| error.to_string())
        })
    }

    pub fn band(&mut self, band: &openfuse_core::preset::band::Band) -> Result<(), String> {
        self.edit_local(|document| document.set_band(band).map_err(|e| e.to_string()))
    }

    pub fn metadata(&mut self, metadata: &PresetMetadata) -> Result<(), String> {
        self.edit_local(|doc| doc.set_metadata(metadata).map_err(|e| e.to_string()))
    }

    pub fn annotations(&mut self, values: &PresetAnnotations) -> Result<(), String> {
        self.edit_local(|doc| doc.set_annotations(values).map_err(|e| e.to_string()))
    }

    pub fn pedal_colors(&mut self, values: [i32; 4]) -> Result<(), String> {
        self.edit_local(|doc| doc.set_pedal_colors(values).map_err(|e| e.to_string()))
    }

    pub fn parameter(&mut self, key: &str, index: u8, value: u16) -> Result<(), String> {
        let before = self
            .document
            .modules()
            .into_iter()
            .find(|module| module.key == key);
        self.edit_local(|doc| {
            use openfuse_core::model_catalog::{self, ModuleCategory};
            let module = doc.modules().into_iter().find(|m| m.key == key);
            if let (Some(product), Some(category), Some(module)) =
                (doc.product_id(), ModuleCategory::from_key(key), module)
                && let Some(model_id) = module.model_id
                && model_catalog::model(product, category, model_id).is_some()
            {
                let current: Vec<_> = module
                    .parameters
                    .iter()
                    .map(|p| (p.index, p.value))
                    .collect();
                let values = model_catalog::cascade_parameters(
                    product, category, model_id, &current, index, value,
                );
                let parameters: Vec<_> = values
                    .into_iter()
                    .map(|(index, value)| PresetParameter { index, value })
                    .collect();
                doc.set_parameters(key, &parameters)
                    .map_err(|e| e.to_string())
            } else {
                doc.set_parameter(key, index, value)
                    .map_err(|e| e.to_string())
            }
        })?;
        if let (Some(before), Some(after)) = (
            before,
            self.document
                .modules()
                .into_iter()
                .find(|module| module.key == key),
        ) {
            let indices = after
                .parameters
                .iter()
                .filter(|parameter| !before.parameters.contains(parameter))
                .map(|parameter| parameter.index)
                .collect::<Vec<_>>();
            if !indices.is_empty() {
                self.record_device_edit(DeviceEdit::Parameters {
                    module: key.into(),
                    indices,
                });
            }
        }
        Ok(())
    }

    pub fn model(
        &mut self,
        key: &str,
        model_id: u16,
        parameters: &[PresetParameter],
        bypass_mode: Option<u8>,
    ) -> Result<(), String> {
        self.edit(|doc| {
            doc.replace_module_model(key, model_id, parameters, bypass_mode)
                .map_err(|e| e.to_string())?;
            doc.set_module_active(key, true).map_err(|e| e.to_string())
        })
    }

    pub fn active(&mut self, key: &str, active: bool) -> Result<(), String> {
        self.edit(|doc| {
            doc.set_module_active(key, active)
                .map_err(|e| e.to_string())
        })
    }

    #[cfg(test)]
    pub fn positions(&mut self, positions: &[(&str, u16)]) -> Result<(), String> {
        self.edit(|doc| {
            doc.set_module_positions(positions)
                .map_err(|e| e.to_string())
        })
    }

    pub fn move_module(&mut self, key: &str, position: u16) -> Result<(), String> {
        self.edit(|doc| doc.move_module(key, position).map_err(|e| e.to_string()))
    }

    pub fn undo(&mut self) -> Result<(), String> {
        if let Some(xml) = self.undo.last() {
            let document = PresetDocument::parse(xml).map_err(|e| e.to_string())?;
            let previous = self.undo.pop().expect("checked nonempty");
            self.redo
                .push(std::mem::replace(&mut self.current, previous));
            self.document = document;
            self.trim_history();
            self.record_device_edit(DeviceEdit::Structure);
        }
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), String> {
        if let Some(xml) = self.redo.last() {
            let document = PresetDocument::parse(xml).map_err(|e| e.to_string())?;
            let next = self.redo.pop().expect("checked nonempty");
            self.undo.push(std::mem::replace(&mut self.current, next));
            self.document = document;
            self.trim_history();
            self.record_device_edit(DeviceEdit::Structure);
        }
        Ok(())
    }

    fn trim_history(&mut self) {
        while self.undo.len() + self.redo.len() > MAX_HISTORY_ENTRIES
            || self
                .undo
                .iter()
                .chain(&self.redo)
                .map(String::len)
                .sum::<usize>()
                > MAX_HISTORY_BYTES
        {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
        }
    }

    /// Detach from the original file while retaining the whole source as one undo step.
    pub fn apply_conversion(
        &mut self,
        document: PresetDocument,
        label: String,
    ) -> Result<(), String> {
        self.edit_local(|candidate| {
            *candidate = document;
            Ok(())
        })?;
        self.path = None;
        self.source_label = Some(label);
        self.saved.clear();
        self.conversion_copy = true;
        self.storage_copy = false;
        self.device_edits.clear();
        Ok(())
    }

    pub fn is_converted(&self) -> bool {
        self.conversion_copy
    }
    pub fn is_storage_copy(&self) -> bool {
        self.storage_copy
    }
    pub fn from_storage(document: &PresetDocument, label: String) -> Result<Self, String> {
        let mut session = Self::converted(document, label)?;
        session.storage_copy = true;
        Ok(session)
    }

    pub fn converted(document: &PresetDocument, label: String) -> Result<Self, String> {
        let mut session = Self::from_device(document, label)?;
        session.conversion_copy = true;
        Ok(session)
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        if self.conversion_copy && self.path.as_deref() != Some(path) {
            openfuse_core::preset_file::write_new(&self.document, path)
                .map_err(|e| e.to_string())?;
            self.path = Some(path.to_owned());
            self.saved = self.current.clone();
            return Ok(());
        }
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Save destination is a symbolic link; choose a regular file".into());
        }
        if self.path.as_deref() == Some(path) {
            let disk = Self::open(path)
                .map_err(|e| format!("Cannot verify original file before saving: {e}"))?;
            if disk.current != self.saved {
                return Err("The file changed outside OpenFUSE. Reopen it or use Save as with a different filename.".into());
            }
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(self.current.as_bytes())
            .map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        temp.persist(path)
            .map_err(|e| format!("Cannot save {}: {e}", path.display()))?;
        self.path = Some(path.to_owned());
        self.saved = self.current.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_profiles_have_catalog_defaults_and_start_unsaved() {
        use openfuse_core::model_catalog::{self, ModuleCategory};
        for product in model_catalog::products() {
            let session = Session::new_preset(product.id).unwrap();
            assert!(session.dirty());
            assert!(!session.can_undo());
            assert_eq!(session.document.product_id(), Some(product.id));
            let modules = session.document.modules();
            assert_eq!(modules.len(), 5);
            assert_eq!(modules[0].model_id, Some(product.default_amp_id));
            let amp = model_catalog::model(
                product.id,
                ModuleCategory::Amplifier,
                product.default_amp_id,
            )
            .unwrap();
            assert_eq!(modules[0].parameters.len(), amp.parameters.len());
            export_generated(&format!("new-{}.fuse", product.id), &session);
            assert!(
                modules[1..]
                    .iter()
                    .all(|m| m.model_id == Some(0) && m.active == Some(false))
            );
        }
        let example = Session::example().unwrap();
        assert!(!example.dirty());
        assert_eq!(example.document.modules().len(), 5);
        assert_eq!(example.document.metadata().name, "Warm room");
        export_generated("example.fuse", &example);
    }

    fn export_generated(name: &str, session: &Session) {
        if let Some(directory) = std::env::var_os("OPENFUSE_EXPORT_GENERATED_PRESETS") {
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(name), session.document.to_xml().unwrap()).unwrap();
        }
    }

    #[test]
    fn file_size_and_utf8_are_checked_before_parsing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized.fuse");
        let file = File::create(&path).unwrap();
        file.set_len((MAX_PRESET_BYTES + 1024) as u64).unwrap();
        assert!(Session::open(&path).err().unwrap().contains("exceeds"));
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(Session::open(&path).err().unwrap().contains("UTF-8"));
    }

    #[test]
    fn history_is_bounded_across_undo_and_redo() {
        let mut session = Session::parse(DEMO, None).unwrap();
        session.undo = vec!["x".repeat(MAX_HISTORY_BYTES / 2); 2];
        session.redo = vec!["y".repeat(MAX_HISTORY_BYTES / 2)];
        session.trim_history();
        assert_eq!(session.undo.len(), 1);
        assert_eq!(session.redo.len(), 1);
        session.undo = vec![String::new(); MAX_HISTORY_ENTRIES];
        session.trim_history();
        assert_eq!(session.undo.len() + session.redo.len(), MAX_HISTORY_ENTRIES);
    }

    #[test]
    fn edits_undo_redo_and_save_track_actual_saved_state() {
        let mut session = Session::parse(DEMO, None).unwrap();
        assert!(!session.dirty());
        let mut metadata = session.document.metadata();
        metadata.name = "My edited preset".into();
        session.metadata(&metadata).unwrap();
        assert!(session.dirty());
        session.undo().unwrap();
        assert!(!session.dirty());
        session.redo().unwrap();
        assert!(session.dirty());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("roundtrip.fuse");
        session.save_as(&path).unwrap();
        assert!(!session.dirty());
        assert_eq!(
            Session::open(&path).unwrap().document.metadata().name,
            metadata.name
        );
        session.undo().unwrap();
        assert!(session.dirty());
    }

    #[test]
    fn failed_save_retains_unsaved_work() {
        let mut session = Session::parse(DEMO, None).unwrap();
        let mut metadata = session.document.metadata();
        metadata.name = "Keep this".into();
        session.metadata(&metadata).unwrap();
        let directory = tempfile::tempdir().unwrap();
        assert!(
            session
                .save_as(&directory.path().join("missing/file.fuse"))
                .is_err()
        );
        assert!(session.dirty());
        assert!(session.path.is_none());
        assert_eq!(session.document.metadata().name, "Keep this");
    }

    #[test]
    fn rejected_edit_does_not_change_document_or_history() {
        let mut session = Session::parse(DEMO, None).unwrap();
        assert!(session.parameter("absent", 0, 10).is_err());
        assert!(!session.dirty());
        assert!(!session.can_undo());
    }

    #[test]
    fn model_bypass_and_chain_edits_have_transactional_history() {
        let mut session = Session::example().unwrap();
        let original = session.document.to_xml().unwrap();
        session
            .model(
                "Stompbox",
                42,
                &[PresetParameter {
                    index: 0,
                    value: 123,
                }],
                Some(1),
            )
            .unwrap();
        let after_model = session.document.to_xml().unwrap();
        session.active("Stompbox", false).unwrap();
        let after_bypass = session.document.to_xml().unwrap();
        session.positions(&[("Stompbox", 2), ("Delay", 0)]).unwrap();
        let after_chain = session.document.to_xml().unwrap();
        assert!(
            session
                .positions(&[("Stompbox", 1), ("Missing", 3)])
                .is_err()
        );
        assert_eq!(session.document.to_xml().unwrap(), after_chain);
        session.undo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), after_bypass);
        session.undo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), after_model);
        session.undo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), original);
        assert!(!session.dirty());
        for _ in 0..3 {
            session.redo().unwrap();
        }
        assert_eq!(session.document.to_xml().unwrap(), after_chain);
    }
    #[test]
    fn metadata_palette_edits_undo_and_roundtrip_without_precision_loss() {
        use openfuse_core::preset::annotations::{GENRES, PEDAL_COLORS};
        let mut session = Session::new_preset(13).unwrap();
        let original = session.document.to_xml().unwrap();
        let mut values = session.document.annotations().unwrap();
        values.rating = Some("4.1234567890123456789012345678".into());
        values.fender_id = Some(123456);
        values.genres = [Some(6), Some(25), Some(127)];
        session.annotations(&values).unwrap();
        session.pedal_colors([0, 5, 9, 15]).unwrap();
        let final_xml = session.document.to_xml().unwrap();
        session.undo().unwrap();
        session.undo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), original);
        session.redo().unwrap();
        session.redo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), final_xml);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exact.fuse");
        session.save_as(&path).unwrap();
        assert!(!session.dirty());
        let reopened = Session::open(&path).unwrap();
        assert_eq!(reopened.document.annotations().unwrap(), values);
        assert_eq!(
            reopened.document.pedal_colors().unwrap(),
            Some([0, 5, 9, 15])
        );
        export_generated("annotated-exact.fuse", &session);
        for genre in GENRES {
            values.genres = [Some(genre.id); 3];
            session.annotations(&values).unwrap();
            export_generated(&format!("genre-{}.fuse", genre.id), &session);
        }
        for color in PEDAL_COLORS {
            session.pedal_colors([color.id; 4]).unwrap();
            export_generated(&format!("color-{}.fuse", color.id), &session);
        }
        values.genres = [Some(500), Some(-20), Some(i32::MAX)];
        values.fender_id = Some(-200);
        session.annotations(&values).unwrap();
        session.pedal_colors([-5, 500, 16, i32::MAX]).unwrap();
        export_generated("unknown-annotations.fuse", &session);
    }

    #[test]
    fn external_edits_and_deletion_cannot_be_silently_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.fuse");
        std::fs::write(&path, DEMO).unwrap();
        let mut session = Session::open(&path).unwrap();
        let mut metadata = session.document.metadata();
        metadata.name = "Local change".into();
        session.metadata(&metadata).unwrap();
        let external = DEMO.replace("Synthetic", "External");
        assert_ne!(external, DEMO);
        std::fs::write(&path, &external).unwrap();
        assert!(
            session
                .save_as(&path)
                .unwrap_err()
                .contains("outside OpenFUSE")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
        assert!(session.dirty());
        std::fs::remove_file(&path).unwrap();
        assert!(session.save_as(&path).is_err());
        assert!(!path.exists());
        let recovery = directory.path().join("recovered.fuse");
        session.save_as(&recovery).unwrap();
        assert_eq!(
            Session::open(&recovery).unwrap().document.metadata().name,
            "Local change"
        );
    }
}

#[cfg(test)]
#[path = "session_extended_tests.rs"]
mod extended_tests;

#[cfg(test)]
mod device_journal_tests {
    use super::*;
    #[test]
    fn cascades_record_all_changed_parameters_without_replaying_metadata_or_noops() {
        let mut session = Session::parse("<Preset ProductId=\"13\"><Amplifier><Module ID=\"103\"><Param ControlIndex=\"15\">0</Param><Param ControlIndex=\"16\">9</Param><Param ControlIndex=\"9\">123</Param></Module></Amplifier></Preset>", None).unwrap();
        assert!(session.take_device_edits().is_empty());
        session.parameter("Amplifier", 15, 2).unwrap();
        assert_eq!(
            session.take_device_edits(),
            vec![DeviceEdit::Parameters {
                module: "Amplifier".into(),
                indices: vec![15, 16, 9]
            }]
        );
        session.parameter("Amplifier", 15, 2).unwrap();
        assert!(session.take_device_edits().is_empty());
        let mut metadata = session.document.metadata();
        metadata.name = "Local metadata".into();
        session.metadata(&metadata).unwrap();
        assert!(session.take_device_edits().is_empty());
        session.undo().unwrap();
        assert_eq!(session.take_device_edits(), vec![DeviceEdit::Structure]);
    }
    #[test]
    fn rejected_parameter_and_device_load_emit_nothing_and_save_keeps_original_snapshot_label() {
        let example = Session::example().unwrap();
        let mut session =
            Session::from_device(&example.document, "Captured device · read 5".into()).unwrap();
        assert!(session.take_device_edits().is_empty());
        let xml = session.document.to_xml().unwrap();
        assert!(session.parameter("Missing", 0, 5).is_err());
        assert_eq!(session.document.to_xml().unwrap(), xml);
        assert!(session.take_device_edits().is_empty());
        let folder = tempfile::tempdir().unwrap();
        session
            .save_as(&folder.path().join("capture.fuse"))
            .unwrap();
        assert!(!session.dirty());
        assert!(session.source_label.is_some());
        assert!(session.take_device_edits().is_empty());
    }
}
