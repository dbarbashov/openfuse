use super::*;
use crate::{Options, session::Session, theme};
use openfuse_core::mounted_storage::{CollisionPolicy, FileKind};
use std::path::Path;
use std::time::{Duration, Instant};
fn preset() -> String {
    Session::new_preset(10)
        .unwrap()
        .document
        .to_xml()
        .unwrap()
        .replacen("<Preset ", "<Preset amplifier=\"G-DEC 3.0\" ", 1)
}
fn fixture() -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    for path in [
        "sd/Audio",
        "sd/Midi",
        "sd/Phrase",
        "sd/Presets",
        "local",
        "exports",
        "library",
    ] {
        std::fs::create_dir_all(folder.path().join(path)).unwrap();
    }
    std::fs::write(
        folder.path().join("sd/Audio/Groove.wav"),
        super::super::media_demo::demo_wave(),
    )
    .unwrap();
    std::fs::write(
        folder.path().join("sd/Midi/Pattern.mid"),
        b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk\0\0\0\x04\0\xff\x2f\0",
    )
    .unwrap();
    std::fs::write(
        folder.path().join("sd/Phrase/recording.raw"),
        b"retained original phrase bytes",
    )
    .unwrap();
    std::fs::write(folder.path().join("sd/Presets/Practice.fuse"), preset()).unwrap();
    std::fs::write(
        folder.path().join("local/Groove.wav"),
        super::super::media_demo::demo_wave(),
    )
    .unwrap();
    std::fs::write(folder.path().join("local/New preset.fuse"), preset()).unwrap();
    folder
}
fn pump(app: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        app.media_update(&ctx);
        app.storage_update(&ctx);
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("Storage worker timed out: {:?}", app.storage_ui.error);
}
fn setup() -> (egui::Context, Editor, tempfile::TempDir) {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = Editor::from_options(Options::default(), &ctx, None);
    app.media_ui.player = openfuse_audio::Player::headless();
    app.session = Some(Session::example().unwrap());
    let folder = fixture();
    app.storage_ui
        .set_root(folder.path().join("sd"), Volume::Sd);
    app.storage_ui.open = true;
    pump(&mut app, |a| !a.storage_ui.busy());
    (ctx, app, folder)
}
fn frame(app: &mut Editor, ctx: &egui::Context, events: Vec<egui::Event>, size: egui::Vec2) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| {
            app.confirm(ctx);
            app.storage_dialog(ctx);
        },
    );
}
fn click(app: &mut Editor, ctx: &egui::Context, id: &str) {
    for _ in 0..3 {
        frame(app, ctx, vec![], egui::vec2(1220.0, 930.0));
    }
    let (rect, enabled) = ctx.data(|d| {
        d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id))
            .expect("drawn storage control")
    });
    assert!(enabled, "{id}");
    pointer(app, ctx, rect);
}
fn pointer(app: &mut Editor, ctx: &egui::Context, rect: egui::Rect) {
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
fn upload(app: &mut Editor, paths: Vec<PathBuf>, policy: CollisionPolicy) {
    app.storage_ui.start(Operation::Upload(paths, policy));
    pump(app, |a| !a.storage_ui.busy());
    assert!(
        app.storage_ui.preview.is_some(),
        "{:?}",
        app.storage_ui.error
    );
}
#[cfg(unix)]
fn download(app: &mut Editor, destination: &Path) {
    app.storage_ui.start(Operation::Download(
        app.storage_ui.selected_entries(),
        destination.to_owned(),
        app.storage_ui.policy(),
    ));
    pump(app, |a| !a.storage_ui.busy());
    assert!(
        app.storage_ui.preview.is_some(),
        "{:?}",
        app.storage_ui.error
    );
}
fn apply(app: &mut Editor) {
    app.storage_ui.preview.as_mut().unwrap().confirmed = true;
    app.storage_ui.apply();
    pump(app, |a| !a.storage_ui.busy());
    assert!(app.storage_ui.receipt.is_some());
}
#[test]
fn explicit_inventory_search_and_selection_do_not_connect_device_or_edit_preset() {
    let (ctx, mut app, _folder) = setup();
    let xml = app.session.as_ref().unwrap().document.to_xml().unwrap();
    assert_eq!(app.storage_ui.entries.len(), 4);
    assert!(
        app.storage_ui
            .entries
            .iter()
            .any(|e| e.kind == FileKind::Phrase && e.name == "recording.raw")
    );
    app.storage_ui.query = "Groove".into();
    click(
        &mut app,
        &ctx,
        &format!(
            "storage-entry-{}",
            Path::new("Audio").join("Groove.wav").display()
        ),
    );
    assert_eq!(app.storage_ui.selected.len(), 1);
    app.storage_ui.filter = Some(FileKind::Midi);
    app.storage_ui.query.clear();
    click(
        &mut app,
        &ctx,
        &format!(
            "storage-entry-{}",
            Path::new("Midi").join("Pattern.mid").display()
        ),
    );
    assert_eq!(app.storage_ui.selected.len(), 2);
    assert!(!app.device.has_connection());
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        xml
    );
}
#[test]
#[cfg(unix)]
fn pointer_keep_both_copy_preview_freezes_names_and_preserves_originals() {
    let (ctx, mut app, folder) = setup();
    let input = folder.path().join("local/New preset.fuse");
    let original = std::fs::read(&input).unwrap();
    upload(
        &mut app,
        vec![folder.path().join("local/Groove.wav"), input.clone()],
        CollisionPolicy::KeepBoth,
    );
    let items = app.storage_ui.preview.as_ref().unwrap().plan.items();
    assert!(items[0].destination.ends_with("Groove (2).wav"));
    assert!(!items[1].changes.is_empty());
    assert!(!items[0].destination.exists());
    app.storage_ui.apply();
    assert!(!app.storage_ui.busy());
    assert!(app.storage_ui.preview.is_some());
    app.storage_ui.keep_both = false;
    click(&mut app, &ctx, "storage-confirm");
    click(&mut app, &ctx, "storage-apply");
    pump(&mut app, |a| !a.storage_ui.busy());
    let receipt = &app.storage_ui.receipt.as_ref().unwrap().result;
    assert_eq!(receipt.completed.len(), 2);
    assert!(receipt.error.is_none());
    assert_eq!(std::fs::read(&input).unwrap(), original);
    let copied = std::fs::read_to_string(folder.path().join("sd/Presets/New preset.fuse")).unwrap();
    assert!(copied.contains("amplifier=\"G-DEC3\""));
    assert!(folder.path().join("sd/Audio/Groove.wav").exists());
    assert!(folder.path().join("sd/Audio/Groove (2).wav").exists());
}
#[test]
fn default_collision_refuses_without_touching_either_file() {
    let (_ctx, mut app, folder) = setup();
    let target = folder.path().join("sd/Audio/Groove.wav");
    let before = std::fs::read(&target).unwrap();
    app.storage_ui.start(Operation::Upload(
        vec![folder.path().join("local/Groove.wav")],
        CollisionPolicy::Reject,
    ));
    pump(&mut app, |a| !a.storage_ui.busy());
    assert!(app.storage_ui.preview.is_none());
    assert!(app.storage_ui.error.is_some());
    assert_eq!(std::fs::read(target).unwrap(), before);
}
#[test]
#[cfg(unix)]
fn successful_download_can_add_only_verified_local_tracks_to_managed_library() {
    let (ctx, mut app, folder) = setup();
    app.storage_ui.selected.extend([
        PathBuf::from("Audio/Groove.wav"),
        PathBuf::from("Midi/Pattern.mid"),
        PathBuf::from("Phrase/recording.raw"),
    ]);
    download(&mut app, &folder.path().join("exports"));
    apply(&mut app);
    assert_eq!(
        app.storage_ui
            .receipt
            .as_ref()
            .unwrap()
            .result
            .completed
            .len(),
        3
    );
    app.media_folder(folder.path().join("library"));
    pump(&mut app, |a| !a.media_ui.library.busy());
    click(&mut app, &ctx, "storage-add-media");
    pump(&mut app, |a| {
        a.storage_ui.pending_media.is_none() && !a.media_ui.library.busy()
    });
    assert!(app.media_ui.open);
    assert_eq!(app.media_ui.library.entries.len(), 2);
    assert!(app.media_ui.library.entries.iter().all(|e| {
        e.original_path
            .starts_with(folder.path().join("exports").canonicalize().unwrap())
    }));
    assert_eq!(
        app.media_ui.player.snapshot().phase,
        openfuse_audio::Phase::Empty
    );
    assert_eq!(
        std::fs::read(folder.path().join("exports/recording.raw")).unwrap(),
        b"retained original phrase bytes"
    );
}

