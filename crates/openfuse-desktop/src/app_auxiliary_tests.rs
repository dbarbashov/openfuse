use super::*;
use crate::{Options, session::Session, theme};
use openfuse_core::auxiliary::Key;
use openfuse_device::OperationPhase;
use std::time::{Duration, Instant};
fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
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
        "Auxiliary flow timed out: {:?} {:?} {:?}",
        app.auxiliary_ui.error, app.device.error, app.device.bank
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
    }
    pump(&mut app, |a| a.device.current_binding().is_some());
    app.session = Some(Session::example().unwrap());
    let directory = tempfile::tempdir().unwrap();
    app.archives.set_folder(directory.path().to_owned());
    pump(&mut app, |a| !a.archives.busy());
    app.open_auxiliary();
    (ctx, app, directory)
}
fn read(app: &mut Editor) {
    app.auxiliary_action(Action::Read);
    pump(app, |a| {
        !a.device.operation_busy() && a.device.current_auxiliary().is_some()
    });
    app.auxiliary_action(Action::UseRead);
}
fn pair(app: &Editor) -> (Key, Key) {
    let entries = app.auxiliary_ui.source.as_ref().unwrap().snapshot.entries();
    let first = entries
        .iter()
        .find(|e| e.restore_error().is_none())
        .expect("writable source")
        .key();
    let second = entries
        .iter()
        .find(|e| e.key() != first && first.same_kind(e.key()) && e.restore_error().is_none())
        .expect("compatible target")
        .key();
    (first, second)
}
fn choose(app: &mut Editor) -> (Key, Key) {
    let (source, target) = pair(app);
    app.auxiliary_ui.selected.insert(source);
    app.auxiliary_ui.targets.insert(source, target);
    (source, target)
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
    let (rect, enabled) = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id))
            .expect("drawn auxiliary control")
    });
    assert!(enabled);
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
fn read_archives_raw_system_and_banks_without_changing_local_document() {
    let (_ctx, mut app, _directory) = setup(false);
    let original = app.session.as_ref().unwrap().document.to_xml().unwrap();
    read(&mut app);
    let snapshot = app.device.current_auxiliary().unwrap();
    assert_eq!(snapshot.snapshot.entries().len(), 25);
    assert!(
        snapshot
            .snapshot
            .entries()
            .iter()
            .any(|e| e.key() == Key::System)
    );
    let fingerprint = snapshot.snapshot.fingerprint();
    app.auxiliary_action(Action::SaveRead);
    pump(&mut app, |a| !a.archives.busy());
    let archive = &app.archives.loaded.as_ref().unwrap().1;
    assert_eq!(archive.auxiliary().unwrap().fingerprint(), fingerprint);
    assert_eq!(app.archives.entries.len(), 1);
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        original
    );
    app.open_archives();
    app.open_archive_auxiliary();
    assert!(app.auxiliary_ui.open);
    assert!(!app.archive_ui.open);
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
fn system_entries_are_preserved_but_cannot_be_selected_or_written() {
    let (ctx, mut app, _directory) = setup(false);
    read(&mut app);
    frame(&mut app, &ctx, vec![]);
    let (_, enabled) = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new("aux-entry-System"))
            .unwrap()
    });
    assert!(!enabled);
    app.auxiliary_ui.selected.insert(Key::System);
    app.prepare_auxiliary_restore();
    assert!(app.auxiliary_ui.preview.is_none());
    assert!(app.auxiliary_ui.error.is_some());
    assert!(!app.device.operation_busy());
    let system = app
        .auxiliary_ui
        .source
        .as_ref()
        .unwrap()
        .snapshot
        .entries()
        .iter()
        .find(|e| e.key() == Key::System)
        .unwrap();
    assert!(!system.raw_reports().is_empty());
    assert!(system.restore_error().is_some());
}
#[test]
fn frozen_preview_requires_confirmation_and_verifies_mapped_bank_preserving_local_history() {
    let (ctx, mut app, directory) = setup(false);
    read(&mut app);
    let (source, target) = choose(&mut app);
    let path = directory.path().join("local.fuse");
    let local = app.session.as_mut().unwrap();
    local.save_as(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut metadata = local.document.metadata();
    metadata.name = "Local unsaved sound".into();
    local.metadata(&metadata).unwrap();
    let xml = local.document.to_xml().unwrap();
    app.prepare_auxiliary_restore();
    assert!(
        app.auxiliary_ui.preview.is_some(),
        "{:?}",
        app.auxiliary_ui.error
    );
    app.apply_auxiliary_restore();
    assert!(!app.device.operation_busy());
    app.auxiliary_ui.targets.insert(source, source);
    app.auxiliary_ui.selected.clear();
    assert_eq!(
        app.auxiliary_ui
            .preview
            .as_ref()
            .unwrap()
            .selection
            .mappings()[0]
            .target,
        target
    );
    click(&mut app, &ctx, "aux-confirm");
    click(&mut app, &ctx, "aux-apply");
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
    assert!(app.session.as_ref().unwrap().can_undo());
    assert!(app.dirty());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
#[test]
fn stale_preview_and_duplicate_destination_are_rejected_before_write() {
    let (ctx, mut app, _directory) = setup(false);
    read(&mut app);
    let (source, target) = choose(&mut app);
    app.auxiliary_ui.selected.insert(target);
    app.auxiliary_ui.targets.insert(target, target);
    app.prepare_auxiliary_restore();
    assert!(app.auxiliary_ui.preview.is_none());
    assert!(!app.device.operation_busy());
    app.auxiliary_ui.selected.remove(&target);
    app.auxiliary_ui.targets.insert(source, target);
    app.prepare_auxiliary_restore();
    app.auxiliary_ui.preview.as_mut().unwrap().confirmed = true;
    app.device.capture.as_mut().unwrap().generation += 1;
    app.apply_auxiliary_restore();
    assert!(app.auxiliary_ui.error.as_ref().unwrap().contains("stale"));
    assert!(!app.device.operation_busy());
    click(&mut app, &ctx, "aux-back");
    assert!(app.auxiliary_ui.preview.is_none());
    assert!(app.auxiliary_ui.open);
}
#[test]
fn cancelling_selected_restore_does_not_replace_local_editor() {
    let (_ctx, mut app, _directory) = setup(false);
    read(&mut app);
    choose(&mut app);
    let xml = app.session.as_ref().unwrap().document.to_xml().unwrap();
    app.prepare_auxiliary_restore();
    app.auxiliary_ui.preview.as_mut().unwrap().confirmed = true;
    app.apply_auxiliary_restore();
    app.auxiliary_action(Action::Cancel);
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
fn superchamp_reads_voice_clean_and_fx_without_assuming_full_preset_bank() {
    let (_ctx, mut app, _directory) = setup(true);
    read(&mut app);
    let snapshot = &app.device.current_auxiliary().unwrap().snapshot;
    assert_eq!(snapshot.pid(), 0xe);
    assert_eq!(snapshot.entries().len(), 32);
    assert!(!snapshot.entries().iter().any(|e| e.key() == Key::System));
    assert!(snapshot.entries().iter().any(|e| e.key()
        == Key::Bank {
            control: 1,
            slot: 16
        }));
    app.auxiliary_action(Action::SaveRead);
    pump(&mut app, |a| !a.archives.busy());
    assert!(
        app.archives
            .loaded
            .as_ref()
            .unwrap()
            .1
            .auxiliary()
            .is_some()
    );
    assert!(app.device.bank_backup.is_none());
    choose(&mut app);
    app.prepare_auxiliary_restore();
    assert!(
        app.auxiliary_ui.preview.is_some(),
        "{:?}",
        app.auxiliary_ui.error
    );
    app.auxiliary_ui.preview.as_mut().unwrap().confirmed = true;
    app.apply_auxiliary_restore();
    pump(&mut app, |a| !a.device.operation_busy());
    assert_eq!(
        app.device.bank.phase,
        OperationPhase::Verified,
        "{:?}",
        app.device.bank
    );
}

#[test]
fn disconnect_invalidates_destination_and_preview_but_keeps_owned_source() {
    let (_ctx, mut app, _directory) = setup(false);
    read(&mut app);
    choose(&mut app);
    app.prepare_auxiliary_restore();
    app.auxiliary_ui.preview.as_mut().unwrap().confirmed = true;
    let source = app
        .auxiliary_ui
        .source
        .as_ref()
        .unwrap()
        .snapshot
        .fingerprint();
    app.device.disconnect();
    assert!(app.device.auxiliary.is_none());
    app.apply_auxiliary_restore();
    assert!(app.auxiliary_ui.error.as_ref().unwrap().contains("stale"));
    assert!(!app.device.operation_busy());
    assert_eq!(
        app.auxiliary_ui
            .source
            .as_ref()
            .unwrap()
            .snapshot
            .fingerprint(),
        source
    );
}

#[test]
fn connection_loss_clears_latest_read_and_reconnect_does_not_restore_it() {
    let (_ctx, mut app, _directory) = setup(false);
    read(&mut app);
    app.device.apply_event(openfuse_device::DeviceEvent::Status(
        openfuse_device::DeviceStatus {
            phase: "Waiting for amplifier".into(),
            reconnecting: true,
            ..Default::default()
        },
    ));
    assert!(app.device.auxiliary.is_none());
    app.device.apply_event(openfuse_device::DeviceEvent::Status(
        openfuse_device::DeviceStatus {
            phase: "Connected".into(),
            can_sync: true,
            ..Default::default()
        },
    ));
    assert!(app.device.current_auxiliary().is_none());
    assert!(app.auxiliary_ui.source.is_some());
}

#[test]
fn auxiliary_archive_exposes_preserved_presets_for_extraction_and_exact_profile_restore() {
    let (ctx, mut app, directory) = setup(false);
    read(&mut app);
    app.auxiliary_action(Action::SaveRead);
    pump(&mut app, |a| !a.archives.busy());
    assert_eq!(app.archives.loaded.as_ref().unwrap().1.presets().len(), 24);
    app.open_archives();
    assert!(!app.auxiliary_ui.open);
    app.archives.marked.insert(0);
    app.archives.targets.insert(0, 1);
    let library = directory.path().join("extracted");
    std::fs::create_dir(&library).unwrap();
    app.library.set_folder(library.clone());
    app.archive_action(crate::archive_view::Action::Extract, &ctx);
    pump(&mut app, |a| !a.archives.busy());
    assert_eq!(std::fs::read_dir(library).unwrap().count(), 1);
    app.archive_action(crate::archive_view::Action::ReadNames, &ctx);
    pump(&mut app, |a| {
        !a.device.operation_busy() && a.device.current_bank_names().is_some()
    });
    assert!(app.archives.legacy_profile.is_none());
    app.prepare_archive_restore();
    assert!(app.archive_ui.restore.is_some(), "{:?}", app.archives.error);
}

#[test]
fn smallest_window_keeps_snapshot_and_confirmation_actions_visible() {
    let (ctx, mut app, _directory) = setup(false);
    read(&mut app);
    let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(940.0, 680.0));
    for preview in [false, true] {
        if preview {
            choose(&mut app);
            app.prepare_auxiliary_restore();
        }
        for _ in 0..3 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    ..Default::default()
                },
                |ctx| app.auxiliary_dialog(ctx),
            );
        }
        for id in if preview {
            &["aux-confirm", "aux-back"][..]
        } else {
            &["aux-read", "aux-save-read", "aux-close"][..]
        } {
            let (rect, _) =
                ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id)).unwrap());
            assert!(bounds.contains_rect(rect), "{id}: {rect:?}");
        }
    }
}
