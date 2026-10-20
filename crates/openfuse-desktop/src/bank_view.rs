//! Native bank previews own their inputs; drawing never sends a device command.
use crate::{audition_view::remember, device_read::ReadDraft, device_state::DeviceState, theme};
use eframe::egui::{self, RichText};
use openfuse_core::bank::Backup;
use openfuse_device::{DeviceInfo, LiveBinding, WireCapture};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct BankUi {
    pub open: bool,
    pub marked_slots: std::collections::BTreeSet<u16>,
    pub save: Option<SaveDraft>,
    pub restore: Option<RestoreDraft>,
    pub demo_pending: bool,
    pub demo_started: bool,
    pub demo_save: bool,
    pub demo_restore: bool,
}
impl BankUi {
    pub fn new(demo: bool, save: bool, restore: bool) -> Self {
        Self {
            demo_pending: demo,
            demo_save: save,
            demo_restore: restore,
            ..Default::default()
        }
    }
    pub fn modal_open(&self) -> bool {
        self.open || self.save.is_some() || self.restore.is_some()
    }
}
pub struct SaveDraft {
    pub binding: LiveBinding,
    pub device: DeviceInfo,
    pub slot: u16,
    pub source_name: String,
    pub replaced_name: String,
    pub confirmed: bool,
}
impl SaveDraft {
    pub fn prepare(
        capture: &WireCapture,
        slot: u16,
        replaced_name: String,
    ) -> Result<Self, String> {
        let count = openfuse_core::audition::slot_count(capture.device.product_id)
            .map_err(|e| e.to_string())?;
        if slot >= count {
            return Err("Save destination is outside this amplifier’s bank".into());
        }
        let read = ReadDraft::decode(capture.clone())?;
        Ok(Self {
            binding: LiveBinding {
                device_id: capture.device.id.clone(),
                generation: capture.generation,
            },
            device: capture.device.clone(),
            slot,
            source_name: read.document.metadata().name,
            replaced_name,
            confirmed: false,
        })
    }
}
pub struct RestoreDraft {
    pub binding: LiveBinding,
    pub device: DeviceInfo,
    pub backup: Backup,
    pub source: PathBuf,
    pub confirmed: bool,
}
impl RestoreDraft {
    pub fn prepare(capture: &WireCapture, backup: Backup, source: PathBuf) -> Result<Self, String> {
        if !backup.is_complete() {
            return Err("This archive contains selected presets. Use Archive catalog to map selected presets".into());
        }
        backup
            .validate_for_pid(capture.device.product_id)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            binding: LiveBinding {
                device_id: capture.device.id.clone(),
                generation: capture.generation,
            },
            device: capture.device.clone(),
            backup,
            source,
            confirmed: false,
        })
    }
}
pub fn read_backup(path: &Path) -> Result<Backup, String> {
    use std::io::Read;
    const LIMIT: u64 = openfuse_core::bank::MAX_BACKUP_BYTES as u64;
    if !std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("Bank archive must be a regular file".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Bank archive must be a regular file".into());
    }
    let mut text = String::new();
    file.take(LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > LIMIT {
        return Err("Bank archive exceeds 4 MiB".into());
    }
    Backup::parse(&text).map_err(|e| e.to_string())
}
pub fn export_backup(path: &Path, backup: &Backup) -> Result<(), String> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary
        .write_all(backup.to_json().map_err(|e| e.to_string())?.as_bytes())
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist_noclobber(path)
        .map_err(|e| format!("Bank export did not overwrite an existing file: {e}"))?;
    Ok(())
}
#[derive(Clone, Copy)]
pub enum Action {
    Close,
    ReadNames,
    Backup,
    BackupSelected,
    Catalog,
    Export,
    Import,
    Select(u16),
    Save(u16),
    Cancel,
    ApplySave,
    ApplyRestore,
    Dismiss,
}
fn button(ui: &mut egui::Ui, id: &str, label: &str, enabled: bool) -> bool {
    let r = ui.add_enabled(
        enabled,
        egui::Button::new(label).min_size(egui::vec2(110.0, 40.0)),
    );
    remember(&r, id);
    r.clicked()
}
pub fn dialog(ctx: &egui::Context, state: &DeviceState, bank_ui: &mut BankUi) -> Option<Action> {
    let mut action = None;
    let valid = state.current_binding().is_some() && !state.operation_busy();
    crate::dialog::show(ctx, "bank-browser", 760.0, |ui| {
        theme::eyebrow(
            ui,
            if state.simulated {
                "SIMULATOR · STORED SOUNDS"
            } else {
                "AMPLIFIER BANK"
            },
        );
        ui.heading("Your amplifier’s sounds");
        ui.label("Select a stored sound to read it. Save writes the current amplifier sound to one slot; your local editor stays unchanged.");
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            if button(ui, "bank-read", "Read names", valid) {
                action = Some(Action::ReadNames);
            }
            if button(ui, "bank-backup", "Read full backup", valid) {
                action = Some(Action::Backup);
            }
            if button(
                ui,
                "bank-backup-selected",
                "Read selected",
                valid && !bank_ui.marked_slots.is_empty() && state.current_bank_names().is_some(),
            ) {
                action = Some(Action::BackupSelected);
            }
            if button(ui, "bank-catalog", "Archive catalog…", true) {
                action = Some(Action::Catalog);
            }
            if button(
                ui,
                "bank-export",
                "Export backup…",
                state.bank_backup.is_some() && !state.operation_busy(),
            ) {
                action = Some(Action::Export);
            }
            if button(ui, "bank-import", "Restore file…", valid) {
                action = Some(Action::Import);
            }
        });
        progress(ui, state, &mut action);
        if let Some(backup) = &state.bank_backup {
            ui.label(
                RichText::new(format!(
                    "Backup retained: {} · {} slots · read {}",
                    backup.device.name,
                    backup.backup.slots().len(),
                    backup.generation
                ))
                .size(12.0)
                .color(theme::muted(ui.ctx())),
            );
        }
        ui.add_space(10.0);
        if let Some(bank) = &state.bank_names {
            let fresh = state.current_bank_names().is_some();
            ui.label(
                RichText::new(format!(
                    "{} · {} names · read {}{}",
                    bank.device.name,
                    bank.entries.len(),
                    bank.generation,
                    if fresh {
                        ""
                    } else {
                        " · refresh names before saving"
                    }
                ))
                .monospace()
                .size(12.0),
            );
            egui::ScrollArea::vertical()
                .id_salt("bank-slots")
                .max_height((ctx.screen_rect().height() - 445.0).clamp(180.0, 500.0))
                .show(ui, |ui| {
                    egui::Grid::new("bank-slot-grid")
                        .num_columns(4)
                        .spacing([16.0, 5.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Index");
                            ui.strong("Stored name");
                            ui.label("");
                            ui.label("");
                            ui.end_row();
                            for (&slot, name) in &bank.entries {
                                let mut marked = bank_ui.marked_slots.contains(&slot);
                                let r = ui.checkbox(&mut marked, format!("{slot:02}"));
                                remember(&r, &format!("bank-mark-{slot}"));
                                if r.changed() {
                                    if marked {
                                        bank_ui.marked_slots.insert(slot);
                                    } else {
                                        bank_ui.marked_slots.remove(&slot);
                                    }
                                }
                                ui.add_sized([290.0, 40.0], egui::Label::new(name).truncate());
                                if button(
                                    ui,
                                    &format!("bank-select-{slot}"),
                                    "Select + read",
                                    valid && fresh,
                                ) {
                                    action = Some(Action::Select(slot));
                                }
                                if button(
                                    ui,
                                    &format!("bank-save-{slot}"),
                                    "Save here…",
                                    valid && fresh,
                                ) {
                                    action = Some(Action::Save(slot));
                                }
                                ui.end_row();
                            }
                        });
                });
        } else {
            ui.add_space(20.0);
            ui.label(RichText::new("Read names to browse and choose a destination. Full backup reads every stored preset into a local archive.").color(theme::muted(ui.ctx())));
            ui.add_space(20.0);
        }
        if let Some(error) = &state.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        }
        ui.add_space(12.0);
        if button(ui, "bank-close", "Close", true) {
            action = Some(Action::Close);
        }
    });
    action
}
pub fn progress(ui: &mut egui::Ui, state: &DeviceState, action: &mut Option<Action>) {
    if state.bank.phase == openfuse_device::OperationPhase::Idle {
        return;
    }
    ui.add_space(10.0);
    ui.label(
        RichText::new(format!("{} · {:?}", state.bank.kind, state.bank.phase))
            .strong()
            .color(theme::accent(ui.ctx())),
    );
    ui.label(&state.bank.detail);
    if state.bank.busy() {
        if state.bank.total > 0 {
            ui.add(
                egui::ProgressBar::new(state.bank.completed as f32 / state.bank.total as f32).text(
                    format!("{} / {} slots", state.bank.completed, state.bank.total),
                ),
            );
        }
        if button(ui, "bank-cancel", "Cancel operation", true) {
            *action = Some(Action::Cancel);
        }
    }
}
pub fn save_dialog(ctx: &egui::Context, draft: &mut SaveDraft, valid: bool) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "bank-save-preview", 570.0, |ui| {
        theme::eyebrow(ui, "PERSISTENT AMPLIFIER SAVE");
        ui.heading("Replace this stored slot");
        ui.label(format!(
            "{} · PID {:04X} · read {}",
            draft.device.name, draft.device.product_id, draft.binding.generation
        ));
        ui.add_space(12.0);
        egui::Grid::new("save-comparison")
            .num_columns(2)
            .min_col_width(180.0)
            .max_col_width(280.0)
            .spacing([24.0, 12.0])
            .show(ui, |ui| {
                ui.label("Destination index");
                ui.label(RichText::new(draft.slot.to_string()).monospace().strong());
                ui.end_row();
                ui.label("Currently stored");
                ui.label(&draft.replaced_name);
                ui.end_row();
                ui.label("Replace with amplifier sound");
                ui.label(RichText::new(&draft.source_name).strong());
                ui.end_row();
            });
        ui.add_space(14.0);
        ui.label("Saves the last captured amplifier working sound. To save your local edits, audition them and verify the readback first.");
        ui.label("This changes a stored slot. Cancellation cannot undo a save already accepted by the amplifier.");
        let r = ui.checkbox(
            &mut draft.confirmed,
            format!("Overwrite slot {} — {}", draft.slot, draft.replaced_name),
        );
        remember(&r, "bank-save-confirm");
        if !valid {
            ui.colored_label(
                theme::accent(ui.ctx()),
                "This preview is stale. Read again and choose the destination again.",
            );
        }
        ui.add_space(12.0);
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "bank-save-apply",
                "Save to amplifier",
                valid && draft.confirmed,
            ) {
                action = Some(Action::ApplySave);
            }
            if button(ui, "bank-save-dismiss", "Cancel", true) {
                action = Some(Action::Dismiss);
            }
        });
    });
    action
}
pub fn restore_dialog(
    ctx: &egui::Context,
    draft: &mut RestoreDraft,
    valid: bool,
) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "bank-restore-preview", 640.0, |ui| {
        theme::eyebrow(ui, "RESTORE STORED AMPLIFIER BANK");
        ui.heading("Review the bank replacement");
        ui.label(format!(
            "Destination: {} · PID {:04X}",
            draft.device.name, draft.device.product_id
        ));
        ui.label(format!(
            "Archive: {} · PID {:04X} · {} slots",
            draft
                .source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            draft.backup.pid(),
            draft.backup.slots().len()
        ));
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .id_salt("restore-slots")
            .max_height((ctx.screen_rect().height() - 485.0).clamp(120.0, 340.0))
            .show(ui, |ui| {
                for slot in draft.backup.slots() {
                    ui.label(format!("{:02}   {}", slot.slot(), slot.name()));
                }
            });
        ui.add_space(10.0);
        ui.label("Preset sounds only. System settings and voice/FX knob banks are not included. Global pedal volume endpoints stay at current amplifier values; the last restored sound becomes active.");
        ui.add_space(10.0);
        ui.label("Every listed slot will be overwritten, one at a time, with readback verification. Keep a backup of this amplifier before restoring. Cancelling stops future writes; completed slots stay changed.");
        let r = ui.checkbox(
            &mut draft.confirmed,
            format!("Replace all {} stored slots", draft.backup.slots().len()),
        );
        remember(&r, "bank-restore-confirm");
        if !valid {
            ui.colored_label(
                theme::accent(ui.ctx()),
                "The amplifier or current read changed. Close and prepare a new restore preview.",
            );
        }
        ui.add_space(12.0);
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "bank-restore-apply",
                "Restore bank",
                valid && draft.confirmed,
            ) {
                action = Some(Action::ApplyRestore);
            }
            if button(ui, "bank-restore-dismiss", "Cancel", true) {
                action = Some(Action::Dismiss);
            }
        });
    });
    action
}
