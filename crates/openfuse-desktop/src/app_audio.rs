use super::Editor;
use crate::audio_view::Action;
use eframe::egui;
impl Editor {
    pub(super) fn audio_restore_preferences(&mut self, storage: Option<&dyn eframe::Storage>) {
        if !self.persist_library || self.audio_ui.demo {
            return;
        }
        if let Some(text) =
            storage.and_then(|storage| storage.get_string(crate::audio_state::SETTINGS_KEY))
            && let Some(settings) = self.audio_ui.restore(&text)
            && let Err(error) = self.media_ui.player.configure_output(settings)
        {
            self.audio_ui.error = Some(error);
        }
    }
    pub(super) fn open_audio(&mut self) {
        self.audio_ui.open = true;
    }
    pub(super) fn audio_update(&mut self, ctx: &egui::Context) {
        self.audio_ui.poll();
        if self.audio_ui.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if ctx.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::A,
            )
        }) {
            self.open_audio();
            self.audio_action(Action::SystemPanel);
        }
    }
    pub(super) fn audio_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        if let Some(action) = crate::audio_view::show(
            ctx,
            &mut self.audio_ui,
            self.media_ui.player.output_settings(),
            self.media_ui.player.active_output(),
        ) {
            self.audio_action(action);
        }
    }
    pub(super) fn audio_action(&mut self, action: Action) {
        match action {
            Action::Refresh => self.audio_ui.refresh(),
            Action::SystemPanel => self.audio_ui.open_system_panel(),
            Action::Close => self.audio_ui.open = false,
            Action::Reset => {
                self.audio_ui.draft = Default::default();
                self.audio_ui.error = None;
                self.audio_ui.notice = Some(
                    "Default output is now a draft. Apply to pause playback and save it.".into(),
                );
            }
            Action::Revert => {
                self.audio_ui.draft = self.media_ui.player.output_settings().clone();
                self.audio_ui.error = None;
            }
            Action::Apply => {
                let settings = self.audio_ui.draft.clone();
                match self
                    .audio_ui
                    .validate_draft()
                    .and_then(|()| self.media_ui.player.configure_output(settings.clone()))
                {
                    Ok(()) => self.audio_ui.committed(&settings),
                    Err(error) => self.audio_ui.error = Some(error),
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_dialog_render_and_draft_changes_never_open_devices_or_dirty_presets() {
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
        app.open_audio();
        for _ in 0..3 {
            let _ = ctx.run(Default::default(), |ctx| app.audio_dialog(ctx));
        }
        assert!(app.audio_ui.inventory.is_none());
        assert!(!app.audio_ui.busy());
        assert!(app.media_ui.player.active_output().is_none());
        app.audio_ui.draft.sample_rate = Some(96000);
        app.audio_action(Action::Revert);
        assert_eq!(app.audio_ui.draft, *app.media_ui.player.output_settings());
        app.audio_action(Action::Apply);
        assert!(app.media_ui.player.active_output().is_none());
        assert_eq!(
            before,
            app.session.as_ref().unwrap().document.to_xml().unwrap()
        );
        assert!(!app.dirty());
    }
    #[test]
    fn demo_inventory_apply_keeps_audio_closed_and_rejects_unsupported_rate() {
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            crate::Options {
                audio_settings_demo: true,
                ..Default::default()
            },
            &ctx,
            None,
        );
        assert!(app.audio_ui.open);
        assert!(app.audio_ui.inventory.is_some());
        app.audio_ui.draft.sample_rate = Some(192000);
        app.audio_action(Action::Apply);
        assert!(app.audio_ui.error.is_some());
        assert_eq!(*app.media_ui.player.output_settings(), Default::default());
        app.audio_ui.draft.sample_rate = Some(48000);
        app.audio_ui.draft.buffer_frames = Some(256);
        app.audio_action(Action::Apply);
        assert!(app.audio_ui.error.is_none());
        assert_eq!(
            app.media_ui.player.output_settings().buffer_frames,
            Some(256)
        );
        assert!(app.media_ui.player.active_output().is_none());
    }
}
