//! Module import preview. Source XML remains a fixed snapshot until confirmation.
use crate::theme;
use eframe::egui::{self, RichText};
use openfuse_core::{
    model_catalog::{self, ModuleCategory},
    preset::{PresetDocument, module_import::ModuleImportReport},
};
use std::{collections::BTreeSet, path::PathBuf};

pub struct ImportDraft {
    pub source: PresetDocument,
    pub path: PathBuf,
    pub selected: BTreeSet<String>,
    pub error: Option<String>,
}
impl ImportDraft {
    pub fn new(
        source: PresetDocument,
        path: PathBuf,
        destination: &PresetDocument,
        current_key: &str,
    ) -> Self {
        let selected = destination
            .preview_module_import(&source)
            .into_iter()
            .filter(|option| option.compatible && option.key == current_key)
            .map(|option| option.key)
            .collect();
        Self {
            source,
            path,
            selected,
            error: None,
        }
    }
}
pub enum Action {
    Apply,
    Cancel,
}

pub fn show(
    ctx: &egui::Context,
    draft: &mut ImportDraft,
    destination: &PresetDocument,
) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "module-import", 630.0, |ui| {
        ui.heading("Import modules");
        ui.label(
            RichText::new(draft.path.display().to_string())
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
        ui.label(format!(
            "{}  to  {}",
            product_name(&draft.source),
            product_name(destination)
        ));
        ui.label("Choose the amplifier or effects to bring into your current preset.");
        ui.add_space(10.0);
        let options = destination.preview_module_import(&draft.source);
        let common_notes: BTreeSet<_> = options
            .first()
            .into_iter()
            .flat_map(|option| &option.notes)
            .filter(|note| {
                options
                    .iter()
                    .filter(|option| option.compatible)
                    .all(|option| option.notes.contains(note))
            })
            .cloned()
            .collect();
        for note in &common_notes {
            ui.label(RichText::new(note).size(12.0).color(theme::muted(ui.ctx())));
        }
        let source_modules = draft.source.modules();
        egui::ScrollArea::vertical()
            .id_salt("module-import-preview")
            .max_height(410.0)
            .show(ui, |ui| {
                for option in options {
                    ui.push_id(&option.key, |ui| {
                        let source = source_modules
                            .iter()
                            .find(|module| module.key == option.key);
                        let model_name = source
                            .and_then(|module| {
                                model_catalog::model(
                                    draft.source.product_id()?,
                                    ModuleCategory::from_key(&option.key)?,
                                    module.model_id?,
                                )
                            })
                            .map(|model| model.name.clone())
                            .unwrap_or_else(|| {
                                source
                                    .and_then(|module| module.model_id)
                                    .map(|id| format!("Model {id}"))
                                    .unwrap_or_else(|| "Not present in source".into())
                            });
                        let mut selected = draft.selected.contains(&option.key);
                        let response = ui.add_enabled(
                            option.compatible,
                            egui::Checkbox::new(
                                &mut selected,
                                format!("{} · {model_name}", option.key),
                            ),
                        );
                        remember(&response, &option.key);
                        if response.changed() {
                            if selected {
                                draft.selected.insert(option.key.clone());
                            } else {
                                draft.selected.remove(&option.key);
                            }
                        }
                        if let Some(reason) = &option.reason {
                            ui.label(
                                RichText::new(reason)
                                    .size(12.0)
                                    .color(theme::accent(ui.ctx())),
                            );
                        }
                        for note in option
                            .notes
                            .iter()
                            .filter(|note| !common_notes.contains(*note))
                        {
                            ui.label(RichText::new(note).size(12.0).color(theme::muted(ui.ctx())));
                        }
                        ui.add_space(6.0);
                    });
                }
            });
        if let Some(error) = &draft.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        }
        ui.add_space(12.0);
        crate::dialog::footer(ui, |ui| {
            let apply = ui.add_enabled(
                !draft.selected.is_empty(),
                egui::Button::new(format!("Import {} selected", draft.selected.len()))
                    .fill(egui::Color32::from_rgb(99, 77, 40)),
            );
            remember(&apply, "apply");
            if apply.clicked() {
                action = Some(Action::Apply);
            }
            let cancel = ui.button("Cancel");
            remember(&cancel, "cancel");
            if cancel.clicked() {
                action = Some(Action::Cancel);
            }
        });
    });
    action
}
fn product_name(document: &PresetDocument) -> String {
    document
        .product_id()
        .and_then(model_catalog::product)
        .map(|product| product.name.to_owned())
        .unwrap_or_else(|| {
            format!(
                "Product {}",
                document
                    .product_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "unspecified".into())
            )
        })
}
fn remember(response: &egui::Response, key: &str) {
    #[cfg(test)]
    response.ctx.data_mut(|data| {
        data.insert_temp(egui::Id::new(("module-import-control", key)), response.rect)
    });
    #[cfg(not(test))]
    let _ = (response, key);
}

pub struct ImportReceipt {
    pub source: PathBuf,
    pub report: ModuleImportReport,
}
/// Kept until a different document is opened or another import succeeds.
pub fn receipt(ui: &mut egui::Ui, receipt: &ImportReceipt) {
    egui::CollapsingHeader::new("Last module import · changes and omissions")
        .id_salt("last-module-import-report")
        .default_open(true)
        .show(ui, |ui| {
            ui.label(
                RichText::new(receipt.source.display().to_string()).color(theme::muted(ui.ctx())),
            );
            ui.label(format!("Imported: {}", receipt.report.imported.join(", ")));
            for (title, items) in [
                ("Changes", &receipt.report.changes),
                ("Not carried over", &receipt.report.losses),
            ] {
                if !items.is_empty() {
                    ui.strong(title);
                    for item in items {
                        ui.label(format!("• {item}"));
                    }
                }
            }
            ui.label(
                RichText::new(
                    "This records the import operation. Undo may have restored earlier settings.",
                )
                .size(12.0)
                .color(theme::muted(ui.ctx())),
            );
        });
    ui.add_space(12.0);
}
