use super::Editor;
use crate::{
    bank_view::{self, Action, RestoreDraft, SaveDraft},
    theme,
};
use eframe::egui::{self, RichText};

impl Editor {
    pub(super) fn bank_panel(&mut self, ui: &mut egui::Ui) {
        if !self.device.has_connection() && self.device.bank_backup.is_none() {
            return;
        }
        egui::Frame::new().fill(theme::panel(ui.ctx())).corner_radius(10).inner_margin(14).show(ui,|ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                theme::eyebrow(ui,"STORED AMPLIFIER BANK");
                let r=ui.add_sized([180.0,40.0],egui::Button::new("Sounds & backups…"));
                crate::audition_view::remember(&r,"bank-open");
                if r.clicked() {self.bank_ui.open=true;}
            });
            let mut action=None;bank_view::progress(ui,&self.device,&mut action);
            if let Some(action)=action {self.bank_action(action);}
            ui.label(RichText::new("Browse slot names, save the amplifier’s working sound or preserve its complete bank.").size(12.0).color(theme::muted(ui.ctx())));
        });
        ui.add_space(12.0);
    }
    pub(super) fn bank_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_some()
            || self.new_dialog
            || self.archive_pending.is_some()
            || self.import_draft.is_some()
            || self.read_draft.is_some()
            || self.audition_draft.is_some()
            || self.archive_ui.modal_open()
            || self.auxiliary_ui.modal_open()
            || self.controls_ui.modal_open()
        {
            return;
        }
        let action = if let Some(draft) = &mut self.bank_ui.save {
            let valid =
                self.device.matches_capture(&draft.binding) && !self.device.operation_busy();
            bank_view::save_dialog(ctx, draft, valid)
        } else if let Some(draft) = &mut self.bank_ui.restore {
            let valid =
                self.device.matches_capture(&draft.binding) && !self.device.operation_busy();
            bank_view::restore_dialog(ctx, draft, valid)
        } else if self.bank_ui.open {
            bank_view::dialog(ctx, &self.device, &mut self.bank_ui)
        } else {
            None
        };
        if let Some(action) = action {
            self.bank_action(action);
        }
    }
    pub(super) fn bank_action(&mut self, action: Action) {
        match action {
            Action::Close => self.bank_ui.open = false,
            Action::Catalog => self.open_archives(),
            Action::BackupSelected => {
                if self.device.operation_busy() {
                    self.error = Some("Wait for the current amplifier operation".into());
                    return;
                }
                let Some(binding) = self
                    .device
                    .current_binding()
                    .filter(|_| self.device.current_bank_names().is_some())
                else {
                    self.error = Some("Read fresh bank names before selecting a backup".into());
                    return;
                };
                let slots = self.bank_ui.marked_slots.iter().copied().collect();
                self.disable_live();
                if let Err(error) = self.device.backup_slots(&binding, slots) {
                    self.error = Some(error);
                }
            }
            Action::Dismiss => {
                self.bank_ui.save = None;
                self.bank_ui.restore = None;
                self.bank_ui.open = true;
            }
            Action::Cancel => {
                if let Err(error) = self.device.cancel_operation() {
                    self.error = Some(error);
                }
            }
            Action::Select(slot) => {
                self.bank_ui.open = false;
                self.select_device_slot(slot);
            }
            Action::Save(slot) => self.prepare_bank_save(slot),
            Action::ReadNames | Action::Backup => {
                if self.device.operation_busy() {
                    self.error = Some("Wait for the current amplifier operation".into());
                    return;
                }
                let Some(binding) = self.device.current_binding() else {
                    self.error = Some("Read the connected amplifier first".into());
                    return;
                };
                self.disable_live();
                let result = if matches!(action, Action::ReadNames) {
                    self.device.read_bank(&binding)
                } else {
                    self.device.backup_bank(&binding)
                };
                if let Err(error) = result {
                    self.error = Some(error);
                } else {
                    self.error = None;
                }
            }
            Action::Export => self.export_bank(),
            Action::Import => self.import_bank(),
            Action::ApplySave => self.apply_bank_save(),
            Action::ApplyRestore => self.apply_bank_restore(),
        }
    }
    pub(super) fn prepare_bank_save(&mut self, slot: u16) {
        if self.device.operation_busy() {
            self.error = Some("Wait for the current amplifier operation".into());
            return;
        }
        let Some(names) = self.device.current_bank_names() else {
            self.error = Some("Refresh bank names before choosing an overwrite destination".into());
            return;
        };
        let Some(name) = names.entries.get(&slot) else {
            self.error = Some("The selected destination has no bank name".into());
            return;
        };
        let Some(capture) = &self.device.capture else {
            return;
        };
        match SaveDraft::prepare(capture, slot, name.clone()) {
            Ok(draft) => {
                self.bank_ui.save = Some(draft);
                self.bank_ui.open = false;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn apply_bank_save(&mut self) {
        let Some(draft) = self.bank_ui.save.as_ref() else {
            return;
        };
        if !draft.confirmed {
            self.error = Some("Confirm the slot overwrite before saving".into());
            return;
        }
        if !self.device.matches_capture(&draft.binding) || self.device.operation_busy() {
            self.error = Some("The save preview is stale. Read and preview again".into());
            return;
        }
        let draft = self.bank_ui.save.take().expect("confirmed save preview");
        self.disable_live();
        self.slot_preview_pending = false;
        match self.device.save_slot(&draft.binding, draft.slot) {
            Ok(()) => {
                self.bank_ui.open = true;
                self.status = format!(
                    "Saving amplifier sound to slot {}; local edits retained",
                    draft.slot
                );
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn prepare_bank_restore(
        &mut self,
        backup: openfuse_core::bank::Backup,
        path: std::path::PathBuf,
    ) {
        if self.device.operation_busy() {
            self.error = Some("Wait for the current amplifier operation".into());
            return;
        }
        let Some(capture) = self
            .device
            .capture
            .as_ref()
            .filter(|_| self.device.current_binding().is_some())
        else {
            self.error = Some("Read the connected amplifier before preparing a restore".into());
            return;
        };
        match RestoreDraft::prepare(capture, backup, path) {
            Ok(draft) => {
                self.bank_ui.restore = Some(draft);
                self.bank_ui.open = false;
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Cannot restore this archive: {error}")),
        }
    }
    pub(super) fn apply_bank_restore(&mut self) {
        let Some(draft) = self.bank_ui.restore.as_ref() else {
            return;
        };
        if !draft.confirmed {
            self.error = Some("Confirm the bank replacement before restoring".into());
            return;
        }
        if !self.device.matches_capture(&draft.binding) || self.device.operation_busy() {
            self.error = Some("The restore preview is stale. Read and preview again".into());
            return;
        }
        let draft = self
            .bank_ui
            .restore
            .take()
            .expect("confirmed restore preview");
        self.disable_live();
        self.slot_preview_pending = false;
        match self.device.restore_bank(&draft.binding, draft.backup) {
            Ok(()) => {
                self.bank_ui.open = true;
                self.status = "Restoring amplifier bank; local edits retained".into();
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn import_bank(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("OpenFUSE bank backup", &["json"])
            .pick_file()
        else {
            return;
        };
        match bank_view::read_backup(&path) {
            Ok(backup) => self.prepare_bank_restore(backup, path),
            Err(error) => self.error = Some(error),
        }
    }
    fn export_bank(&mut self) {
        let Some(backup) = &self.device.bank_backup else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("OpenFUSE bank backup", &["json"])
            .set_file_name(format!("openfuse-bank-{:04x}.json", backup.backup.pid()))
            .save_file()
        else {
            return;
        };
        match bank_view::export_backup(&path, &backup.backup) {
            Ok(()) => {
                self.status = format!("Bank backup saved: {}", path.display());
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn bank_demo_update(&mut self) {
        if !self.bank_ui.demo_pending || !self.device.simulated || self.usb_open_demo {
            return;
        }
        if self.error.is_some() || self.device.error.is_some() {
            self.bank_ui.demo_pending = false;
            self.frames = 0;
            return;
        }
        if !self.bank_ui.demo_started {
            if self.device.current_binding().is_some() {
                self.bank_ui.demo_started = true;
                self.bank_ui.open = true;
                self.device_dialog = false;
                self.bank_action(if self.bank_ui.demo_restore {
                    Action::Backup
                } else {
                    Action::ReadNames
                });
            }
            return;
        }
        if self.device.operation_busy() || self.device.current_binding().is_none() {
            return;
        }
        if matches!(
            self.device.bank.phase,
            openfuse_device::OperationPhase::Failed | openfuse_device::OperationPhase::Cancelled
        ) {
            self.bank_ui.demo_pending = false;
            self.frames = 0;
            return;
        }
        if self.bank_ui.demo_restore {
            if let Some(backup) = self.device.bank_backup.clone() {
                self.prepare_bank_restore(backup.backup, "simulated-bank.json".into());
                self.bank_ui.demo_pending = false;
                self.frames = 0;
            }
        } else if self.device.current_bank_names().is_some() {
            if self.bank_ui.demo_save {
                self.prepare_bank_save(self.device.selected_slot);
            }
            self.bank_ui.demo_pending = false;
            self.frames = 0;
        }
    }
}

#[cfg(test)]
#[path = "app_bank_tests.rs"]
mod tests;
