use super::Editor;
use crate::whole_restore_view::{self, Action, Preview, Row, Tab};
use eframe::egui;
use openfuse_core::auxiliary::WholeRestore;

impl Editor {
    fn whole_restore_valid(&self) -> bool {
        let Some(preview) = &self.auxiliary_ui.whole_preview else {
            return false;
        };
        !self.device.operation_busy()
            && self.device.matches_capture(&preview.binding)
            && self.auxiliary_ui.source_revision == preview.source_revision
            && self.auxiliary_ui.source.as_ref().is_some_and(|source| {
                source.snapshot.fingerprint() == preview.source_snapshot_fingerprint
            })
            && self
                .device
                .current_auxiliary()
                .is_some_and(|target| preview.plan.validate_destination(&target.snapshot).is_ok())
    }
    pub(super) fn whole_restore_dialog(&mut self, ctx: &egui::Context) {
        let valid = self.whole_restore_valid();
        let action = whole_restore_view::dialog(
            ctx,
            self.auxiliary_ui
                .whole_preview
                .as_mut()
                .expect("whole preview"),
            valid,
            self.auxiliary_ui.error.as_deref(),
        );
        match action {
            Some(Action::Apply) => self.apply_whole_restore(),
            Some(Action::Back) => {
                self.auxiliary_ui.whole_preview = None;
                self.auxiliary_ui.error = None;
            }
            None => {}
        }
    }
    pub(super) fn prepare_whole_restore(&mut self) {
        let result = (|| {
            if self.device.operation_busy() {
                return Err(
                    "Finish the amplifier operation before reviewing a whole-device restore"
                        .to_string(),
                );
            }
            if self.auxiliary_ui.whole_preparation.is_some() {
                return Err("Wait for the current archive validation to finish".to_string());
            }
            let source = self
                .auxiliary_ui
                .source
                .as_ref()
                .ok_or("Choose a complete source archive first")?
                .clone();
            let target = self
                .device
                .current_auxiliary()
                .ok_or("Read fresh destination system and control banks first")?
                .clone();
            let binding = self
                .device
                .current_binding()
                .ok_or("Read the amplifier first")?;
            let source_revision = self.auxiliary_ui.source_revision;
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("whole-restore-preview".into())
                .spawn(move || {
                    let _ = sender.send(build_preview(source, target, binding, source_revision));
                })
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(whole_restore_view::Preparation {
                receiver,
                cancelled: false,
            })
        })();
        match result {
            Ok(task) => {
                self.auxiliary_ui.preview = None;
                self.auxiliary_ui.whole_preview = None;
                self.auxiliary_ui.whole_preparation = Some(task);
                self.auxiliary_ui.error = None;
            }
            Err(error) => {
                self.auxiliary_ui.error = Some(format!("Whole-device restore unavailable: {error}"))
            }
        }
    }
    pub(super) fn whole_restore_update(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let Some(task) = &self.auxiliary_ui.whole_preparation else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Archive validation stopped unexpectedly".into())
            }
        };
        let cancelled = task.cancelled;
        self.auxiliary_ui.whole_preparation = None;
        if cancelled {
            return;
        }
        match result {
            Ok(preview) => {
                self.auxiliary_ui.whole_preview = Some(preview);
                if !self.whole_restore_valid() {
                    self.auxiliary_ui.whole_preview = None;
                    self.auxiliary_ui.error=Some("Source or destination changed during archive validation; read and review again".into());
                }
            }
            Err(error) => {
                self.auxiliary_ui.error =
                    Some(format!("Inspect-only for whole-device restore: {error}"))
            }
        }
    }
    pub(super) fn apply_whole_restore(&mut self) {
        let Some(preview) = &self.auxiliary_ui.whole_preview else {
            return;
        };
        if !preview.confirmed {
            self.auxiliary_ui.error = Some("Confirm all listed device overwrites first".into());
            return;
        }
        if !self.whole_restore_valid() {
            self.auxiliary_ui.error = Some(
                "Whole-device restore preview is stale; read the destination and review again"
                    .into(),
            );
            return;
        }
        let preview = self
            .auxiliary_ui
            .whole_preview
            .take()
            .expect("checked preview");
        self.disable_live();
        self.slot_preview_pending = false;
        self.auxiliary_ui.error = self
            .device
            .restore_whole(&preview.binding, preview.plan)
            .err();
    }
}

fn build_preview(
    source: crate::auxiliary_view::Source,
    target: openfuse_device::AuxiliarySnapshot,
    binding: openfuse_device::LiveBinding,
    source_revision: u64,
) -> Result<Preview, String> {
    let plan = if let Some(archive) = &source.archive {
        WholeRestore::from_archive(archive, source.snapshot.pid(), &target.snapshot)
    } else {
        WholeRestore::new(&source.snapshot, &target.snapshot)
    }
    .map_err(|error| error.to_string())?;
    let (mut presets, banks, system) = Preview::rows(&source.snapshot, &target.snapshot);
    if let Some(archive) = &source.archive {
        presets = archive
            .presets()
            .iter()
            .map(|slot| Row {
                label: format!("Slot {}", slot.slot),
                before: target
                    .snapshot
                    .presets()
                    .iter()
                    .find(|p| p.slot() == slot.slot)
                    .map_or_else(|| "Missing".into(), |p| p.name()),
                after: slot.document.metadata().name,
            })
            .collect();
    }
    let mut warnings = source.snapshot.warnings().to_vec();
    if let Some(archive) = &source.archive {
        for warning in archive.warnings() {
            if !warnings.contains(warning) {
                warnings.push(warning.clone());
            }
        }
    }
    for warning in target.snapshot.warnings() {
        if !warnings.contains(warning) {
            warnings.push(warning.clone());
        }
    }
    let normalizations = plan
        .normalizations()
        .iter()
        .map(|value| Row {
            label: format!(
                "{} · {} · byte {}",
                value.section,
                match value.kind {
                    5 => "Amplifier",
                    6 => "Stomp",
                    7 => "Modulation",
                    8 => "Delay",
                    9 => "Reverb",
                    10 => "Expression",
                    _ => "Report",
                },
                value.offset
            ),
            before: value.before.to_string(),
            after: value.after.to_string(),
        })
        .collect::<Vec<_>>();
    for warning in plan.warnings() {
        if !warnings.contains(warning) {
            warnings.push(warning.clone());
        }
    }
    let tab = if plan.has_system() {
        Tab::System
    } else {
        Tab::Banks
    };
    Ok::<_, String>(Preview {
        binding,
        plan,
        source_revision,
        source_snapshot_fingerprint: source.snapshot.fingerprint(),
        source_label: source.label.clone(),
        destination_label: target.device.name.clone(),
        presets,
        banks,
        system,
        normalizations,
        normalization_page: 0,
        warnings,
        confirmed: false,
        tab,
    })
}

#[cfg(test)]
#[path = "app_whole_restore_tests.rs"]
mod tests;
