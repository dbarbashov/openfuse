use super::*;
use crate::{Options, auxiliary_view, session::Session, theme};
use openfuse_core::{
    auxiliary::{Key, Snapshot},
    protocol::Report64,
};
use openfuse_device::OperationPhase;
use std::time::{Duration, Instant};

fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let ctx = egui::Context::default();
    while Instant::now() < deadline {
        app.device.poll();
        app.operation_update();
        app.archives_update(&ctx);
        app.auxiliary_update();
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "Whole restore timed out: {:?} {:?} {:?}",
        app.auxiliary_ui.error, app.device.error, app.device.bank
    );
}
fn setup(sc: bool) -> (egui::Context, Editor) {
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
    if sc {
        app.device = crate::device_state::DeviceState::with_model(true, false, true);
    }
    pump(&mut app, |a| a.device.current_binding().is_some());
    app.session = Some(Session::example().unwrap());
    app.open_auxiliary();
    app.auxiliary_action(auxiliary_view::Action::Read);
    pump(&mut app, |a| {
        !a.device.operation_busy() && a.device.current_auxiliary().is_some()
    });
    app.auxiliary_action(auxiliary_view::Action::UseRead);
    (ctx, app)
}
fn prepare(app: &mut Editor) {
    app.prepare_whole_restore();
    pump(app, |a| a.auxiliary_ui.whole_preparation.is_none());
    assert!(
        app.auxiliary_ui.whole_preview.is_some(),
        "{:?}",
        app.auxiliary_ui.error
    );
}
fn frame(app: &mut Editor, ctx: &egui::Context, events: Vec<egui::Event>, size: egui::Vec2) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| app.auxiliary_dialog(ctx),
    );
}
fn click(app: &mut Editor, ctx: &egui::Context, id: &str) {
    for _ in 0..3 {
        frame(app, ctx, vec![], egui::vec2(1220.0, 930.0));
    }
    let (rect, enabled) = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id))
            .expect("drawn whole restore control")
    });
    assert!(enabled, "{id}");
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
            egui::vec2(1220.0, 930.0),
        );
    }
}
fn modified_system(source: &Snapshot) -> Snapshot {
    let reports = source
        .reports()
        .iter()
        .map(|r| {
            let mut b = *r.as_bytes();
            if b[0] == 0x1c && b[2] == 11 {
                b[16] = 23;
                b[32] = 77;
                b[33] = 2;
            }
            Report64::from_bytes(b)
        })
        .collect::<Vec<_>>();
    Snapshot::from_reports(source.pid(), &reports).unwrap()
}
#[test]
fn pointer_full_preview_shows_system_changes_and_keeps_dirty_local_history() {
    let (ctx, mut app) = setup(false);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("local.fuse");
    let local = app.session.as_mut().unwrap();
    local.save_as(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut metadata = local.document.metadata();
    metadata.name = "Independent unsaved sound".into();
    local.metadata(&metadata).unwrap();
    let xml = local.document.to_xml().unwrap();
    let modified = modified_system(&app.auxiliary_ui.source.as_ref().unwrap().snapshot);
    app.auxiliary_ui
        .source("Stored full archive".into(), modified);
    click(&mut app, &ctx, "aux-review-whole");
    pump(&mut app, |a| a.auxiliary_ui.whole_preparation.is_none());
    assert!(
        app.auxiliary_ui.whole_preview.is_some(),
        "{:?}",
        app.auxiliary_ui.error
    );
    let draft = app.auxiliary_ui.whole_preview.as_ref().unwrap();
    assert_eq!(draft.plan.preset_count(), 24);
    assert_eq!(draft.plan.bank_count(), 24);
    assert!(draft.plan.has_system());
    assert_eq!(draft.system.len(), 22);
    assert_eq!(draft.system[6].after, "23");
    assert_eq!(draft.system[0].after, "77");
    assert_eq!(draft.system[1].after, "2");
    app.apply_whole_restore();
    assert!(!app.device.operation_busy());
    assert!(app.auxiliary_ui.whole_preview.is_some());
    click(&mut app, &ctx, "whole-confirm");
    click(&mut app, &ctx, "whole-apply");
    pump(&mut app, |a| !a.device.operation_busy());
    assert_eq!(
        app.device.bank.phase,
        OperationPhase::Verified,
        "{:?}",
        app.device.bank
    );
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        xml
    );
    assert!(app.dirty());
    assert!(app.session.as_ref().unwrap().can_undo());
    assert_eq!(std::fs::read(path).unwrap(), original);
    pump(&mut app, |a| a.device.current_auxiliary().is_some());
    let raw = app
        .device
        .current_auxiliary()
        .unwrap()
        .snapshot
        .entries()
        .iter()
        .find(|e| e.key() == Key::System)
        .unwrap()
        .raw_reports()[0];
    assert_eq!(raw.as_bytes()[16], 23);
    assert_eq!(raw.as_bytes()[32], 77);
    assert_eq!(raw.as_bytes()[33], 2);
}
#[test]
fn stale_source_and_destination_disable_apply_without_writing() {
    let (ctx, mut app) = setup(false);
    prepare(&mut app);
    app.auxiliary_ui.whole_preview.as_mut().unwrap().confirmed = true;
    app.auxiliary_ui.source_revision += 1;
    app.apply_whole_restore();
    assert!(!app.device.operation_busy());
    assert!(app.auxiliary_ui.error.as_ref().unwrap().contains("stale"));
    frame(&mut app, &ctx, vec![], egui::vec2(1220.0, 930.0));
    assert!(
        !ctx.data(|d| d
            .get_temp::<(egui::Rect, bool)>(egui::Id::new("whole-apply"))
            .unwrap())
            .1
    );
    click(&mut app, &ctx, "whole-back");
    prepare(&mut app);
    app.auxiliary_ui.whole_preview.as_mut().unwrap().confirmed = true;
    app.device.capture.as_mut().unwrap().generation += 1;
    app.apply_whole_restore();
    assert!(!app.device.operation_busy());
    assert!(app.auxiliary_ui.error.as_ref().unwrap().contains("stale"));
}
#[test]
fn disconnect_keeps_source_and_refuses_whole_preview() {
    let (_ctx, mut app) = setup(false);
    prepare(&mut app);
    app.auxiliary_ui.whole_preview.as_mut().unwrap().confirmed = true;
    let fingerprint = app
        .auxiliary_ui
        .source
        .as_ref()
        .unwrap()
        .snapshot
        .fingerprint();
    app.device.disconnect();
    app.apply_whole_restore();
    assert!(!app.device.operation_busy());
    assert!(app.auxiliary_ui.error.as_ref().unwrap().contains("stale"));
    assert_eq!(
        app.auxiliary_ui
            .source
            .as_ref()
            .unwrap()
            .snapshot
            .fingerprint(),
        fingerprint
    );
}
#[test]
fn whole_restore_is_exclusive_and_cancel_preserves_local_document() {
    let (_ctx, mut app) = setup(false);
    let xml = app.session.as_ref().unwrap().document.to_xml().unwrap();
    prepare(&mut app);
    app.auxiliary_ui.whole_preview.as_mut().unwrap().confirmed = true;
    app.apply_whole_restore();
    assert!(app.device.operation_busy());
    app.prepare_whole_restore();
    assert!(app.auxiliary_ui.error.as_ref().unwrap().contains("Finish"));
    assert!(app.auxiliary_ui.whole_preview.is_none());
    app.auxiliary_action(auxiliary_view::Action::Cancel);
    pump(&mut app, |a| {
        matches!(
            a.device.bank.phase,
            OperationPhase::Cancelled | OperationPhase::Failed
        )
    });
    assert_eq!(app.device.bank.phase, OperationPhase::Cancelled);
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        xml
    );
}
#[test]
fn superchamp_preview_has_only_voice_and_fx_then_verified_readback() {
    let (ctx, mut app) = setup(true);
    prepare(&mut app);
    assert!(
        app.auxiliary_ui.whole_preview.is_some(),
        "{:?}",
        app.auxiliary_ui.error
    );
    let draft = app.auxiliary_ui.whole_preview.as_ref().unwrap();
    assert_eq!(draft.plan.preset_count(), 0);
    assert_eq!(draft.plan.bank_count(), 32);
    assert!(!draft.plan.has_system());
    assert!(draft.system.is_empty());
    assert!(draft.presets.is_empty());
    click(&mut app, &ctx, "whole-confirm");
    click(&mut app, &ctx, "whole-apply");
    pump(&mut app, |a| !a.device.operation_busy());
    assert_eq!(
        app.device.bank.phase,
        OperationPhase::Verified,
        "{:?}",
        app.device.bank
    );
}
#[test]
fn source_replacement_closes_owned_preview_and_mapping_edits_do_not_change_it() {
    let (_ctx, mut app) = setup(false);
    prepare(&mut app);
    let fingerprint = app
        .auxiliary_ui
        .whole_preview
        .as_ref()
        .unwrap()
        .plan
        .source_fingerprint()
        .to_string();
    app.auxiliary_ui.selected.insert(Key::System);
    app.auxiliary_ui.targets.clear();
    assert_eq!(
        app.auxiliary_ui
            .whole_preview
            .as_ref()
            .unwrap()
            .plan
            .source_fingerprint(),
        fingerprint
    );
    let snapshot = app.auxiliary_ui.source.as_ref().unwrap().snapshot.clone();
    app.auxiliary_ui.source("Other source".into(), snapshot);
    assert!(app.auxiliary_ui.whole_preview.is_none());
}

