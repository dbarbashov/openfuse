use super::*;
use crate::{Options, media_state::Operation, session::Session, theme};
use openfuse_audio::{Phase, Player};
use openfuse_core::media_library::{MediaFormat, MediaLibrary};
use std::time::{Duration, Instant};
fn frame(ctx: &egui::Context, app: &mut Editor, events: Vec<egui::Event>) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1220.0, 930.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest()),
    );
}
fn click(ctx: &egui::Context, app: &mut Editor, id: &str) {
    let (rect, enabled): (egui::Rect, bool) = ctx.data(|d| d.get_temp(egui::Id::new(id)).unwrap());
    assert!(enabled, "{id} disabled");
    for pressed in [true, false] {
        frame(
            ctx,
            app,
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
    for _ in 0..4 {
        frame(ctx, app, vec![]);
    }
}
fn settle(ctx: &egui::Context, app: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        frame(ctx, app, vec![]);
        if !app.media_ui.library.busy() && app.media_ui.player.snapshot().phase != Phase::Loading {
            break;
        }
        assert!(Instant::now() < deadline, "media worker timeout");
        std::thread::sleep(Duration::from_millis(3));
    }
    for _ in 0..3 {
        frame(ctx, app, vec![]);
    }
}
fn setup() -> (egui::Context, Editor, tempfile::TempDir) {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = Editor::from_options(
        Options {
            demo: true,
            ..Default::default()
        },
        &ctx,
        None,
    );
    app.media_ui.player = Player::headless();
    app.open_media();
    let root = tempfile::tempdir().unwrap();
    let library = root.path().join("library");
    std::fs::create_dir(&library).unwrap();
    app.media_folder(library);
    settle(&ctx, &mut app);
    let input = root.path().join("Beat.wav");
    std::fs::write(&input, super::super::media_demo::demo_wave()).unwrap();
    app.media_ui.library.start(Operation::Import(vec![input]));
    settle(&ctx, &mut app);
    app.media_ui.library.selected = Some(app.media_ui.library.entries[0].id.clone());
    frame(&ctx, &mut app, vec![]);
    (ctx, app, root)
}
fn xml(app: &Editor) -> String {
    app.session.as_ref().unwrap().document.to_xml().unwrap()
}
#[test]
fn pointer_player_load_seek_mix_repeat_stop_never_changes_preset_or_autoplays() {
    let (ctx, mut app, _root) = setup();
    let source = xml(&app);
    click(&ctx, &mut app, "media-load");
    settle(&ctx, &mut app);
    let snap = app.media_ui.player.snapshot();
    assert_eq!(snap.phase, Phase::Ready);
    assert!((snap.duration - 8.0).abs() < 0.01);
    app.media_action(Action::Seek(3.0));
    app.media_action(Action::Volume(0.25));
    app.media_action(Action::Balance(-0.5));
    click(&ctx, &mut app, "media-repeat");
    let snap = app.media_ui.player.snapshot();
    assert!((snap.position - 3.0).abs() < 0.01);
    assert_eq!(snap.volume, 0.25);
    assert_eq!(snap.balance, -0.5);
    assert!(snap.repeat);
    click(&ctx, &mut app, "media-play");
    assert!(app.media_ui.error.as_ref().unwrap().contains("headless"));
    assert_ne!(app.media_ui.player.snapshot().phase, Phase::Playing);
    click(&ctx, &mut app, "media-stop");
    assert_eq!(app.media_ui.player.snapshot().position, 0.0);
    assert_eq!(xml(&app), source);
    assert!(!app.session.as_ref().unwrap().dirty());
}
#[test]
fn pending_verified_load_is_cancelled_by_stop_and_close() {
    let (ctx, mut app, _root) = setup();
    let entry = app.media_ui.library.entries[0].clone();
    app.media_action(Action::Load(entry.clone()));
    app.media_action(Action::Stop);
    settle(&ctx, &mut app);
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Empty);
    assert!(app.media_ui.loaded.is_none());
    app.media_action(Action::Load(entry));
    app.media_action(Action::Close);
    settle(&ctx, &mut app);
    assert!(!app.media_ui.open);
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Empty);
}
#[test]
fn folder_switch_discards_old_read_and_empty_catalog_clears_player() {
    let (ctx, mut app, root) = setup();
    app.media_action(Action::Load(app.media_ui.library.entries[0].clone()));
    let other = root.path().join("other");
    std::fs::create_dir(&other).unwrap();
    app.media_folder(other.clone());
    settle(&ctx, &mut app);
    assert_eq!(app.media_ui.library.folder, Some(other));
    assert!(app.media_ui.library.entries.is_empty());
    assert!(app.media_ui.loaded.is_none());
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Empty);
}
#[test]
fn archive_restore_and_search_keep_original_media_and_preset_intact() {
    let (ctx, mut app, root) = setup();
    let source = xml(&app);
    let entry = app.media_ui.library.entries[0].clone();
    let original = std::fs::read(root.path().join("Beat.wav")).unwrap();
    app.media_ui.library.query = "BEAT wav".into();
    assert!(entry.matches_query(&app.media_ui.library.query));
    app.media_action(Action::Load(entry));
    settle(&ctx, &mut app);
    click(&ctx, &mut app, "media-archive");
    settle(&ctx, &mut app);
    assert!(app.media_ui.library.entries.is_empty());
    assert_eq!(app.media_ui.library.trash.len(), 1);
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Empty);
    app.media_ui.library.deleted = true;
    frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, "media-restore");
    settle(&ctx, &mut app);
    assert_eq!(app.media_ui.library.entries.len(), 1);
    assert!(app.media_ui.library.trash.is_empty());
    assert_eq!(
        std::fs::read(root.path().join("Beat.wav")).unwrap(),
        original
    );
    assert_eq!(xml(&app), source);
}
#[test]
fn pointer_track_link_is_one_local_undo_step_and_main_dirty_guard_applies() {
    let (ctx, mut app, _root) = setup();
    let source = xml(&app);
    app.media_ui.show_band = true;
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]);
    }
    click(&ctx, &mut app, "media-link");
    let session = app.session.as_mut().unwrap();
    let band = session.document.band().unwrap().unwrap();
    assert_eq!(band.filename, "Beat.wav");
    assert_eq!(band.location, 9);
    assert_eq!(band.speed, 32767);
    assert_eq!(band.pitch, 32767);
    assert_eq!(band.audio_mix, 1);
    assert_eq!(band.balance, 29127);
    assert!(session.dirty());
    assert!(session.take_device_edits().is_empty());
    assert!(app.device_origin.is_none());
    assert!(!app.live_requested);
    session.undo().unwrap();
    assert_eq!(session.document.to_xml().unwrap(), source);
    session.redo().unwrap();
    assert_eq!(session.document.band().unwrap().unwrap(), band);
    app.request(super::super::Pending::Demo, &ctx);
    assert!(app.pending.is_some());
    app.pending = None;
}
#[test]
fn stale_band_patch_is_refused_without_erasing_new_preset_edits() {
    let (ctx, mut app, _root) = setup();
    let source = xml(&app);
    let mut band = Band::default();
    band.link_pc_audio("Beat.wav").unwrap();
    app.session
        .as_mut()
        .unwrap()
        .parameter("Amplifier", 0, 12345)
        .unwrap();
    let changed = xml(&app);
    app.apply_band(&source, band);
    assert_eq!(xml(&app), changed);
    assert!(
        app.media_ui
            .error
            .as_ref()
            .unwrap()
            .contains("preset changed")
    );
    frame(&ctx, &mut app, vec![]);
}
#[test]
fn folder_import_keeps_partial_success_and_midi_is_preserved_but_not_playable() {
    let (ctx, mut app, root) = setup();
    let inputs = root.path().join("batch");
    std::fs::create_dir(&inputs).unwrap();
    std::fs::write(inputs.join("Empty.wav"), []).unwrap();
    std::fs::write(
        inputs.join("Pattern.mid"),
        b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk\0\0\0\x04\0\xff\x2f\0",
    )
    .unwrap();
    app.media_ui.library.start(Operation::ImportFolder(inputs));
    settle(&ctx, &mut app);
    assert_eq!(app.media_ui.library.entries.len(), 2);
    assert!(!app.media_ui.library.diagnostics.is_empty());
    let midi = app
        .media_ui
        .library
        .entries
        .iter()
        .find(|entry| entry.format == MediaFormat::Midi)
        .unwrap();
    app.media_ui.library.selected = Some(midi.id.clone());
    app.media_ui.show_band = true;
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]);
    }
    for key in ["media-load", "media-link"] {
        let (_, enabled): (egui::Rect, bool) =
            ctx.data(|d| d.get_temp(egui::Id::new(key)).unwrap());
        assert!(!enabled, "{key}");
    }
}
#[test]
fn rendering_gdec_unknown_band_values_never_clamps_them_and_edit_has_undo() {
    let (ctx, mut app, _root) = setup();
    let mut session = Session::new_preset(10).unwrap();
    let mut band = Band::default();
    band.set_filename("Practice.mid").unwrap();
    band.location = 4;
    band.repeat = 7;
    band.tempo = 500;
    band.transpose = -20;
    band.count_in = 20;
    session.band(&band).unwrap();
    app.session = Some(session);
    app.media_ui.show_band = true;
    let before = xml(&app);
    for _ in 0..4 {
        frame(&ctx, &mut app, vec![]);
    }
    assert_eq!(xml(&app), before);
    band.tempo = 120;
    app.media_action(Action::SetBand(before.clone(), band));
    let session = app.session.as_mut().unwrap();
    assert_eq!(session.document.band().unwrap().unwrap().tempo, 120);
    session.undo().unwrap();
    assert_eq!(session.document.to_xml().unwrap(), before);
}
#[test]
fn linked_name_resolution_never_picks_the_first_ambiguous_or_missing_file() {
    let (ctx, mut app, root) = setup();
    app.media_action(Action::FindLinked("Beat.wav".into()));
    assert!(app.media_ui.library.selected.is_some());
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let bytes = super::super::media_demo::demo_wave();
    std::fs::write(elsewhere.join("Beat.wav"), &bytes).unwrap();
    app.media_ui
        .library
        .start(Operation::Import(vec![elsewhere.join("Beat.wav")]));
    settle(&ctx, &mut app);
    app.media_action(Action::FindLinked("Beat.wav".into()));
    assert!(app.media_ui.library.selected.is_none());
    assert!(
        app.media_ui
            .library
            .notice
            .as_ref()
            .unwrap()
            .contains("Several")
    );
    app.media_action(Action::FindLinked("Missing.wav".into()));
    assert!(app.media_ui.library.selected.is_none());
    assert!(
        app.media_ui
            .library
            .notice
            .as_ref()
            .unwrap()
            .contains("not in this library")
    );
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Empty);
}
#[test]
fn changed_managed_copy_is_refused_and_previous_audio_is_cleared_before_load() {
    let (ctx, mut app, _root) = setup();
    let entry = app.media_ui.library.entries[0].clone();
    app.media_action(Action::Load(entry.clone()));
    settle(&ctx, &mut app);
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Ready);
    std::fs::write(&entry.path, b"changed").unwrap();
    app.media_action(Action::Load(entry));
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Empty);
    settle(&ctx, &mut app);
    assert!(app.media_ui.library.error.is_some());
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Empty);
}
#[test]
fn automatic_catalog_refresh_observes_external_import_without_autoplay() {
    let (ctx, mut app, root) = setup();
    let source = root.path().join("External.wav");
    std::fs::write(&source, super::super::media_demo::demo_wave()).unwrap();
    MediaLibrary::open(app.media_ui.library.folder.as_ref().unwrap())
        .unwrap()
        .import_file(&source)
        .unwrap();
    app.media_ui.last_refresh = Instant::now() - Duration::from_secs(4);
    settle(&ctx, &mut app);
    assert_eq!(app.media_ui.library.entries.len(), 2);
    assert_eq!(app.media_ui.player.snapshot().phase, Phase::Empty);
}
#[test]
fn minimum_window_keeps_close_action_visible() {
    let (ctx, mut app, _root) = setup();
    app.media_ui.library.entries[0].name = format!("{}.wav", "Long track title ".repeat(40));
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(940.0, 680.0));
    for _ in 0..4 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| app.media_dialog(ctx),
        );
    }
    let (rect, _): (egui::Rect, bool) =
        ctx.data(|d| d.get_temp(egui::Id::new("media-close")).unwrap());
    assert!(screen.contains_rect(rect));
}
#[test]
fn preset_playback_preferences_apply_only_to_the_verified_matching_track_without_autoplay() {
    let (ctx, mut app, _root) = setup();
    let entry = app.media_ui.library.entries[0].clone();
    app.media_action(Action::Load(entry.clone()));
    settle(&ctx, &mut app);
    let source = xml(&app);
    let mut band = Band::default();
    band.link_pc_audio(&entry.name).unwrap();
    band.balance = 13107;
    band.repeat = 1;
    app.apply_band(&source, band.clone());
    let linked = xml(&app);
    app.media_action(Action::UsePreset(linked.clone(), band.clone()));
    let snapshot = app.media_ui.player.snapshot();
    assert_eq!(snapshot.phase, Phase::Ready);
    assert!(snapshot.repeat);
    assert!((snapshot.balance + 0.6).abs() < 0.0001);
    assert_eq!(xml(&app), linked);
    app.media_action(Action::Balance(0.5));
    app.media_action(Action::UsePreset(source, band));
    assert_eq!(app.media_ui.player.snapshot().balance, 0.5);
    assert!(app.media_ui.error.is_some());
}
#[test]
fn filename_edit_stays_local_until_committed_and_branch_reset_is_one_undo_step() {
    let (ctx, mut app, _root) = setup();
    app.session = Some(Session::new_preset(10).unwrap());
    app.media_ui.show_band = true;
    let source = xml(&app);
    for _ in 0..4 {
        frame(&ctx, &mut app, vec![]);
    }
    app.media_ui.filename = "New pattern.mid".into();
    frame(&ctx, &mut app, vec![]);
    assert_eq!(xml(&app), source);
    click(&ctx, &mut app, "media-band-filename");
    let session = app.session.as_mut().unwrap();
    let band = session.document.band().unwrap().unwrap();
    assert_eq!(band.filename, "New pattern.mid");
    assert_eq!(band.band_type, 1);
    session.undo().unwrap();
    assert_eq!(session.document.to_xml().unwrap(), source);
}
#[test]
fn unknown_band_type_is_visible_and_rendering_preserves_its_midi_storage_branch() {
    let (ctx, mut app, _root) = setup();
    let source=Session::new_preset(10).unwrap().document.to_xml().unwrap().replace("</Preset>","<Band Type=\"255\" Repeat=\"3\"><SongFile Location=\"4\"></SongFile><Tempo>0</Tempo><Transpose>-3</Transpose><CountIn>4</CountIn><DrumSolo>1</DrumSolo></Band></Preset>");
    app.session = Some(Session::parse(&source, None).unwrap());
    app.media_ui.show_band = true;
    for _ in 0..4 {
        frame(&ctx, &mut app, vec![]);
    }
    assert_eq!(xml(&app), source);
    assert_eq!(
        app.session
            .as_ref()
            .unwrap()
            .document
            .band()
            .unwrap()
            .unwrap()
            .band_type,
        255
    );
}
