use super::Editor;
use crate::{
    archive_state::Operation,
    auxiliary_view::{self, Action, Preview},
};
use eframe::egui;
use openfuse_core::auxiliary::Selection;

impl Editor {
    pub(super) fn open_auxiliary(&mut self) {
        self.control_ui.open = false;
        self.auxiliary_ui.open = true;
        self.archive_ui.open = false;
        self.bank_ui.open = false;
        self.controls_ui.open = false;
        self.device_dialog = false;
    }
    pub(super) fn open_archive_auxiliary(&mut self) {
        let result = (|| {
            let (entry, archive) = self
                .archives
                .loaded
                .as_ref()
                .ok_or("Inspect an archive first".to_string())?;
            let snapshot = if let Some(snapshot) = archive.auxiliary() {
                snapshot.clone()
            } else {
                let pid = self.archives.legacy_profile.ok_or("Choose the original amplifier profile explicitly before interpreting historical control banks".to_string())?;
                archive.auxiliary_for_pid(pid).map_err(|e| e.to_string())?
            };
            Ok::<_, String>((entry.label.clone(), snapshot, archive.clone()))
        })();
        match result {
            Ok((label, snapshot, archive)) => {
                self.auxiliary_ui.source(label, snapshot);
                self.auxiliary_ui
                    .source
                    .as_mut()
                    .expect("source set")
                    .archive = Some(archive);
                self.open_auxiliary();
            }
            Err(error) => self.archives.error = Some(error),
        }
    }
    pub(super) fn auxiliary_dialog(&mut self, ctx: &egui::Context) {
        if !self.auxiliary_ui.modal_open()
            || self.pending.is_some()
            || self.new_dialog
            || self.archive_pending.is_some()
            || self.import_draft.is_some()
            || self.read_draft.is_some()
            || self.audition_draft.is_some()
            || self.archive_ui.modal_open()
            || self.bank_ui.modal_open()
            || self.controls_ui.modal_open()
        {
            return;
        }
        if self.auxiliary_ui.whole_preview.is_some() {
            self.whole_restore_dialog(ctx);
            return;
        }
        if let Some(task) = &mut self.auxiliary_ui.whole_preparation {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            if !task.cancelled {
                if crate::whole_restore_view::preparing(ctx) {
                    task.cancelled = true;
                }
                return;
            }
        }
        if self.control_ui.open && self.auxiliary_ui.preview.is_none() {
            self.control_dialog(ctx);
            return;
        }
        let action = if let Some(draft) = &mut self.auxiliary_ui.preview {
            let valid = self.device.matches_capture(&draft.binding)
                && !self.device.operation_busy()
                && self.device.current_auxiliary().is_some_and(|snapshot| {
                    snapshot.snapshot.fingerprint() == draft.destination_fingerprint
                });
            auxiliary_view::preview(ctx, draft, valid)
        } else {
            auxiliary_view::dialog(ctx, &mut self.auxiliary_ui, &self.device, &self.archives)
        };
        if let Some(action) = action {
            self.auxiliary_action(action);
        }
    }
    pub(super) fn auxiliary_action(&mut self, action: Action) {
        match action {
            Action::Editor => self.open_control_editor(),
            Action::Edit(key) => self.edit_auxiliary_entry(key),
            Action::Close => self.auxiliary_ui.open = false,
            Action::Back => self.auxiliary_ui.preview = None,
            Action::Catalog => {
                self.auxiliary_ui.open = false;
                self.open_archives();
            }
            Action::Read => {
                let Some(binding) = self.device.current_binding() else {
                    self.auxiliary_ui.error = Some("Read the connected amplifier first".into());
                    return;
                };
                if self.device.operation_busy() {
                    self.auxiliary_ui.error =
                        Some("Finish the current amplifier operation first".into());
                    return;
                }
                self.disable_live();
                self.auxiliary_ui.error = self.device.read_auxiliary(&binding).err();
            }
            Action::UseRead => {
                if let Some(capture) = &self.device.auxiliary {
                    self.auxiliary_ui.source(
                        format!("{} · read {}", capture.device.name, capture.generation),
                        capture.snapshot.clone(),
                    );
                }
            }
            Action::SaveRead => {
                if let Some(capture) = &self.device.auxiliary {
                    let snapshot = capture.snapshot.clone();
                    if self.archives.folder.is_none() {
                        if let Some(folder) = rfd::FileDialog::new()
                            .set_title("Archive catalog folder")
                            .pick_folder()
                        {
                            self.archives.set_folder(folder);
                            self.auxiliary_ui.pending_archive = Some(snapshot);
                        }
                    } else if self.archives.busy() {
                        self.auxiliary_ui.error =
                            Some("Wait for the current archive operation".into());
                    } else {
                        self.archives.start(Operation::SaveAuxiliary(snapshot));
                    }
                }
            }
            Action::Review => self.prepare_auxiliary_restore(),
            Action::ReviewWhole => self.prepare_whole_restore(),
            Action::Apply => self.apply_auxiliary_restore(),
            Action::Cancel => self.auxiliary_ui.error = self.device.cancel_operation().err(),
        }
    }
    pub(super) fn prepare_auxiliary_restore(&mut self) {
        let result = (|| {
            if self.device.operation_busy() {
                return Err("Finish the amplifier operation before reviewing a restore".into());
            }
            let source = self
                .auxiliary_ui
                .source
                .as_ref()
                .ok_or("Choose a source snapshot first")?;
            let destination = self
                .device
                .current_auxiliary()
                .ok_or("Read fresh destination system and control banks first")?;
            let binding = self
                .device
                .current_binding()
                .ok_or("Read the amplifier first")?;
            let mappings = self.auxiliary_ui.mappings();
            let selection = Selection::new(&source.snapshot, &destination.snapshot, &mappings)
                .map_err(|e| e.to_string())?;
            let rows = mappings
                .iter()
                .map(|mapping| {
                    let source_entry = source
                        .snapshot
                        .entries()
                        .iter()
                        .find(|entry| entry.key() == mapping.source)
                        .ok_or("Source entry missing")?;
                    let target_entry = destination
                        .snapshot
                        .entries()
                        .iter()
                        .find(|entry| entry.key() == mapping.target)
                        .ok_or("Destination entry missing")?;
                    Ok((source_entry.label(), target_entry.label()))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut warnings = source.snapshot.warnings().to_vec();
            for warning in destination.snapshot.warnings() {
                if !warnings.contains(warning) {
                    warnings.push(warning.clone());
                }
            }
            Ok::<_, String>(Preview {
                binding,
                destination_fingerprint: destination.snapshot.fingerprint(),
                selection,
                source_label: source.label.clone(),
                destination_label: destination.device.name.clone(),
                rows,
                warnings,
                confirmed: false,
            })
        })();
        match result {
            Ok(preview) => {
                self.auxiliary_ui.preview = Some(preview);
                self.auxiliary_ui.error = None;
            }
            Err(error) => self.auxiliary_ui.error = Some(error),
        }
    }
    pub(super) fn apply_auxiliary_restore(&mut self) {
        let Some(preview) = &self.auxiliary_ui.preview else {
            return;
        };
        if !preview.confirmed {
            self.auxiliary_ui.error = Some("Confirm the selected entry overwrites first".into());
            return;
        }
        if self.device.operation_busy()
            || !self.device.matches_capture(&preview.binding)
            || !self.device.current_auxiliary().is_some_and(|snapshot| {
                snapshot.snapshot.fingerprint() == preview.destination_fingerprint
            })
        {
            self.auxiliary_ui.error =
                Some("Restore preview is stale; read destination banks and review again".into());
            return;
        }
        let preview = self.auxiliary_ui.preview.take().expect("checked preview");
        self.disable_live();
        self.slot_preview_pending = false;
        self.auxiliary_ui.error = self
            .device
            .restore_auxiliary(&preview.binding, preview.selection)
            .err();
    }
    pub(super) fn auxiliary_update(&mut self) {
        self.whole_restore_update();
        if !self.archives.busy()
            && let Some(snapshot) = self.auxiliary_ui.pending_archive.take()
        {
            self.archives.start(Operation::SaveAuxiliary(snapshot));
        }
        if self.auxiliary_ui.demo == 0 || self.usb_open_demo || !self.device.simulated {
            return;
        }
        match self.auxiliary_ui.demo_stage {
            0 if self.device.current_binding().is_some() => {
                self.open_auxiliary();
                self.auxiliary_action(Action::Read);
                self.auxiliary_ui.demo_stage = 1;
            }
            1 if self.device.current_auxiliary().is_some() && !self.device.operation_busy() => {
                self.auxiliary_action(Action::UseRead);
                if self.archives.folder.is_none() {
                    match tempfile::tempdir() {
                        Ok(folder) => {
                            self.archives.set_folder(folder.path().to_owned());
                            self.auxiliary_ui.demo_directory = Some(folder);
                        }
                        Err(error) => self.auxiliary_ui.error = Some(error.to_string()),
                    }
                }
                self.auxiliary_ui.pending_archive = self
                    .device
                    .auxiliary
                    .as_ref()
                    .map(|capture| capture.snapshot.clone());
                self.auxiliary_ui.demo_stage = 2;
            }
            2 if !self.archives.busy()
                && self.auxiliary_ui.pending_archive.is_none()
                && self
                    .archives
                    .loaded
                    .as_ref()
                    .is_some_and(|(_, archive)| archive.auxiliary().is_some()) =>
            {
                if matches!(self.auxiliary_ui.demo, 5 | 6) {
                    let source = &self.auxiliary_ui.source.as_ref().unwrap().snapshot;
                    let reports = source
                        .reports()
                        .iter()
                        .map(|report| {
                            let mut bytes = *report.as_bytes();
                            if bytes[0] == 0x1c && bytes[2] == 11 {
                                bytes[16] = 23;
                                bytes[32] = 77;
                            }
                            openfuse_core::protocol::Report64::from_bytes(bytes)
                        })
                        .collect::<Vec<_>>();
                    match openfuse_core::auxiliary::Snapshot::from_reports(source.pid(), &reports) {
                        Ok(snapshot) => self
                            .auxiliary_ui
                            .source("Preserved full-device snapshot".into(), snapshot),
                        Err(error) => self.auxiliary_ui.error = Some(error.to_string()),
                    }
                    self.prepare_whole_restore();
                    self.auxiliary_ui.demo_stage = 4;
                } else if matches!(self.auxiliary_ui.demo, 2 | 3) {
                    let source = &self.auxiliary_ui.source.as_ref().unwrap().snapshot;
                    let writable: Vec<_> = source
                        .entries()
                        .iter()
                        .filter(|entry| entry.restore_error().is_none())
                        .map(|entry| entry.key())
                        .collect();
                    if let Some(&first) = writable.first() {
                        self.auxiliary_ui.selected.insert(first);
                        if let Some(&target) = writable
                            .iter()
                            .find(|&&key| key != first && first.same_kind(key))
                        {
                            self.auxiliary_ui.targets.insert(first, target);
                        }
                    }
                    self.prepare_auxiliary_restore();
                    if self.auxiliary_ui.demo == 3
                        && let Some(preview) = &mut self.auxiliary_ui.preview
                    {
                        preview.confirmed = true;
                        self.apply_auxiliary_restore();
                        self.auxiliary_ui.demo_stage = 3;
                    } else {
                        self.auxiliary_ui.demo = 0;
                        self.frames = 0;
                    }
                } else {
                    self.auxiliary_ui.demo = 0;
                    self.frames = 0;
                }
            }
            3 if !self.device.operation_busy() => {
                self.auxiliary_ui.demo = 0;
                self.frames = 0;
            }
            4 if self.auxiliary_ui.whole_preparation.is_none() => {
                if self.auxiliary_ui.demo == 6 && self.auxiliary_ui.whole_preview.is_some() {
                    self.auxiliary_ui.whole_preview.as_mut().unwrap().confirmed = true;
                    self.apply_whole_restore();
                    self.auxiliary_ui.demo_stage = 3;
                } else {
                    self.auxiliary_ui.demo = 0;
                    self.frames = 0;
                }
            }
            _ => {}
        }
        if self.auxiliary_ui.error.is_some()
            || self.archives.error.is_some()
            || self.device.error.is_some()
        {
            self.auxiliary_ui.demo = 0;
            self.frames = 0;
        }
    }
}
#[cfg(test)]
#[path = "app_auxiliary_tests.rs"]
mod tests;