#[test]
fn original_loader_normalizations_are_listed_before_confirmation() {
    let (ctx, mut app) = setup(false);
    prepare(&mut app);
    let preview = app.auxiliary_ui.whole_preview.as_ref().unwrap();
    assert!(!preview.normalizations.is_empty());
    assert_eq!(
        preview.normalizations.len(),
        preview.plan.normalizations().len()
    );
    assert!(
        preview
            .normalizations
            .iter()
            .all(|row| row.before != row.after)
    );
    assert!(!preview.confirmed);
    click(&mut app, &ctx, "whole-tab-Normalizations");
    assert!(app.auxiliary_ui.whole_preview.as_ref().unwrap().tab == Tab::Normalizations);
    click(&mut app, &ctx, "whole-normalizations-next");
    assert_eq!(
        app.auxiliary_ui
            .whole_preview
            .as_ref()
            .unwrap()
            .normalization_page,
        1
    );
    click(&mut app, &ctx, "whole-normalizations-prev");
    assert_eq!(
        app.auxiliary_ui
            .whole_preview
            .as_ref()
            .unwrap()
            .normalization_page,
        0
    );
    assert!(
        !ctx.data(|d| d
            .get_temp::<(egui::Rect, bool)>(egui::Id::new("whole-apply"))
            .unwrap())
            .1
    );
}
#[test]
fn minimum_window_keeps_complete_restore_confirmation_and_back_visible() {
    let (ctx, mut app) = setup(false);
    prepare(&mut app);
    let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(940.0, 680.0));
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![], bounds.size());
    }
    for id in ["whole-confirm", "whole-back", "whole-apply"] {
        let (rect, _) = ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id)).unwrap());
        assert!(bounds.contains_rect(rect), "{id}: {rect:?}");
    }
}

