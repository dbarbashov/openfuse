use super::{Editor, Pending};
use crate::{
    archive_state::Operation,
    archive_view::{self, Action, RestoreDraft, RestoreRow},
};
use eframe::egui;
use openfuse_core::{bank::RestoreSelection, protocol::Report64};
use openfuse_device::LiveBinding;
impl Editor {
    pub(super) fn open_archives(&mut self) {
        self.auxiliary_ui.open = false;
        self.archive_ui.open = true;
        self.bank_ui.open = false;
        self.controls_ui.open = false;
        self.device_dialog = false;
    }
    pub(super) fn archives_dialog(&mut self, ctx: &egui::Context) {
        if !self.archive_ui.modal_open()
            || self.auxiliary_ui.modal_open()
            || self.pending.is_some()
            || self.new_dialog
            || self.archive_pending.is_some()
            || self.import_draft.is_some()
            || self.read_draft.is_some()
            || self.audition_draft.is_some()
            || self.controls_ui.modal_open()
            || self.bank_ui.modal_open()
        {
            return;
        }
        let action = if let Some(draft) = &mut self.archive_ui.restore {
            archive_view::restore_preview(
                ctx,
                draft,
                self.device.matches_capture(&draft.binding) && !self.device.operation_busy(),
            )
        } else {
            archive_view::dialog(
                ctx,
                &mut self.archives,
                &self.device,
                self.library.folder.as_ref(),
            )
        };
        if let Some(action) = action {
            self.archive_action(action, ctx);
        }
    }
    pub(super) fn archive_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Auxiliary => self.open_archive_auxiliary(),
            Action::ControlPreset => {
                if let Some((entry, archive)) = &self.archives.loaded
                    && let Some(document) = archive.control_preset()
                {
                    self.control_replace(document.clone(), entry.label.clone(), true);
                }
            }
            Action::Close => self.archive_ui.open = false,
            Action::Dismiss => self.archive_ui.restore = None,
            Action::ChooseFolder => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Archive catalog folder")
                    .pick_folder()
                {
                    self.archives.set_folder(folder);
                }
            }
            Action::Refresh => self.archives.start(Operation::Scan),
            Action::ImportFile => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Import OpenFUSE backup or system/control snapshot")
                    .add_filter("OpenFUSE backup", &["json"])
                    .pick_file()
                {
                    self.archives.start(Operation::ImportFile(path));
                }
            }
            Action::ImportLegacy => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Import original FUSE backup folder")
                    .pick_folder()
                {
                    self.archives.start(Operation::ImportLegacy(path));
                }
            }
            Action::Load(entry) => self.archives.start(Operation::Load(entry)),
            Action::Delete(entry) => self.archives.start(Operation::Delete(entry)),
            Action::Recover(entry) => self.archives.start(Operation::Restore(entry)),
            Action::SaveCapture => {
                if let Some(capture) = &self.device.bank_backup {
                    self.archives
                        .start(Operation::SaveCapture(capture.backup.clone()));
                }
            }
            Action::Extract => {
                let Some((entry, _)) = &self.archives.loaded else {
                    return;
                };
                let entry = entry.clone();
                let slots = self.archives.marked.iter().copied().collect();
                let folder = self.library.folder.clone().or_else(|| {
                    rfd::FileDialog::new()
                        .set_title("Extract presets to library folder")
                        .pick_folder()
                });
                if let Some(folder) = folder {
                    if self.library.folder.as_ref() != Some(&folder) {
                        self.library.set_folder(folder.clone());
                    }
                    self.archives
                        .start(Operation::Extract(entry, slots, folder));
                }
            }
            Action::OpenPreset(slot) => {
                let Some((entry, archive)) = &self.archives.loaded else {
                    return;
                };
                if let Some(preset) = archive.presets().iter().find(|p| p.slot == slot) {
                    let document = preset.document.clone();
                    let label = format!("Archive: {} · slot {slot}", entry.label);
                    self.archive_ui.open = false;
                    self.request(Pending::OpenArchive(document, label), ctx);
                }
            }
            Action::ReadNames => {
                let Some(binding) = self.device.current_binding() else {
                    self.archives.error = Some("Read the connected amplifier first".into());
                    return;
                };
                if self.device.operation_busy() {
                    self.archives.error =
                        Some("Finish the current amplifier operation first".into());
                    return;
                }
                self.disable_live();
                if let Err(error) = self.device.read_bank(&binding) {
                    self.archives.error = Some(error);
                }
            }
            Action::Review => self.prepare_archive_restore(),
            Action::Apply => self.apply_archive_restore(),
            Action::Cancel => {
                if let Err(error) = self.device.cancel_operation() {
                    self.archives.error = Some(error);
                }
            }
        }
    }
    pub(super) fn prepare_archive_restore(&mut self) {
        let result = (|| -> Result<RestoreDraft, String> {
            if self.device.operation_busy() {
                return Err("Finish the amplifier operation before reviewing a restore".into());
            }
            let (entry, archive) = self
                .archives
                .loaded
                .as_ref()
                .ok_or("Inspect an archive first")?;
            let names = self
                .device
                .current_bank_names()
                .ok_or("Read fresh target slot names before restore")?;
            let capture = self
                .device
                .capture
                .as_ref()
                .ok_or("Read the amplifier first")?;
            let mapping = archive_view::mappings(&self.archives);
            let selection = if archive.native().is_some() || archive.auxiliary().is_some() {
                archive
                    .restore_selection(capture.device.product_id, &mapping)
                    .map_err(|e| e.to_string())?
            } else {
                let source_pid = self.archives.legacy_profile.ok_or(
                    "Choose the original FUSE amplifier profile explicitly before restoring",
                )?;
                RestoreSelection::from_documents(
                    source_pid,
                    archive
                        .presets()
                        .iter()
                        .map(|p| (p.slot, p.document.clone()))
                        .collect(),
                    capture.device.product_id,
                    &mapping,
                )
                .map_err(|e| e.to_string())?
            };
            let reports = capture
                .reports
                .iter()
                .map(|r| Report64::try_from(r.as_slice()).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            let plans = selection.plans(&reports).map_err(|e| e.to_string())?;
            let mut warnings = archive.warnings().to_vec();
            for plan in plans {
                for warning in plan.warnings() {
                    if !warnings.contains(warning) {
                        warnings.push(warning.clone());
                    }
                }
            }
            let rows = selection
                .entries()
                .iter()
                .map(|entry| {
                    Ok(RestoreRow {
                        source: entry.source(),
                        target: entry.target(),
                        source_name: entry.document().metadata().name,
                        target_name: names
                            .entries
                            .get(&entry.target())
                            .cloned()
                            .ok_or("Destination name is missing")?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(RestoreDraft {
                binding: LiveBinding {
                    device_id: capture.device.id.clone(),
                    generation: capture.generation,
                },
                device: capture.device.clone(),
                selection,
                rows,
                warnings,
                confirmed: false,
                archive_label: entry.label.clone(),
            })
        })();
        match result {
            Ok(draft) => {
                self.archive_ui.restore = Some(draft);
                self.archives.error = None;
            }
            Err(error) => self.archives.error = Some(error),
        }
    }
    pub(super) fn apply_archive_restore(&mut self) {
        let Some(draft) = &self.archive_ui.restore else {
            return;
        };
        if !draft.confirmed {
            self.archives.error = Some("Confirm the listed destination overwrites first".into());
            return;
        }
        if !self.device.matches_capture(&draft.binding) || self.device.operation_busy() {
            self.archives.error =
                Some("Restore preview is stale; read target names and review again".into());
            return;
        }
        let draft = self
            .archive_ui
            .restore
            .take()
            .expect("validated restore preview");
        self.disable_live();
        self.slot_preview_pending = false;
        match self.device.restore_slots(&draft.binding, draft.selection) {
            Ok(()) => {
                self.archives.error = None;
                self.status = "Restoring mapped slots with per-slot readback verification".into();
            }
            Err(error) => self.archives.error = Some(error),
        }
    }
    pub(super) fn archives_update(&mut self, ctx: &egui::Context) {
        self.archives.poll();
        if std::mem::take(&mut self.archives.library_refresh) {
            self.library.refresh();
        }
        if self.archives.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.archive_ui.demo == 0 || self.usb_open_demo || !self.device.simulated {
            return;
        }
        match self.archive_ui.demo_stage {
            0 => {
                if self.device.current_binding().is_some() {
                    if self.archives.folder.is_none() {
                        match tempfile::tempdir() {
                            Ok(folder) => {
                                self.archives.set_folder(folder.path().to_owned());
                                self.archive_ui.demo_directory = Some(folder);
                            }
                            Err(error) => {
                                self.error = Some(error.to_string());
                                return;
                            }
                        }
                    }
                    self.open_archives();
                    self.archive_action(Action::ReadNames, ctx);
                    self.archive_ui.demo_stage = 1;
                }
            }
            1 => {
                if self.device.current_bank_names().is_some() && !self.device.operation_busy() {
                    let binding = self.device.current_binding().expect("fresh names");
                    if let Err(error) = self.device.backup_slots(&binding, vec![1, 7, 20]) {
                        self.error = Some(error);
                    } else {
                        self.archive_ui.demo_stage = 2;
                    }
                }
            }
            2 => {
                if self
                    .device
                    .bank_backup
                    .as_ref()
                    .is_some_and(|b| b.backup.slots().len() == 3)
                    && self.device.current_binding().is_some()
                    && !self.archives.busy()
                {
                    self.archive_action(Action::SaveCapture, ctx);
                    self.archive_ui.demo_stage = 3;
                }
            }
            3 => {
                if self.archives.loaded.is_some() && !self.archives.busy() {
                    self.archives.marked.extend([1, 7]);
                    self.archives.targets.insert(1, 3);
                    self.archives.targets.insert(7, 12);
                    self.archives.inspected = Some(7);
                    if self.archive_ui.demo == 3 {
                        let source = self
                            .archives
                            .folder
                            .as_ref()
                            .unwrap()
                            .join("2026_10_13_11_30_00");
                        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                            std::fs::create_dir_all(source.join("Presets"))?;
                            let archive = &self.archives.loaded.as_ref().unwrap().1;
                            let xml = archive.presets()[0].document.to_xml()?;
                            std::fs::write(source.join("Presets/01_Rhythm.fuse"), xml)?;
                            std::fs::write(
                                source.join("M2_BackupName.fuse"),
                                "Rehearsal collection · independent demonstration fixture",
                            )?;
                            std::fs::write(
                                source.join("SystemSettings.fuse"),
                                "<System><Unknown>retained</Unknown></System>",
                            )?;
                            std::fs::write(
                                source.join("notes.txt"),
                                "Original unknown bytes retained for preservation",
                            )?;
                            Ok(())
                        })();
                        match result {
                            Ok(()) => self.archives.start(Operation::ImportLegacy(source)),
                            Err(error) => self.error = Some(error.to_string()),
                        }
                        self.archive_ui.demo_stage = 5;
                    } else if self.archive_ui.demo == 2 {
                        self.archive_action(Action::ReadNames, ctx);
                        self.archive_ui.demo_stage = 4;
                    } else {
                        self.archive_ui.demo = 0;
                        self.frames = 0;
                    }
                }
            }
            4 => {
                if self.device.current_bank_names().is_some() && !self.device.operation_busy() {
                    self.prepare_archive_restore();
                    self.archive_ui.demo = 0;
                    self.frames = 0;
                }
            }
            5 if !self.archives.busy()
                && self
                    .archives
                    .loaded
                    .as_ref()
                    .is_some_and(|(_, archive)| archive.legacy().is_some()) =>
            {
                self.archives.inspected = Some(1);
                self.archive_ui.demo = 0;
                self.frames = 0;
            }
            _ => {}
        }
        if self.error.is_some() || self.archives.error.is_some() || self.device.error.is_some() {
            self.archive_ui.demo = 0;
            self.frames = 0;
        }
    }
}
#[cfg(test)]
#[path = "app_archives_tests.rs"]
mod tests;
