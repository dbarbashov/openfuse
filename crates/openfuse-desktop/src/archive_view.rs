//! Archive browser and explicit source-to-target restore review.
use crate::{
    archive_state::{self, ArchiveState},
    audition_view::remember,
    controls_view::button,
    device_state::DeviceState,
    theme,
};
use eframe::egui::{self, RichText};
use openfuse_core::{
    archive::CatalogEntry,
    bank::{RestoreSelection, SlotMapping},
};
use openfuse_device::{DeviceInfo, LiveBinding};
use std::path::PathBuf;
#[derive(Default)]
pub struct ArchiveUi {
    pub open: bool,
    pub restore: Option<RestoreDraft>,
    pub demo: u8,
    pub demo_stage: u8,
    pub demo_directory: Option<tempfile::TempDir>,
}
impl ArchiveUi {
    pub fn modal_open(&self) -> bool {
        self.open || self.restore.is_some()
    }
}
pub struct RestoreDraft {
    pub binding: LiveBinding,
    pub device: DeviceInfo,
    pub selection: RestoreSelection,
    pub rows: Vec<RestoreRow>,
    pub warnings: Vec<String>,
    pub confirmed: bool,
    pub archive_label: String,
}
pub struct RestoreRow {
    pub source: u16,
    pub source_name: String,
    pub target: u16,
    pub target_name: String,
}
#[derive(Clone)]
pub enum Action {
    Close,
    Auxiliary,
    ControlPreset,
    ChooseFolder,
    Refresh,
    ImportFile,
    ImportLegacy,
    Load(CatalogEntry),
    Delete(CatalogEntry),
    Recover(CatalogEntry),
    SaveCapture,
    Extract,
    OpenPreset(u16),
    ReadNames,
    Review,
    Apply,
    Dismiss,
    Cancel,
}
pub fn mappings(state: &ArchiveState) -> Vec<SlotMapping> {
    state
        .marked
        .iter()
        .map(|&source| SlotMapping {
            source,
            target: state.targets.get(&source).copied().unwrap_or(source),
        })
        .collect()
}
pub fn dialog(
    ctx: &egui::Context,
    state: &mut ArchiveState,
    device: &DeviceState,
    library: Option<&PathBuf>,
) -> Option<Action> {
    let mut action = None;
    let idle = !state.busy();
    crate::dialog::show(ctx, "archive-catalog", 1050.0, |ui| {
        theme::eyebrow(ui, "LOCAL ARCHIVE CATALOG");
        crate::dialog::header(
            ui,
            "Backup archives",
            "Browse your collection, extract presets or review a restore.",
        );
        ui.label(state.folder.as_ref().map_or_else(
            || "Choose a folder to organize backups and imported FUSE archives.".into(),
            |p| p.display().to_string(),
        ));
        ui.horizontal_wrapped(|ui| {
            if button(ui, "archives-folder", "Catalog folder…", idle) {
                action = Some(Action::ChooseFolder);
            }
            if button(
                ui,
                "archives-refresh",
                "Refresh",
                idle && state.folder.is_some(),
            ) {
                action = Some(Action::Refresh);
            }
            if button(
                ui,
                "archives-import-json",
                "Import JSON…",
                idle && state.folder.is_some(),
            ) {
                action = Some(Action::ImportFile);
            }
            if button(
                ui,
                "archives-import-fuse",
                "Import FUSE folder…",
                idle && state.folder.is_some(),
            ) {
                action = Some(Action::ImportLegacy);
            }
            if button(
                ui,
                "archives-save-capture",
                "Save latest capture",
                idle && state.folder.is_some() && device.bank_backup.is_some(),
            ) {
                action = Some(Action::SaveCapture);
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text("Search archive, model or preset name…")
                    .desired_width(400.0),
            );
            ui.checkbox(&mut state.show_deleted, "Deleted archives");
        });
        if state.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading archive files…");
            });
        }
        let height = (ctx.screen_rect().height() - 440.0).clamp(150.0, 500.0);
        ui.columns(2,|columns| {
            egui::ScrollArea::vertical().id_salt("archive-list").max_height(height).show(&mut columns[0],|ui| {
                let rows:Vec<_>=state.rows().into_iter().cloned().collect();
                if rows.is_empty(){ui.label(if state.show_deleted{"No deleted archives match this search."}else{"No archives match this search. Import a backup or save a device capture."});}
                for entry in rows {
                    let selected=state.loaded.as_ref().is_some_and(|(old,_)|old.id==entry.id);
                    egui::Frame::new().fill(if selected {theme::card(ui.ctx())}else{theme::panel(ui.ctx())}).corner_radius(9).inner_margin(12).show(ui,|ui| {
                        ui.set_min_width(ui.available_width());ui.label(RichText::new(&entry.label).strong());
                        ui.label(RichText::new(format!("{:?} · {} · {}",entry.kind, if entry.kind == openfuse_core::archive::ArchiveKind::Auxiliary { format!("System/control snapshot · {} presets", entry.preset_count) } else { format!("{} preset(s)",entry.preset_count) },profile(entry.pid,&entry.families))).size(12.0).color(theme::muted(ui.ctx())));
                        ui.label(RichText::new(format!("Imported {} · {} bytes",archive_state::date(entry.imported_at_ms),entry.original_bytes)).size(11.0).color(theme::muted(ui.ctx())));
                        ui.horizontal_wrapped(|ui| {
                            if entry.trashed {if button(ui,&format!("archive-recover-{}",entry.id),"Recover",idle){action=Some(Action::Recover(entry.clone()));}}
                            else {
                                if button(ui,&format!("archive-inspect-{}",entry.id),"Inspect",idle){action=Some(Action::Load(entry.clone()));}
                                if button(ui,&format!("archive-delete-{}",entry.id),"Move to deleted",idle){action=Some(Action::Delete(entry.clone()));}
                            }
                        });
                    });ui.add_space(6.0);
                }
            });
            egui::ScrollArea::vertical().id_salt("archive-content").max_height(height).show(&mut columns[1],|ui| {
                inspect(ui,state,device,library,&mut action);
            });
        });
        for diagnostic in state.diagnostics.iter().take(2) {
            ui.label(
                RichText::new(diagnostic)
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
            );
        }
        if let Some(notice) = &state.notice {
            ui.label(RichText::new(notice).color(theme::accent(ui.ctx())));
        }
        if let Some(error) = &state.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        }
        ui.add_space(8.0);
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "archives-extract",
                "Extract selected…",
                idle && state.loaded.is_some() && !state.marked.is_empty(),
            ) {
                action = Some(Action::Extract);
            }
            if button(
                ui,
                "archives-read-names",
                "Read target names",
                idle && device.current_binding().is_some() && !device.operation_busy(),
            ) {
                action = Some(Action::ReadNames);
            }
            if button(
                ui,
                "archives-review",
                "Review restore…",
                idle && state.loaded.is_some()
                    && !state.marked.is_empty()
                    && device.current_bank_names().is_some()
                    && !device.operation_busy(),
            ) {
                action = Some(Action::Review);
            }
            if button(ui, "archives-close", "Close", true) {
                action = Some(Action::Close);
            }
        });
        if device.bank.busy() {
            ui.label(format!(
                "{} · {} / {}",
                device.bank.kind, device.bank.completed, device.bank.total
            ));
            if button(ui, "archives-cancel", "Cancel amplifier work", true) {
                action = Some(Action::Cancel);
            }
        } else if device.bank.phase == openfuse_device::OperationPhase::Verified {
            ui.label(
                RichText::new(&device.bank.detail)
                    .size(12.0)
                    .color(theme::accent(ui.ctx())),
            );
        }
    });
    action
}
fn profile(pid: Option<u16>, families: &[u32]) -> String {
    if let Some(pid) = pid {
        return openfuse_core::catalog::find_device(openfuse_core::catalog::FENDER_VENDOR_ID, pid)
            .map_or_else(
                || format!("PID {pid:04X}"),
                |d| format!("{} · {pid:04X}", d.name),
            );
    }
    families
        .iter()
        .map(|&family| {
            openfuse_core::model_catalog::product(family)
                .map_or_else(|| format!("Family {family}"), |p| p.name.to_owned())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn inspect(
    ui: &mut egui::Ui,
    state: &mut ArchiveState,
    device: &DeviceState,
    library: Option<&PathBuf>,
    action: &mut Option<Action>,
) {
    let Some((entry, archive)) = &state.loaded else {
        ui.label("Inspect an archive to browse its slots, extract presets or map a restore.");
        return;
    };
    theme::eyebrow(ui, "ARCHIVE CONTENTS");
    ui.heading(&entry.label);
    for warning in archive.warnings() {
        ui.label(
            RichText::new(warning)
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
    }
    if let Some(legacy) = archive.legacy() {
        if let Some(description) = legacy.description() {
            ui.label(description);
        }
        egui::CollapsingHeader::new(format!(
            "Original files · {} entries",
            legacy.entries().len()
        ))
        .default_open(false)
        .show(ui, |ui| {
            for file in legacy.entries() {
                ui.label(
                    RichText::new(format!(
                        "{} · {:?} · {} bytes",
                        file.path(),
                        file.kind(),
                        file.bytes().len()
                    ))
                    .size(11.0),
                );
                if let Some(model) = &file.metadata().amplifier {
                    ui.label(
                        RichText::new(model)
                            .size(11.0)
                            .color(theme::muted(ui.ctx())),
                    );
                }
            }
        });
    }
    if archive.legacy().is_some() {
        ui.label("Original FUSE folders do not identify the exact USB model reliably. Choose its amplifier profile explicitly before restore.");
        egui::ComboBox::from_id_salt("legacy-profile")
            .selected_text(state.legacy_profile.map_or_else(
                || "Choose source amplifier…".into(),
                |pid| format!("PID {pid:04X}"),
            ))
            .show_ui(ui, |ui| {
                for (pid, label) in [
                    (4, "Mustang I / II"),
                    (5, "Mustang III / IV / V"),
                    (0x10, "Mustang Mini"),
                    (0x14, "Mustang I / II V2"),
                    (0x16, "Mustang III / IV / V V2"),
                    (0x12, "Mustang Floor"),
                    (0xa, "Bronco 40"),
                    (0x26, "Passport Mini"),
                    (0xe, "Super Champ X2"),
                ] {
                    if openfuse_core::wire_preset::product_for_pid(pid).is_some_and(|family| {
                        entry.families.is_empty() || entry.families.contains(&family)
                    }) {
                        ui.selectable_value(
                            &mut state.legacy_profile,
                            Some(pid),
                            format!("{label} · {pid:04X}"),
                        );
                    }
                }
            });
    }
    if let Some(document) = archive.control_preset() {
        ui.label(format!(
            "{} · {}",
            document.name(),
            crate::control_editor_view::kind(document.pid(), document.control())
        ));
        if button(
            ui,
            "archive-control-preset",
            "Open FX / VOICE editor…",
            !state.busy(),
        ) {
            *action = Some(Action::ControlPreset);
        }
        return;
    }
    if (archive.auxiliary().is_some() || archive.legacy().is_some())
        && button(
            ui,
            "archive-auxiliary",
            "Inspect system & control banks…",
            !state.busy(),
        )
    {
        *action = Some(Action::Auxiliary);
    }
    if let Some(snapshot) = archive.auxiliary() {
        ui.label(format!(
            "{} system/control entries · PID {:04X}",
            snapshot.entries().len(),
            snapshot.pid()
        ));
        egui::CollapsingHeader::new("System and control-bank inventory").show(ui, |ui| {
            for entry in snapshot.entries() {
                ui.label(entry.label());
                if let Some(reason) = entry.restore_error() {
                    ui.label(
                        RichText::new(reason)
                            .size(11.0)
                            .color(theme::muted(ui.ctx())),
                    );
                }
            }
        });
        if archive.presets().is_empty() {
            return;
        }
        ui.separator();
        ui.strong(format!("{} preserved presets", archive.presets().len()));
    }
    let names = device.current_bank_names().map(|n| &n.entries);
    let count = device
        .capture
        .as_ref()
        .and_then(|c| openfuse_core::audition::slot_count(c.device.product_id).ok())
        .unwrap_or(0);
    ui.label(
        RichText::new("Source selection to destination slot (zero-based)")
            .size(12.0)
            .color(theme::muted(ui.ctx())),
    );
    ui.horizontal(|ui| {
        if ui.small_button("Select all").clicked() {
            state.marked = archive.presets().iter().map(|p| p.slot).collect();
        }
        if ui.small_button("Clear").clicked() {
            state.marked.clear();
        }
    });
    for preset in archive.presets() {
        let mut marked = state.marked.contains(&preset.slot);
        ui.horizontal(|ui| {
            let r = ui.checkbox(&mut marked, format!("{:02}", preset.slot));
            remember(&r, &format!("archive-slot-{}", preset.slot));
            if r.changed() {
                if marked {
                    state.marked.insert(preset.slot);
                } else {
                    state.marked.remove(&preset.slot);
                }
            }
            let r = ui.selectable_label(
                state.inspected == Some(preset.slot),
                preset.document.metadata().name,
            );
            remember(&r, &format!("archive-detail-{}", preset.slot));
            if r.clicked() {
                state.inspected = Some(preset.slot);
            }
            if marked && count > 0 {
                ui.label("to");
                let target = state
                    .targets
                    .entry(preset.slot)
                    .or_insert(preset.slot.min(count - 1));
                let r = ui.add(egui::DragValue::new(target).range(0..=count - 1));
                remember(&r, &format!("archive-target-{}", preset.slot));
            }
        });
        if marked && let Some(target) = state.targets.get(&preset.slot) {
            ui.label(
                RichText::new(names.and_then(|names| names.get(target)).map_or_else(
                    || "Read target names before restore".into(),
                    |name| format!("Replace: {target:02} · {name}"),
                ))
                .size(11.0)
                .color(theme::muted(ui.ctx())),
            );
        }
    }
    if let Some(preset) = archive
        .presets()
        .iter()
        .find(|p| Some(p.slot) == state.inspected)
    {
        ui.add_space(10.0);
        ui.separator();
        ui.strong(format!(
            "Slot {:02} · {}",
            preset.slot,
            preset.document.metadata().name
        ));
        for module in preset.document.modules() {
            let category = openfuse_core::model_catalog::ModuleCategory::from_key(&module.key);
            let name = category
                .and_then(|category| {
                    openfuse_core::model_catalog::model(
                        preset.document.product_id()?,
                        category,
                        module.model_id?,
                    )
                })
                .map_or_else(
                    || format!("ID {:?}", module.model_id),
                    |model| model.name.clone(),
                );
            ui.label(format!(
                "{} · {} · {}",
                module.label,
                name,
                if module.active == Some(false) {
                    "bypassed"
                } else {
                    "active"
                }
            ));
        }
        if button(
            ui,
            "archive-open-preset",
            "Open as local preset",
            !state.busy(),
        ) {
            *action = Some(Action::OpenPreset(preset.slot));
        }
    }
    ui.add_space(10.0);
    ui.label(
        RichText::new(library.map_or_else(
            || "Extraction will ask for a preset library folder.".into(),
            |path| format!("Extract to library: {}", path.display()),
        ))
        .size(11.0)
        .color(theme::muted(ui.ctx())),
    );
}
pub fn restore_preview(
    ctx: &egui::Context,
    draft: &mut RestoreDraft,
    valid: bool,
) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "archive-restore-preview", 740.0, |ui| {
        theme::eyebrow(ui, "PERSISTENT AMPLIFIER WRITE");
        crate::dialog::header(
            ui,
            "Restore selected presets",
            &format!("{} → {}", draft.archive_label, draft.device.name),
        );
        egui::ScrollArea::vertical().id_salt("archive-mapping-review")
            .max_height((ctx.screen_rect().height() - 390.0).clamp(100.0, 430.0)).show(ui, |ui| {
            crate::dialog::section(ui, "Destinations", |ui| {
                egui::Grid::new("archive-mapping-grid").num_columns(3).max_col_width(290.0).min_row_height(24.0).spacing([24.0, 12.0]).show(ui, |ui| {
                    ui.strong("From archive"); ui.strong("Slot"); ui.strong("Replaces"); ui.end_row();
                    for row in &draft.rows {
                        ui.label(format!("{:02} · {}", row.source, row.source_name));
                        ui.monospace(format!("{:02}", row.target));
                        ui.label(&row.target_name); ui.end_row();
                    }
                });
            });
                crate::dialog::review_notes(ui, &draft.warnings);
            egui::CollapsingHeader::new("Verification details").show(ui, |ui| {
                ui.label(format!("Destination read {}", draft.binding.generation));
                crate::dialog::hint(ui, "Only listed preset slots are replaced. Each write is read back before continuing.");
            });
        });
        ui.label("Selected slots will be overwritten. Cancelling stops future writes; completed slots remain changed.");
        let r = ui.checkbox(
            &mut draft.confirmed,
            format!("Overwrite these {} destination slot(s)", draft.rows.len()),
        );
        remember(&r, "archive-restore-confirm");
        if !valid {
            ui.colored_label(
                theme::danger(ui.ctx()),
                "This destination read changed. Go back and prepare a fresh preview.",
            );
        }
        crate::dialog::footer(ui, |ui| {
            if button(ui, "archive-restore-back", "Back", true) {
                action = Some(Action::Dismiss);
            }
            let apply = crate::dialog::primary(ui, "Restore selected", valid && draft.confirmed);
            remember(&apply, "archive-restore-apply");
            if apply.clicked() {
                action = Some(Action::Apply);
            }
        });
    });
    action
}
