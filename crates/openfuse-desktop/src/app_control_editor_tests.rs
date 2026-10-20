use super::*;
use crate::{Options, session::Session, theme};
use openfuse_device::OperationPhase;
use std::time::{Duration, Instant};
fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        app.device.poll();
        app.operation_update();
        app.control_update(&ctx);
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "Timed out: {:?} {:?} {:?}",
        app.control_ui.error, app.control_ui.library.error, app.device.bank
    );
}
fn setup(sc: bool) -> (egui::Context, Editor, tempfile::TempDir) {
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
        app.control_ui.pid = 0xe;
    }
    pump(&mut app, |a| a.device.current_binding().is_some());
    app.session = Some(Session::example().unwrap());
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("library")).unwrap();
    app.control_ui
        .library
        .set_folder(directory.path().join("library"));
    pump(&mut app, |a| !a.control_ui.library.busy());
    app.open_control_editor();
    (ctx, app, directory)
}
fn read(app: &mut Editor) {
    app.auxiliary_action(crate::auxiliary_view::Action::Read);
    pump(app, |a| {
        !a.device.operation_busy() && a.device.current_auxiliary().is_some()
    });
    app.auxiliary_action(crate::auxiliary_view::Action::UseRead);
}
fn open_bank(app: &mut Editor) {
    read(app);
    let key = app
        .auxiliary_ui
        .source
        .as_ref()
        .unwrap()
        .snapshot
        .entries()
        .iter()
        .find(|e| e.restore_error().is_none())
        .unwrap()
        .key();
    app.edit_auxiliary_entry(key);
    assert!(
        app.control_ui.draft.is_some(),
        "{:?}",
        app.auxiliary_ui.error
    );
}
fn rename(app: &mut Editor, name: &str) {
    app.control_ui
        .draft
        .as_mut()
        .unwrap()
        .edit(|doc| doc.set_name(name).map_err(|e| e.to_string()))
        .unwrap();
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
        |ctx| app.auxiliary_dialog(ctx),
    );
}
fn click(app: &mut Editor, ctx: &egui::Context, id: &str) {
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]);
    let (rect, enabled) =
        ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id)).unwrap());
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
        );
    }
}
#[test]
fn bank_copy_edits_history_are_independent_and_rendering_does_not_mutate() {
    let (ctx, mut app, _dir) = setup(false);
    let original = app.session.as_ref().unwrap().document.to_xml().unwrap();
    open_bank(&mut app);
    let snapshot = app
        .auxiliary_ui
        .source
        .as_ref()
        .unwrap()
        .snapshot
        .fingerprint();
    let xml = app
        .control_ui
        .draft
        .as_ref()
        .unwrap()
        .document
        .to_xml()
        .unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(
        app.control_ui
            .draft
            .as_ref()
            .unwrap()
            .document
            .to_xml()
            .unwrap(),
        xml
    );
    rename(&mut app, "My modulation");
    assert!(app.control_ui.dirty());
    click(&mut app, &ctx, "control-undo");
    assert_eq!(
        app.control_ui
            .draft
            .as_ref()
            .unwrap()
            .document
            .to_xml()
            .unwrap(),
        xml
    );
    click(&mut app, &ctx, "control-redo");
    assert_eq!(
        app.control_ui.draft.as_ref().unwrap().document.name(),
        "My modulation"
    );
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        original
    );
    assert_eq!(
        app.auxiliary_ui
            .source
            .as_ref()
            .unwrap()
            .snapshot
            .fingerprint(),
        snapshot
    );
}
#[test]
fn library_save_search_duplicate_trash_recover_and_open_are_real_roundtrips() {
    let (ctx, mut app, _dir) = setup(false);
    open_bank(&mut app);
    rename(&mut app, "Velvet chorus");
    app.control_action(Action::Save, &ctx);
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert!(!app.control_ui.dirty());
    let entry = app.control_ui.library.entries[0].clone();
    assert!(entry.matches("velvet MOD"));
    assert!(!entry.matches("missing"));
    app.control_action(Action::Duplicate(entry), &ctx);
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert_eq!(app.control_ui.library.entries.len(), 2);
    let entry = app.control_ui.library.entries[1].clone();
    app.control_action(Action::Delete(entry), &ctx);
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert_eq!(app.control_ui.library.trash.len(), 1);
    app.control_action(
        Action::Recover(app.control_ui.library.trash[0].clone()),
        &ctx,
    );
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert_eq!(app.control_ui.library.entries.len(), 2);
    app.control_action(
        Action::Open(app.control_ui.library.entries[0].clone()),
        &ctx,
    );
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert_eq!(
        app.control_ui.draft.as_ref().unwrap().document.name(),
        "Velvet chorus"
    );
    assert!(!app.control_ui.dirty());
}
#[test]
fn export_marks_only_written_revision_saved_and_never_overwrites() {
    let (_ctx, mut app, dir) = setup(false);
    open_bank(&mut app);
    rename(&mut app, "Exported");
    let draft = app.control_ui.draft.as_ref().unwrap();
    let xml = draft.document.to_xml().unwrap();
    let path = dir.path().join("sound.fuse");
    app.control_ui.library.start(Operation::Export(
        draft.document.clone(),
        draft.id,
        path.clone(),
    ));
    rename(&mut app, "Further edit");
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), xml);
    assert!(app.control_ui.dirty());
    let draft = app.control_ui.draft.as_ref().unwrap();
    app.control_ui.library.start(Operation::Export(
        draft.document.clone(),
        draft.id,
        path.clone(),
    ));
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert!(app.control_ui.library.error.is_some());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), xml);
    assert_eq!(
        app.control_ui.draft.as_ref().unwrap().document.name(),
        "Further edit"
    );
}
#[test]
fn replacing_dirty_bank_draft_requires_cancel_discard_or_successful_save() {
    let (ctx, mut app, _dir) = setup(false);
    open_bank(&mut app);
    rename(&mut app, "Keep me");
    app.control_ui.name = "New sound".into();
    app.control_action(Action::New, &ctx);
    assert!(app.control_ui.pending.is_some());
    click(&mut app, &ctx, "control-cancel-pending");
    assert_eq!(
        app.control_ui.draft.as_ref().unwrap().document.name(),
        "Keep me"
    );
    app.control_action(Action::New, &ctx);
    click(&mut app, &ctx, "control-save-pending");
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert_eq!(app.control_ui.library.entries[0].name, "Keep me");
    assert_eq!(
        app.control_ui.draft.as_ref().unwrap().document.name(),
        "New sound"
    );
    app.control_ui.name = "Discard destination".into();
    app.control_action(Action::New, &ctx);
    click(&mut app, &ctx, "control-discard");
    assert_eq!(
        app.control_ui.draft.as_ref().unwrap().document.name(),
        "Discard destination"
    );
}
#[test]
fn application_exit_checks_bank_draft_before_full_preset() {
    let (ctx, mut app, _dir) = setup(false);
    open_bank(&mut app);
    let mut metadata = app.session.as_ref().unwrap().document.metadata();
    metadata.name = "Unsaved full preset".into();
    app.session.as_mut().unwrap().metadata(&metadata).unwrap();
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1220.0, 930.0),
        )),
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let out = ctx.run(input, |ctx| {
        eframe::App::update(&mut app, ctx, &mut eframe::Frame::_new_kittest())
    });
    assert!(
        out.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
    );
    assert!(matches!(app.control_ui.pending, Some(Pending::CloseWindow)));
    assert!(!app.allow_close);
    app.control_action(Action::CancelPending, &ctx);
    assert!(!app.allow_close);
    app.control_ui.pending = Some(Pending::CloseWindow);
    app.control_action(Action::Discard, &ctx);
    assert!(!app.allow_close);
    assert!(matches!(app.pending, Some(MainPending::Close)));
}
#[test]
fn current_and_local_capture_select_scope_without_replacing_main_document() {
    let (ctx, mut app, _dir) = setup(false);
    let original = app.session.as_ref().unwrap().document.to_xml().unwrap();
    app.control_ui.control = 2;
    app.control_action(Action::Local, &ctx);
    let capture = app.control_ui.capture.as_mut().unwrap();
    assert_eq!(capture.available, vec!["Delay", "Reverb"]);
    capture.selected.remove("Reverb");
    app.control_action(Action::Capture, &ctx);
    let doc = &app.control_ui.draft.as_ref().unwrap().document;
    assert_eq!(
        doc.modules()
            .iter()
            .find(|m| m.key == "Reverb")
            .unwrap()
            .model_id,
        Some(0)
    );
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        original
    );
    app.control_action(Action::Current, &ctx);
    assert!(
        app.control_ui
            .capture
            .as_ref()
            .unwrap()
            .source
            .contains("current sound")
    );
    app.control_action(Action::CancelCapture, &ctx);
    assert!(app.control_ui.capture.is_none());
}
#[test]
fn edited_bank_write_is_frozen_confirmed_verified_and_does_not_save_local_files() {
    let (ctx, mut app, dir) = setup(false);
    open_bank(&mut app);
    let full = app.session.as_ref().unwrap().document.to_xml().unwrap();
    rename(&mut app, "Frozen destination");
    app.control_ui.target = 1;
    app.control_review();
    assert!(
        app.auxiliary_ui.preview.is_some(),
        "{:?}",
        app.control_ui.error
    );
    app.apply_auxiliary_restore();
    assert!(!app.device.operation_busy());
    rename(&mut app, "Newer local edit");
    click(&mut app, &ctx, "aux-confirm");
    click(&mut app, &ctx, "aux-apply");
    pump(&mut app, |a| {
        !a.device.operation_busy() && a.device.current_auxiliary().is_some()
    });
    assert_eq!(app.device.bank.phase, OperationPhase::Verified);
    let target = app
        .device
        .current_auxiliary()
        .unwrap()
        .snapshot
        .entries()
        .iter()
        .find(|e| {
            e.key()
                == Key::Bank {
                    control: 1,
                    slot: 1,
                }
        })
        .unwrap();
    assert!(target.label().contains("Frozen destination"));
    assert_eq!(
        app.control_ui.draft.as_ref().unwrap().document.name(),
        "Newer local edit"
    );
    assert!(app.control_ui.dirty());
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        full
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("library"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn stale_bank_preview_cannot_write_and_failed_import_keeps_draft() {
    let (ctx, mut app, dir) = setup(false);
    open_bank(&mut app);
    rename(&mut app, "Keep original");
    let xml = app
        .control_ui
        .draft
        .as_ref()
        .unwrap()
        .document
        .to_xml()
        .unwrap();
    app.control_ui.target = 1;
    app.control_review();
    app.auxiliary_ui.preview.as_mut().unwrap().confirmed = true;
    app.device.disconnect();
    app.apply_auxiliary_restore();
    assert!(app.auxiliary_ui.error.as_ref().unwrap().contains("stale"));
    assert!(!app.device.operation_busy());
    app.auxiliary_ui.preview = None;
    let path = dir.path().join("bad.fuse");
    std::fs::write(&path, "<not-a-bank/>").unwrap();
    app.control_ui
        .library
        .start(Operation::OpenFile(path, Some((0x14, 1))));
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert!(app.control_ui.library.error.is_some());
    assert_eq!(
        app.control_ui
            .draft
            .as_ref()
            .unwrap()
            .document
            .to_xml()
            .unwrap(),
        xml
    );
    app.control_action(Action::Close, &ctx);
    assert!(app.control_ui.dirty());
}
#[test]
fn superchamp_clean_voice_creation_retains_usb_and_only_targets_clean_slot() {
    let (ctx, mut app, _dir) = setup(true);
    read(&mut app);
    app.control_ui.clean = true;
    app.control_ui.name = "Clean channel".into();
    app.control_action(Action::New, &ctx);
    let draft = app.control_ui.draft.as_mut().unwrap();
    assert!(draft.document.is_clean_voice());
    draft
        .edit(|d| d.set_usb_gain(92).map_err(|e| e.to_string()))
        .unwrap();
    app.control_ui.target = 0;
    app.control_review();
    assert!(app.auxiliary_ui.preview.is_none());
    app.control_ui.target = 16;
    app.control_review();
    assert!(
        app.auxiliary_ui.preview.is_some(),
        "{:?}",
        app.control_ui.error
    );
    app.auxiliary_ui.preview.as_mut().unwrap().confirmed = true;
    app.apply_auxiliary_restore();
    pump(&mut app, |a| {
        !a.device.operation_busy() && a.device.current_auxiliary().is_some()
    });
    assert_eq!(app.device.bank.phase, OperationPhase::Verified);
    assert_eq!(
        app.control_ui
            .draft
            .as_ref()
            .unwrap()
            .document
            .usb_gain()
            .unwrap(),
        Some(92)
    );
}
#[test]
fn file_import_requires_xml_profile_and_folder_switch_discards_old_completion() {
    let (_ctx, mut app, dir) = setup(false);
    open_bank(&mut app);
    let doc = app.control_ui.draft.as_ref().unwrap().document.clone();
    let path = dir.path().join("legacy.fuse");
    std::fs::write(&path, doc.to_xml().unwrap()).unwrap();
    app.control_ui
        .library
        .start(Operation::Import(path.clone(), None));
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert!(app.control_ui.library.error.is_some());
    app.control_ui
        .library
        .start(Operation::Import(path, Some((0x14, 1))));
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert_eq!(app.control_ui.library.entries.len(), 1);
    let entry = app.control_ui.library.entries[0].clone();
    app.control_ui.library.start(Operation::Open(entry));
    std::fs::create_dir(dir.path().join("other")).unwrap();
    app.control_ui.library.set_folder(dir.path().join("other"));
    pump(&mut app, |a| !a.control_ui.library.busy());
    assert!(app.control_ui.library.entries.is_empty());
    assert!(app.control_ui.pending.is_none());
}
#[test]
fn minimal_window_keeps_workspace_navigation_reachable() {
    let (ctx, mut app, _dir) = setup(false);
    open_bank(&mut app);
    let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(940.0, 680.0));
    for _ in 0..3 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(bounds),
                ..Default::default()
            },
            |ctx| app.auxiliary_dialog(ctx),
        );
    }
    for id in ["control-close", "control-browse"] {
        let (rect, _) = ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id)).unwrap());
        assert!(bounds.contains_rect(rect), "{id} {rect:?}");
    }
}
