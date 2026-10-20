use super::{Editor, Pending};
use crate::{device_read::ReadDraft, theme};
use eframe::egui::{self, RichText};

impl Editor {
    pub(super) fn choose_capture(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("USB capture", &["json"])
            .pick_file()
        else {
            return;
        };
        let result = (|| {
            use std::io::Read;
            if !std::fs::metadata(&path)
                .map_err(|e| e.to_string())?
                .is_file()
            {
                return Err("Capture must be a regular file".into());
            }
            let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("Capture must be a regular file".into());
            }
            let mut text = String::new();
            file.take(openfuse_device::MAX_CAPTURE_BYTES as u64 + 1)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            let capture = openfuse_device::parse_capture_json(&text)?;
            let mut draft = ReadDraft::decode(capture)?;
            draft.offline_file = true;
            Ok(draft)
        })();
        match result {
            Ok(draft) => self.read_draft = Some(draft),
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn export_capture(&mut self) {
        let Some(capture) = &self.device.capture else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("USB capture", &["json"])
            .set_file_name("openfuse-usb-capture.json")
            .save_file()
        else {
            return;
        };
        let result = (|| {
            use std::io::Write;
            let bytes = serde_json::to_vec_pretty(capture).map_err(|e| e.to_string())?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            let mut temporary =
                tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            temporary.write_all(&bytes).map_err(|e| e.to_string())?;
            temporary.as_file().sync_all().map_err(|e| e.to_string())?;
            temporary
                .persist_noclobber(&path)
                .map_err(|e| format!("Capture export did not overwrite an existing file: {e}"))?;
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => self.status = "Raw USB capture saved".into(),
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn prepare_device_read(&mut self) {
        let Some(capture) = self.device.capture.clone() else {
            return;
        };
        match ReadDraft::decode(capture) {
            Ok(draft) => {
                self.read_draft = Some(draft);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!("Cannot open device capture: {error}"));
                self.device.error = self.error.clone();
            }
        }
    }
    pub(super) fn read_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let Some(draft) = &self.read_draft else {
            return;
        };
        let mut open = false;
        let mut cancel = false;
        crate::dialog::show(ctx, "read-preview", 600.0, |ui| {
            theme::eyebrow(ui, "DEVICE SNAPSHOT · LOCAL PRESET");
            ui.heading("Open the read preset");
            ui.label(draft.source_label());
            ui.label(RichText::new(&draft.document.metadata().name).strong());
            ui.label("Creates an unsaved local copy. Parameter transmission requires a separate explicit action.");
            egui::ScrollArea::vertical()
                .max_height(280.0)
                .show(ui, |ui| {
                    for module in draft.document.modules() {
                        ui.label(format!(
                            "{} · model {}",
                            module.key,
                            module
                                .model_id
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| "unknown".into())
                        ));
                    }
                    for warning in &draft.warnings {
                        ui.label(RichText::new(warning).color(theme::muted(ui.ctx())));
                    }
                });
            ui.add_space(12.0);
            crate::dialog::footer(ui, |ui| {
                let response =
                    ui.add_sized([240.0, 42.0], egui::Button::new("Open as local preset"));
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("read-open"), response.rect));
                open = response.clicked();
                cancel = ui
                    .add_sized([110.0, 42.0], egui::Button::new("Cancel"))
                    .clicked();
            });
        });
        if open {
            let draft = self.read_draft.take().expect("read preview exists");
            self.device_dialog = false;
            self.request(Pending::OpenDevice(Box::new(draft)), ctx);
        } else if cancel {
            self.read_draft = None;
        }
    }
}

#[cfg(test)]
#[path = "app_device_read_tests.rs"]
mod tests;
