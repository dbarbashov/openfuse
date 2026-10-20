//! External file requests use the same unsaved-preset guard as Open.
use super::{Editor, Pending};
use eframe::egui;
use std::path::PathBuf;
impl Editor {
    fn document_open_blocked(&self) -> bool {
        self.pending.is_some()
            || self.new_dialog
            || self.archive_pending.is_some()
            || self.import_draft.is_some()
            || self.conversion_draft.is_some()
            || self.audio_ui.open
            || self.media_ui.open
            || self.storage_ui.open
            || self.device_dialog
            || self.read_draft.is_some()
            || self.audition_draft.is_some()
            || self.bank_ui.modal_open()
            || self.controls_ui.modal_open()
            || self.archive_ui.modal_open()
            || self.auxiliary_ui.modal_open()
            || self.control_ui.open
    }
    pub(super) fn document_events(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        if !dropped.is_empty() {
            if self.document_open_blocked() {
                self.error=Some("Finish the current dialog, then drop the preset again. Your pending action was preserved.".into());
            } else if dropped.len() != 1 {
                self.error = Some("Open one preset at a time.".into());
            } else if let Some(path) = dropped[0].path.clone() {
                self.open_external_document(path, ctx);
            } else {
                self.error = Some("This drop does not provide a local file path.".into());
            }
            return;
        }
        if !self.document_open_blocked()
            && let Some(result) = crate::document_events::take()
        {
            match result {
                Ok(path) => self.open_external_document(path, ctx),
                Err(error) => self.error = Some(error),
            }
        }
    }
    fn open_external_document(&mut self, path: PathBuf, ctx: &egui::Context) {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("fuse") || extension.eq_ignore_ascii_case("xml")
            })
        {
            self.error = Some("Open a local .FUSE or .xml preset file.".into());
            return;
        }
        self.wait_for_document_open = false;
        self.request(Pending::Open(path), ctx);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_gate_waits_through_invalid_and_blocked_drops_until_admission() {
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            crate::Options {
                demo: true,
                wait_for_document_open: true,
                screenshot: Some(PathBuf::from("unused-smoke.png")),
                ..Default::default()
            },
            &ctx,
            None,
        );
        app.frames = 20;
        let _ = ctx.run(Default::default(), |ctx| app.capture(ctx));
        assert!(!app.capture_requested);
        app.open_external_document(PathBuf::from("invalid.bin"), &ctx);
        assert!(app.wait_for_document_open);
        let drop = || egui::RawInput {
            dropped_files: vec![egui::DroppedFile {
                path: Some(PathBuf::from("admitted.FUSE")),
                ..Default::default()
            }],
            ..Default::default()
        };
        app.audio_ui.open = true;
        let _ = ctx.run(drop(), |ctx| app.document_events(ctx));
        assert!(app.wait_for_document_open);
        assert!(app.pending.is_none());
        app.audio_ui.open = false;
        let mut metadata = app.session.as_ref().unwrap().document.metadata();
        metadata.name = "Unsaved smoke preset".into();
        app.session.as_mut().unwrap().metadata(&metadata).unwrap();
        let _ = ctx.run(drop(), |ctx| app.document_events(ctx));
        assert!(!app.wait_for_document_open);
        assert!(
            matches!(&app.pending, Some(Pending::Open(path)) if path == &PathBuf::from("admitted.FUSE"))
        );
        let _ = ctx.run(
            egui::RawInput {
                time: Some(1.0),
                ..Default::default()
            },
            |ctx| app.capture(ctx),
        );
        assert!(!app.capture_requested, "modal must settle before capture");
        let _ = ctx.run(
            egui::RawInput {
                time: Some(1.2),
                ..Default::default()
            },
            |ctx| app.capture(ctx),
        );
        assert!(!app.capture_requested);
        let _ = ctx.run(
            egui::RawInput {
                time: Some(1.36),
                ..Default::default()
            },
            |ctx| app.capture(ctx),
        );
        assert!(app.capture_requested);
    }
    #[test]
    fn dropped_file_uses_unsaved_guard_and_does_not_replace_pending_action() {
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            crate::Options {
                demo: true,
                ..Default::default()
            },
            &ctx,
            None,
        );
        let mut metadata = app.session.as_ref().unwrap().document.metadata();
        metadata.name = "Unsaved".into();
        app.session.as_mut().unwrap().metadata(&metadata).unwrap();
        let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
        app.open_external_document(PathBuf::from("requested.FUSE"), &ctx);
        assert!(
            matches!(&app.pending,Some(Pending::Open(path)) if path==&PathBuf::from("requested.FUSE"))
        );
        let input = egui::RawInput {
            dropped_files: vec![egui::DroppedFile {
                path: Some(PathBuf::from("other.fuse")),
                ..Default::default()
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.document_events(ctx));
        assert!(
            matches!(&app.pending,Some(Pending::Open(path)) if path==&PathBuf::from("requested.FUSE"))
        );
        assert_eq!(
            before,
            app.session.as_ref().unwrap().document.to_xml().unwrap()
        );
    }
    #[test]
    fn invalid_extension_and_multiple_drops_leave_preset_unchanged() {
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            crate::Options {
                demo: true,
                ..Default::default()
            },
            &ctx,
            None,
        );
        let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
        app.open_external_document(PathBuf::from("firmware.bin"), &ctx);
        assert!(app.error.is_some());
        let input = egui::RawInput {
            dropped_files: vec![
                egui::DroppedFile {
                    path: Some(PathBuf::from("one.fuse")),
                    ..Default::default()
                },
                egui::DroppedFile {
                    path: Some(PathBuf::from("two.fuse")),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.document_events(ctx));
        assert!(app.error.as_ref().unwrap().contains("one preset"));
        assert_eq!(
            before,
            app.session.as_ref().unwrap().document.to_xml().unwrap()
        );
    }
}
