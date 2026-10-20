use super::Editor;
use crate::{
    audition_view::{self, AuditionDraft},
    theme,
};
use eframe::egui::{self, RichText};

impl Editor {
    pub(super) fn operation_update(&mut self) {
        use openfuse_device::OperationPhase;
        if self.operation_seen != self.device.operation {
            self.operation_seen = self.device.operation.clone();
            if self.device.operation.phase != OperationPhase::Idle {
                self.status = self.device.operation.detail.clone();
            }
        }
        if !self.slot_preview_pending {
            return;
        }
        match self.device.operation.phase {
            OperationPhase::Verified => {
                if self.pending.is_some()
                    || self.new_dialog
                    || self.archive_pending.is_some()
                    || self.import_draft.is_some()
                    || self.read_draft.is_some()
                    || self.audition_draft.is_some()
                    || (self.bank_ui.modal_open()
                        || (self.controls_ui.modal_open()
                            || self.archive_ui.modal_open()
                            || self.auxiliary_ui.modal_open()))
                {
                    return;
                }
                if self
                    .device
                    .operation
                    .readback_generation
                    .is_some_and(|generation| {
                        self.device
                            .capture
                            .as_ref()
                            .is_some_and(|capture| capture.generation == generation)
                    })
                {
                    self.slot_preview_pending = false;
                    self.prepare_device_read();
                }
            }
            OperationPhase::Failed | OperationPhase::Cancelled => self.slot_preview_pending = false,
            _ => {}
        }
    }

    pub(super) fn prepare_audition(&mut self) {
        if self.device.operation_busy() || self.device.current_binding().is_none() {
            self.error = Some("Wait for a complete current preset read before auditioning.".into());
            return;
        }
        let (Some(capture), Some(session)) = (&self.device.capture, &self.session) else {
            return;
        };
        match AuditionDraft::prepare(capture, &session.document) {
            Ok(draft) => {
                self.audition_draft = Some(draft);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Cannot audition this preset: {error}")),
        }
    }
    pub(super) fn apply_audition(&mut self) {
        let Some(draft) = self.audition_draft.take() else {
            return;
        };
        if !self.device.matches_capture(&draft.binding) || self.device.operation_busy() {
            self.error = Some(
                "The audition preview is stale. Read the amplifier and prepare a new preview."
                    .into(),
            );
            return;
        }
        self.disable_live();
        self.slot_preview_pending = false;
        self.operation_source = Some(draft.document.metadata().name);
        match self.device.apply_preset(&draft.binding, draft.document) {
            Ok(()) => {
                self.device_dialog = false;
                self.status = "Audition submitted; waiting for amplifier readback".into();
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn cancel_audition(&mut self) {
        self.audition_draft = None;
        self.slot_preview_pending = false;
        if let Err(error) = self.device.cancel_operation() {
            self.error = Some(error);
        }
    }
    pub(super) fn select_device_slot(&mut self, slot: u16) {
        if self.device.operation_busy() {
            self.error = Some("Wait for the current operation before selecting a slot.".into());
            return;
        }
        let Some(binding) = self.device.current_binding() else {
            self.error = Some("Read the connected amplifier before selecting a slot.".into());
            return;
        };
        self.audition_draft = None;
        self.disable_live();
        match self.device.select_slot(&binding, slot) {
            Ok(()) => {
                self.slot_preview_pending = true;
                self.operation_source = Some(format!("Amplifier slot index {slot}"));
                self.status =
                    format!("Selecting amplifier slot index {slot}; local edits retained");
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn audition_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_some()
            || self.read_draft.is_some()
            || (self.bank_ui.modal_open()
                || (self.controls_ui.modal_open()
                    || self.archive_ui.modal_open()
                    || self.auxiliary_ui.modal_open()))
        {
            return;
        }
        let Some(draft) = &self.audition_draft else {
            return;
        };
        let valid = self.device.matches_capture(&draft.binding) && !self.device.operation_busy();
        match audition_view::dialog(ctx, draft, valid) {
            Some(audition_view::Action::Apply) => self.apply_audition(),
            Some(audition_view::Action::Cancel) => self.audition_draft = None,
            None => {}
        }
    }
    pub(super) fn working_panel(&mut self, ui: &mut egui::Ui) {
        if !self.device.has_connection()
            && self.device.operation.phase == openfuse_device::OperationPhase::Idle
        {
            return;
        }
        let can_send = self.session.is_some()
            && self.device.current_binding().is_some()
            && !self.device.operation_busy();
        egui::Frame::new().fill(theme::panel(ui.ctx())).corner_radius(10).inner_margin(14).show(ui,|ui| {
            ui.set_min_width(ui.available_width());
            theme::eyebrow(ui,if self.device.simulated{"SIMULATOR · TEMPORARY PRESET"}else{"AMPLIFIER WORKING PRESET"});
            let audition=ui.add_enabled(can_send,egui::Button::new("Audition local preset…"));audition_view::remember(&audition,"audition-open");
            if audition.clicked(){self.prepare_audition();}
            if self.device.operation.phase != openfuse_device::OperationPhase::Idle {
                ui.label(RichText::new(format!("{} · {:?}",if self.device.operation.kind.is_empty(){"Working preset"}else{&self.device.operation.kind},self.device.operation.phase)).color(theme::accent(ui.ctx())));
                ui.label(&self.device.operation.detail);
                if let Some(source)=&self.operation_source {ui.label(RichText::new(format!("Operation source: {source}")).size(12.0).color(theme::muted(ui.ctx())));}
                if self.device.operation_busy() {
                    let total=self.device.operation.total;
                    if total>0 {ui.add(egui::ProgressBar::new(self.device.operation.sent as f32/total as f32).text(format!("{} / {} messages",self.device.operation.sent,total)));}
                    let cancel=ui.button("Cancel operation");audition_view::remember(&cancel,"operation-cancel");
                    if cancel.clicked(){self.cancel_audition();}
                }
                if let Some(generation)=self.device.operation.readback_generation {
                    let available=self.device.capture.as_ref().is_some_and(|c|c.generation==generation);
                    let open=ui.add_enabled(available,egui::Button::new("Open operation readback…"));audition_view::remember(&open,"operation-readback");
                    if open.clicked(){self.prepare_device_read();}
                    ui.label(RichText::new("Open the readback to bind live parameters to the amplifier’s updated sound.").size(12.0).color(theme::muted(ui.ctx())));
                }
            } else {ui.label(RichText::new("Preview the complete sound before sending models, bypass states and effect routing. Saving a file stays local.").size(12.0).color(theme::muted(ui.ctx())));}
        });
        ui.add_space(12.0);
    }
}

#[cfg(test)]
#[path = "app_audition_tests.rs"]
mod tests;