#[test]
#[cfg(unix)]
fn changed_downloaded_file_is_refused_before_media_library_publication() {
    let (_ctx, mut app, folder) = setup();
    app.storage_ui
        .selected
        .insert(PathBuf::from("Audio/Groove.wav"));
    download(&mut app, &folder.path().join("exports"));
    apply(&mut app);
    let mut bytes = super::super::media_demo::demo_wave();
    bytes[48] ^= 1;
    std::fs::write(folder.path().join("exports/Groove.wav"), bytes).unwrap();
    app.media_folder(folder.path().join("library"));
    pump(&mut app, |a| !a.media_ui.library.busy());
    app.storage_add_media();
    pump(&mut app, |a| {
        a.storage_ui.pending_media.is_none() && !a.media_ui.library.busy()
    });
    assert!(app.media_ui.library.entries.is_empty());
    assert!(!app.media_ui.library.diagnostics.is_empty());
    assert!(
        app.storage_ui
            .receipt
            .as_ref()
            .unwrap()
            .result
            .completed
            .len()
            == 1
    );
}
#[test]
fn changed_source_refusal_remains_visible_after_automatic_rescan() {
    let (_ctx, mut app, folder) = setup();
    let source = folder.path().join("local/New preset.fuse");
    upload(&mut app, vec![source.clone()], CollisionPolicy::Reject);
    std::fs::write(
        &source,
        preset().replace("Untitled preset", "Changed after preview"),
    )
    .unwrap();
    apply(&mut app);
    let receipt = &app.storage_ui.receipt.as_ref().unwrap().result;
    assert!(receipt.completed.is_empty());
    assert!(receipt.error.is_some());
    assert!(!folder.path().join("sd/Presets/New preset.fuse").exists());
    assert_eq!(app.storage_ui.entries.len(), 4);
}
#[test]
fn destination_created_after_preview_is_never_overwritten() {
    let (_ctx, mut app, folder) = setup();
    upload(
        &mut app,
        vec![folder.path().join("local/New preset.fuse")],
        CollisionPolicy::Reject,
    );
    let target = folder.path().join("sd/Presets/New preset.fuse");
    std::fs::write(&target, b"external owner data").unwrap();
    apply(&mut app);
    assert!(
        app.storage_ui
            .receipt
            .as_ref()
            .unwrap()
            .result
            .error
            .is_some()
    );
    assert_eq!(std::fs::read(target).unwrap(), b"external owner data");
}
#[test]
fn internal_layout_excludes_presets_and_refuses_preset_upload() {
    let (_ctx, mut app, folder) = setup();
    app.storage_ui
        .set_root(folder.path().join("sd"), Volume::Internal);
    pump(&mut app, |a| !a.storage_ui.busy());
    assert_eq!(app.storage_ui.entries.len(), 3);
    app.storage_ui.start(Operation::Upload(
        vec![folder.path().join("local/New preset.fuse")],
        CollisionPolicy::Reject,
    ));
    pump(&mut app, |a| !a.storage_ui.busy());
    assert!(app.storage_ui.preview.is_none());
    assert!(app.storage_ui.error.is_some());
}
#[test]
fn verified_preset_open_uses_dirty_guard_detaches_and_refuses_original_overwrite() {
    let (ctx, mut app, folder) = setup();
    let mut metadata = app.session.as_ref().unwrap().document.metadata();
    metadata.name = "Unsaved local edits".into();
    app.session.as_mut().unwrap().metadata(&metadata).unwrap();
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    let source = folder.path().join("sd/Presets/Practice.fuse");
    let bytes = std::fs::read(&source).unwrap();
    app.storage_ui
        .selected
        .insert(PathBuf::from("Presets/Practice.fuse"));
    click(&mut app, &ctx, "storage-open-preset");
    pump(&mut app, |a| a.pending.is_some());
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![], egui::vec2(1220.0, 930.0));
    }
    let rect = ctx.data(|d| {
        d.get_temp::<egui::Rect>(egui::Id::new("discard-button"))
            .unwrap()
    });
    pointer(&mut app, &ctx, rect);
    let session = app.session.as_mut().unwrap();
    assert!(session.path.is_none());
    assert!(session.is_storage_copy());
    assert!(session.dirty());
    assert!(session.save_as(&source).is_err());
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    session
        .save_as(&folder.path().join("exports/edited.fuse"))
        .unwrap();
    assert!(!session.dirty());
    assert!(app.device_origin.is_none());
}
#[test]
fn changing_root_discards_pending_readonly_preview() {
    let (_ctx, mut app, folder) = setup();
    app.storage_ui.start(Operation::Upload(
        vec![folder.path().join("local/New preset.fuse")],
        CollisionPolicy::Reject,
    ));
    app.storage_ui
        .set_root(folder.path().join("exports"), Volume::Sd);
    pump(&mut app, |a| !a.storage_ui.busy());
    assert!(app.storage_ui.preview.is_none());
    assert!(app.storage_ui.entries.is_empty());
    assert!(!folder.path().join("sd/Presets/New preset.fuse").exists());
}
#[test]
#[cfg(unix)]
fn active_copy_refuses_root_switch_and_keeps_its_receipt() {
    let (_ctx, mut app, folder) = setup();
    upload(
        &mut app,
        vec![folder.path().join("local/New preset.fuse")],
        CollisionPolicy::Reject,
    );
    app.storage_ui.preview.as_mut().unwrap().confirmed = true;
    app.storage_ui.apply();
    app.storage_ui
        .set_root(folder.path().join("exports"), Volume::Sd);
    assert_eq!(
        app.storage_ui.root.as_deref(),
        Some(folder.path().join("sd").as_path())
    );
    assert!(app.storage_ui.error.as_ref().unwrap().contains("receipt"));
    pump(&mut app, |a| !a.storage_ui.busy());
    assert_eq!(
        app.storage_ui
            .receipt
            .as_ref()
            .unwrap()
            .result
            .completed
            .len(),
        1
    );
}
#[test]
fn cancel_and_close_during_plan_never_writes_or_opens_a_late_preview() {
    let (_ctx, mut app, folder) = setup();
    app.storage_ui.start(Operation::Upload(
        vec![folder.path().join("local/New preset.fuse")],
        CollisionPolicy::Reject,
    ));
    app.storage_action(Action::Close);
    pump(&mut app, |a| !a.storage_ui.busy());
    assert!(!app.storage_ui.open);
    assert!(app.storage_ui.preview.is_none());
    assert!(!folder.path().join("sd/Presets/New preset.fuse").exists());
}
#[test]
fn malformed_preset_read_does_not_replace_the_local_editor() {
    let (_ctx, mut app, folder) = setup();
    std::fs::write(folder.path().join("sd/Presets/Broken.fuse"), "not XML").unwrap();
    app.storage_ui.start(Operation::Scan);
    pump(&mut app, |a| !a.storage_ui.busy());
    let before = app.session.as_ref().unwrap().document.to_xml().unwrap();
    app.storage_ui
        .selected
        .insert(PathBuf::from("Presets/Broken.fuse"));
    app.storage_action(Action::OpenPreset);
    pump(&mut app, |a| !a.storage_ui.busy());
    assert!(app.storage_ui.error.is_some());
    assert!(app.pending.is_none());
    assert_eq!(
        app.session.as_ref().unwrap().document.to_xml().unwrap(),
        before
    );
}
#[test]
fn minimum_viewport_keeps_copy_confirmation_and_close_accessible() {
    let (ctx, mut app, folder) = setup();
    let size = egui::vec2(940.0, 680.0);
    let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    app.storage_ui.receipt = Some(crate::storage_state::Receipt {
        direction: openfuse_core::mounted_storage::Direction::Download,
        result: openfuse_core::mounted_storage::TransferReceipt {
            completed: vec![],
            interrupted: None,
            cancelled: false,
            error: Some(
                "A changed source was refused; retained copies remain available. ".repeat(15),
            ),
        },
    });
    for preview in [false, true] {
        if preview {
            upload(
                &mut app,
                vec![folder.path().join("local/New preset.fuse")],
                CollisionPolicy::Reject,
            );
        }
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![], size);
        }
        for id in if preview {
            &["storage-confirm", "storage-back"][..]
        } else {
            &["storage-root", "storage-close"][..]
        } {
            let (rect, _) =
                ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new(id)).unwrap());
            assert!(bounds.contains_rect(rect), "{id}: {rect:?}");
        }
    }
}