#[test]
fn cancelled_background_review_cannot_reappear_or_start_a_second_worker() {
    let (ctx, mut app) = setup(false);
    app.prepare_whole_restore();
    assert!(app.auxiliary_ui.whole_preparation.is_some());
    click(&mut app, &ctx, "whole-cancel-preview");
    assert!(
        app.auxiliary_ui
            .whole_preparation
            .as_ref()
            .unwrap()
            .cancelled
    );
    app.prepare_whole_restore();
    assert!(app.auxiliary_ui.error.as_ref().unwrap().contains("Wait"));
    pump(&mut app, |a| a.auxiliary_ui.whole_preparation.is_none());
    assert!(app.auxiliary_ui.whole_preview.is_none());
    assert!(!app.device.operation_busy());
}

#[test]
fn changed_destination_discards_completed_background_review() {
    let (_ctx, mut app) = setup(false);
    app.prepare_whole_restore();
    app.device.capture.as_mut().unwrap().generation += 1;
    pump(&mut app, |a| a.auxiliary_ui.whole_preparation.is_none());
    assert!(app.auxiliary_ui.whole_preview.is_none());
    assert!(
        app.auxiliary_ui
            .error
            .as_ref()
            .unwrap()
            .contains("changed during")
    );
    assert!(!app.device.operation_busy());
}

