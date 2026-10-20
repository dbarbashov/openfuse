//! Native snapshot browser. Selections and overwrite previews own their data.
use crate::{audition_view::remember, controls_view::button, device_state::DeviceState, theme};
use eframe::egui::{self, RichText};
use openfuse_core::auxiliary::{Key, Mapping, Selection, Snapshot};
use openfuse_device::LiveBinding;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct AuxiliaryUi {
    pub open: bool,
    pub source: Option<Source>,
    pub filter: String,
    pub selected: BTreeSet<Key>,
    pub targets: BTreeMap<Key, Key>,
    pub preview: Option<Preview>,
    pub whole_preview: Option<crate::whole_restore_view::Preview>,
    pub whole_preparation: Option<crate::whole_restore_view::Preparation>,
    pub source_revision: u64,
    pub error: Option<String>,
    pub demo: u8,
    pub demo_stage: u8,
    pub pending_archive: Option<Snapshot>,
    pub demo_directory: Option<tempfile::TempDir>,
}
impl AuxiliaryUi {
    pub fn modal_open(&self) -> bool {
        self.open || self.preview.is_some() || self.whole_preview.is_some()
    }
    pub fn source(&mut self, label: String, snapshot: Snapshot) {
        self.source_revision = self.source_revision.wrapping_add(1);
        self.source = Some(Source {
            label,
            snapshot,
            archive: None,
        });
        self.filter.clear();
        self.selected.clear();
        self.targets.clear();
        self.preview = None;
        self.whole_preview = None;
        if let Some(task) = &mut self.whole_preparation {
            task.cancelled = true;
        }
        self.error = None;
    }
    pub fn mappings(&self) -> Vec<Mapping> {
        self.selected
            .iter()
            .map(|key| Mapping {
                source: *key,
                target: *self.targets.get(key).unwrap_or(key),
            })
            .collect()
    }
}
#[derive(Clone)]
pub struct Source {
    pub label: String,
    pub snapshot: Snapshot,
    pub archive: Option<openfuse_core::archive::Archive>,
}
pub struct Preview {
    pub binding: LiveBinding,
    pub destination_fingerprint: String,
    pub selection: Selection,
    pub source_label: String,
    pub destination_label: String,
    pub rows: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub confirmed: bool,
}
#[derive(Clone, Copy)]
pub enum Action {
    Editor,
    Edit(Key),
    Close,
    Read,
    UseRead,
    SaveRead,
    Catalog,
    Review,
    ReviewWhole,
    Apply,
    Back,
    Cancel,
}

