//! The library panel returns explicit actions; the app owns dirty-document guards.
use crate::{
    library_state::{LibraryState, Sort},
    theme,
};
use eframe::egui::{self, RichText};
use openfuse_core::library::{ArchiveToken, LibraryEntry};
use std::path::Path;

pub enum Action {
    ChooseFolder,
    Refresh,
    Import,
    Open(LibraryEntry),
    ImportModules(LibraryEntry),
    Duplicate(LibraryEntry),
    DuplicateMany(Vec<LibraryEntry>),
    Archive(LibraryEntry),
    ArchiveMany(Vec<LibraryEntry>),
    Restore(ArchiveToken),
}

pub fn panel(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    active: Option<&Path>,
    has_destination: bool,
) -> Option<Action> {
    let mut action = None;
    let active_path = active.map(|path| path.canonicalize().unwrap_or_else(|_| path.to_owned()));
    let active = active_path.as_deref();
    if let Some(folder) = &state.folder {
        let name = folder
            .file_name()
            .unwrap_or(folder.as_os_str())
            .to_string_lossy();
        ui.label(RichText::new(name).color(theme::muted(ui.ctx())))
            .on_hover_text(folder.display().to_string());
    } else {
        ui.label("Keep your presets together in a local folder.");
    }
    ui.horizontal(|ui| {
        let folder = ui.button("Folder…");
        remember(&folder, "folder");
        if folder.clicked() {
            action = Some(Action::ChooseFolder);
        }
        if ui
            .add_enabled(
                state.folder.is_some() && !state.busy(),
                egui::Button::new("Refresh"),
            )
            .clicked()
        {
            action = Some(Action::Refresh);
        }
    });
    if state.folder.is_none() {
        return action;
    }
    if ui
        .add_enabled(!state.busy(), egui::Button::new("Import presets…"))
        .clicked()
    {
        action = Some(Action::Import);
    }
    ui.add_space(6.0);
    let query = ui.add(
        egui::TextEdit::singleline(&mut state.query)
            .hint_text("Search name, author, tags…")
            .min_size(egui::vec2(0.0, 30.0))
            .margin(egui::vec2(8.0, 7.0))
            .desired_width(f32::INFINITY),
    );
    remember(&query, "query");
    egui::ComboBox::from_id_salt("library-sort")
        .width(210.0)
        .selected_text(format!("Sort: {}", state.sort.label()))
        .show_ui(ui, |ui| {
            for sort in [Sort::Name, Sort::Author, Sort::Product, Sort::File] {
                ui.selectable_value(&mut state.sort, sort, sort.label());
            }
        });
    let shown_paths: Vec<_> = state
        .entries()
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    let mut all_shown =
        !shown_paths.is_empty() && shown_paths.iter().all(|path| state.marked.contains(path));
    ui.horizontal(|ui| {
        let select_all = ui.checkbox(&mut all_shown, "Select shown");
        remember(&select_all, "select-all");
        if select_all.changed() {
            for path in &shown_paths {
                if all_shown {
                    state.marked.insert(path.clone());
                } else {
                    state.marked.remove(path);
                }
            }
        }
        if !state.marked.is_empty() && ui.button("Clear").clicked() {
            state.marked.clear();
        }
    });
    if !state.marked.is_empty() {
        let active_selected = active.is_some_and(|path| state.marked.contains(path));
        ui.label(
            RichText::new(format!("{} selected", state.marked.len()))
                .size(12.0)
                .color(theme::accent(ui.ctx())),
        );
        ui.horizontal(|ui| {
            let duplicate = ui.add_enabled(!state.busy(), egui::Button::new("Duplicate"));
            remember(&duplicate, "duplicate-many");
            if duplicate.clicked() {
                action = Some(Action::DuplicateMany(state.marked_entries()));
            }
            let archive = ui.add_enabled(
                !state.busy() && !active_selected,
                egui::Button::new("Archive…"),
            );
            remember(&archive, "archive-many");
            if archive
                .on_disabled_hover_text(
                    "Deselect the active preset or open another preset before archiving.",
                )
                .clicked()
            {
                action = Some(Action::ArchiveMany(state.marked_entries()));
            }
        });
    }
    let entries = state.entries();
    let total = state.scan.as_ref().map(|s| s.entries.len()).unwrap_or(0);
    ui.label(
        RichText::new(format!("{} of {total} presets", entries.len()))
            .size(12.0)
            .color(theme::muted(ui.ctx())),
    );
    let busy = state.busy();
    let selected = state.selected.clone();
    let mut next_selected = None;
    let mut marked_changes = Vec::new();
    if entries.is_empty() && !busy {
        ui.label("No matching presets.");
    }
    egui::ScrollArea::vertical()
        .id_salt("library-items")
        .max_height(240.0)
        .show_rows(ui, 48.0, entries.len(), |ui, range| {
            for entry in entries[range].iter().copied() {
                let title = if entry.metadata.name.is_empty() {
                    entry.relative_path.to_string_lossy().into_owned()
                } else {
                    entry.metadata.name.clone()
                };
                let author = if entry.metadata.author.is_empty() {
                    entry.relative_path.to_string_lossy().into_owned()
                } else {
                    entry.metadata.author.clone()
                };
                let is_active = active == Some(entry.path.as_path());
                let response = ui
                    .horizontal(|ui| {
                        let mut marked = state.marked.contains(&entry.path);
                        let checkbox = ui.checkbox(&mut marked, "");
                        remember(
                            &checkbox,
                            &format!("mark:{}", entry.relative_path.display()),
                        );
                        if checkbox.changed() {
                            marked_changes.push((entry.path.clone(), marked));
                        }
                        ui.add_sized(
                            [ui.available_width(), 48.0],
                            egui::Button::new(format!("{title}\n{author}"))
                                .selected(is_active || selected.as_ref() == Some(&entry.path)),
                        )
                    })
                    .inner;
                remember(
                    &response,
                    &format!("entry:{}", entry.relative_path.display()),
                );
                if response.clicked() {
                    next_selected = Some(entry.path.clone());
                    action = Some(Action::Open(entry.clone()));
                }
                response
                    .on_hover_text(format!(
                        "{}\nRight-click for duplicate and archive",
                        entry.path.display()
                    ))
                    .context_menu(|ui| {
                        if ui
                            .add_enabled(has_destination, egui::Button::new("Import modules…"))
                            .clicked()
                        {
                            action = Some(Action::ImportModules(entry.clone()));
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("Duplicate"))
                            .clicked()
                        {
                            action = Some(Action::Duplicate(entry.clone()));
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(!busy && !is_active, egui::Button::new("Archive…"))
                            .on_disabled_hover_text(
                                "Open another preset before archiving this file.",
                            )
                            .clicked()
                        {
                            action = Some(Action::Archive(entry.clone()));
                            ui.close_menu();
                        }
                    });
            }
        });
    for (path, marked) in marked_changes {
        if marked {
            state.marked.insert(path);
        } else {
            state.marked.remove(&path);
        }
    }
    if let Some(selected) = next_selected {
        state.selected = Some(selected);
    }
    ui.label(
        RichText::new("Right-click a preset to duplicate or archive it.")
            .size(11.0)
            .color(theme::muted(ui.ctx())),
    );
    if let Some(scan) = &state.scan {
        if scan.truncated {
            ui.colored_label(
                theme::accent(ui.ctx()),
                "Index limit reached. Choose a smaller folder.",
            );
        }
        if !scan.diagnostics.is_empty() {
            egui::CollapsingHeader::new(format!("{} files need attention", scan.diagnostics.len()))
                .show(ui, |ui| {
                    for diagnostic in &scan.diagnostics {
                        ui.label(format!(
                            "{}: {}",
                            diagnostic.path.display(),
                            diagnostic.message
                        ));
                    }
                });
        }
    }
    if let Some(error) = &state.error {
        ui.colored_label(theme::accent(ui.ctx()), error);
    }
    if !state.notices.is_empty() {
        egui::CollapsingHeader::new("Latest activity")
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("library-activity")
                    .max_height(150.0)
                    .show_rows(ui, 18.0, state.notices.len(), |ui, range| {
                        for notice in &state.notices[range] {
                            ui.add(
                                egui::Label::new(RichText::new(notice).size(12.0))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    });
            });
    }
    egui::CollapsingHeader::new(format!("Archived presets ({})", state.archives.len())).show(
        ui,
        |ui| {
            if state.archives.is_empty() {
                ui.label("No archived presets.");
            }
            for token in &state.archives {
                ui.label(token.original_relative_path.display().to_string());
                if ui
                    .add_enabled(!busy, egui::Button::new("Restore"))
                    .clicked()
                {
                    action = Some(Action::Restore(token.clone()));
                }
            }
        },
    );
    action
}

fn remember(response: &egui::Response, key: &str) {
    theme::remember(response, &format!("library-{key}"));
    #[cfg(test)]
    response
        .ctx
        .data_mut(|data| data.insert_temp(egui::Id::new(("library-control", key)), response.rect));
    #[cfg(not(test))]
    let _ = (response, key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn frame(
        ctx: &egui::Context,
        state: &mut LibraryState,
        events: Vec<egui::Event>,
    ) -> Option<Action> {
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(310.0, 1200.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = panel(ui, state, None, true);
                });
            },
        );
        action
    }
    fn click(ctx: &egui::Context, state: &mut LibraryState, key: &str) -> Option<Action> {
        let rect: egui::Rect = ctx.data(|data| {
            data.get_temp(egui::Id::new(("library-control", key)))
                .unwrap()
        });
        let pos = rect.center();
        frame(
            ctx,
            state,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            ctx,
            state,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    #[test]
    fn pointer_actions_open_folder_and_filtered_indexed_preset() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = LibraryState::default();
        frame(&ctx, &mut state, Vec::new());
        assert!(matches!(
            click(&ctx, &mut state, "folder"),
            Some(Action::ChooseFolder)
        ));
        let directory = tempfile::tempdir().unwrap();
        for name in ["Alpha", "Beta"] {
            std::fs::write(
                directory.path().join(format!("{name}.fuse")),
                format!("<Preset><FUSE><Info name=\"{name}\" author=\"Player\"/></FUSE></Preset>"),
            )
            .unwrap();
        }
        state.set_folder(directory.path().to_owned());
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.busy() && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!state.busy());
        frame(&ctx, &mut state, Vec::new());
        click(&ctx, &mut state, "query");
        frame(&ctx, &mut state, vec![egui::Event::Text("Beta".into())]);
        assert_eq!(state.query, "Beta");
        assert_eq!(state.entries().len(), 1);
        frame(&ctx, &mut state, Vec::new());
        match click(&ctx, &mut state, "entry:Beta.fuse") {
            Some(Action::Open(entry)) => assert_eq!(entry.metadata.name, "Beta"),
            _ => panic!("preset pointer click did not produce Open"),
        }
        state.query.clear();
        frame(&ctx, &mut state, Vec::new());
        click(&ctx, &mut state, "mark:Alpha.fuse");
        assert_eq!(state.marked.len(), 1);
        frame(&ctx, &mut state, Vec::new());
        click(&ctx, &mut state, "mark:Beta.fuse");
        assert_eq!(state.marked.len(), 2);
        frame(&ctx, &mut state, Vec::new());
        match click(&ctx, &mut state, "duplicate-many") {
            Some(Action::DuplicateMany(entries)) => assert_eq!(entries.len(), 2),
            _ => panic!("batch duplicate did not receive selected entries"),
        }
        match click(&ctx, &mut state, "archive-many") {
            Some(Action::ArchiveMany(entries)) => assert_eq!(entries.len(), 2),
            _ => panic!("batch archive did not receive selected entries"),
        }
    }
}
