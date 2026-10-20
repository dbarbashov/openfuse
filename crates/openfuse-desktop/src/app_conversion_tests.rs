use super::*;
use crate::{Options, theme};
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
fn click(ctx: &egui::Context, app: &mut Editor, key: &str) {
    let (rect, enabled): (egui::Rect, bool) = ctx.data(|d| d.get_temp(egui::Id::new(key)).unwrap());
    assert!(enabled, "{key} disabled");
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
fn editor(ctx: &egui::Context) -> Editor {
    theme::install(ctx);
    Editor::from_options(
        Options {
            demo: true,
            ..Default::default()
        },
        ctx,
        None,
    )
}
fn xml(app: &Editor) -> String {
    app.session.as_ref().unwrap().document.to_xml().unwrap()
}
fn preview(app: &mut Editor) {
    app.prepare_current_conversion();
    let draft = app.conversion_draft.as_mut().unwrap();
    draft.options.target_product = 1;
    draft.review();
    assert!(draft.plan.is_some(), "{:?}", draft.error);
}
#[test]
fn pointer_review_apply_keeps_source_in_one_undo_step_and_disables_live() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    let source = xml(&app);
    app.prepare_current_conversion();
    app.conversion_draft
        .as_mut()
        .unwrap()
        .options
        .target_product = 1;
    for _ in 0..2 {
        frame(&ctx, &mut app, vec![]);
    }
    click(&ctx, &mut app, "conversion-review");
    assert_eq!(xml(&app), source);
    let (_, enabled): (egui::Rect, bool) =
        ctx.data(|d| d.get_temp(egui::Id::new("conversion-apply")).unwrap());
    assert!(!enabled);
    click(&ctx, &mut app, "conversion-ack");
    click(&ctx, &mut app, "conversion-apply");
    assert!(app.conversion_draft.is_none());
    let converted = xml(&app);
    assert_ne!(converted, source);
    let session = app.session.as_mut().unwrap();
    assert!(session.dirty());
    assert!(session.path.is_none());
    assert_eq!(session.document.product_id(), Some(1));
    session.undo().unwrap();
    assert_eq!(session.document.to_xml().unwrap(), source);
    assert!(!session.can_undo());
    session.redo().unwrap();
    assert_eq!(session.document.to_xml().unwrap(), converted);
    assert!(!app.live_requested);
    assert!(app.device_origin.is_none());
}
#[test]
fn cancelling_and_unreviewed_apply_leave_editor_unchanged() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    let source = xml(&app);
    preview(&mut app);
    app.apply_conversion(&ctx);
    assert_eq!(xml(&app), source);
    assert!(app.conversion_draft.as_ref().unwrap().error.is_some());
    for _ in 0..2 {
        frame(&ctx, &mut app, vec![]);
    }
    click(&ctx, &mut app, "conversion-cancel");
    assert!(app.conversion_draft.is_none());
    assert_eq!(xml(&app), source);
}
#[test]
fn settings_change_invalidates_frozen_plan_and_acknowledgment() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    preview(&mut app);
    app.conversion_draft.as_mut().unwrap().reviewed = true;
    for _ in 0..2 {
        frame(&ctx, &mut app, vec![]);
    }
    click(&ctx, &mut app, "conversion-repair");
    let draft = app.conversion_draft.as_ref().unwrap();
    assert!(draft.plan.is_none());
    assert!(!draft.reviewed);
}
#[test]
fn stale_current_source_cannot_apply_or_erase_a_new_edit() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    preview(&mut app);
    app.conversion_draft.as_mut().unwrap().reviewed = true;
    app.session
        .as_mut()
        .unwrap()
        .parameter("Amplifier", 0, 12345)
        .unwrap();
    let changed = xml(&app);
    app.apply_conversion(&ctx);
    assert_eq!(xml(&app), changed);
    assert!(
        app.conversion_draft
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("source changed")
    );
}
#[test]
fn file_conversion_dirty_guard_can_cancel_then_open_unsaved_copy() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source.fuse");
    let source = xml(&app);
    std::fs::write(&path, &source).unwrap();
    app.session
        .as_mut()
        .unwrap()
        .parameter("Amplifier", 0, 12345)
        .unwrap();
    let changed = xml(&app);
    app.prepare_file_conversion(path.clone());
    let draft = app.conversion_draft.as_mut().unwrap();
    draft.options.target_product = 1;
    draft.review();
    draft.reviewed = true;
    app.apply_conversion(&ctx);
    assert!(matches!(app.pending, Some(Pending::OpenConverted(..))));
    assert_eq!(xml(&app), changed);
    app.pending = None;
    assert!(app.conversion_draft.is_some());
    assert_eq!(xml(&app), changed);
    app.apply_conversion(&ctx);
    let action = app.pending.take().unwrap();
    app.perform(action, &ctx);
    let session = app.session.as_mut().unwrap();
    assert!(session.dirty());
    assert!(session.path.is_none());
    assert!(!session.can_undo());
    assert_eq!(session.document.product_id(), Some(1));
    assert!(session.save_as(&path).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn converted_current_copy_never_overwrites_source_and_can_save_its_new_file() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source.fuse");
    app.session.as_mut().unwrap().save_as(&path).unwrap();
    let source = std::fs::read(&path).unwrap();
    preview(&mut app);
    app.conversion_draft.as_mut().unwrap().reviewed = true;
    app.apply_conversion(&ctx);
    let session = app.session.as_mut().unwrap();
    assert!(session.save_as(&path).is_err());
    let alias = temp.path().join("alias.fuse");
    std::fs::hard_link(&path, &alias).unwrap();
    assert!(session.save_as(&alias).is_err());
    let new = temp.path().join("converted.fuse");
    session.save_as(&new).unwrap();
    session.parameter("Amplifier", 0, 12500).unwrap();
    session.save_as(&new).unwrap();
    assert!(!session.dirty());
    assert_eq!(std::fs::read(path).unwrap(), source);
}
#[test]
fn raw_file_repair_preview_does_not_require_normal_parser_success() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    let source = xml(&app);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.fuse");
    // A malformed source is retained for inspection, never silently replaced.
    std::fs::write(&path, "<Preset ProductId=\"13\"><broken").unwrap();
    app.perform(Pending::Open(path.clone()), &ctx);
    assert_eq!(xml(&app), source);
    let draft = app.conversion_draft.as_mut().unwrap();
    assert!(draft.error.is_some());
    assert_eq!(draft.source, std::fs::read_to_string(path).unwrap());
    draft.review();
    assert!(draft.plan.is_none());
    assert!(draft.error.is_some());
}
#[test]
fn entity_repair_is_opt_in_and_frozen_export_preserves_editor_and_source() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    let before = xml(&app);
    let raw = before.replace("Warm room", "Warm &amp;amp; room");
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.fuse");
    std::fs::write(&path, &raw).unwrap();
    app.prepare_file_conversion(path.clone());
    let draft = app.conversion_draft.as_mut().unwrap();
    assert!(!draft.options.repair_double_entities);
    draft.review();
    assert_eq!(
        draft.plan.as_ref().unwrap().result().metadata().name,
        "Warm &amp; room"
    );
    draft.options.repair_double_entities = true;
    draft.review();
    draft.reviewed = true;
    let result = draft.result().unwrap();
    assert_eq!(result.metadata().name, "Warm & room");
    let output = temp.path().join("repaired.fuse");
    openfuse_core::preset_file::write_new(&result, &output).unwrap();
    assert!(openfuse_core::preset_file::write_new(&result, &path).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), raw);
    assert_eq!(xml(&app), before);
}
#[test]
fn missing_product_requires_explicit_gdec_option_and_never_modifies_raw_source() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    let gdec = crate::session::Session::new_preset(10)
        .unwrap()
        .document
        .to_xml()
        .unwrap();
    let raw = gdec.replace(" ProductId=\"10\"", "");
    app.conversion_draft = Some(Draft::new(raw.clone(), "Legacy G-DEC source".into(), false));
    app.conversion_draft.as_mut().unwrap().review();
    assert!(app.conversion_draft.as_ref().unwrap().plan.is_none());
    for _ in 0..2 {
        frame(&ctx, &mut app, vec![]);
    }
    click(&ctx, &mut app, "conversion-assume-gdec");
    assert_eq!(
        app.conversion_draft
            .as_ref()
            .unwrap()
            .options
            .target_product,
        10
    );
    click(&ctx, &mut app, "conversion-review");
    let draft = app.conversion_draft.as_ref().unwrap();
    assert!(draft.plan.is_some(), "{:?}", draft.error);
    assert_eq!(draft.source, raw);
    assert_eq!(draft.plan.as_ref().unwrap().result().product_id(), Some(10));
}
#[test]
fn large_change_report_is_paginated_and_all_pages_are_reachable() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    let source = xml(&app).replace("Warm room", &"&amp;amp;".repeat(85));
    app.conversion_draft = Some(Draft::new(source, "Many legacy entities".into(), false));
    let draft = app.conversion_draft.as_mut().unwrap();
    draft.options.repair_double_entities = true;
    draft.review();
    assert!(draft.plan.as_ref().unwrap().changes().len() > 80);
    for _ in 0..2 {
        frame(&ctx, &mut app, vec![]);
    }
    click(&ctx, &mut app, "conversion-next");
    assert_eq!(app.conversion_draft.as_ref().unwrap().page, 1);
    click(&ctx, &mut app, "conversion-next");
    assert_eq!(app.conversion_draft.as_ref().unwrap().page, 2);
    click(&ctx, &mut app, "conversion-prev");
    assert_eq!(app.conversion_draft.as_ref().unwrap().page, 1);
}
#[test]
fn modal_actions_stay_inside_the_minimum_supported_window() {
    let ctx = egui::Context::default();
    let mut app = editor(&ctx);
    preview(&mut app);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(940.0, 680.0));
    for _ in 0..3 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| app.conversion_dialog(ctx),
        );
    }
    for id in ["conversion-apply", "conversion-export", "conversion-cancel"] {
        let (rect, _): (egui::Rect, bool) = ctx.data(|d| d.get_temp(egui::Id::new(id)).unwrap());
        assert!(screen.contains_rect(rect), "{id}: {rect:?}");
    }
}
