use super::{Editor, Pending};
use crate::{
    library_state::{Operation, same_file},
    library_view::Action,
};
use eframe::egui;

impl Editor {
    pub(super) fn library_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::ChooseFolder => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Choose preset library folder")
                    .pick_folder()
                {
                    self.library.set_folder(folder);
                }
            }
            Action::Refresh => self.library.refresh(),
            Action::Import => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("FUSE presets", &["fuse", "FUSE", "xml"])
                    .pick_files()
                {
                    self.library.start(Operation::Import(paths));
                }
            }
            Action::Open(entry) => {
                if let Some(folder) = &self.library.folder {
                    match openfuse_core::library::PresetLibrary::open(folder) {
                        Ok(library) => {
                            self.request(Pending::OpenIndexed(library, Box::new(entry)), ctx)
                        }
                        Err(error) => self.library.error = Some(error.to_string()),
                    }
                }
            }
            Action::ImportModules(entry) => {
                if let Some(folder) = &self.library.folder {
                    match openfuse_core::library::PresetLibrary::open(folder)
                        .and_then(|library| library.read(&entry))
                    {
                        Ok(source) => self.prepare_module_import(source, entry.path),
                        Err(error) => self.library.error = Some(error.to_string()),
                    }
                }
            }
            Action::Duplicate(entry) => self.library.start(Operation::Duplicate(vec![entry])),
            Action::DuplicateMany(entries) => self.library.start(Operation::Duplicate(entries)),
            Action::Archive(entry) => self.request_archive(vec![entry]),
            Action::ArchiveMany(entries) => self.request_archive(entries),
            Action::Restore(token) => self.library.start(Operation::Restore(token)),
        }
    }

    fn includes_active_preset(&self, entries: &[openfuse_core::library::LibraryEntry]) -> bool {
        self.session
            .as_ref()
            .and_then(|session| session.path.as_ref())
            .is_some_and(|active| entries.iter().any(|entry| same_file(active, &entry.path)))
    }

    fn request_archive(&mut self, entries: Vec<openfuse_core::library::LibraryEntry>) {
        if entries.is_empty() {
            return;
        }
        if self.includes_active_preset(&entries) {
            self.library.error = Some("Selection includes the active preset. Deselect it or open another preset before archiving.".into());
        } else {
            self.archive_pending = Some(entries);
        }
    }

    fn submit_archive(&mut self, entries: Vec<openfuse_core::library::LibraryEntry>) {
        if self.includes_active_preset(&entries) {
            self.library.error =
                Some("Cannot archive a selection containing the active preset.".into());
        } else {
            self.library.start(Operation::Archive(entries));
        }
    }

    pub(super) fn archive_dialog(&mut self, ctx: &egui::Context) {
        let Some(entries) = self.archive_pending.clone() else {
            return;
        };
        crate::dialog::show(ctx, "archive-preset", 480.0, |ui| {
            ui.heading(format!(
                "Archive {} preset{}?",
                entries.len(),
                if entries.len() == 1 { "" } else { "s" }
            ));
            ui.label("These files will move to this library’s archive. You can restore each one from Archived presets.");
            egui::ScrollArea::both()
                .id_salt("archive-confirm-paths")
                .max_height(220.0)
                .show_rows(ui, 20.0, entries.len(), |ui, range| {
                    for entry in &entries[range] {
                        ui.add(
                            egui::Label::new(entry.path.display().to_string())
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    }
                });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let archive = ui.button(format!("Archive {}", entries.len()));
                #[cfg(test)]
                ctx.data_mut(|data| {
                    data.insert_temp(egui::Id::new("archive-confirm-button"), archive.rect)
                });
                if archive.clicked() {
                    self.archive_pending = None;
                    self.submit_archive(entries.clone());
                }
                if ui.button("Cancel").clicked() {
                    self.archive_pending = None;
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, session::Session};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct MemoryStorage(std::collections::BTreeMap<String, String>);
    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn folder_preferences_restore_and_explicit_screenshot_folder_is_isolated() {
        let folder = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            Options {
                library: Some(folder.path().to_owned()),
                ..Default::default()
            },
            &ctx,
            None,
        );
        settle(&mut app);
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);
        let saved = storage
            .0
            .get(crate::library_state::FOLDER_KEY)
            .map(std::path::PathBuf::from);
        let mut restored = Editor::from_options(Options::default(), &ctx, saved.clone());
        settle(&mut restored);
        assert_eq!(restored.library.folder.as_deref(), Some(folder.path()));
        let mut capture = Editor::from_options(
            Options {
                library: Some(other.path().to_owned()),
                screenshot: Some(other.path().join("capture.png")),
                ..Default::default()
            },
            &ctx,
            saved,
        );
        settle(&mut capture);
        assert_eq!(capture.library.folder.as_deref(), Some(other.path()));
        eframe::App::save(&mut capture, &mut storage);
        assert_eq!(
            storage.0[crate::library_state::FOLDER_KEY],
            folder.path().to_str().unwrap()
        );
    }

    fn settle(app: &mut Editor) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.library.busy() && Instant::now() < deadline {
            app.library.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!app.library.busy());
        assert!(app.library.error.is_none(), "{:?}", app.library.error);
    }
    #[test]
    fn indexed_open_respects_dirty_guard_save_refresh_and_active_archive_guard() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.fuse");
        let mut original = Session::example().unwrap();
        original.save_as(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            Options {
                library: Some(directory.path().to_owned()),
                ..Default::default()
            },
            &ctx,
            None,
        );
        settle(&mut app);
        let entry = app.library.entries()[0].clone();
        app.session = Some(Session::new_preset(13).unwrap());
        app.library_action(Action::Open(entry.clone()), &ctx);
        assert!(matches!(app.pending, Some(Pending::OpenIndexed(_, _))));
        assert_eq!(
            app.session.as_ref().unwrap().document.metadata().name,
            "Untitled preset"
        );
        let pending = app.pending.take().unwrap();
        app.perform(pending, &ctx);
        app.library_action(Action::Archive(entry), &ctx);
        assert!(app.archive_pending.is_none());
        assert!(app.library.error.as_ref().unwrap().contains("active"));
        let session = app.session.as_mut().unwrap();
        let mut metadata = session.document.metadata();
        metadata.name = "Edited from library".into();
        session.metadata(&metadata).unwrap();
        assert!(app.save());
        app.library.refresh();
        settle(&mut app);
        assert_eq!(
            app.library.entries()[0].metadata.name,
            "Edited from library"
        );
        assert!(!app.session.as_ref().unwrap().dirty());
    }

    #[test]
    fn indexed_open_revalidates_after_unsaved_changes_dialog() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preset.fuse");
        let mut file = Session::example().unwrap();
        file.save_as(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            Options {
                library: Some(directory.path().to_owned()),
                ..Default::default()
            },
            &ctx,
            None,
        );
        settle(&mut app);
        let entry = app.library.entries()[0].clone();
        app.session = Some(Session::new_preset(13).unwrap());
        let unsaved = app.session.as_ref().unwrap().document.to_xml().unwrap();
        app.library_action(Action::Open(entry), &ctx);
        let mut metadata = file.document.metadata();
        metadata.name = "Changed externally during confirmation".into();
        file.metadata(&metadata).unwrap();
        file.save_as(&path).unwrap();
        let pending = app.pending.take().expect("dirty guard pending");
        app.perform(pending, &ctx);
        assert!(app.error.is_some());
        assert!(app.dirty());
        assert_eq!(
            app.session.as_ref().unwrap().document.to_xml().unwrap(),
            unsaved
        );
    }
    #[test]
    fn batch_archive_guards_active_file_and_reports_stale_entry_without_losing_successes() {
        let directory = tempfile::tempdir().unwrap();
        for filename in ["active.fuse", "fresh.fuse", "stale.fuse"] {
            Session::example()
                .unwrap()
                .save_as(&directory.path().join(filename))
                .unwrap();
        }
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            Options {
                library: Some(directory.path().to_owned()),
                ..Default::default()
            },
            &ctx,
            None,
        );
        settle(&mut app);
        let entries: Vec<_> = app.library.entries().into_iter().cloned().collect();
        app.session = Some(Session::open(&directory.path().join("active.fuse")).unwrap());
        app.library_action(Action::ArchiveMany(entries.clone()), &ctx);
        assert!(app.archive_pending.is_none());
        assert!(app.library.error.as_ref().unwrap().contains("active"));
        assert!(entries.iter().all(|entry| entry.path.exists()));
        let batch: Vec<_> = entries
            .into_iter()
            .filter(|entry| entry.relative_path != std::path::Path::new("active.fuse"))
            .collect();
        app.library_action(Action::ArchiveMany(batch.clone()), &ctx);
        assert_eq!(app.archive_pending.as_ref().unwrap().len(), 2);
        assert!(batch.iter().all(|entry| entry.path.exists()));
        let confirmation = app.archive_pending.take().unwrap();
        app.session = Some(Session::open(&directory.path().join("fresh.fuse")).unwrap());
        app.submit_archive(confirmation);
        assert!(app.library.error.as_ref().unwrap().contains("active"));
        assert!(batch.iter().all(|entry| entry.path.exists()));
        app.session = Some(Session::open(&directory.path().join("active.fuse")).unwrap());
        app.library_action(Action::ArchiveMany(batch.clone()), &ctx);
        let stale_path = directory.path().join("stale.fuse");
        let mut changed = Session::open(&stale_path).unwrap();
        let mut metadata = changed.document.metadata();
        metadata.name = "Changed during confirmation".into();
        changed.metadata(&metadata).unwrap();
        changed.save_as(&stale_path).unwrap();
        let mut draw = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.archive_dialog(ctx),
            );
        };
        draw(Vec::new());
        draw(Vec::new());
        let rect: egui::Rect = ctx.data(|data| {
            data.get_temp(egui::Id::new("archive-confirm-button"))
                .unwrap()
        });
        let pos = rect.center();
        for pressed in [true, false] {
            draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert!(app.archive_pending.is_none());
        settle(&mut app);
        assert!(directory.path().join("active.fuse").exists());
        assert!(!directory.path().join("fresh.fuse").exists());
        assert!(stale_path.exists());
        assert_eq!(app.library.archives.len(), 1);
        assert!(
            app.library
                .notices
                .iter()
                .any(|notice| notice.contains("Archived fresh.fuse"))
        );
        assert!(
            app.library
                .notices
                .iter()
                .any(|notice| notice.contains("Archive failed for stale.fuse"))
        );
    }
}

