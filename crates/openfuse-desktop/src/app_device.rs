use super::Editor;
use crate::device_view::Action;
use eframe::egui;

impl Editor {
    pub(super) fn device_action(&mut self, action: Action) {
        match action {
            Action::SelectSlot(slot) => self.select_device_slot(slot),
            Action::Open => self.device_dialog = true,
            Action::Close => self.device_dialog = false,
            Action::Refresh => self.device.refresh(),
            Action::Connect => self.device.connect(),
            Action::OpenCapture => self.choose_capture(),
            Action::ExportCapture => self.export_capture(),
            Action::OpenRead => self.prepare_device_read(),
            Action::Disconnect => {
                self.cancel_audition();
                self.disable_live();
                self.device.disconnect();
            }
            Action::ReadCurrent => {
                self.cancel_audition();
                self.disable_live();
                self.device.read_current();
            }
        }
    }
    pub(super) fn device_dialog(&mut self, ctx: &egui::Context) {
        if !self.device_dialog
            || self.pending.is_some()
            || self.new_dialog
            || self.archive_pending.is_some()
            || self.import_draft.is_some()
            || self.read_draft.is_some()
            || self.audition_draft.is_some()
            || self.archive_ui.modal_open()
            || self.auxiliary_ui.modal_open()
            || self.controls_ui.modal_open()
            || self.bank_ui.modal_open()
        {
            return;
        }
        if let Some(action) = crate::device_view::dialog(ctx, &mut self.device) {
            self.device_action(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, device_state::DeviceState, session::Session, theme};
    use openfuse_device::{DeviceEvent, DeviceInfo, DeviceStatus, SnapshotSummary, WireCapture};
    use std::time::{Duration, Instant};

    fn draw(app: &mut Editor, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 850.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.device_dialog(ctx),
        );
    }
    fn click(app: &mut Editor, ctx: &egui::Context, name: &str) {
        draw(app, ctx, Vec::new());
        draw(app, ctx, Vec::new());
        let (rect, _): (egui::Rect, bool) = ctx.data(|data| {
            data.get_temp(egui::Id::new(("usb-control", name)))
                .expect("device control rendered")
        });
        for pressed in [true, false] {
            draw(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    fn settle(app: &mut Editor, ready: impl Fn(&DeviceState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.device.poll();
            if ready(&app.device) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "USB state did not settle: {} {:?}",
            app.device.phase(),
            app.device.error
        );
    }
    fn saved_dirty_session() -> (tempfile::TempDir, Session) {
        let folder = tempfile::tempdir().unwrap();
        let mut session = Session::example().unwrap();
        session.save_as(&folder.path().join("local.fuse")).unwrap();
        let mut metadata = session.document.metadata();
        metadata.name = "My unsaved local changes".into();
        session.metadata(&metadata).unwrap();
        (folder, session)
    }
    #[test]
    fn default_start_and_passive_render_do_not_enumerate_or_change_dirty_local_document() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = Editor::from_options(Options::default(), &ctx, None);
        let (_folder, session) = saved_dirty_session();
        app.session = Some(session);
        let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
        app.device_action(Action::Open);
        for _ in 0..4 {
            app.device.poll();
            draw(&mut app, &ctx, Vec::new());
        }
        assert!(!app.device.enumerated);
        assert!(!app.device.enumerating);
        assert!(app.device.devices.is_empty());
        assert!(!app.device.has_connection());
        assert_eq!(
            app.session.as_ref().unwrap().document.to_xml().unwrap(),
            before
        );
        assert!(app.dirty());
        assert!(app.session.as_ref().unwrap().can_undo());
    }
    #[test]
    fn unsupported_device_pointer_selection_disables_connect_and_shows_reason() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = Editor::from_options(Options::default(), &ctx, None);
        app.device_dialog = true;
        let info = |id: &str, reason| DeviceInfo {
            id: id.into(),
            name: format!("Device {id}"),
            product_id: 65500,
            serial: None,
            interface_number: 0,
            unavailable_reason: reason,
        };
        app.device.apply_event(DeviceEvent::Devices(vec![
            info("available", None),
            info(
                "unsupported",
                Some("This bootloader interface has no read-session support".into()),
            ),
        ]));
        assert_eq!(app.device.selected.as_deref(), Some("available"));
        click(&mut app, &ctx, "device-unsupported");
        assert_eq!(app.device.selected.as_deref(), Some("unsupported"));
        draw(&mut app, &ctx, Vec::new());
        let (_, enabled): (egui::Rect, bool) = ctx.data(|data| {
            data.get_temp(egui::Id::new(("usb-control", "connect")))
                .unwrap()
        });
        assert!(!enabled);
        assert!(!app.device.can_connect());
        click(&mut app, &ctx, "connect");
        assert!(!app.device.has_connection());
        assert!(app.device.error.is_none());
        app.device_action(Action::Connect); // Dispatch also enforces the guard.
        assert!(app.device.error.as_ref().unwrap().contains("bootloader"));
        assert!(!app.device.has_connection());
    }
    #[test]
    fn simulated_pointer_read_and_disconnect_preserve_document_file_and_history() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = Editor::from_options(
            Options {
                usb_demo: true,
                ..Default::default()
            },
            &ctx,
            None,
        );
        let (_folder, session) = saved_dirty_session();
        app.session = Some(session);
        let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
        let disk_path = app.session.as_ref().unwrap().path.clone().unwrap();
        let disk_before = std::fs::read(&disk_path).unwrap();
        settle(&mut app, |state| {
            state.capture_ready() && state.status.snapshot.is_some()
        });
        let generation = app.device.status.snapshot.as_ref().unwrap().generation;
        click(&mut app, &ctx, "read");
        settle(&mut app, |state| {
            state.status.can_sync
                && state
                    .status
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.generation > generation)
        });
        assert_eq!(
            app.session.as_ref().unwrap().document.to_xml().unwrap(),
            before
        );
        assert_eq!(std::fs::read(&disk_path).unwrap(), disk_before);
        assert!(app.dirty());
        assert!(app.session.as_ref().unwrap().can_undo());
        app.history(true);
        assert!(!app.dirty());
        app.history(false);
        assert_eq!(
            app.session.as_ref().unwrap().document.to_xml().unwrap(),
            before
        );
        click(&mut app, &ctx, "disconnect");
        settle(&mut app, |state| state.phase() == "Disconnected");
        assert!(!app.device.has_connection());
        assert!(app.device.can_connect());
        click(&mut app, &ctx, "connect");
        settle(&mut app, |state| state.status.can_sync);
        assert!(app.device.has_connection());
        assert_eq!(
            app.session.as_ref().unwrap().document.to_xml().unwrap(),
            before
        );
    }
    #[test]
    fn status_snapshot_and_errors_stay_in_connection_panel_without_opening_a_preset() {
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(Options::default(), &ctx, None);
        app.device.apply_event(DeviceEvent::Status(DeviceStatus {
            phase: "Connected".into(),
            detail: "Synthetic status".into(),
            version: Some("2.0 build 42".into()),
            snapshot: Some(SnapshotSummary {
                generation: 1,
                reports: 8,
                name: Some("A device preset".into()),
                slot: Some(7),
                modules: 5,
            }),
            can_sync: true,
            reconnecting: false,
        }));
        app.device
            .apply_event(DeviceEvent::Error("Read timed out".into()));
        app.device.apply_event(DeviceEvent::Capture(WireCapture {
            device: DeviceInfo {
                id: "synthetic".into(),
                name: "Synthetic capture".into(),
                product_id: 20,
                serial: None,
                interface_number: 0,
                unavailable_reason: None,
            },
            generation: 1,
            version: None,
            reports: vec![vec![0; 64]],
            received: Vec::new(),
            dropped_received: 0,
        }));
        assert!(app.session.is_none());
        assert!(app.error.is_none());
        assert_eq!(app.device.error.as_deref(), Some("Read timed out"));
        app.device_dialog = true;
        draw(&mut app, &ctx, Vec::new());
        assert!(app.session.is_none());
    }
    #[test]
    fn explicit_disconnect_cancels_simulator_automation_before_inventory_arrives() {
        let ctx = egui::Context::default();
        let mut app = Editor::from_options(
            Options {
                usb_demo: true,
                ..Default::default()
            },
            &ctx,
            None,
        );
        app.device_action(Action::Disconnect);
        settle(&mut app, |state| {
            state.enumerated && state.phase() == "Disconnected"
        });
        for _ in 0..10 {
            app.device.poll();
        }
        assert!(!app.device.has_connection());
        assert!(!app.device.status.can_sync);
        assert!(app.device.status.snapshot.is_none());
    }

