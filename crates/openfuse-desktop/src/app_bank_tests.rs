use super::*;
use crate::{Options, session::Session};
use openfuse_device::OperationPhase;
use std::time::{Duration, Instant};

fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        app.device.poll();
        app.operation_update();
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "Bank did not settle: {:?} {:?} {:?}",
        app.device.bank, app.device.operation, app.device.error
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
    app.session = Some(Session::example().unwrap());
    (ctx, app)
}
fn names(app: &mut Editor) {
    app.bank_action(Action::ReadNames);
    pump(app, |a| {
        !a.device.operation_busy() && a.device.current_bank_names().is_some()
    });
}
fn backup(app: &mut Editor) {
    app.bank_action(Action::Backup);
    pump(app, |a| {
        !a.device.operation_busy()
            && a.device.bank_backup.is_some()
            && a.device.current_binding().is_some()
    });
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
        |ctx| app.bank_dialog(ctx),
    );
}
fn click(app: &mut Editor, ctx: &egui::Context, id: &str) {
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]);
    let (rect, _) = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id))
            .expect("bank control drawn")
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
fn named_bank_and_preview_require_explicit_overwrite_and_preserve_local_dirty_file() {
    let (ctx, mut app) = setup();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("local.fuse");
    let local = app.session.as_mut().unwrap();
    local.save_as(&path).unwrap();
    let disk = std::fs::read(&path).unwrap();
    let mut metadata = local.document.metadata();
    metadata.name = "Unsaved local sound".into();
    local.metadata(&metadata).unwrap();
    let xml = local.document.to_xml().unwrap();
    app.prepare_bank_save(0);
    assert!(app.bank_ui.save.is_none());
    names(&mut app);
    assert_eq!(app.device.current_bank_names().unwrap().entries.len(), 24);
    app.prepare_bank_save(0);
    let draft = app.bank_ui.save.as_ref().unwrap();
    assert_ne!(draft.source_name, "Unsaved local sound");
    frame(&mut app, &ctx, vec![]);
    assert!(!ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new("bank-save-apply"))
            .unwrap()
            .1
    }));
    app.apply_bank_save();
    assert!(app.bank_ui.save.is_some());
    assert!(!app.device.operation_busy());
    click(&mut app, &ctx, "bank-save-confirm");
    click(&mut app, &ctx, "bank-save-apply");
    assert!(app.device.operation_busy());
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
    assert_eq!(std::fs::read(&path).unwrap(), disk);
    assert!(app.dirty());
    assert!(app.session.as_ref().unwrap().can_undo());
}
#[test]
fn stale_save_preview_cannot_write_and_cancellation_dismisses_frozen_destination() {
    let (ctx, mut app) = setup();
    names(&mut app);
    app.prepare_bank_save(1);
    app.bank_ui.save.as_mut().unwrap().confirmed = true;
    app.device.capture.as_mut().unwrap().generation += 1;
    frame(&mut app, &ctx, vec![]);
    assert!(!ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new("bank-save-apply"))
            .unwrap()
            .1
    }));
    app.apply_bank_save();
    assert!(app.error.as_ref().unwrap().contains("stale"));
    assert!(!app.device.operation_busy());
    click(&mut app, &ctx, "bank-save-dismiss");
    assert!(app.bank_ui.save.is_none());
    assert!(app.bank_ui.open);
}
#[test]
fn bank_archive_roundtrip_is_atomic_bounded_and_corruption_rejected() {
    let (_ctx, mut app) = setup();
    backup(&mut app);
    let archive = &app.device.bank_backup.as_ref().unwrap().backup;
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("bank.json");
    bank_view::export_backup(&path, archive).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bank_view::export_backup(&path, archive).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let loaded = bank_view::read_backup(&path).unwrap();
    assert_eq!(loaded.slots().len(), 24);
    std::fs::write(&path, b"{\"format\":\"not-a-bank\"}").unwrap();
    assert!(bank_view::read_backup(&path).is_err());
    std::fs::write(&path, vec![b' '; openfuse_core::bank::MAX_BACKUP_BYTES + 1]).unwrap();
    assert!(bank_view::read_backup(&path).unwrap_err().contains("4 MiB"));
    assert!(bank_view::read_backup(folder.path()).is_err());
}
#[test]
fn restore_preview_owns_archive_requires_confirmation_and_preserves_editor() {
    let (ctx, mut app) = setup();
    backup(&mut app);
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    let archive = app.device.bank_backup.as_ref().unwrap().backup.clone();
    app.prepare_bank_restore(archive, "bank.json".into());
    assert_eq!(
        app.bank_ui.restore.as_ref().unwrap().backup.slots().len(),
        24
    );
    app.apply_bank_restore();
    assert!(!app.device.operation_busy());
    assert!(app.bank_ui.restore.is_some());
    click(&mut app, &ctx, "bank-restore-confirm");
    click(&mut app, &ctx, "bank-restore-apply");
    assert!(app.device.operation_busy());
    pump(&mut app, |a| !a.device.operation_busy());
    assert_eq!(
        app.device.bank.phase,
        OperationPhase::Verified,
        "{:?}",
        app.device.bank
    );
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
    assert!(app.read_draft.is_none());
}
#[test]
fn imported_restore_preview_rejects_other_pid_and_stale_binding() {
    let (ctx, mut app) = setup();
    backup(&mut app);
    let archive = app.device.bank_backup.as_ref().unwrap().backup.clone();
    let mut other = app.device.capture.clone().unwrap();
    other.device.product_id = 0x0016;
    assert!(RestoreDraft::prepare(&other, archive.clone(), "bank.json".into()).is_err());
    app.prepare_bank_restore(archive, "bank.json".into());
    app.bank_ui.restore.as_mut().unwrap().confirmed = true;
    app.device.capture.as_mut().unwrap().generation += 1;
    frame(&mut app, &ctx, vec![]);
    app.apply_bank_restore();
    assert!(app.error.as_ref().unwrap().contains("stale"));
    assert!(!app.device.operation_busy());
    click(&mut app, &ctx, "bank-restore-dismiss");
    assert!(app.bank_ui.restore.is_none());
}
#[test]
fn bank_modal_defers_other_device_dialog_and_bank_cancel_clears_submission_latch() {
    let (ctx, mut app) = setup();
    app.bank_ui.open = true;
    app.device_dialog = true;
    let _ = ctx.run(Default::default(), |ctx| app.device_dialog(ctx));
    assert!(
        ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new(("usb-control", "connect"))))
            .is_none()
    );
    app.bank_action(Action::Backup);
    assert!(app.device.operation_busy());
    app.bank_action(Action::Cancel);
    pump(&mut app, |a| {
        !a.device.operation_busy() && a.device.bank.phase != OperationPhase::Idle
    });
    assert!(matches!(
        app.device.bank.phase,
        OperationPhase::Cancelled | OperationPhase::Verified | OperationPhase::Failed
    ));
}
