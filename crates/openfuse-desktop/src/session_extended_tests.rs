use super::*;
use openfuse_core::preset::expression::{ExpressionPedal, PedalSlot};

#[test]
fn expression_edits_undo_and_failed_rebinding_preserve_history() {
    let mut session = Session::example().unwrap();
    let before = session.document.to_xml().unwrap();
    let mut assignment = ExpressionPedal::defaults(PedalSlot::First);
    assignment.expression_mode_behavior = Some(1);
    assignment.parameter_index = Some(1);
    assignment.heel_setting = Some(u16::MAX);
    assignment.toe_setting = Some(0);
    session.expression(PedalSlot::First, &assignment).unwrap();
    let after = session.document.to_xml().unwrap();
    assert_eq!(
        session.document.expression_pedal(PedalSlot::First).unwrap(),
        Some(assignment.clone())
    );
    let history = session.undo.len();
    assignment.parameter_index = Some(255);
    assert!(session.expression(PedalSlot::First, &assignment).is_err());
    assert_eq!(session.document.to_xml().unwrap(), after);
    assert_eq!(session.undo.len(), history);
    session.undo().unwrap();
    assert_eq!(session.document.to_xml().unwrap(), before);
    session.redo().unwrap();
    assert_eq!(session.document.to_xml().unwrap(), after);
    export("expression-editor.fuse", &session);
}

#[test]
fn selected_modules_import_as_one_undo_step_and_failure_preserves_destination() {
    let source = Session::example().unwrap();
    let mut target = Session::new_preset(13).unwrap();
    let before = target.document.to_xml().unwrap();
    let metadata = target.document.metadata();
    let colors = target.document.pedal_colors().unwrap();
    let report = target
        .import_modules(&source.document, &["Stompbox", "Delay"])
        .unwrap();
    assert_eq!(report.imported.len(), 2);
    let after = target.document.to_xml().unwrap();
    assert_ne!(after, before);
    assert_eq!(target.undo.len(), 1);
    assert_eq!(target.document.metadata(), metadata);
    assert_eq!(target.document.pedal_colors().unwrap(), colors);
    for key in ["Stompbox", "Delay"] {
        let loaded = target
            .document
            .modules()
            .into_iter()
            .find(|module| module.key == key)
            .unwrap();
        let original = source
            .document
            .modules()
            .into_iter()
            .find(|module| module.key == key)
            .unwrap();
        assert_eq!(loaded.model_id, original.model_id);
        assert_eq!(loaded.parameters, original.parameters);
    }
    let unknown = PresetDocument::parse(
        &source
            .document
            .to_xml()
            .unwrap()
            .replace("ProductId=\"13\"", "ProductId=\"65000\""),
    )
    .unwrap();
    assert!(
        target
            .import_modules(&unknown, &["Amplifier", "Stompbox"])
            .is_err()
    );
    assert_eq!(target.document.to_xml().unwrap(), after);
    assert_eq!(target.undo.len(), 1);
    target.undo().unwrap();
    assert_eq!(target.document.to_xml().unwrap(), before);
    target.redo().unwrap();
    assert_eq!(target.document.to_xml().unwrap(), after);
    export("module-import-editor.fuse", &target);
}

fn export(name: &str, session: &Session) {
    if let Some(directory) = std::env::var_os("OPENFUSE_EXPORT_DESKTOP_EDIT_PRESETS") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(name), session.document.to_xml().unwrap()).unwrap();
    }
}