pub fn dialog(
    ctx: &egui::Context,
    state: &mut AuxiliaryUi,
    device: &DeviceState,
    archives: &crate::archive_state::ArchiveState,
) -> Option<Action> {
    let mut action = None;
    let idle = !device.operation_busy() && state.whole_preparation.is_none();
    crate::dialog::show(ctx, "auxiliary-browser", 1000.0, |ui| {
        theme::eyebrow(ui, "SYSTEM & CONTROL BANKS");
        ui.heading("Preserve and restore the amplifier");
        ui.add(egui::Label::new("Capture presets, System and control banks together. Restore selected control entries, or review a complete device replacement from a full archive.").wrap());
        ui.horizontal_wrapped(|ui| {
            if button(
                ui,
                "aux-read",
                "Read amplifier",
                idle && device.current_binding().is_some(),
            ) {
                action = Some(Action::Read);
            }
            if button(
                ui,
                "aux-use-read",
                "Use latest read as source",
                idle && device.auxiliary.is_some(),
            ) {
                action = Some(Action::UseRead);
            }
            if button(
                ui,
                "aux-save-read",
                "Archive latest read…",
                device.auxiliary.is_some() && !archives.busy(),
            ) {
                action = Some(Action::SaveRead);
            }
            if button(ui, "aux-editor", "FX / VOICE editor…", true) {
                action = Some(Action::Editor);
            }
            if button(ui, "aux-catalog", "Archive catalog…", true) {
                action = Some(Action::Catalog);
            }
        });
        let destination = device.current_auxiliary();
        if let Some(snapshot) = destination {
            ui.label(
                RichText::new(format!(
                    "Destination: {} · read {} · {} entries",
                    snapshot.device.name,
                    snapshot.generation,
                    snapshot.snapshot.entries().len()
                ))
                .color(theme::accent(ui.ctx())),
            );
        } else {
            ui.label(
                RichText::new("Read fresh system and control banks before reviewing a restore.")
                    .color(theme::muted(ui.ctx())),
            );
        }
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.filter)
                    .hint_text("Filter entries: VOICE, FX, MOD…")
                    .desired_width(280.0),
            );
            ui.label(format!("{} selected", state.selected.len()));
            if button(
                ui,
                "aux-clear",
                "Clear selection",
                !state.selected.is_empty(),
            ) {
                state.selected.clear();
                state.targets.clear();
            }
        });
        let filter = state.filter.to_lowercase();
        egui::ScrollArea::vertical().id_salt("auxiliary-content").max_height((ctx.screen_rect().height() - 485.0).clamp(140.0, 460.0)).show(ui, |ui| {
            if let Some(source) = &state.source {
                ui.heading(&source.label);
                ui.label(format!("{} preserved presets · {} system/control entries · PID {:04X}", source.archive.as_ref().map_or(source.snapshot.presets().len(),|archive| archive.presets().len()),source.snapshot.entries().len(),source.snapshot.pid()));
                for warning in source.snapshot.warnings() {
                    ui.label(RichText::new(warning).size(12.0).color(theme::muted(ui.ctx())));
                }
                let entries: Vec<_> = source.snapshot.entries().iter().filter(|entry| entry.label().to_lowercase().contains(&filter)).collect();
                if entries.is_empty() { ui.label("No entries match this filter."); }
                for entry in entries {
                    ui.add_space(8.0);
                    let key = entry.key();
                    let mut selected = state.selected.contains(&key);
                    ui.horizontal_wrapped(|ui| {
                        let response = ui.add_enabled(entry.restore_error().is_none(), egui::Checkbox::new(&mut selected, entry.label()));
                        remember(&response, &format!("aux-entry-{key:?}"));
                        if response.changed() {
                            if selected { state.selected.insert(key); }
                            else { state.selected.remove(&key); }
                        }
                        if matches!(key,Key::Bank{..}) && button(ui,&format!("aux-edit-{key:?}"),"Edit copy…",entry.xml().is_some()&&entry.restore_error().is_none()) {action=Some(Action::Edit(key));}
                        if selected {
                            let target = state.targets.entry(key).or_insert(key);
                            ui.label("to");
                            egui::ComboBox::from_id_salt(("aux-target", &key)).selected_text(destination.and_then(|d| d.snapshot.entries().iter().find(|e| e.key() == *target)).map_or_else(|| format!("{target:?}"), |entry| entry.label())).show_ui(ui, |ui| {
                                if let Some(destination) = destination {
                                    for candidate in destination.snapshot.entries().iter().filter(|candidate| key.same_kind(candidate.key()) && candidate.restore_error().is_none()) {
                                        ui.selectable_value(target, candidate.key(), candidate.label());
                                    }
                                }
                            });
                        }
                    });
                    if let Some(reason) = entry.restore_error() { ui.label(RichText::new(reason).size(12.0).color(theme::muted(ui.ctx()))); }
                    egui::CollapsingHeader::new(format!("Inspect {}", entry.label())).id_salt(("aux-inspect", &key)).show(ui, |ui| {
                        ui.label(format!("{} original reports retained", entry.raw_reports().len()));
                        if let Some(xml) = entry.xml() { ui.add(egui::Label::new(RichText::new(xml).monospace().size(11.0)).wrap()); }
                        if !entry.raw_reports().is_empty() {
                            egui::CollapsingHeader::new("Original report bytes").id_salt(("aux-raw", &key)).show(ui, |ui| {
                                egui::ScrollArea::horizontal().id_salt(("aux-raw-scroll", &key)).show(ui, |ui| {
                                    for report in entry.raw_reports() {
                                        for bytes in report.as_bytes().chunks(16) {
                                            ui.monospace(bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(" "));
                                        }
                                        ui.add_space(4.0);
                                    }
                                });
                            });
                        }
                    });
                    ui.separator();
                }
            } else {
                ui.label("Choose an archived snapshot or use a completed amplifier read as the source.");
            }
        });
        if let Some(error) = &state.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        }
        if let Some(error) = &archives.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        } else if let Some(notice) = &archives.notice {
            ui.label(
                RichText::new(notice)
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
            );
        }
        if device.bank.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "{} · {} / {}",
                    device.bank.kind, device.bank.completed, device.bank.total
                ));
                if button(ui, "aux-cancel", "Cancel", true) {
                    action = Some(Action::Cancel);
                }
            });
        } else if !device.bank.detail.is_empty() {
            ui.label(
                RichText::new(&device.bank.detail)
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
            );
        }
        if let Some(error) = &device.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        }
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "aux-review",
                "Review selected restore…",
                idle && destination.is_some() && !state.selected.is_empty(),
            ) {
                action = Some(Action::Review);
            }
            if button(
                ui,
                "aux-review-whole",
                "Review whole-device restore…",
                idle && destination.is_some() && state.source.is_some(),
            ) {
                action = Some(Action::ReviewWhole);
            }
            if button(ui, "aux-close", "Close", true) {
                action = Some(Action::Close);
            }
        });
    });
    action
}

