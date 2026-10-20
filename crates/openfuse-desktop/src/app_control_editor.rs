use super::{Editor, Pending as MainPending};
use crate::{
    auxiliary_view::Preview,
    control_editor_state::{Draft, Operation},
    control_editor_view::{self, Action, CaptureDraft, Pending, Tab},
};
use eframe::egui;
use openfuse_core::{auxiliary::Key, control_preset::ControlPreset, protocol::Report64};
impl Editor {
    pub(super) fn open_control_editor(&mut self) {
        self.open_auxiliary();
        self.control_ui.open = true;
    }
    pub(super) fn control_dialog(&mut self, ctx: &egui::Context) {
        if let Some(action) = control_editor_view::dialog(
            ctx,
            &mut self.control_ui,
            &self.device,
            self.session.is_some(),
        ) {
            self.control_action(action, ctx);
        }
    }
    pub(super) fn control_replace(&mut self, document: ControlPreset, source: String, saved: bool) {
        self.control_ui.next_id = self.control_ui.next_id.wrapping_add(1);
        match Draft::new(document, source, saved, self.control_ui.next_id) {
            Ok(draft) => {
                if self.control_ui.dirty() {
                    self.control_ui.pending = Some(Pending::Replace(Box::new(draft)));
                } else {
                    self.control_ui.draft = Some(draft);
                    self.control_ui.selected.clear();
                }
                self.control_ui.tab = Tab::Editor;
                self.control_ui.error = None;
                self.open_control_editor();
            }
            Err(error) => self.control_ui.error = Some(error),
        }
    }
    pub(super) fn edit_auxiliary_entry(&mut self, key: Key) {
        let result = (|| {
            let source = self
                .auxiliary_ui
                .source
                .as_ref()
                .ok_or("Choose a bank snapshot first")?;
            let entry = source
                .snapshot
                .entries()
                .iter()
                .find(|entry| entry.key() == key)
                .ok_or("Bank entry is missing")?;
            let document = ControlPreset::from_entry(source.snapshot.pid(), entry)
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((document, entry.label()))
        })();
        match result {
            Ok((document, label)) => self.control_replace(document, label, false),
            Err(error) => self.auxiliary_ui.error = Some(error),
        }
    }
    pub(super) fn control_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Browse => self.control_ui.open = false,
            Action::Close => {
                self.control_ui.open = false;
                self.auxiliary_ui.open = false;
            }
            Action::New => {
                let ui = &self.control_ui;
                let document = if ui.clean {
                    ControlPreset::new_for_slot(ui.pid, ui.control, 16, &ui.name)
                } else {
                    ControlPreset::new(ui.pid, ui.control, &ui.name)
                };
                match document {
                    Ok(document) => {
                        self.control_replace(document, "New local bank preset".into(), false)
                    }
                    Err(error) => self.control_ui.error = Some(error.to_string()),
                }
            }
            Action::OpenFile | Action::Import => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Open FX/VOICE preset")
                    .add_filter("Bank presets", &["fuse", "xml", "json"])
                    .pick_file()
                {
                    let profile = path
                        .extension()
                        .is_some_and(|ext| !ext.eq_ignore_ascii_case("json"))
                        .then_some((self.control_ui.pid, self.control_ui.control));
                    self.control_ui
                        .library
                        .start(if matches!(action, Action::Import) {
                            Operation::Import(path, profile)
                        } else {
                            Operation::OpenFile(path, profile)
                        });
                }
            }
            Action::Export => {
                if let Some(draft) = &self.control_ui.draft
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title("Export bank preset to a new file")
                        .set_file_name(format!("{}.fuse", draft.document.name()))
                        .add_filter("FUSE XML", &["fuse"])
                        .add_filter("OpenFUSE bank preset", &["json"])
                        .save_file()
                {
                    self.control_ui.library.start(Operation::Export(
                        draft.document.clone(),
                        draft.id,
                        path,
                    ));
                }
            }
            Action::Save | Action::SavePending => {
                let pending = matches!(action, Action::SavePending);
                if self.control_ui.library.folder.is_none() {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("FX/VOICE library folder")
                        .pick_folder()
                    {
                        self.control_ui.library.set_folder(folder);
                        self.control_ui.save_after_scan = true;
                    } else {
                        return;
                    }
                } else if let Some(draft) = &self.control_ui.draft {
                    self.control_ui
                        .library
                        .start(Operation::Save(draft.document.clone(), draft.id));
                }
                self.control_ui.finish_after_save = pending;
            }
            Action::Undo => {
                if let Some(draft) = &mut self.control_ui.draft {
                    draft.undo();
                }
            }
            Action::Redo => {
                if let Some(draft) = &mut self.control_ui.draft {
                    draft.redo();
                }
            }
            Action::Current => self.control_capture(true),
            Action::Local => self.control_capture(false),
            Action::Capture => {
                let Some(capture) = self.control_ui.capture.take() else {
                    return;
                };
                let keys: Vec<_> = capture.selected.iter().map(String::as_str).collect();
                match ControlPreset::from_selected_preset(
                    capture.pid,
                    capture.control,
                    capture.clean.then_some(16),
                    &capture.preset,
                    &keys,
                ) {
                    Ok(document) => self.control_replace(document, capture.source, false),
                    Err(error) => self.control_ui.error = Some(error.to_string()),
                }
            }
            Action::CancelCapture => self.control_ui.capture = None,
            Action::Review => self.control_review(),
            Action::Read => self.auxiliary_action(crate::auxiliary_view::Action::Read),
            Action::ChooseFolder => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("FX/VOICE library folder")
                    .pick_folder()
                {
                    self.control_ui.library.set_folder(folder);
                }
            }
            Action::Refresh => self.control_ui.library.start(Operation::Scan),
            Action::Open(entry) => self.control_ui.library.start(Operation::Open(entry)),
            Action::Duplicate(entry) => self.control_ui.library.start(Operation::Duplicate(entry)),
            Action::Delete(entry) => self.control_ui.library.start(Operation::Delete(entry)),
            Action::Recover(entry) => self.control_ui.library.start(Operation::Recover(entry)),
            Action::Discard => self.control_finish_pending(ctx),
            Action::CancelPending => {
                self.control_ui.pending = None;
                self.control_ui.finish_after_save = false;
            }
            Action::CancelOperation => self.auxiliary_action(crate::auxiliary_view::Action::Cancel),
        }
    }
    fn control_capture(&mut self, current: bool) {
        let result = (|| {
            let ui = &self.control_ui;
            let (pid, preset, source) = if current {
                if self.device.operation_busy() || self.device.current_binding().is_none() {
                    return Err("Read a fresh amplifier sound first".into());
                }
                let capture = self
                    .device
                    .capture
                    .as_ref()
                    .ok_or("Read the amplifier first")?;
                let reports = capture
                    .reports
                    .iter()
                    .map(|report| Report64::try_from(report.as_slice()).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()?;
                ControlPreset::from_reports(capture.device.product_id, ui.control, &reports)
                    .map_err(|e| e.to_string())?;
                let decoded =
                    openfuse_core::wire_preset::decode(capture.device.product_id, &reports)
                        .map_err(|e| e.to_string())?;
                (
                    capture.device.product_id,
                    decoded.document,
                    format!(
                        "{} · current sound read {}",
                        capture.device.name, capture.generation
                    ),
                )
            } else {
                (
                    ui.pid,
                    self.session
                        .as_ref()
                        .ok_or("Open a full preset first")?
                        .document
                        .clone(),
                    "Copy of local full preset".into(),
                )
            };
            let projected =
                ControlPreset::from_preset(pid, ui.control, &preset).map_err(|e| e.to_string())?;
            let available = projected
                .modules()
                .into_iter()
                .filter(|module| !ui.clean || module.key != "Stompbox")
                .map(|module| module.key)
                .collect::<Vec<_>>();
            Ok::<_, String>(CaptureDraft {
                preset,
                pid,
                control: ui.control,
                clean: ui.clean,
                selected: available.iter().cloned().collect(),
                available,
                source,
            })
        })();
        match result {
            Ok(capture) => self.control_ui.capture = Some(capture),
            Err(error) => self.control_ui.error = Some(error),
        }
    }
    pub(super) fn control_review(&mut self) {
        let result = (|| {
            if self.device.operation_busy() {
                return Err("Finish the current amplifier operation first".into());
            }
            let draft = self
                .control_ui
                .draft
                .as_ref()
                .ok_or("Open a bank draft first")?;
            let destination = self
                .device
                .current_auxiliary()
                .ok_or("Read fresh destination banks first")?;
            let binding = self
                .device
                .current_binding()
                .ok_or("Read the amplifier first")?;
            let target = Key::Bank {
                control: draft.document.control(),
                slot: self.control_ui.target,
            };
            let target_label = destination
                .snapshot
                .entries()
                .iter()
                .find(|entry| entry.key() == target)
                .ok_or("Choose a compatible destination bank slot")?
                .label();
            let selection = draft
                .document
                .plan(&destination.snapshot, self.control_ui.target)
                .map_err(|e| e.to_string())?;
            Ok::<_,String>(Preview {binding,destination_fingerprint:destination.snapshot.fingerprint(),selection,source_label:format!("{} · local bank draft",draft.document.name()),destination_label:destination.device.name.clone(),rows:vec![(format!("{} · {}",control_editor_view::kind(draft.document.pid(),draft.document.control()),draft.document.name()),target_label)],warnings:vec!["Writes use the device's byte precision; Super Champ expression fallback follows the original protocol. Only the selected bank entry is replaced.".into()],confirmed:false})
        })();
        match result {
            Ok(preview) => {
                self.auxiliary_ui.preview = Some(preview);
                self.control_ui.error = None;
            }
            Err(error) => self.control_ui.error = Some(error),
        }
    }
    pub(super) fn control_finish_pending(&mut self, ctx: &egui::Context) {
        self.control_ui.finish_after_save = false;
        match self.control_ui.pending.take() {
            Some(Pending::Replace(draft)) => {
                self.control_ui.draft = Some(*draft);
                self.control_ui.selected.clear();
            }
            Some(Pending::CloseWindow) => {
                self.control_ui.open = false;
                self.auxiliary_ui.open = false;
                self.request(MainPending::Close, ctx);
            }
            None => {}
        }
    }
    pub(super) fn control_update(&mut self, ctx: &egui::Context) {
        self.control_ui.library.poll();
        if self.control_ui.save_after_scan && !self.control_ui.library.busy() {
            self.control_ui.save_after_scan = false;
            if let Some(draft) = &self.control_ui.draft {
                self.control_ui
                    .library
                    .start(Operation::Save(draft.document.clone(), draft.id));
            }
        }
        if let Some((document, label)) = self.control_ui.library.opened.take() {
            self.control_replace(document, label, true);
        }
        if let Some((id, xml)) = self.control_ui.library.saved.take()
            && let Some(draft) = &mut self.control_ui.draft
            && draft.id == id
        {
            draft.saved(xml);
            if self.control_ui.finish_after_save && !draft.dirty() {
                self.control_finish_pending(ctx);
            }
        }
        if self.control_ui.library.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        self.control_demo_update();
    }
}
#[cfg(test)]
#[path = "app_control_editor_tests.rs"]
mod tests;