#[test]
fn different_exact_profile_is_refused_before_any_write() {
    let (_ctx, mut app) = setup(false);
    let (_, sc) = setup(true);
    app.auxiliary_ui.source(
        "Super Champ source".into(),
        sc.auxiliary_ui.source.as_ref().unwrap().snapshot.clone(),
    );
    app.prepare_whole_restore();
    pump(&mut app, |a| a.auxiliary_ui.whole_preparation.is_none());
    assert!(app.auxiliary_ui.whole_preview.is_none());
    assert!(!app.device.operation_busy());
    assert!(
        app.auxiliary_ui
            .error
            .as_ref()
            .unwrap()
            .contains("exact USB PID")
    );
}

fn legacy_source(app: &mut Editor, complete: bool) -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    let snapshot = &app.auxiliary_ui.source.as_ref().unwrap().snapshot;
    for entry in snapshot.entries() {
        let path = match entry.key() {
            Key::System => folder.path().join("SystemSettings.fuse"),
            Key::Bank { control, slot } => folder.path().join(format!(
                "Effects/{}/{slot:02}.fuse",
                if control == 1 { "Mod" } else { "DlyRvb" }
            )),
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, entry.xml().unwrap()).unwrap();
    }
    if complete {
        std::fs::create_dir(folder.path().join("Presets")).unwrap();
        for preset in snapshot.presets() {
            std::fs::write(
                folder
                    .path()
                    .join(format!("Presets/{:02}.fuse", preset.slot())),
                preset.document().to_xml().unwrap(),
            )
            .unwrap();
        }
    }
    let archive = openfuse_core::archive::Archive::read_path(folder.path()).unwrap();
    let snapshot = archive.auxiliary_for_pid(snapshot.pid()).unwrap();
    app.auxiliary_ui
        .source("Historical FUSE archive".into(), snapshot);
    app.auxiliary_ui.source.as_mut().unwrap().archive = Some(archive);
    folder
}

#[test]
fn incomplete_historical_archive_is_inspect_only_with_clear_reason() {
    let (_ctx, mut app) = setup(false);
    let _source = legacy_source(&mut app, false);
    app.prepare_whole_restore();
    pump(&mut app, |a| a.auxiliary_ui.whole_preparation.is_none());
    assert!(app.auxiliary_ui.whole_preview.is_none());
    assert!(!app.device.operation_busy());
    let error = app.auxiliary_ui.error.as_ref().unwrap();
    assert!(error.contains("Inspect-only"), "{error}");
    assert!(error.to_lowercase().contains("preset"), "{error}");
    assert_eq!(
        app.auxiliary_ui
            .source
            .as_ref()
            .unwrap()
            .snapshot
            .entries()
            .len(),
        25
    );
}

#[test]
fn historical_complete_source_uses_preset_documents_and_keeps_original_files() {
    let (_ctx, mut app) = setup(false);
    let source = legacy_source(&mut app, true);
    let path = source.path().join("SystemSettings.fuse");
    let bytes = std::fs::read(&path).unwrap();
    prepare(&mut app);
    let preview = app.auxiliary_ui.whole_preview.as_ref().unwrap();
    assert_eq!(preview.presets.len(), 24);
    assert_eq!(preview.plan.preset_count(), 24);
    assert_eq!(preview.system.len(), 22);
    assert!(
        app.auxiliary_ui
            .source
            .as_ref()
            .unwrap()
            .snapshot
            .presets()
            .is_empty()
    );
    app.auxiliary_ui.whole_preview.as_mut().unwrap().confirmed = true;
    app.apply_whole_restore();
    pump(&mut app, |a| !a.device.operation_busy());
    assert_eq!(
        app.device.bank.phase,
        OperationPhase::Verified,
        "{:?}",
        app.device.bank
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
