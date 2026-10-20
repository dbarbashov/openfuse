//! Deterministic native UI examples: synthetic devices and temporary libraries only.
use super::Editor;
use crate::{auxiliary_view::Action, control_editor_state::Operation, control_editor_view::Tab};
impl Editor {
    pub(super) fn control_demo_update(&mut self) {
        if self.control_ui.demo == 0 || self.usb_open_demo || !self.device.simulated {
            return;
        }
        match self.control_ui.demo_stage {
            0 if self.device.current_binding().is_some() => {
                self.open_auxiliary();
                self.auxiliary_action(Action::Read);
                self.control_ui.demo_stage = 1;
            }
            1 if self.device.current_auxiliary().is_some() && !self.device.operation_busy() => {
                self.auxiliary_action(Action::UseRead);
                let key = self
                    .auxiliary_ui
                    .source
                    .as_ref()
                    .unwrap()
                    .snapshot
                    .entries()
                    .iter()
                    .find(|entry| entry.restore_error().is_none())
                    .map(|entry| entry.key());
                if let Some(key) = key {
                    self.edit_auxiliary_entry(key);
                }
                if let Some(draft) = &mut self.control_ui.draft {
                    let name = if draft.document.pid() == 0xe {
                        "Glass voice"
                    } else {
                        "Velvet modulation"
                    };
                    let key = draft.document.modules()[0].key.clone();
                    let category =
                        openfuse_core::model_catalog::ModuleCategory::from_key(&key).unwrap();
                    let model =
                        openfuse_core::model_catalog::models(draft.document.product_id(), category)
                            .find(|model| model.id != 0)
                            .unwrap()
                            .id;
                    let result = draft.edit(|doc| {
                        doc.set_name(name).map_err(|e| e.to_string())?;
                        doc.replace_module_model(&key, model)
                            .map_err(|e| e.to_string())
                    });
                    if let Err(error) = result {
                        self.control_ui.error = Some(error);
                    }
                }
                match tempfile::tempdir() {
                    Ok(directory) => {
                        self.control_ui
                            .library
                            .set_folder(directory.path().to_owned());
                        self.control_ui.directory = Some(directory);
                    }
                    Err(error) => self.control_ui.error = Some(error.to_string()),
                }
                self.control_ui.target = 1;
                self.control_ui.demo_stage = 2;
            }
            2 if !self.control_ui.library.busy() => {
                if let Some(draft) = &self.control_ui.draft {
                    self.control_ui
                        .library
                        .start(Operation::Save(draft.document.clone(), draft.id));
                }
                self.control_ui.demo_stage = 3;
            }
            3 if !self.control_ui.library.busy() => {
                if self.control_ui.demo == 2 {
                    self.control_review();
                }
                if self.control_ui.demo == 4 {
                    self.control_ui.tab = Tab::Library;
                }
                self.control_ui.demo = 0;
                self.frames = 0;
            }
            _ => {}
        }
        if self.control_ui.error.is_some()
            || self.control_ui.library.error.is_some()
            || self.device.error.is_some()
        {
            self.control_ui.demo = 0;
            self.frames = 0;
        }
    }
}