    #[cfg(not(feature = "hid"))]
    #[test]
    fn refresh_in_offline_build_reports_feature_hint_in_device_panel_only() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = Editor::from_options(Options::default(), &ctx, None);
        app.device_dialog = true;
        click(&mut app, &ctx, "refresh");
        settle(&mut app, |state| state.error.is_some());
        assert!(app.device.error.as_deref().unwrap().contains("hid"));
        assert!(app.error.is_none());
        assert!(!app.device.has_connection());
    }

    #[test]
    fn small_window_keeps_connection_buttons_accessible_with_many_devices() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut app = Editor::from_options(Options::default(), &ctx, None);
        app.device_dialog = true;
        app.device.apply_event(DeviceEvent::Devices(
            (0..12)
                .map(|index| DeviceInfo {
                    id: format!("device-{index}"),
                    name: format!("Unsupported interface {index}"),
                    product_id: 3,
                    serial: None,
                    interface_number: index,
                    unavailable_reason: Some(
                        "This interface has no supported current-preset read protocol".into(),
                    ),
                })
                .collect(),
        ));
        for _ in 0..3 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(940.0, 680.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.device_dialog(ctx),
            );
        }
        let (rect, enabled): (egui::Rect, bool) = ctx.data(|data| {
            data.get_temp(egui::Id::new(("usb-control", "close")))
                .unwrap()
        });
        assert!(enabled);
        assert!(rect.min.y >= 0.0 && rect.max.y <= 680.0, "{rect:?}");
        assert!(rect.height() >= 40.0);
    }
}
