use super::*;
use crate::{Options, session::Session, theme};
use openfuse_device::OperationPhase;
use std::time::{Duration, Instant};

fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let ctx = egui::Context::default();
    while Instant::now() < deadline {
        app.device.poll();
        app.operation_update();
        app.archives_update(&ctx);
        app.library.poll();
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "archive flow timed out: {:?} {:?} {:?}",
        app.archives.error, app.device.bank, app.device.error
    );
}
fn setup() -> (egui::Context, Editor, tempfile::TempDir) {
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
    let folder = tempfile::tempdir().unwrap();
    app.archives.set_folder(folder.path().to_owned());
    pump(&mut app, |a| !a.archives.busy());
    app.open_archives();
    (ctx, app, folder)
}
fn capture(app: &mut Editor, ctx: &egui::Context) {
    let binding = app.device.current_binding().unwrap();
    app.device.backup_slots(&binding, vec![1, 7]).unwrap();
    pump(app, |a| {
        !a.device.operation_busy()
            && a.device.bank_backup.is_some()
            && a.device.current_binding().is_some()
    });
    assert!(
        !app.device
            .bank_backup
            .as_ref()
            .unwrap()
            .backup
            .is_complete()
    );
    app.archive_action(Action::SaveCapture, ctx);
    pump(app, |a| !a.archives.busy());
    assert!(app.archives.error.is_none(), "{:?}", app.archives.error);
    assert_eq!(app.archives.loaded.as_ref().unwrap().1.presets().len(), 2);
}
fn names(app: &mut Editor, ctx: &egui::Context) {
    app.archive_action(Action::ReadNames, ctx);
    pump(app, |a| {
        !a.device.operation_busy() && a.device.current_bank_names().is_some()
    });
}
fn mapping(app: &mut Editor) {
    app.archives.marked.extend([1, 7]);
    app.archives.targets.extend([(1, 3), (7, 12)]);
}
fn frame(app: &mut Editor, ctx: &egui::Context, events: Vec<egui::Event>) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1220., 930.),
            )),
            events,
            ..Default::default()
        },
        |ctx| app.archives_dialog(ctx),
    );
}
fn click(app: &mut Editor, ctx: &egui::Context, id: &str) {
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]);
    let (rect, enabled) = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id))
            .expect("drawn archive control")
    });
    assert!(enabled, "{id} disabled");
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
fn selected_capture_catalog_search_delete_and_recover_preserve_original() {
    let (ctx, mut app, _folder) = setup();
    capture(&mut app, &ctx);
    let partial = app.device.bank_backup.as_ref().unwrap().backup.clone();
    let result = crate::bank_view::RestoreDraft::prepare(
        app.device.capture.as_ref().unwrap(),
        partial,
        std::path::PathBuf::from("selected.json"),
    );
    assert!(
        matches!(result, Err(message) if message.contains("Use Archive catalog to map selected presets"))
    );
    assert!(!app.device.operation_busy());
    let entry = app.archives.loaded.as_ref().unwrap().0.clone();
    let original = std::fs::read(&entry.path).unwrap();
    app.archives.query = "Mustang".into();
    assert_eq!(app.archives.rows().len(), 1);
    app.archives.query = "missing sound".into();
    assert!(app.archives.rows().is_empty());
    app.archives.query.clear();
    app.archive_action(Action::Delete(entry.clone()), &ctx);
    pump(&mut app, |a| !a.archives.busy());
    assert!(app.archives.entries.is_empty());
    assert!(app.archives.loaded.is_none());
    assert_eq!(app.archives.trash.len(), 1);
    app.archive_action(Action::Recover(app.archives.trash[0].clone()), &ctx);
    pump(&mut app, |a| !a.archives.busy());
    assert_eq!(app.archives.entries.len(), 1);
    assert!(app.archives.trash.is_empty());
    assert_eq!(std::fs::read(&entry.path).unwrap(), original);
}
#[test]
fn mapped_restore_is_frozen_requires_confirmation_and_preserves_dirty_local_history() {
    let (ctx, mut app, folder) = setup();
    capture(&mut app, &ctx);
    names(&mut app, &ctx);
    mapping(&mut app);
    let path = folder.path().join("local.fuse");
    let local = app.session.as_mut().unwrap();
    local.save_as(&path).unwrap();
    let disk = std::fs::read(&path).unwrap();
    let mut metadata = local.document.metadata();
    metadata.name = "Unsaved local".into();
    local.metadata(&metadata).unwrap();
    let xml = local.document.to_xml().unwrap();
    app.prepare_archive_restore();
    assert!(app.archive_ui.restore.is_some(), "{:?}", app.archives.error);
    app.apply_archive_restore();
    assert!(!app.device.operation_busy());
    app.archives.targets.insert(1, 19);
    app.archives.marked.clear();
    let draft = app.archive_ui.restore.as_ref().unwrap();
    assert_eq!(
        draft.rows.iter().map(|r| r.target).collect::<Vec<_>>(),
        vec![3, 12]
    );
    click(&mut app, &ctx, "archive-restore-confirm");
    click(&mut app, &ctx, "archive-restore-apply");
    pump(&mut app, |a| !a.device.operation_busy());
    assert_eq!(app.device.bank.phase, OperationPhase::Verified);
    assert_eq!(app.device.bank.completed, 2);
    let local = app.session.as_ref().unwrap();
    assert_eq!(local.document.to_xml().unwrap(), xml);
    assert!(local.can_undo());
    assert!(app.dirty());
    assert_eq!(std::fs::read(path).unwrap(), disk);
}
#[test]
fn stale_duplicate_and_dismissed_previews_do_not_write() {
    let (ctx, mut app, _folder) = setup();
    capture(&mut app, &ctx);
    names(&mut app, &ctx);
    mapping(&mut app);
    app.archives.targets.insert(7, 3);
    app.prepare_archive_restore();
    assert!(app.archive_ui.restore.is_none());
    assert!(app.archives.error.is_some());
    assert!(!app.device.operation_busy());
    app.archives.targets.insert(7, 12);
    app.prepare_archive_restore();
    app.archive_ui.restore.as_mut().unwrap().confirmed = true;
    app.device.capture.as_mut().unwrap().generation += 1;
    app.apply_archive_restore();
    assert!(app.archives.error.as_ref().unwrap().contains("stale"));
    assert!(!app.device.operation_busy());
    click(&mut app, &ctx, "archive-restore-back");
    assert!(app.archive_ui.restore.is_none());
    assert!(app.archive_ui.open);
}
#[test]
fn extraction_refreshes_library_without_opening_or_overwriting_editor() {
    let (ctx, mut app, _folder) = setup();
    capture(&mut app, &ctx);
    let lib = tempfile::tempdir().unwrap();
    app.library.set_folder(lib.path().to_owned());
    app.archives.marked.insert(7);
    let xml = app.session.as_ref().unwrap().document.to_xml().unwrap();
    app.archive_action(Action::Extract, &ctx);
    pump(&mut app, |a| !a.archives.busy() && !a.library.busy());
    assert_eq!(app.library.entries().len(), 1);
    let path = app.library.entries()[0].path.clone();
    let bytes = std::fs::read(&path).unwrap();
    app.archive_action(Action::Extract, &ctx);
    pump(&mut app, |a| !a.archives.busy() && !a.library.busy());
    assert_eq!(app.library.entries().len(), 2);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        xml
    );
}
#[test]
fn legacy_import_requires_explicit_compatible_profile_and_retains_auxiliary_files() {
    let (ctx, mut app, _folder) = setup();
    capture(&mut app, &ctx);
    let old = tempfile::tempdir().unwrap();
    std::fs::create_dir(old.path().join("Presets")).unwrap();
    let xml = app.archives.loaded.as_ref().unwrap().1.presets()[0]
        .document
        .to_xml()
        .unwrap();
    std::fs::write(old.path().join("Presets/01_Rhythm.fuse"), xml).unwrap();
    std::fs::write(
        old.path().join("M2_BackupName.fuse"),
        "Rehearsal original folder",
    )
    .unwrap();
    std::fs::write(old.path().join("unknown.bin"), [0, 255, 32]).unwrap();
    app.archives
        .start(Operation::ImportLegacy(old.path().to_owned()));
    pump(&mut app, |a| !a.archives.busy());
    assert!(app.archives.error.is_none(), "{:?}", app.archives.error);
    let legacy = app.archives.loaded.as_ref().unwrap().1.legacy().unwrap();
    assert_eq!(
        legacy
            .entries()
            .iter()
            .find(|e| e.path() == "unknown.bin")
            .unwrap()
            .bytes(),
        &[0, 255, 32]
    );
    names(&mut app, &ctx);
    app.archives.marked.insert(1);
    app.archives.targets.insert(1, 3);
    app.prepare_archive_restore();
    assert!(app.archive_ui.restore.is_none());
    assert!(app.archives.error.as_ref().unwrap().contains("explicitly"));
    app.archives.legacy_profile = Some(4);
    app.prepare_archive_restore();
    assert!(app.archive_ui.restore.is_none());
    app.archives.legacy_profile = Some(0x14);
    app.prepare_archive_restore();
    assert!(app.archive_ui.restore.is_some(), "{:?}", app.archives.error);
}
#[test]
fn opening_archive_preset_obeys_dirty_guard_and_has_no_device_origin() {
    let (ctx, mut app, _folder) = setup();
    capture(&mut app, &ctx);
    let local = app.session.as_mut().unwrap();
    let mut metadata = local.document.metadata();
    metadata.name = "Unsaved before archive".into();
    local.metadata(&metadata).unwrap();
    let xml = local.document.to_xml().unwrap();
    assert!(app.dirty());
    app.archive_action(Action::OpenPreset(1), &ctx);
    assert!(matches!(app.pending, Some(Pending::OpenArchive(..))));
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        xml
    );
    let pending = app.pending.take().unwrap();
    app.perform(pending, &ctx);
    assert!(app.device_origin.is_none());
    assert!(app.dirty());
    assert!(!app.device.operation_busy());
}
#[test]
fn folder_generation_discards_pending_result_and_bad_import_is_recoverable() {
    let (ctx, mut app, _folder) = setup();
    capture(&mut app, &ctx);
    let fresh = tempfile::tempdir().unwrap();
    let previous = app.archives.entries[0].clone();
    app.archives.start(Operation::Load(previous));
    app.archives.set_folder(fresh.path().to_owned());
    pump(&mut app, |a| !a.archives.busy());
    assert!(app.archives.entries.is_empty());
    assert!(app.archives.loaded.is_none());
    let bad = fresh.path().join("bad.json");
    std::fs::write(&bad, "{}").unwrap();
    app.archives.start(Operation::ImportFile(bad));
    pump(&mut app, |a| !a.archives.busy());
    assert!(app.archives.error.is_some());
    assert!(app.archives.entries.is_empty());
    app.archive_action(Action::SaveCapture, &ctx);
    pump(&mut app, |a| !a.archives.busy());
    assert!(app.archives.error.is_none());
    assert_eq!(app.archives.entries.len(), 1);
}
#[test]
fn import_timestamp_is_explicit_utc_calendar_date() {
    assert_eq!(crate::archive_state::date(0), "1970-01-01 00:00 UTC");
    assert_eq!(
        crate::archive_state::date(1709164800000),
        "2024-02-29 00:00 UTC"
    );
}
