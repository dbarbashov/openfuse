use super::*;
use crate::{Options, device_view::Action, session::Session};
use openfuse_core::{
    model_catalog::{self, ModuleCategory},
    preset::PresetParameter,
    protocol::Report64,
};
use openfuse_device::{DeviceEvent, OperationPhase};
use std::time::{Duration, Instant};

fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        app.device.poll();
        app.operation_update();
        app.reconcile_live();
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "Operation did not settle: {:?} {:?}",
        app.device.operation, app.error
    );
}
fn setup() -> (egui::Context, Editor) {
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
    pump(&mut app, |a| a.device.current_binding().is_some());
    let draft = crate::device_read::ReadDraft::decode(app.device.capture.clone().unwrap()).unwrap();
    app.perform(super::super::Pending::OpenDevice(Box::new(draft)), &ctx);
    (ctx, app)
}
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
            egui::CentralPanel::default().show(ctx, |ui| app.working_panel(ui));
            app.audition_dialog(ctx);
            app.read_dialog(ctx);
            app.confirm(ctx);
        },
    );
}
fn click(app: &mut Editor, ctx: &egui::Context, id: &str) {
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]);
    let rect = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id))
            .map(|(r, _)| r)
            .or_else(|| d.get_temp::<egui::Rect>(egui::Id::new(id)))
            .expect("control drawn")
    });
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
#[test]
fn cancel_preview_never_sends_and_stale_apply_is_disabled() {
    let (ctx, mut app) = setup();
    let generation = app.device.capture.as_ref().unwrap().generation;
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    click(&mut app, &ctx, "audition-open");
    assert!(app.audition_draft.is_some(), "{:?}", app.error);
    click(&mut app, &ctx, "audition-cancel");
    assert!(app.audition_draft.is_none());
    assert_eq!(app.device.operation.phase, OperationPhase::Idle);
    click(&mut app, &ctx, "audition-open");
    app.device.capture.as_mut().unwrap().generation += 1;
    frame(&mut app, &ctx, vec![]);
    let (_, enabled) = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new("audition-apply"))
            .unwrap()
    });
    assert!(!enabled);
    app.apply_audition();
    assert!(app.error.as_ref().unwrap().contains("stale"));
    assert_eq!(app.device.operation.phase, OperationPhase::Idle);
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
    assert_eq!(
        app.device.capture.as_ref().unwrap().generation,
        generation + 1
    );
}
#[test]
fn audition_sends_frozen_models_bypass_routing_then_verifies_readback_without_changing_dirty_local_file()
 {
    let (ctx, mut app) = setup();
    let old = app.session.as_ref().unwrap().document.modules()[0]
        .model_id
        .unwrap();
    let model = model_catalog::models(13, ModuleCategory::Amplifier)
        .find(|m| {
            m.id != old
                && m.parameters
                    .iter()
                    .any(|p| p.index == 0 && p.name != "NULL")
        })
        .unwrap();
    let parameters = model
        .parameters
        .iter()
        .map(|p| PresetParameter {
            index: p.index,
            value: p.default_raw,
        })
        .collect::<Vec<_>>();
    let session = app.session.as_mut().unwrap();
    session
        .model("Amplifier", model.id, &parameters, Some(model.bypass_mode))
        .unwrap();
    session.active("Delay", false).unwrap();
    session.move_module("Reverb", 4).unwrap();
    session.parameter("Amplifier", 0, 13000).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("local.fuse");
    session.save_as(&path).unwrap();
    let saved = std::fs::read(&path).unwrap();
    click(&mut app, &ctx, "audition-open");
    let draft = app.audition_draft.as_ref().expect("valid model audition");
    assert!(!draft.warnings.is_empty());
    let raw = app
        .device
        .capture
        .as_ref()
        .unwrap()
        .reports
        .iter()
        .map(|r| Report64::try_from(r.as_slice()).unwrap())
        .collect::<Vec<_>>();
    let expected = openfuse_core::audition::preset(20, &draft.document, &raw).unwrap();
    // Simulate a second edit while the immutable preview exists.
    app.session
        .as_mut()
        .unwrap()
        .parameter("Amplifier", 0, 22000)
        .unwrap();
    let local = app.session.as_ref().unwrap().document.to_xml().unwrap();
    assert!(
        app.dirty(),
        "preview-time parameter edit must differ from saved preset"
    );
    click(&mut app, &ctx, "audition-apply");
    assert!(app.device.operation_busy());
    assert!(!app.live_requested);
    pump(&mut app, |a| {
        matches!(
            a.device.operation.phase,
            OperationPhase::Verified | OperationPhase::Failed
        )
    });
    assert_eq!(
        app.device.operation.phase,
        OperationPhase::Verified,
        "{:?}",
        app.device.operation
    );
    let capture = app.device.capture.as_ref().unwrap();
    let readback = capture
        .reports
        .iter()
        .map(|r| Report64::try_from(r.as_slice()).unwrap())
        .collect::<Vec<_>>();
    expected.matches_readback(&readback).unwrap();
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        local
    );
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert!(app.dirty());
    assert!(!app.live_requested);
    click(&mut app, &ctx, "operation-readback");
    click(&mut app, &ctx, "read-open");
    assert!(app.pending.is_some());
    click(&mut app, &ctx, "cancel-button");
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        local
    );
    app.enable_live();
    assert!(!app.live_requested); // old imported generation cannot resume
}
#[test]
fn slot_selection_reads_before_preview_and_dirty_cancel_keeps_local_history() {
    let (ctx, mut app) = setup();
    app.session
        .as_mut()
        .unwrap()
        .parameter("Amplifier", 0, 31000)
        .unwrap();
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    let binding = app.device.current_binding().unwrap();
    app.new_dialog = true; // A readback must not stack a modal over another active dialog.
    app.device_action(Action::SelectSlot(8));
    assert!(app.slot_preview_pending);
    assert!(app.read_draft.is_none());
    pump(&mut app, |a| {
        matches!(
            a.device.operation.phase,
            OperationPhase::Verified | OperationPhase::Failed
        )
    });
    assert!(app.read_draft.is_none());
    assert!(app.slot_preview_pending);
    app.new_dialog = false;
    app.operation_update();
    assert!(app.read_draft.is_some());
    assert_eq!(
        app.device.operation.phase,
        OperationPhase::Verified,
        "{:?}",
        app.device.operation
    );
    assert!(app.device.capture.as_ref().unwrap().generation > binding.generation);
    assert_eq!(app.device.status.snapshot.as_ref().unwrap().slot, Some(8));
    click(&mut app, &ctx, "read-open");
    assert!(app.pending.is_some());
    click(&mut app, &ctx, "cancel-button");
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
    assert!(app.session.as_ref().unwrap().can_undo());
    assert!(app.dirty());
}
#[test]
fn cancellation_releases_submission_latch_and_never_replays() {
    let (_ctx, mut app) = setup();
    app.prepare_audition();
    app.apply_audition();
    app.cancel_audition();
    assert!(!app.slot_preview_pending);
    // A still-published Sending status can remain briefly; the local submission latch is cleared synchronously.
    app.device
        .apply_event(DeviceEvent::Operation(Default::default()));
    assert!(!app.device.operation_busy());
    pump(&mut app, |a| {
        matches!(
            a.device.operation.phase,
            OperationPhase::Cancelled | OperationPhase::Failed | OperationPhase::Verified
        )
    });
    assert!(!app.live_requested);
    app.device_action(Action::ReadCurrent);
    pump(&mut app, |a| a.device.current_binding().is_some());
    assert!(!app.device.operation_busy());
}
#[test]
fn invalid_preview_and_failed_operation_never_normalize_local_document() {
    let (_ctx, mut app) = setup();
    app.session =
        Some(Session::parse("<Preset ProductId=\"13\"><Unknown/></Preset>", None).unwrap());
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    app.prepare_audition();
    assert!(app.audition_draft.is_none());
    assert!(app.error.is_some());
    app.device
        .apply_event(DeviceEvent::Operation(openfuse_device::OperationStatus {
            phase: OperationPhase::Failed,
            detail: "Synthetic ACK failure; read again".into(),
            ..Default::default()
        }));
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
    assert!(!app.device.operation_busy());
    assert!(!app.live_requested);
}
