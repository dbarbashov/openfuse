use super::Editor;
use crate::{
    module_import_view::{self, ImportDraft},
    session::Session,
};
use eframe::egui;
use openfuse_core::preset::PresetDocument;
use std::path::PathBuf;

impl Editor {
    pub(super) fn choose_module_import(&mut self) {
        if self.session.is_none() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Import modules from a preset")
            .add_filter("FUSE preset", &["fuse", "FUSE", "xml"])
            .pick_file()
        {
            match Session::open(&path) {
                Ok(source) => self.prepare_module_import(source.document, path),
                Err(error) => self.error = Some(error),
            }
        }
    }
    pub(super) fn prepare_module_import(&mut self, source: PresetDocument, path: PathBuf) {
        if let Some(destination) = &self.session {
            self.import_draft = Some(ImportDraft::new(
                source,
                path,
                &destination.document,
                &self.selected,
            ));
        }
    }
    pub(super) fn import_dialog(&mut self, ctx: &egui::Context) {
        let (Some(draft), Some(destination)) = (&mut self.import_draft, &self.session) else {
            return;
        };
        match module_import_view::show(ctx, draft, &destination.document) {
            Some(module_import_view::Action::Cancel) => self.import_draft = None,
            Some(module_import_view::Action::Apply) => self.apply_module_import(),
            None => {}
        }
    }
    fn apply_module_import(&mut self) {
        let (Some(draft), Some(destination)) = (&mut self.import_draft, &mut self.session) else {
            return;
        };
        let keys: Vec<_> = draft.selected.iter().map(String::as_str).collect();
        match destination.import_modules(&draft.source, &keys) {
            Ok(report) => {
                self.status = format!(
                    "Imported {} modules · report retained in editor",
                    report.imported.len()
                );
                self.last_import = Some(module_import_view::ImportReceipt {
                    source: draft.path.clone(),
                    report,
                });
                self.error = None;
                self.import_draft = None;
            }
            Err(error) => draft.error = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, theme};

    fn frame(ctx: &egui::Context, app: &mut Editor, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1220.0, 930.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest()),
        );
    }
    fn click(ctx: &egui::Context, app: &mut Editor, key: &str) {
        let rect: egui::Rect = ctx.data(|data| {
            data.get_temp(egui::Id::new(("module-import-control", key)))
                .unwrap()
        });
        let pos = rect.center();
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        for _ in 0..2 {
            frame(ctx, app, Vec::new());
        }
    }
    fn editor(ctx: &egui::Context) -> Editor {
        theme::install(ctx);
        let mut app = Editor::from_options(Options::default(), ctx, None);
        app.session = Some(Session::new_preset(13).unwrap());
        app.selected = "Stompbox".into();
        app.prepare_module_import(Session::example().unwrap().document, "source.fuse".into());
        for _ in 0..2 {
            frame(ctx, &mut app, Vec::new());
        }
        app
    }
    #[test]
    fn pointer_selection_imports_two_modules_with_one_undo_and_retained_report() {
        let ctx = egui::Context::default();
        let mut app = editor(&ctx);
        let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
        click(&ctx, &mut app, "Delay");
        assert_eq!(app.import_draft.as_ref().unwrap().selected.len(), 2);
        click(&ctx, &mut app, "apply");
        assert!(app.import_draft.is_none());
        let receipt = app.last_import.as_ref().unwrap();
        assert_eq!(receipt.report.imported, ["Stompbox", "Delay"]);
        assert!(!receipt.report.losses.is_empty());
        let session = app.session.as_mut().unwrap();
        let imported = session.document.to_xml().unwrap();
        assert_ne!(imported, before);
        session.undo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), before);
        session.redo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), imported);
    }
    #[test]
    fn cancel_and_failed_import_preserve_destination_and_keep_actionable_error() {
        let ctx = egui::Context::default();
        let mut app = editor(&ctx);
        let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
        click(&ctx, &mut app, "cancel");
        assert!(app.import_draft.is_none());
        assert_eq!(
            app.session.as_ref().unwrap().document.to_xml().unwrap(),
            before
        );
        let source_xml = Session::example()
            .unwrap()
            .document
            .to_xml()
            .unwrap()
            .replace("ProductId=\"13\"", "ProductId=\"65000\"");
        app.prepare_module_import(
            PresetDocument::parse(&source_xml).unwrap(),
            "unsupported.fuse".into(),
        );
        assert!(app.import_draft.as_ref().unwrap().selected.is_empty());
        app.import_draft
            .as_mut()
            .unwrap()
            .selected
            .insert("Amplifier".into());
        app.apply_module_import();
        assert!(app.import_draft.as_ref().unwrap().error.is_some());
        assert!(app.last_import.is_none());
        assert_eq!(
            app.session.as_ref().unwrap().document.to_xml().unwrap(),
            before
        );
        assert!(!app.session.as_ref().unwrap().can_undo());
    }
}
