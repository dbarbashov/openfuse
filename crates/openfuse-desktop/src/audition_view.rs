//! Explicit, immutable preview of a temporary amplifier operation.
use crate::theme;
use eframe::egui::{self, RichText};
use openfuse_core::{preset::PresetDocument, protocol::Report64};
use openfuse_device::{DeviceInfo, LiveBinding, WireCapture};

pub struct AuditionDraft {
    pub binding: LiveBinding,
    pub device: DeviceInfo,
    pub document: PresetDocument,
    pub warnings: Vec<String>,
    pub reports: usize,
}
impl AuditionDraft {
    pub fn prepare(capture: &WireCapture, document: &PresetDocument) -> Result<Self, String> {
        let current = capture
            .reports
            .iter()
            .map(|report| Report64::try_from(report.as_slice()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let plan = openfuse_core::audition::preset(capture.device.product_id, document, &current)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            binding: LiveBinding {
                device_id: capture.device.id.clone(),
                generation: capture.generation,
            },
            device: capture.device.clone(),
            document: document.clone(),
            warnings: plan.warnings().to_vec(),
            reports: plan.steps().len(),
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Apply,
    Cancel,
}
pub fn dialog(ctx: &egui::Context, draft: &AuditionDraft, valid: bool) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "audition-preview", 650.0, |ui| {
        theme::eyebrow(ui, "TEMPORARY AMPLIFIER SOUND");
        crate::dialog::header(
            ui,
            "Audition this sound",
            "Replaces the working sound. Stored slots are not overwritten.",
        );
        egui::ScrollArea::vertical().id_salt("audition-preview-scroll")
            .max_height((ctx.screen_rect().height() - 310.0).clamp(180.0, 540.0))
            .show(ui, |ui| {
                crate::dialog::section(ui, &draft.document.metadata().name, |ui| {
                    crate::dialog::hint(ui, &draft.device.name);
                    ui.add_space(6.0);
                    egui::Grid::new("audition-modules").num_columns(3).min_col_width(110.0).max_col_width(300.0).min_row_height(20.0).spacing([20.0, 8.0]).show(ui, |ui| {
                        for module in draft.document.modules() {
                            let name = module.model_id.and_then(|id| openfuse_core::model_catalog::ModuleCategory::from_key(&module.key).and_then(|category| draft.document.product_id().and_then(|pid| openfuse_core::model_catalog::model(pid, category, id)))).map(|m| m.name.as_str()).unwrap_or("Unknown model");
                            ui.label(RichText::new(&module.key).color(theme::muted(ui.ctx())));
                            ui.label(name);
                            ui.label(if module.active == Some(true) { "On" } else { "Bypassed" });
                            ui.end_row();
                        }
                    });
                });
                crate::dialog::review_notes(ui, &draft.warnings);
                egui::CollapsingHeader::new("Transfer details").show(ui, |ui| {
                    ui.label(format!("Read {} · {} messages", draft.binding.generation, draft.reports));
                    for module in draft.document.modules() {
                        ui.label(format!("{} · position {}", module.key, module.position.map(|v|v.to_string()).unwrap_or_else(||"unspecified".into())));
                    }
                    crate::dialog::hint(ui, "Models, bypass and routing are sent together. The amplifier is read again after transmission. Open that readback explicitly before enabling live parameters. Your local document and undo history stay here.");
                });
            });
        if !valid {
            ui.colored_label(theme::danger(ui.ctx()), "The connected device or read changed. Close this preview and prepare a new audition.");
        }
        crate::dialog::footer(ui, |ui| {
            let cancel = ui.add_sized([100.0, 40.0], egui::Button::new("Cancel"));
            remember(&cancel, "audition-cancel");
            if cancel.clicked() {
                action = Some(Action::Cancel);
            }
            let apply = crate::dialog::primary(ui, "Apply to working preset", valid);
            remember(&apply, "audition-apply");
            if apply.clicked() {
                action = Some(Action::Apply);
            }
        });
    });
    action
}

pub fn remember(response: &egui::Response, name: &str) {
    #[cfg(test)]
    response.ctx.data_mut(|data| {
        data.insert_temp(egui::Id::new(name), (response.rect, response.enabled()))
    });
    #[cfg(not(test))]
    let _ = (response, name);
}
