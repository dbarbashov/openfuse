//! Native G-DEC folder workspace. Rendering performs no filesystem operations.
use crate::{
    audition_view::remember,
    controls_view::button,
    storage_state::{Preview, State},
    theme,
};
use eframe::egui::{self, RichText};
use openfuse_core::mounted_storage::{Direction, FileKind, Volume};
#[derive(Clone, Copy)]
pub enum Action {
    ChooseRoot,
    Refresh,
    Upload,
    Download,
    OpenPreset,
    AddMedia,
    Close,
    Cancel,
    Back,
    Apply,
    Volume(Volume),
}
pub fn kind(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Audio => "Audio",
        FileKind::Phrase => "Recorded phrases",
        FileKind::Midi => "MIDI",
        FileKind::Preset => "Presets",
    }
}
pub fn volume(volume: Volume) -> &'static str {
    match volume {
        Volume::Internal => "Internal storage",
        Volume::Sd => "SD card layout",
    }
}
fn short(value: &str, max: usize) -> String {
    if value.chars().count() > max {
        format!("{}…", value.chars().take(max).collect::<String>())
    } else {
        value.into()
    }
}
fn label(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) {
    ui.add(egui::Label::new(text).wrap());
}
pub fn show(ctx: &egui::Context, state: &mut State) -> Option<Action> {
    if let Some(preview) = &mut state.preview {
        return preview_dialog(ctx, preview, state.error.as_deref());
    }
    let mut action = None;
    let idle = !state.busy();
    crate::dialog::show(ctx, "gdec-storage-manager", 940.0, |ui| {
        theme::eyebrow(ui, "G-DEC · FOLDER WORKSPACE");
        ui.heading("Manage backing tracks and SD presets");
        egui::ScrollArea::vertical()
            .id_salt("gdec-storage-body")
            .max_height((ctx.screen_rect().height() - 295.0).max(240.0))
            .show(ui, |ui| {
                label(
                    ui,
                    "Choose a mounted storage root or an extracted folder with the same layout. Copies affect files in that folder. No amplifier connection or USB mode is inferred or changed.",
                );
                ui.horizontal_wrapped(|ui| {
                    let mut selected = state.volume;
                    ui.add_enabled_ui(idle, |ui| {
                        egui::ComboBox::from_id_salt("storage-volume").selected_text(volume(selected)).show_ui(ui, |ui| {
                            ui.selectable_value(&mut selected, Volume::Sd, volume(Volume::Sd));
                            ui.selectable_value(&mut selected, Volume::Internal, volume(Volume::Internal));
                        });
                    });
                    if selected != state.volume {
                        action = Some(Action::Volume(selected));
                    }
                    if button(ui, "storage-root", "Choose root…", idle) {
                        action = Some(Action::ChooseRoot);
                    }
                    if button(ui, "storage-refresh", "Refresh", idle && state.root.is_some()) {
                        action = Some(Action::Refresh);
                    }
                });
                if let Some(root) = &state.root {
                    ui.add(egui::Label::new(RichText::new(short(&root.display().to_string(), 160)).size(12.0).color(theme::muted(ui.ctx()))).wrap())
                        .on_hover_text(root.display().to_string());
                } else {
                    label(
                        ui,
                        RichText::new("Select the folder containing Audio, Phrase and Midi; SD storage also uses Presets.").color(theme::muted(ui.ctx())),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut state.filter, None, "All files");
                    for k in [FileKind::Preset, FileKind::Audio, FileKind::Midi, FileKind::Phrase] {
                        ui.selectable_value(&mut state.filter, Some(k), kind(k));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut state.query).hint_text("Search filenames, paths or preset names").desired_width(330.0));
                    ui.label(format!("{} selected", state.selected.len()));
                    if button(ui, "storage-clear", "Clear selection", idle && !state.selected.is_empty()) {
                        state.selected.clear();
                    }
                });
                let query = state.query.to_lowercase();
                let rows = state
                    .entries
                    .iter()
                    .filter(|entry| state.filter.is_none_or(|kind| kind == entry.kind))
                    .filter(|entry| {
                        format!("{} {} {}", entry.name, entry.relative_path.display(), entry.preset_name.as_deref().unwrap_or(""))
                            .to_lowercase()
                            .contains(&query)
                    })
                    .collect::<Vec<_>>();
                egui::ScrollArea::vertical()
                    .id_salt("gdec-storage-inventory")
                    .max_height((ctx.screen_rect().height() - 545.0).clamp(100.0, 380.0))
                    .show_rows(ui, 50.0, rows.len(), |ui, range| {
                        for index in range {
                            let entry = rows[index];
                            ui.horizontal(|ui| {
                                let mut checked = state.selected.contains(&entry.relative_path);
                                let response = ui.add_enabled(idle, egui::Checkbox::without_text(&mut checked));
                                remember(&response, &format!("storage-entry-{}", entry.relative_path.display()));
                                if response.changed() {
                                    if checked {
                                        state.selected.insert(entry.relative_path.clone());
                                    } else {
                                        state.selected.remove(&entry.relative_path);
                                    }
                                }
                                ui.vertical(|ui| {
                                    ui.add(egui::Label::new(&entry.name).truncate()).on_hover_text(&entry.name);
                                    let detail = format!(
                                        "{} · {:.2} MiB{}",
                                        entry.relative_path.display(),
                                        entry.size as f64 / 1048576.0,
                                        entry.preset_name.as_ref().map_or(String::new(), |name| format!(" · {name}"))
                                    );
                                    ui.add(egui::Label::new(RichText::new(&detail).size(11.0).color(theme::muted(ui.ctx()))).truncate()).on_hover_text(detail);
                                });
                            });
                        }
                    });
                if rows.is_empty() {
                    ui.label(if state.busy() { "Reading files…" } else { "No matching files in this inventory." });
                }
                ui.horizontal_wrapped(|ui| {
                    let response = ui.checkbox(&mut state.keep_both, "Keep both when a filename already exists");
                    remember(&response, "storage-keep-both");
                    ui.label(RichText::new("Existing files are never replaced.").size(12.0).color(theme::muted(ui.ctx())));
                });
                if !state.diagnostics.is_empty() || state.truncated {
                    egui::CollapsingHeader::new(format!("Inventory notes ({})", state.diagnostics.len())).show(ui, |ui| {
                        if state.truncated {
                            label(ui, "Inventory limit reached; only listed files can be selected.");
                        }
                        egui::ScrollArea::vertical().id_salt("storage-diagnostics").max_height(100.0).show(ui, |ui| {
                            for note in state.diagnostics.iter().take(100) {
                                label(ui, note);
                            }
                        });
                    });
                }
                if let Some(receipt) = &state.receipt {
                    let result = &receipt.result;
                    label(
                        ui,
                        RichText::new(format!(
                            "Last copy: {} files verified{}",
                            result.completed.len(),
                            if result.cancelled { " · cancelled before all files finished" } else { "" }
                        ))
                        .color(theme::accent(ui.ctx())),
                    );
                    if let Some(error) = &result.error {
                        label(ui, RichText::new(error).color(theme::accent(ui.ctx())));
                    }
                    egui::CollapsingHeader::new("Copy results").show(ui, |ui| {
                        egui::ScrollArea::vertical().id_salt("storage-receipt").max_height(120.0).show(ui, |ui| {
                            for item in &result.completed {
                                label(ui, format!("Verified: {}", item.destination.display()));
                            }
                            if let Some(path) = &result.interrupted {
                                label(ui, format!("Stopped at: {}", path.display()));
                            }
                        });
                    });
                }
                if let Some(error) = &state.error {
                    label(ui, RichText::new(error).color(theme::accent(ui.ctx())));
                }
                if state.busy() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spinner();
                        ui.label(&state.phase);
                        if state.progress.1 > 0 {
                            ui.monospace(format!("{} / {} verified", state.progress.0, state.progress.1));
                        }
                        if button(ui, "storage-cancel", "Cancel remaining", true) {
                            action = Some(Action::Cancel);
                        }
                    });
                } else if state.receipt.is_none()
                    && let Some(notice) = &state.notice
                {
                    label(ui, RichText::new(notice).size(12.0).color(theme::muted(ui.ctx())));
                }
            });
        if state.receipt.as_ref().is_some_and(|r| {
            matches!(r.direction, Direction::Download)
                && r.result
                    .completed
                    .iter()
                    .any(|item| is_media(&item.destination))
        }) && button(
            ui,
            "storage-add-media",
            "Add copied tracks to library…",
            idle,
        ) {
            action = Some(Action::AddMedia);
        }
        let selected = state.selected_entries();
        ui.horizontal_wrapped(|ui| {
            if button(
                ui,
                "storage-upload",
                "Copy files to storage…",
                idle && state.root.is_some(),
            ) {
                action = Some(Action::Upload);
            }
            if button(
                ui,
                "storage-download",
                "Copy selected to folder…",
                idle && !selected.is_empty(),
            ) {
                action = Some(Action::Download);
            }
            if button(
                ui,
                "storage-open-preset",
                "Open preset copy",
                idle && selected.len() == 1 && selected[0].kind == FileKind::Preset,
            ) {
                action = Some(Action::OpenPreset);
            }
            if button(
                ui,
                "storage-close",
                if state.busy() {
                    "Cancel & close"
                } else {
                    "Close"
                },
                true,
            ) {
                action = Some(Action::Close);
            }
        });
    });
    action
}
pub fn is_media(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "wav" | "mp3" | "mid"))
}
fn preview_dialog(ctx: &egui::Context, draft: &mut Preview, error: Option<&str>) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "gdec-storage-preview", 860.0, |ui| {
        theme::eyebrow(ui, "FROZEN FILE COPY PREVIEW");
        ui.heading(match draft.plan.direction() {
            Direction::Upload => "Copy local files into storage",
            Direction::Download => "Copy selected files into a local folder",
        });
        let items = draft.plan.items();
        let bytes = items.iter().map(|i| i.size).sum::<u64>();
        ui.label(format!(
            "{} files · {:.2} MiB · originals retained",
            items.len(),
            bytes as f64 / 1048576.0
        ));
        label(
            ui,
            "Every destination below is a new file. Existing files are never replaced. Source identity and destination availability are checked again before writing.",
        );
        let pages = items.len().div_ceil(20).max(1);
        ui.horizontal(|ui| {
            if button(ui, "storage-preview-prev", "Previous", draft.page > 0) {
                draft.page -= 1;
            }
            ui.monospace(format!("Page {} / {pages}", draft.page + 1));
            if button(ui, "storage-preview-next", "Next", draft.page + 1 < pages) {
                draft.page += 1;
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("storage-copy-items")
            .max_height((ctx.screen_rect().height() - 410.0).clamp(110.0, 420.0))
            .show(ui, |ui| {
                for item in items.iter().skip(draft.page * 20).take(20) {
                    label(
                        ui,
                        RichText::new(
                            item.destination
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                        )
                        .strong(),
                    );
                    label(ui, format!("From: {}", item.source.display()));
                    label(ui, format!("To: {}", item.destination.display()));
                    for change in &item.changes {
                        label(ui, RichText::new(change).color(theme::accent(ui.ctx())));
                    }
                    ui.separator();
                }
            });
        label(
            ui,
            "Cancel stops remaining copies. Files already verified stay in their destination; a final receipt lists completed and interrupted items.",
        );
        if !cfg!(unix) {
            label(
                ui,
                RichText::new("File copy execution currently requires macOS or Linux. This platform can inspect folders and prepare copy plans.").color(theme::accent(ui.ctx())),
            );
        }
        let response = ui.checkbox(&mut draft.confirmed, "Create these reviewed file copies");
        remember(&response, "storage-confirm");
        if let Some(error) = error {
            label(ui, RichText::new(error).color(theme::accent(ui.ctx())));
        }
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "storage-apply",
                "Copy reviewed files",
                cfg!(unix) && draft.confirmed && !items.is_empty(),
            ) {
                action = Some(Action::Apply);
            }
            if button(ui, "storage-back", "Back", true) {
                action = Some(Action::Back);
            }
        });
    });
    action
}
