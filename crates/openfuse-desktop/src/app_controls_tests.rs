use super::*;
use crate::{Options, controls_view, session::Session};
use openfuse_core::{
    controls::{TunerNote, TunerReading},
    protocol::Report64,
};
use openfuse_device::{OperationPhase, TunerStatus};
use std::time::{Duration, Instant};
fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        app.device.poll();
        app.operation_update();
        app.controls_update();
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "Controls did not settle: {:?} {:?} {:?}",
        app.device.tuner, app.device.operation, app.device.error
    );
}
fn setup(floor: bool) -> (egui::Context, Editor) {
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
    if floor {
        app.device = crate::device_state::DeviceState::with_floor(true, true);
    }
    pump(&mut app, |a| a.device.current_binding().is_some());
    app.session = Some(Session::example().unwrap());
    app.device_dialog = false;
    app.open_controls();
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
        |ctx| app.controls_dialog(ctx),
    );
}
fn click(app: &mut Editor, ctx: &egui::Context, id: &str) {
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]);
    let (rect, enabled) = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id))
            .expect("controls rendered")
    });
    assert!(
        enabled,
        "{id} should be enabled: busy={} binding={:?} patch={:?} tuner={:?}",
        app.device.operation_busy(),
        app.device.current_binding(),
        app.controls_ui.draft.as_ref().map(|d| (
            &d.patch,
            &d.binding,
            app.device.matches_capture(&d.binding)
        )),
        app.device.tuner
    );
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
fn settings_review_freezes_patch_and_verified_apply_preserves_local_dirty_document() {
    let (ctx, mut app) = setup(true);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local.fuse");
    let session = app.session.as_mut().unwrap();
    session.save_as(&path).unwrap();
    let disk = std::fs::read(&path).unwrap();
    let mut metadata = session.document.metadata();
    metadata.name = "Unsaved local".into();
    session.metadata(&metadata).unwrap();
    let xml = session.document.to_xml().unwrap();
    click(&mut app, &ctx, "settings-usb-enable");
    app.controls_ui.draft.as_mut().unwrap().patch.usb_gain = Some(192);
    click(&mut app, &ctx, "controls-review");
    assert_eq!(
        app.controls_ui.preview.as_ref().unwrap().patch.usb_gain,
        Some(192)
    );
    app.controls_ui.draft.as_mut().unwrap().patch.usb_gain = Some(17);
    click(&mut app, &ctx, "controls-apply");
    assert!(app.device.operation_busy());
    pump(&mut app, |a| {
        a.device.operation.phase == OperationPhase::Verified && a.device.current_binding().is_some()
    });
    assert_eq!(
        app.controls_ui.draft.as_ref().unwrap().current.usb_gain,
        Some(192)
    );
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        xml
    );
    assert_eq!(std::fs::read(&path).unwrap(), disk);
    assert!(app.dirty());
    assert!(app.session.as_ref().unwrap().can_undo());
    assert!(app.read_draft.is_none());
}
#[test]
fn stale_settings_preview_cannot_apply_and_back_keeps_draft() {
    let (ctx, mut app) = setup(true);
    app.controls_ui.draft.as_mut().unwrap().patch.usb_gain = Some(100);
    app.prepare_settings();
    app.device.capture.as_mut().unwrap().generation += 1;
    frame(&mut app, &ctx, vec![]);
    assert!(!ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new("controls-apply"))
            .unwrap()
            .1
    }));
    app.apply_settings();
    assert!(app.error.as_ref().unwrap().contains("stale"));
    assert!(!app.device.operation_busy());
    click(&mut app, &ctx, "controls-back");
    assert!(app.controls_ui.preview.is_none());
    assert_eq!(
        app.controls_ui.draft.as_ref().unwrap().patch.usb_gain,
        Some(100)
    );
}
#[test]
fn unsupported_settings_hidden_and_missing_reports_not_treated_as_defaults() {
    let (ctx, mut app) = setup(false);
    frame(&mut app, &ctx, vec![]);
    for id in [
        "settings-volume-enable",
        "settings-loop-enable",
        "settings-first-enable",
        "settings-second-enable",
    ] {
        assert!(
            ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id)))
                .is_none(),
            "{id}"
        );
    }
    let mut capture = app.device.capture.clone().unwrap();
    capture.reports.retain(|r| !(r[0] == 0x1c && r[2] == 13));
    app.controls_ui.draft = Some(SettingsDraft::prepare(&capture).unwrap());
    frame(&mut app, &ctx, vec![]);
    assert!(!ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new("settings-usb-enable"))
            .unwrap()
            .1
    }));
    assert!(
        app.controls_ui
            .draft
            .as_ref()
            .unwrap()
            .current
            .usb_gain
            .is_none()
    );
}
#[test]
fn tuner_confirmation_reads_pitch_blocks_writes_and_stop_accepts_stale_generation() {
    let (ctx, mut app) = setup(true);
    let original = app.session.as_ref().unwrap().document.to_xml().unwrap();
    app.controls_ui.mute = true;
    click(&mut app, &ctx, "tuner-toggle");
    assert!(app.device.operation_busy());
    pump(&mut app, |a| {
        a.device.tuner.confirmed && a.device.tuner.reading.is_some()
    });
    assert!(app.device.tuner.enabled);
    assert!(app.device.tuner.muted);
    assert!(!app.device.tuner.stale);
    app.controls_ui.draft.as_mut().unwrap().patch.usb_gain = Some(99);
    app.prepare_settings();
    assert!(app.controls_ui.preview.is_none());
    app.device.capture.as_mut().unwrap().generation += 1;
    click(&mut app, &ctx, "tuner-toggle");
    pump(&mut app, |a| !a.device.operation_busy());
    assert!(!app.device.tuner.enabled);
    app.controls_action(Action::Reload);
    pump(&mut app, |a| a.device.current_binding().is_some());
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        original
    );
}
#[test]
fn tuner_display_never_turns_stale_invalid_unknown_or_out_of_range_into_in_tune() {
    let mut status = TunerStatus {
        enabled: true,
        confirmed: true,
        stale: false,
        reading: Some(TunerReading {
            tone: 7,
            offset: 0,
            note: TunerNote::E,
        }),
        ..Default::default()
    };
    assert_eq!(controls_view::tuner_display(&status).0, "E");
    status.stale = true;
    assert_eq!(controls_view::tuner_display(&status).0, "—");
    status.stale = false;
    for tone in [12, 255] {
        status.reading = Some(TunerReading {
            tone,
            offset: 0,
            note: TunerNote::from_raw(tone),
        });
        let display = controls_view::tuner_display(&status);
        assert_ne!(display.0, "E");
        assert!(!display.1.contains("Raw deviation"));
    }
    status.reading = Some(TunerReading {
        tone: 7,
        offset: 127,
        note: TunerNote::E,
    });
    assert!(
        controls_view::tuner_display(&status)
            .1
            .contains("Out of range")
    );
    status.enabled = false;
    status.pending = true;
    assert!(controls_view::tuner_display(&status).1.contains("Stopping"));
    status.pending = false;
    status.error = Some("Timed out".into());
    assert!(controls_view::tuner_display(&status).1.contains("unknown"));
}
#[test]
fn floor_preview_preserves_modes_and_shows_fx_loop_semantics_without_local_edits() {
    let (_ctx, mut app) = setup(true);
    let draft = app.controls_ui.draft.as_mut().unwrap();
    draft.patch.fx_loop_bypassed = Some(true);
    draft.patch.global_volume = Some(openfuse_core::controls::VolumeEndpoints {
        heel: 8193,
        toe: 65535,
    });
    draft.patch.first_pedal_mode = Some(1);
    draft.patch.second_pedal_mode = Some(0);
    app.prepare_settings();
    let preview = app.controls_ui.preview.as_ref().unwrap();
    assert!(
        preview
            .changes
            .iter()
            .any(|(name, _, after)| name == "FX loop state" && after == "Active")
    );
    assert!(preview.warnings.iter().any(|w| w.contains("quantized")));
    assert_eq!(preview.changes.len(), 4);
    app.apply_settings();
    pump(&mut app, |a| {
        a.device.operation.phase == OperationPhase::Verified && a.device.current_binding().is_some()
    });
    let state = &app.controls_ui.draft.as_ref().unwrap().current;
    assert_eq!(state.global_volume.unwrap().heel, 8192);
    assert_eq!(state.global_volume.unwrap().toe, 65280);
    assert_eq!(state.first_pedal_mode, Some(1));
    assert_eq!(state.second_pedal_mode, Some(0));
}
#[test]
fn controls_modal_prevents_device_dialog_stacking_and_capture_unknown_modes_stay_visible() {
    let (ctx, mut app) = setup(true);
    app.device_dialog = true;
    let _ = ctx.run(Default::default(), |ctx| app.device_dialog(ctx));
    assert!(
        ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new(("usb-control", "connect"))))
            .is_none()
    );
    let mut capture = app.device.capture.clone().unwrap();
    for raw in &mut capture.reports {
        if raw[0] == 0x1c && raw[2] == 10 && raw[25] == 0 {
            raw[24] = 200;
        }
    }
    let draft = SettingsDraft::prepare(&capture).unwrap();
    assert_eq!(draft.current.first_pedal_mode, Some(200));
    assert!(draft.patch.is_empty());
    let reports: Vec<_> = capture
        .reports
        .iter()
        .map(|r| Report64::try_from(r.as_slice()).unwrap())
        .collect();
    assert_eq!(
        openfuse_core::controls::settings(capture.device.product_id, &reports)
            .unwrap()
            .first_pedal_mode,
        Some(200)
    );
}