#[cfg(test)]
mod dirty_dialog_tests {
    use super::*;
    use crate::{Options, session::Session};

    fn frame(ctx: &egui::Context, app: &mut Editor, mut input: egui::RawInput) -> egui::FullOutput {
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1220.0, 930.0),
        ));
        ctx.run(input, |ctx| {
            eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest());
        })
    }
    fn click(ctx: &egui::Context, app: &mut Editor, key: &str) {
        let rect: egui::Rect = ctx.data(|data| data.get_temp(egui::Id::new(key)).unwrap());
        let pos = rect.center();
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
            );
        }
    }
    #[test]
    fn modal_cancel_retains_unsaved_work_and_discard_opens_requested_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("destination.fuse");
        Session::example().unwrap().save_as(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(Options::default(), &ctx, None);
        app.session = Some(Session::new_preset(13).unwrap());
        let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
        app.request(Pending::Open(path.clone()), &ctx);
        for _ in 0..2 {
            frame(&ctx, &mut app, egui::RawInput::default());
        }
        click(&ctx, &mut app, "cancel-button");
        assert!(app.pending.is_none());
        assert!(app.dirty());
        assert_eq!(
            app.session.as_ref().unwrap().document.to_xml().unwrap(),
            before
        );
        app.request(Pending::Open(path.clone()), &ctx);
        for _ in 0..2 {
            frame(&ctx, &mut app, egui::RawInput::default());
        }
        click(&ctx, &mut app, "discard-button");
        assert!(app.pending.is_none());
        assert!(!app.dirty());
        assert_eq!(app.session.as_ref().unwrap().path.as_ref(), Some(&path));
        assert_eq!(
            app.session.as_ref().unwrap().document.metadata().name,
            "Warm room"
        );
    }
    #[test]
    fn window_close_is_cancelled_while_unsaved_work_needs_a_decision() {
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(Options::default(), &ctx, None);
        app.session = Some(Session::new_preset(13).unwrap());
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let output = frame(&ctx, &mut app, input);
        assert!(matches!(app.pending, Some(Pending::Close)));
        assert!(!app.allow_close);
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::CancelClose))
        );
    }
}
