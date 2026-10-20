use super::*;
use crate::{Options, device_view::Action, session::Session};
use openfuse_device::{DeviceEvent, LiveStatus};
use std::time::{Duration, Instant};

fn frame(app: &mut Editor, ctx: &egui::Context, events: Vec<egui::Event>) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1220.0, 930.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.live_panel(ui));
            app.read_dialog(ctx);
            app.confirm(ctx);
        },
    );
}
fn click(app: &mut Editor, ctx: &egui::Context, id: &str) {
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]);
    let rect: egui::Rect = ctx.data(|d| d.get_temp(egui::Id::new(id)).unwrap());
    for pressed in [true, false] {
        frame(
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
fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        app.device.poll();
        app.reconcile_live();
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "Device did not settle: {:?} {:?}",
        (&app.device.error, &app.error, app.live_requested),
        app.device.live
    );
}
fn simulator() -> (egui::Context, Editor) {
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
    pump(&mut app, |app| {
        app.device.capture.is_some() && app.device.status.can_sync
    });
    (ctx, app)
}
fn open(app: &mut Editor, ctx: &egui::Context) {
    app.prepare_device_read();
    assert!(app.read_draft.is_some(), "{:?}", app.error);
    click(app, ctx, "read-open");
    assert!(
        app.pending.is_some() || app.device_origin.is_some(),
        "read click failed: draft={} session={} err={:?}",
        app.read_draft.is_some(),
        app.session.is_some(),
        app.error
    );
}
#[test]
fn read_preview_cancel_then_discard_uses_fixed_snapshot_and_retains_raw_bytes() {
    let (ctx, mut app) = simulator();
    let mut session = Session::new_preset(13).unwrap();
    let mut metadata = session.document.metadata();
    metadata.name = "Unsaved local sound".into();
    session.metadata(&metadata).unwrap();
    app.session = Some(session);
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    open(&mut app, &ctx);
    assert!(app.pending.is_some());
    click(&mut app, &ctx, "cancel-button");
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
    assert!(app.device_origin.is_none());
    open(&mut app, &ctx);
    let expected = app.device.capture.clone().unwrap();
    let mut later = expected.clone();
    later.generation += 1;
    later.reports.clear();
    app.device.apply_event(DeviceEvent::Capture(later));
    click(&mut app, &ctx, "discard-button");
    let session = app.session.as_ref().unwrap();
    assert!(session.path.is_none());
    assert!(session.dirty());
    assert!(!session.can_undo());
    assert!(session.source_label.as_ref().unwrap().contains("read"));
    assert_eq!(
        app.device_origin.as_ref().unwrap().read.capture.reports,
        expected.reports
    );
    assert_eq!(
        app.device_origin.as_ref().unwrap().binding.generation,
        expected.generation
    );
    assert!(!app.live_requested);
    assert!(!app.device.live.enabled);
    app.enable_live();
    assert!(!app.live_requested); // newer read cannot authorize old selected bytes
}
#[test]
fn malformed_read_is_transactional_and_local_file_save_does_not_upload() {
    let (ctx, mut app) = simulator();
    app.session = Some(Session::example().unwrap());
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    app.device.capture.as_mut().unwrap().reports[0].truncate(6);
    app.prepare_device_read();
    assert!(app.error.as_ref().unwrap().contains("expected 64"));
    assert!(app.read_draft.is_none());
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
    let folder = tempfile::tempdir().unwrap();
    app.session
        .as_mut()
        .unwrap()
        .save_as(&folder.path().join("saved.fuse"))
        .unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(!app.live_requested);
    assert_eq!(app.device.live.acknowledged, 0);
}
#[test]
fn pointer_opt_in_sends_parameter_only_and_failure_keeps_offline_edits_without_replay() {
    let (ctx, mut app) = simulator();
    open(&mut app, &ctx);
    assert!(app.session.is_some());
    assert!(!app.device.live.enabled);
    click(&mut app, &ctx, "live-enable");
    pump(&mut app, |a| a.device.live.enabled);
    app.session
        .as_mut()
        .unwrap()
        .parameter("Amplifier", 0, 14000)
        .unwrap();
    app.process_device_edits();
    pump(&mut app, |a| {
        a.device.live.acknowledged > 0 && a.device.live.pending == 0
    });
    let ack = app.device.live.acknowledged;
    let mut metadata = app.session.as_ref().unwrap().document.metadata();
    metadata.name = "Local name only".into();
    app.session.as_mut().unwrap().metadata(&metadata).unwrap();
    app.process_device_edits();
    assert!(app.live_requested);
    assert_eq!(app.device.live.acknowledged, ack);
    app.device.apply_event(DeviceEvent::Live(LiveStatus {
        error: Some("Synthetic NACK: read again".into()),
        ..Default::default()
    }));
    app.reconcile_live();
    assert!(!app.live_requested);
    app.session
        .as_mut()
        .unwrap()
        .parameter("Amplifier", 0, 16000)
        .unwrap();
    app.process_device_edits();
    assert!(app.dirty());
    assert!(!app.live_requested);
    assert!(app.session.as_mut().unwrap().take_device_edits().is_empty());
}
#[test]
fn structural_edit_and_undo_disable_live_before_parameter_batch_can_be_sent() {
    let (ctx, mut app) = simulator();
    open(&mut app, &ctx);
    app.enable_live();
    pump(&mut app, |a| a.device.live.enabled);
    app.session
        .as_mut()
        .unwrap()
        .parameter("Amplifier", 0, 12000)
        .unwrap();
    app.session.as_mut().unwrap().undo().unwrap();
    app.process_device_edits();
    assert!(!app.live_requested);
    assert!(app.device_origin.as_ref().unwrap().invalidated.is_some());
    pump(&mut app, |a| !a.device.live.enabled);
    assert_eq!(app.device.live.acknowledged, 0);
    app.enable_live();
    assert!(!app.live_requested);
}
#[test]
fn disconnect_and_new_document_cancel_binding_without_replacing_local_edits() {
    let (ctx, mut app) = simulator();
    open(&mut app, &ctx);
    app.enable_live();
    pump(&mut app, |a| a.device.live.enabled);
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    app.device_action(Action::Disconnect);
    assert!(!app.live_requested);
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
    app.perform(Pending::New(13), &ctx);
    assert!(app.device_origin.is_none());
    assert!(app.session.as_ref().unwrap().source_label.is_none());
}
#[test]
fn saved_capture_matching_current_identity_still_cannot_enable_live() {
    let (ctx, mut app) = simulator();
    let mut draft = ReadDraft::decode(app.device.capture.clone().unwrap()).unwrap();
    draft.offline_file = true;
    app.perform(Pending::OpenDevice(Box::new(draft)), &ctx);
    app.enable_live();
    assert!(!app.live_requested);
    assert!(app.error.as_ref().unwrap().contains("offline"));
}
