use super::Editor;
use crate::{
    controls_view::{self, Action, SettingsDraft},
    theme,
};
use eframe::egui::{self, RichText};
impl Editor {
    pub(super) fn controls_panel(&mut self, ui: &mut egui::Ui) {
        if !self.device.has_connection() {
            return;
        }
        egui::Frame::new()
            .fill(theme::panel(ui.ctx()))
            .corner_radius(10)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    theme::eyebrow(ui, "AMPLIFIER CONTROLS");
                    if controls_view::button(ui, "controls-open", "Tuner & settings…", true) {
                        self.open_controls();
                    }
                    if controls_view::button(ui, "auxiliary-open", "System & control banks…", true)
                    {
                        self.open_auxiliary();
                    }
                    if self.device.tuner.enabled
                        || self.device.tuner.pending
                        || self.device.tuner.error.is_some()
                    {
                        ui.label(
                            RichText::new("Tuner active / pending").color(theme::accent(ui.ctx())),
                        );
                        if controls_view::button(ui, "controls-stop-inline", "Stop tuner", true) {
                            self.controls_action(Action::Tuner(false));
                        }
                    }
                });
            });
        ui.add_space(12.0);
    }
    pub(super) fn open_controls(&mut self) {
        self.controls_ui.open = true;
        self.controls_ui.preview = None;
        if let Some(capture) = self
            .device
            .capture
            .as_ref()
            .filter(|_| self.device.current_binding().is_some())
        {
            match SettingsDraft::prepare(capture) {
                Ok(draft) => self.controls_ui.draft = Some(draft),
                Err(error) => self.error = Some(error),
            }
        }
    }
    pub(super) fn controls_dialog(&mut self, ctx: &egui::Context) {
        if !self.controls_ui.modal_open()
            || self.pending.is_some()
            || self.new_dialog
            || self.archive_pending.is_some()
            || self.import_draft.is_some()
            || self.read_draft.is_some()
            || self.audition_draft.is_some()
            || self.archive_ui.modal_open()
            || self.auxiliary_ui.modal_open()
            || self.bank_ui.modal_open()
        {
            return;
        }
        let action = if let Some(preview) = &self.controls_ui.preview {
            controls_view::preview(
                ctx,
                preview,
                self.device.matches_capture(&preview.binding) && !self.device.operation_busy(),
            )
        } else {
            controls_view::dialog(ctx, &self.device, &mut self.controls_ui)
        };
        if let Some(action) = action {
            self.controls_action(action);
        }
    }
    pub(super) fn controls_action(&mut self, action: Action) {
        match action {
            Action::Close => {
                self.controls_ui.open = false;
                self.controls_ui.preview = None;
            }
            Action::Back => self.controls_ui.preview = None,
            Action::Reload => {
                if self.device.operation_busy() {
                    self.error=Some("Stop the tuner or wait for the amplifier operation before reloading settings".into());
                    return;
                }
                self.controls_ui.draft = None;
                self.controls_ui.preview = None;
                self.disable_live();
                self.device.read_current();
            }
            Action::Review => self.prepare_settings(),
            Action::Apply => self.apply_settings(),
            Action::Cancel => {
                if let Err(error) = self.device.cancel_operation() {
                    self.error = Some(error);
                }
            }
            Action::Tuner(enabled) => {
                let binding = if enabled {
                    if self.device.operation.busy() || self.device.bank.busy() {
                        self.error = Some("Wait for the current amplifier operation".into());
                        return;
                    }
                    self.device.current_binding().or_else(|| {
                        self.device
                            .tuner
                            .enabled
                            .then(|| {
                                self.device
                                    .capture
                                    .as_ref()
                                    .map(|c| openfuse_device::LiveBinding {
                                        device_id: c.device.id.clone(),
                                        generation: c.generation,
                                    })
                            })
                            .flatten()
                    })
                } else {
                    self.device
                        .capture
                        .as_ref()
                        .map(|c| openfuse_device::LiveBinding {
                            device_id: c.device.id.clone(),
                            generation: c.generation,
                        })
                };
                let Some(binding) = binding else {
                    self.error = Some("Read the connected amplifier before using the tuner".into());
                    return;
                };
                self.disable_live();
                match self
                    .device
                    .set_tuner(&binding, enabled, self.controls_ui.mute)
                {
                    Ok(()) => self.error = None,
                    Err(error) => self.error = Some(error),
                }
            }
        }
    }
    pub(super) fn prepare_settings(&mut self) {
        let Some(draft) = &self.controls_ui.draft else {
            return;
        };
        if !self.device.matches_capture(&draft.binding) || self.device.operation_busy() {
            self.error =
                Some("The settings read is stale or the amplifier is busy; reload settings".into());
            return;
        }
        match draft.preview() {
            Ok(preview) => {
                self.controls_ui.preview = Some(preview);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn apply_settings(&mut self) {
        let Some(preview) = &self.controls_ui.preview else {
            return;
        };
        if !self.device.matches_capture(&preview.binding) || self.device.operation_busy() {
            self.error = Some("The settings preview is stale; reload and review again".into());
            return;
        }
        let preview = self
            .controls_ui
            .preview
            .take()
            .expect("settings preview exists");
        self.disable_live();
        self.slot_preview_pending = false;
        match self.device.apply_settings(&preview.binding, preview.patch) {
            Ok(()) => {
                self.controls_ui.draft = None;
                self.status = "Applying amplifier settings and checking readback".into();
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn controls_update(&mut self) {
        if self.controls_ui.open
            && self.controls_ui.draft.is_none()
            && !self.device.operation_busy()
            && let Some(capture) = self
                .device
                .capture
                .as_ref()
                .filter(|_| self.device.current_binding().is_some())
        {
            match SettingsDraft::prepare(capture) {
                Ok(draft) => self.controls_ui.draft = Some(draft),
                Err(error) => self.error = Some(error),
            }
        }
        if self.controls_ui.demo == 0 || self.usb_open_demo || !self.device.simulated {
            return;
        }
        if !self.controls_ui.demo_started && self.device.current_binding().is_some() {
            self.controls_ui.demo_started = true;
            self.device_dialog = false;
            self.open_controls();
            if self.controls_ui.demo == 3 {
                self.controls_ui.mute = true;
                self.controls_action(Action::Tuner(true));
            } else if let Some(draft) = &mut self.controls_ui.draft {
                draft.patch.usb_gain = Some(192);
                draft.patch.global_volume = Some(openfuse_core::controls::VolumeEndpoints {
                    heel: 8192,
                    toe: 57344,
                });
                draft.patch.fx_loop_bypassed = Some(true);
                draft.patch.first_pedal_mode = Some(1);
                self.prepare_settings();
                if self.controls_ui.demo == 2 {
                    self.apply_settings();
                }
            }
        }
        let ready = match self.controls_ui.demo {
            1 => self.controls_ui.preview.is_some(),
            2 => {
                self.device.operation.phase == openfuse_device::OperationPhase::Verified
                    && self.device.current_binding().is_some()
            }
            3 => self.device.tuner.confirmed && self.device.tuner.reading.is_some(),
            _ => false,
        };
        if ready || self.error.is_some() || self.device.error.is_some() {
            self.controls_ui.demo = 0;
            self.frames = 0;
        }
    }
}
#[cfg(test)]
#[path = "app_controls_tests.rs"]
mod tests;