pub fn preview(ctx: &egui::Context, draft: &mut Preview, valid: bool) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "auxiliary-restore-preview", 730.0, |ui| {
        theme::eyebrow(ui, "SELECTIVE BANK RESTORE");
        ui.heading("Review selected control-bank changes");
        ui.add(
            egui::Label::new(format!(
                "{} to {} · read {}",
                draft.source_label, draft.destination_label, draft.binding.generation
            ))
            .wrap(),
        );
        egui::ScrollArea::vertical()
            .id_salt("auxiliary-review-rows")
            .max_height((ctx.screen_rect().height() - 475.0).clamp(120.0, 400.0))
            .show(ui, |ui| {
                egui::Grid::new("auxiliary-overwrites")
                    .num_columns(2)
                    .striped(true)
                    .max_col_width((ui.available_width() - 24.0) / 2.0)
                    .spacing([24.0, 12.0])
                    .show(ui, |ui| {
                        ui.strong("Source entry");
                        ui.strong("Destination to replace");
                        ui.end_row();
                        for (source, target) in &draft.rows {
                            ui.add(egui::Label::new(source).wrap());
                            ui.add(egui::Label::new(target).wrap());
                            ui.end_row();
                        }
                    });
                for warning in &draft.warnings {
                    ui.add(
                        egui::Label::new(
                            RichText::new(warning)
                                .size(12.0)
                                .color(theme::muted(ui.ctx())),
                        )
                        .wrap(),
                    );
                }
            });
        ui.add(egui::Label::new("The destination is checked again before writing. Cancel stops future writes; completed entries remain changed. The local preset and its undo history stay untouched.").wrap());
        let confirmation = if draft.rows.len() == 1 {
            "Overwrite this selected entry".to_owned()
        } else {
            format!("Overwrite these {} selected entries", draft.rows.len())
        };
        let response = ui.checkbox(&mut draft.confirmed, confirmation);
        remember(&response, "aux-confirm");
        if !valid {
            ui.colored_label(
                theme::accent(ui.ctx()),
                "The destination read changed. Go back and prepare a fresh preview.",
            );
        }
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "aux-apply",
                "Restore selected",
                valid && draft.confirmed,
            ) {
                action = Some(Action::Apply);
            }
            if button(ui, "aux-back", "Back", true) {
                action = Some(Action::Back);
            }
        });
    });
    action
}
