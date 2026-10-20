use super::{Editor, Pending};
use eframe::egui;

impl Editor {
    pub(super) fn confirm(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        crate::dialog::show(ctx, "unsaved", 460.0, |ui| {
            ui.heading("Save your changes?");
            ui.label("This preset has unsaved edits.");
            ui.add_space(12.0);
            crate::dialog::footer(ui, |ui| {
                if ui.button("Save as…").clicked() && self.save_as() {
                    let action = self.pending.take().expect("pending action exists");
                    self.perform(action, ctx);
                }
                let discard = ui.button("Discard changes");
                #[cfg(test)]
                ctx.data_mut(|data| {
                    data.insert_temp(egui::Id::new("discard-button"), discard.rect)
                });
                if discard.clicked() {
                    let action = self.pending.take().expect("pending action exists");
                    self.perform(action, ctx);
                }
                let cancel = ui.button("Cancel");
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("cancel-button"), cancel.rect));
                if cancel.clicked() {
                    self.pending = None;
                }
            });
        });
    }

    pub(super) fn new_preset_dialog(&mut self, ctx: &egui::Context) {
        if !self.new_dialog {
            return;
        }
        crate::dialog::show(ctx, "new-preset", 460.0, |ui| {
            ui.heading("New preset");
            ui.label("Choose the amplifier family for this file.");
            egui::ComboBox::from_id_salt("new-product")
                .width(360.0)
                .selected_text(
                    openfuse_core::model_catalog::product(self.new_product)
                        .map(|p| p.name)
                        .unwrap_or("Select family"),
                )
                .show_ui(ui, |ui| {
                    for product in openfuse_core::model_catalog::products() {
                        ui.selectable_value(&mut self.new_product, product.id, product.name);
                    }
                });
            ui.label("Starts with the default amplifier and empty effect slots.");
            ui.add_space(12.0);
            crate::dialog::footer(ui, |ui| {
                if ui.button("Create preset").clicked() {
                    self.new_dialog = false;
                    self.request(Pending::New(self.new_product), ctx);
                }
                if ui.button("Cancel").clicked() {
                    self.new_dialog = false;
                }
            });
        });
    }

    pub(super) fn capture(&mut self, ctx: &egui::Context) {
        if self.screenshot.is_none() {
            return;
        }
        let ready = self.frames >= 20
            && !self.wait_for_document_open
            && !self.capture_requested
            && !self.library.busy()
            && (self.device.capture_ready()
                || (self.device.simulated
                    && self.device.tuner.enabled
                    && self.device.tuner.confirmed
                    && self.device.tuner.reading.is_some()))
            && !self.usb_audition_demo
            && !self.bank_ui.demo_pending
            && self.controls_ui.demo == 0
            && self.archive_ui.demo == 0
            && self.auxiliary_ui.demo == 0
            && self.auxiliary_ui.whole_preparation.is_none()
            && self.control_ui.demo == 0
            && self.media_ui.demo == 0
            && self.storage_ui.demo == 0
            && !self.storage_ui.busy()
            && !self.media_ui.library.busy()
            && self.media_ui.player.snapshot().phase != openfuse_audio::Phase::Loading
            && !self.control_ui.library.busy()
            && !self.archives.busy()
            && (!self.usb_audition_applied_demo
                || matches!(
                    self.device.operation.phase,
                    openfuse_device::OperationPhase::Verified
                        | openfuse_device::OperationPhase::Failed
                        | openfuse_device::OperationPhase::Cancelled
                )
                || self.error.is_some())
            && (!self.usb_live_demo
                || self.device.live.enabled
                || self.device.live.error.is_some()
                || self.error.is_some());
        if ready {
            let now = ctx.input(|i| i.time);
            let since = self.capture_ready_since.get_or_insert(now);
            // Capture settled opacity and layout after an asynchronous demo opens a modal.
            if now - *since >= 0.35 {
                self.capture_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
        } else {
            self.capture_ready_since = None;
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let path = self.screenshot.take().expect("capture path exists");
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            match image::save_buffer(
                &path,
                &rgba,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            ) {
                Ok(()) => {
                    println!("Screenshot saved: {}", path.display());
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(error) => self.error = Some(format!("Screenshot failed: {error}")),
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}
