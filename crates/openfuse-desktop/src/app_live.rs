//! Only explicit parameter operations enter the live queue; XML/history is never replayed.
use super::Editor;
use crate::{session::DeviceEdit, theme};
use eframe::egui::{self, RichText};
use openfuse_core::wire_preset;

impl Editor {
    pub(super) fn disable_live(&mut self) {
        if self.live_requested {
            if let Some(origin) = &self.device_origin {
                let _ = self.device.set_live(&origin.binding, false);
            }
            self.live_requested = false;
        }
    }
    fn live_blocker(&self) -> Option<String> {
        let origin = self.device_origin.as_ref()?;
        if self.device.operation_busy() {
            return Some(
                "Wait for the operation readback and open it before enabling live parameters."
                    .into(),
            );
        }
        if let Some(reason) = &origin.invalidated {
            return Some(reason.clone());
        }
        if !self.device.matches_capture(&origin.binding) {
            return Some(
                "Read and open the connected amplifier’s preset again to enable live parameters."
                    .into(),
            );
        }
        if self
            .session
            .as_ref()
            .is_none_or(|s| s.document.modules() != origin.read.document.modules())
        {
            return Some("Local parameters differ from the read snapshot. Read and open again before enabling live parameters.".into());
        }
        None
    }
    pub(super) fn reconcile_live(&mut self) {
        if self.live_requested
            && (self.device.live.error.is_some()
                || self
                    .device_origin
                    .as_ref()
                    .is_none_or(|o| !self.device.matches_capture(&o.binding)))
        {
            self.disable_live();
        }
    }
    pub(super) fn enable_live(&mut self) {
        // Drain edits first: enabling can never transmit edits made while offline.
        self.process_device_edits();
        if let Some(reason) = self.live_blocker() {
            self.error = Some(reason);
            return;
        }
        let Some(origin) = &self.device_origin else {
            return;
        };
        match self.device.set_live(&origin.binding, true) {
            Ok(()) => self.live_requested = true,
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn process_device_edits(&mut self) {
        let edits = self
            .session
            .as_mut()
            .map(|s| s.take_device_edits())
            .unwrap_or_default();
        if edits
            .iter()
            .any(|edit| matches!(edit, DeviceEdit::Structure))
        {
            self.disable_live();
            if let Some(origin) = &mut self.device_origin {
                origin.invalidated = Some("The structure or history changed locally. Audition this sound and open its verified readback, or read the amplifier again, before enabling live parameters.".into());
            }
            return;
        }
        if !self.live_requested || edits.is_empty() {
            return;
        }
        self.reconcile_live();
        if !self.live_requested {
            return;
        }
        let (Some(session), Some(origin)) = (&self.session, &self.device_origin) else {
            return;
        };
        let mut changes = Vec::new();
        for edit in edits {
            if let DeviceEdit::Parameters { module, indices } = edit {
                for index in indices {
                    match wire_preset::parameter_change(
                        origin.read.capture.device.product_id,
                        &session.document,
                        &module,
                        index,
                    ) {
                        Ok(change) => changes.push(change),
                        Err(error) => {
                            self.error = Some(error.to_string());
                            self.disable_live();
                            return;
                        }
                    }
                }
            }
        }
        if let Err(error) = self.device.send_parameters(&origin.binding, changes) {
            self.error = Some(error);
            self.disable_live();
        }
    }
    pub(super) fn live_panel(&mut self, ui: &mut egui::Ui) {
        let Some(origin) = &self.device_origin else {
            return;
        };
        if self.device.operation.phase == openfuse_device::OperationPhase::Verified
            && self.device.operation.readback_generation != Some(origin.binding.generation)
        {
            return;
        }
        self.reconcile_live();
        let blocker = self.live_blocker();
        egui::Frame::new().fill(theme::panel(ui.ctx())).corner_radius(10).inner_margin(14).show(ui, |ui| {
            theme::eyebrow(ui, if self.device.simulated { "SIMULATOR · PARAMETER EDITING" } else { "AMPLIFIER PARAMETER EDITING" });
            ui.label("Live mode sends parameter adjustments to the amplifier’s temporary working preset. Save as keeps a local .FUSE file.");
            if self.live_requested {
                ui.label(RichText::new(format!("{} · {} queued · {} acknowledged", if self.device.live.enabled { "Live parameters enabled" } else { "Enabling live parameters…" }, self.device.live.pending, self.device.live.acknowledged)).color(theme::accent(ui.ctx())));
                if ui.button("Disable live parameters").clicked() { self.disable_live(); }
            } else {
                let enable = ui.add_enabled(blocker.is_none(), egui::Button::new("Enable live parameters"));
                #[cfg(test)] ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("live-enable"), enable.rect));
                if enable.clicked() { self.enable_live(); }
                if let Some(reason) = blocker { ui.label(RichText::new(reason).color(theme::muted(ui.ctx()))); }
            }
            if let Some(error) = &self.device.live.error { ui.colored_label(theme::danger(ui.ctx()), error); }
            if let Some(origin) = &self.device_origin {
                egui::CollapsingHeader::new("Read limitations and retained data").show(ui, |ui| {
                    for warning in &origin.read.warnings { ui.label(warning); }
                    ui.label(format!("{} original preset reports retained with this local copy. Snapshot generation {} remains the last read, not a live monitor.", origin.read.capture.reports.len(), origin.binding.generation));
                });
            }
        });
        ui.add_space(14.0);
    }
}
