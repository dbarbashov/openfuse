use super::{Editor, Pending};
use crate::conversion_view::{self, Action, Draft};
use eframe::egui;
use std::path::PathBuf;

impl Editor {
    pub(super) fn conversion_demo(&mut self, kind: u8, ctx: &egui::Context) {
        if kind != 2
            && let Some(session) = &mut self.session
        {
            use openfuse_core::{
                model_catalog::{self, ModuleCategory},
                preset::PresetParameter,
            };
            if let Some(model) = model_catalog::model(13, ModuleCategory::Amplifier, 249) {
                let parameters: Vec<_> = model
                    .parameters
                    .iter()
                    .map(|p| PresetParameter {
                        index: p.index,
                        value: p.default_raw,
                    })
                    .collect();
                if let Err(error) =
                    session.model("Amplifier", model.id, &parameters, Some(model.bypass_mode))
                {
                    self.error = Some(error);
                }
            }
        }
        self.prepare_current_conversion();
        if let Some(draft) = &mut self.conversion_draft {
            if kind == 2 {
                draft.source = draft.source.replace("Warm room", "Warm &amp;amp; room");
                draft.label = "Synthetic legacy file · double-encoded preset name".into();
                draft.current = false;
                draft.options.repair_double_entities = true;
            } else {
                draft.options.target_product = 1;
            }
            draft.review();
            if kind == 3 {
                draft.reviewed = true;
            }
        }
        if kind == 3 {
            self.apply_conversion(ctx);
        }
    }

    pub(super) fn prepare_current_conversion(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        match session.document.to_xml() {
            Ok(source) => {
                let label = session
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Current local preset (including unsaved edits)".into());
                self.conversion_draft = Some(Draft::new(source, label, true));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    pub(super) fn prepare_file_conversion(&mut self, path: PathBuf) {
        match openfuse_core::preset_file::read(&path) {
            Ok(source) => {
                self.conversion_draft = Some(Draft::new(source, path.display().to_string(), false))
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    pub(super) fn choose_conversion(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Convert or repair a FUSE preset")
            .add_filter("FUSE preset", &["fuse", "FUSE", "xml"])
            .pick_file()
        {
            self.prepare_file_conversion(path);
        }
    }
    fn conversion_source_valid(&self) -> bool {
        self.conversion_draft.as_ref().is_some_and(|draft| {
            !draft.current
                || self
                    .session
                    .as_ref()
                    .and_then(|s| s.document.to_xml().ok())
                    .as_deref()
                    == Some(draft.source.as_str())
        })
    }
    pub(super) fn conversion_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let valid = self.conversion_source_valid();
        let Some(draft) = &mut self.conversion_draft else {
            return;
        };
        match conversion_view::show(ctx, draft, valid) {
            Some(Action::Cancel) => self.conversion_draft = None,
            Some(Action::Apply) => self.apply_conversion(ctx),
            Some(Action::Export) => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Export converted copy to a new file")
                    .add_filter("FUSE preset", &["fuse"])
                    .set_file_name("converted-preset.fuse")
                    .save_file()
                {
                    let result = draft.result().and_then(|doc| {
                        openfuse_core::preset_file::write_new(&doc, &path)
                            .map_err(|e| e.to_string())
                    });
                    match result {
                        Ok(()) => draft.notice = Some(format!("Exported {}", path.display())),
                        Err(e) => draft.error = Some(e),
                    }
                }
            }
            None => {}
        }
    }
    pub(super) fn apply_conversion(&mut self, ctx: &egui::Context) {
        if !self.conversion_source_valid() {
            if let Some(draft) = &mut self.conversion_draft {
                draft.error =
                    Some("The editor source changed. Cancel and review a fresh preview.".into());
            }
            return;
        }
        let Some(draft) = &mut self.conversion_draft else {
            return;
        };
        let document = match draft.result() {
            Ok(doc) => doc,
            Err(e) => {
                draft.error = Some(e);
                return;
            }
        };
        let label = format!("Converted copy of {}", draft.label);
        if !draft.current {
            self.request(Pending::OpenConverted(document, label), ctx);
            return;
        }
        self.disable_live();
        let result = self
            .session
            .as_mut()
            .ok_or("Source editor is missing".to_string())
            .and_then(|session| session.apply_conversion(document, label));
        match result {
            Ok(()) => {
                self.device_origin = None;
                self.read_draft = None;
                self.audition_draft = None;
                self.import_draft = None;
                self.last_import = None;
                self.conversion_draft = None;
                self.status =
                    "Converted copy opened · source retained in Undo · save to a new file".into();
                self.error = None;
            }
            Err(error) => {
                if let Some(draft) = &mut self.conversion_draft {
                    draft.error = Some(error);
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "app_conversion_tests.rs"]
mod tests;
