//! FX/VOICE editing stays independent of the full-preset editor.
use crate::{
    audition_view::remember,
    control_editor_state::{Draft, Library},
    controls_view::button,
    theme,
};
use eframe::egui::{self, RichText};
use openfuse_core::{
    control_library::ControlEntry,
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument,
        expression::{ExpressionPedal, PedalSlot},
    },
};
use std::collections::BTreeSet;
#[derive(Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Editor,
    Library,
}
pub enum Pending {
    Replace(Box<Draft>),
    CloseWindow,
}
pub struct CaptureDraft {
    pub preset: PresetDocument,
    pub pid: u16,
    pub control: u8,
    pub clean: bool,
    pub selected: BTreeSet<String>,
    pub available: Vec<String>,
    pub source: String,
}
pub struct State {
    pub open: bool,
    pub tab: Tab,
    pub draft: Option<Draft>,
    pub selected: String,
    pub library: Library,
    pub pending: Option<Pending>,
    pub capture: Option<CaptureDraft>,
    pub pid: u16,
    pub control: u8,
    pub clean: bool,
    pub target: u16,
    pub name: String,
    pub error: Option<String>,
    pub finish_after_save: bool,
    pub save_after_scan: bool,
    pub next_id: u64,
    pub demo: u8,
    pub demo_stage: u8,
    pub directory: Option<tempfile::TempDir>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Editor,
            draft: None,
            selected: String::new(),
            library: Library::default(),
            pending: None,
            capture: None,
            pid: 0x14,
            control: 1,
            clean: false,
            target: 0,
            name: "Untitled bank preset".into(),
            error: None,
            finish_after_save: false,
            save_after_scan: false,
            next_id: 0,
            demo: 0,
            demo_stage: 0,
            directory: None,
        }
    }
}
impl State {
    pub fn dirty(&self) -> bool {
        self.draft.as_ref().is_some_and(Draft::dirty)
    }
}
#[derive(Clone)]
pub enum Action {
    Browse,
    Close,
    New,
    OpenFile,
    Export,
    Save,
    Undo,
    Redo,
    Current,
    Local,
    Capture,
    CancelCapture,
    Review,
    Read,
    ChooseFolder,
    Refresh,
    Import,
    Open(ControlEntry),
    Duplicate(ControlEntry),
    Delete(ControlEntry),
    Recover(ControlEntry),
    Discard,
    CancelPending,
    SavePending,
    CancelOperation,
}

pub fn dialog(
    ctx: &egui::Context,
    state: &mut State,
    device: &crate::device_state::DeviceState,
    has_local: bool,
) -> Option<Action> {
    if state.pending.is_some() {
        return dirty_dialog(ctx);
    }
    if state.capture.is_some() {
        return capture_dialog(ctx, state);
    }
    let mut action = None;
    crate::dialog::show(ctx, "control-editor", 1040.0, |ui| {
        theme::eyebrow(ui, "FX / VOICE WORKSPACE");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut state.tab, Tab::Editor, "Edit a bank preset");
            ui.selectable_value(&mut state.tab, Tab::Library, "Local FX/VOICE library");
            if button(ui, "control-browse", "Amplifier banks…", true) {
                action = Some(Action::Browse);
            }
            if button(ui, "control-close", "Close", true) {
                action = Some(Action::Close);
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("control-workspace-scroll")
            .max_height((ctx.screen_rect().height() - 285.0).max(300.0))
            .show(ui, |ui| match state.tab {
                Tab::Editor => editor(ui, state, device, has_local, &mut action),
                Tab::Library => library(ui, state, &mut action),
            });
        if let Some(error) = &state.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        }
        if let Some(error) = &state.library.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        } else if let Some(notice) = &state.library.notice {
            ui.label(
                RichText::new(notice)
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
            );
        }
        if device.bank.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&device.bank.detail);
                if button(
                    ui,
                    "control-cancel-operation",
                    "Cancel amplifier operation",
                    true,
                ) {
                    action = Some(Action::CancelOperation);
                }
            });
        } else if !device.bank.detail.is_empty() {
            ui.label(
                RichText::new(&device.bank.detail)
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
            );
        }
        if let Some(error) = &device.error {
            ui.colored_label(theme::accent(ui.ctx()), error);
        }
        if state.library.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Working with the local library…");
            });
        }
    });
    action
}
fn profiles(ui: &mut egui::Ui, state: &mut State) {
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("control-profile")
            .selected_text(profile(state.pid))
            .show_ui(ui, |ui| {
                for pid in [4, 0x14, 0x10, 0xa, 0x26, 0xe] {
                    ui.selectable_value(&mut state.pid, pid, profile(pid));
                }
            });
        let caps = openfuse_core::auxiliary::capabilities(state.pid).expect("known profile");
        if !caps.banks.contains_key(&state.control) {
            state.control = *caps.banks.keys().next().unwrap();
        }
        egui::ComboBox::from_id_salt("control-kind")
            .selected_text(kind(state.pid, state.control))
            .show_ui(ui, |ui| {
                for control in caps.banks.keys() {
                    ui.selectable_value(&mut state.control, *control, kind(state.pid, *control));
                }
            });
        if state.pid == 0xe && state.control == 1 {
            ui.checkbox(&mut state.clean, "Clean channel (16)");
        } else {
            state.clean = false;
        }
    });
}
pub fn profile(pid: u16) -> &'static str {
    match pid {
        4 => "Mustang I / II",
        0x14 => "Mustang I / II V2",
        0x10 => "Mustang Mini",
        0xa => "Bronco 40",
        0x26 => "Passport Mini",
        0xe => "Super Champ X2",
        _ => "Unknown profile",
    }
}
pub fn kind(pid: u16, control: u8) -> &'static str {
    match (pid, control) {
        (4 | 0x14, 1) => "MOD",
        (4 | 0x14, 2) => "DLY/RVB",
        (0xe, 1) => "VOICE",
        _ => "FX",
    }
}
fn editor(
    ui: &mut egui::Ui,
    state: &mut State,
    device: &crate::device_state::DeviceState,
    has_local: bool,
    action: &mut Option<Action>,
) {
    egui::CollapsingHeader::new("New, import or capture a bank preset").default_open(state.draft.is_none()).show(ui,|ui| {
        profiles(ui,state);
        ui.horizontal_wrapped(|ui| {
            ui.label("New name");ui.text_edit_singleline(&mut state.name);
            if button(ui,"control-new","New",true) {*action=Some(Action::New);}
            if button(ui,"control-open","Open file…",!state.library.busy()) {*action=Some(Action::OpenFile);}
        });
        ui.horizontal_wrapped(|ui| {
            if button(ui,"control-current","Copy current amplifier sound…",device.current_binding().is_some()&&!device.operation_busy()) {*action=Some(Action::Current);}
            if button(ui,"control-local","Copy local full preset…",has_local) {*action=Some(Action::Local);}
        });
        ui.label(RichText::new("Choose the exact source profile for historical .FUSE XML. Native OpenFUSE files retain their own profile.").size(12.0).color(theme::muted(ui.ctx())));
    });
    let Some(draft) = &mut state.draft else {
        ui.heading("Build your own physical-knob presets");
        ui.label("Start from an amplifier bank, a full preset or a new bank document.");
        return;
    };
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.heading(draft.document.name());
        if draft.dirty() {
            ui.colored_label(theme::accent(ui.ctx()), "Unsaved");
        }
    });
    ui.label(
        RichText::new(format!(
            "{} · {} · {}",
            profile(draft.document.pid()),
            kind(draft.document.pid(), draft.document.control()),
            draft.source
        ))
        .size(12.0)
        .color(theme::muted(ui.ctx())),
    );
    ui.horizontal_wrapped(|ui| {
        if button(ui, "control-undo", "Undo", draft.can_undo()) {
            *action = Some(Action::Undo);
        }
        if button(ui, "control-redo", "Redo", draft.can_redo()) {
            *action = Some(Action::Redo);
        }
        if button(ui, "control-save", "Save to library", !state.library.busy()) {
            *action = Some(Action::Save);
        }
        if button(ui, "control-export", "Export file…", !state.library.busy()) {
            *action = Some(Action::Export);
        }
    });
    let mut name = draft.document.name();
    ui.horizontal(|ui| {
        ui.label("Name");
        let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(450.0));
        remember(&response, "control-name");
        if response.changed() {
            record(
                draft.edit(|doc| doc.set_name(&name).map_err(|e| e.to_string())),
                &mut state.error,
            );
        }
    });
    let modules = draft.document.modules();
    if !modules.iter().any(|m| m.key == state.selected) {
        state.selected = modules.first().map(|m| m.key.clone()).unwrap_or_default();
    }
    ui.horizontal_wrapped(|ui| {
        for module in &modules {
            let model = module.model_id.and_then(|id| {
                ModuleCategory::from_key(&module.key)
                    .and_then(|cat| model_catalog::model(draft.document.product_id(), cat, id))
            });
            let name = model.map_or_else(
                || format!("Model {}", module.model_id.unwrap_or(0)),
                |m| m.name.clone(),
            );
            if ui
                .add_sized(
                    [160.0, 58.0],
                    egui::Button::new(format!("{}\n{}", module.key, name))
                        .selected(module.key == state.selected),
                )
                .clicked()
            {
                state.selected = module.key.clone();
            }
        }
    });
    if let Some(module) = modules.iter().find(|m| m.key == state.selected) {
        egui::Frame::new()
            .fill(theme::panel(ui.ctx()))
            .inner_margin(16)
            .corner_radius(10)
            .show(ui, |ui| {
                ui.heading(&module.key);
                ui.horizontal_wrapped(|ui| {
                    if let Some(category) = ModuleCategory::from_key(&module.key) {
                        egui::ComboBox::from_id_salt("control-model")
                            .width(320.0)
                            .selected_text(
                                module
                                    .model_id
                                    .and_then(|id| {
                                        model_catalog::model(
                                            draft.document.product_id(),
                                            category,
                                            id,
                                        )
                                    })
                                    .map_or_else(
                                        || {
                                            format!(
                                                "Unknown model {}",
                                                module.model_id.unwrap_or(0)
                                            )
                                        },
                                        |m| m.name.clone(),
                                    ),
                            )
                            .show_ui(ui, |ui| {
                                for model in
                                    model_catalog::models(draft.document.product_id(), category)
                                {
                                    if ui
                                        .selectable_label(
                                            module.model_id == Some(model.id),
                                            &model.name,
                                        )
                                        .clicked()
                                    {
                                        record(
                                            draft.edit(|doc| {
                                                doc.replace_module_model(&module.key, model.id)
                                                    .map_err(|e| e.to_string())
                                            }),
                                            &mut state.error,
                                        );
                                    }
                                }
                            });
                    }
                    if let Some(mut active) = module.active {
                        let response = ui.checkbox(&mut active, "Enabled");
                        remember(&response, "control-active");
                        if response.changed() {
                            record(
                                draft.edit(|doc| {
                                    doc.set_module_active(&module.key, active)
                                        .map_err(|e| e.to_string())
                                }),
                                &mut state.error,
                            );
                        }
                    }
                });
                let model = module.model_id.and_then(|id| {
                    ModuleCategory::from_key(&module.key)
                        .and_then(|cat| model_catalog::model(draft.document.product_id(), cat, id))
                });
                egui::Grid::new("control-parameters")
                    .num_columns(2)
                    .spacing([20.0, 8.0])
                    .show(ui, |ui| {
                        for parameter in &module.parameters {
                            let definition = model.and_then(|m| {
                                m.parameters
                                    .iter()
                                    .find(|p| p.index == parameter.index && p.name != "NULL")
                            });
                            ui.label(definition.map_or_else(
                                || format!("Raw parameter {}", parameter.index),
                                |p| p.name.clone(),
                            ));
                            let mut value = parameter.value;
                            ui.push_id(parameter.index, |ui| {
                                if crate::views::parameter_control(ui, &mut value, definition) {
                                    record(
                                        draft.edit(|doc| {
                                            doc.set_parameter(&module.key, parameter.index, value)
                                                .map_err(|e| e.to_string())
                                        }),
                                        &mut state.error,
                                    );
                                }
                            });
                            ui.end_row();
                        }
                    });
                if draft.document.pid() != 0xe
                    && let Some(mut position) = module.position
                {
                    ui.horizontal(|ui| {
                        ui.label("Signal position");
                        if ui
                            .add(egui::DragValue::new(&mut position).range(0..=7))
                            .changed()
                        {
                            record(
                                draft.edit(|doc| {
                                    doc.set_module_position(&module.key, position)
                                        .map_err(|e| e.to_string())
                                }),
                                &mut state.error,
                            );
                        }
                    });
                }
            });
    }
    if draft.document.pid() == 0xe {
        if let Ok(Some(mut gain)) = draft.document.usb_gain() {
            ui.horizontal(|ui| {
                ui.label("USB gain");
                if ui.add(egui::Slider::new(&mut gain, 0..=255)).changed() {
                    record(
                        draft.edit(|doc| doc.set_usb_gain(gain).map_err(|e| e.to_string())),
                        &mut state.error,
                    );
                }
            });
        }
        if draft.document.control() == 2 {
            expression(ui, draft, &mut state.error);
        }
    }
    ui.separator();
    ui.strong("Write this bank preset to an amplifier");
    let destination = device
        .current_auxiliary()
        .filter(|snapshot| snapshot.snapshot.pid() == draft.document.pid());
    let targets:Vec<_>=destination.into_iter().flat_map(|snapshot|snapshot.snapshot.entries()).filter(|entry| matches!(entry.key(),openfuse_core::auxiliary::Key::Bank{control,slot} if control==draft.document.control() && (draft.document.pid()!=0xe || control!=1 || (!draft.document.is_clean_voice() || slot==16)))).collect();
    let target_valid = targets.iter().any(|entry| {
        entry.key()
            == openfuse_core::auxiliary::Key::Bank {
                control: draft.document.control(),
                slot: state.target,
            }
    });
    ui.horizontal_wrapped(|ui| {
        if button(
            ui,
            "control-read",
            "Read destination banks",
            device.current_binding().is_some() && !device.operation_busy(),
        ) {
            *action = Some(Action::Read);
        }
        egui::ComboBox::from_id_salt("control-destination")
            .width(350.0)
            .selected_text(
                targets
                    .iter()
                    .find(|entry| {
                        entry.key()
                            == openfuse_core::auxiliary::Key::Bank {
                                control: draft.document.control(),
                                slot: state.target,
                            }
                    })
                    .map_or_else(|| "Choose destination…".into(), |entry| entry.label()),
            )
            .show_ui(ui, |ui| {
                for entry in targets {
                    if let openfuse_core::auxiliary::Key::Bank { slot, .. } = entry.key() {
                        ui.selectable_value(&mut state.target, slot, entry.label());
                    }
                }
            });
        if button(
            ui,
            "control-review",
            "Review overwrite…",
            target_valid && !device.operation_busy(),
        ) {
            *action = Some(Action::Review);
        }
    });
    ui.label(RichText::new("Local changes stay in this bank draft. The full-preset editor has its own document and undo history.").size(12.0).color(theme::muted(ui.ctx())));
}
fn record(result: Result<(), String>, error: &mut Option<String>) {
    if let Err(e) = result {
        *error = Some(e);
    }
}
fn expression(ui: &mut egui::Ui, draft: &mut Draft, error: &mut Option<String>) {
    let response = egui::CollapsingHeader::new("Super Champ expression settings").show(ui, |ui| {
        let mut pedal = match draft.document.expression_pedal() {
            Ok(Some(pedal)) => pedal,
            Ok(None) => {
                if ui.button("Add expression defaults").clicked() {
                    record(
                        draft.edit(|doc| {
                            doc.set_expression_pedal(&ExpressionPedal::defaults(PedalSlot::First))
                                .map_err(|e| e.to_string())
                        }),
                        error,
                    );
                }
                return;
            }
            Err(e) => {
                *error = Some(e.to_string());
                return;
            }
        };
        let changed = crate::views::expression::controls(
            ui,
            draft.document.preset_projection(),
            PedalSlot::First,
            "Bank expression",
            &mut pedal,
        );
        if changed {
            record(
                draft.edit(|doc| doc.set_expression_pedal(&pedal).map_err(|e| e.to_string())),
                error,
            );
        }
    });
    remember(&response.header_response, "control-expression-header");
}
fn library(ui: &mut egui::Ui, state: &mut State, action: &mut Option<Action>) {
    ui.heading("Your FX and VOICE presets");
    ui.label(state.library.folder.as_ref().map_or_else(
        || "Choose a folder for standalone bank presets".into(),
        |p| p.display().to_string(),
    ));
    ui.horizontal_wrapped(|ui| {
        for (id, label, a) in [
            ("control-folder", "Folder…", Action::ChooseFolder),
            ("control-refresh", "Refresh", Action::Refresh),
            ("control-import", "Import file…", Action::Import),
        ] {
            if button(ui, id, label, !state.library.busy()) {
                *action = Some(a);
            }
        }
    });
    egui::CollapsingHeader::new("Historical XML import profile").show(ui, |ui| profiles(ui, state));
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.library.query)
                .hint_text("Search name, bank or profile"),
        );
        ui.checkbox(&mut state.library.deleted, "Show trash");
    });
    let entries = if state.library.deleted {
        &state.library.trash
    } else {
        &state.library.entries
    };
    for entry in entries
        .iter()
        .filter(|entry| entry.matches(&state.library.query))
    {
        egui::Frame::new()
            .fill(theme::panel(ui.ctx()))
            .corner_radius(10)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.strong(&entry.name);
                ui.label(
                    RichText::new(format!(
                        "{} · {}",
                        profile(entry.pid),
                        kind(entry.pid, entry.control)
                    ))
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
                );
                ui.horizontal_wrapped(|ui| {
                    if state.library.deleted {
                        if button(
                            ui,
                            &format!("control-recover-{}", entry.catalog.id),
                            "Recover",
                            !state.library.busy(),
                        ) {
                            *action = Some(Action::Recover(entry.clone()));
                        }
                    } else {
                        for (label, a) in [
                            ("Open", Action::Open(entry.clone())),
                            ("Duplicate", Action::Duplicate(entry.clone())),
                            ("Move to trash", Action::Delete(entry.clone())),
                        ] {
                            if button(
                                ui,
                                &format!("control-{label}-{}", entry.catalog.id),
                                label,
                                !state.library.busy(),
                            ) {
                                *action = Some(a);
                            }
                        }
                    }
                });
            });
    }
    for diagnostic in &state.library.diagnostics {
        ui.colored_label(theme::accent(ui.ctx()), diagnostic);
    }
}
fn dirty_dialog(ctx: &egui::Context) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "control-unsaved", 520.0, |ui| {
        ui.heading("Keep your FX/VOICE changes?");
        ui.label("This independent bank draft has unsaved edits.");
        ui.horizontal_wrapped(|ui| {
            for (id, label, a) in [
                (
                    "control-save-pending",
                    "Save to library",
                    Action::SavePending,
                ),
                ("control-discard", "Discard changes", Action::Discard),
                ("control-cancel-pending", "Cancel", Action::CancelPending),
            ] {
                if button(ui, id, label, true) {
                    action = Some(a);
                }
            }
        });
    });
    action
}
fn capture_dialog(ctx: &egui::Context, state: &mut State) -> Option<Action> {
    let mut action = None;
    let capture = state.capture.as_mut().unwrap();
    crate::dialog::show(ctx, "control-capture", 600.0, |ui| {
        ui.heading("Choose modules for this bank preset");
        ui.label(&capture.source);
        ui.label(format!(
            "{} · {}",
            profile(capture.pid),
            kind(capture.pid, capture.control)
        ));
        for key in &capture.available {
            let mut selected = capture.selected.contains(key);
            if ui.checkbox(&mut selected, key).changed() {
                if selected {
                    capture.selected.insert(key.clone());
                } else {
                    capture.selected.remove(key);
                }
            }
        }
        ui.label("Unchecked modules become empty/off in the new bank preset. The original full preset remains unchanged.");
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "control-capture-confirm",
                "Create bank draft",
                !capture.selected.is_empty(),
            ) {
                action = Some(Action::Capture);
            }
            if button(ui, "control-capture-cancel", "Cancel", true) {
                action = Some(Action::CancelCapture);
            }
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(ctx: &egui::Context, draft: &mut Draft, events: Vec<egui::Event>) {
        let mut error = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(940.0, 1100.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| expression(ui, draft, &mut error));
            },
        );
        assert!(error.is_none(), "{error:?}");
    }
    fn click(ctx: &egui::Context, draft: &mut Draft, rect: egui::Rect) {
        for pressed in [true, false] {
            frame(
                ctx,
                draft,
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
        for _ in 0..3 {
            frame(ctx, draft, vec![]);
        }
    }
    #[test]
    fn superchamp_expression_pointer_edit_records_undo_and_preserves_other_fields() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let document =
            openfuse_core::control_preset::ControlPreset::new(0xe, 2, "FX pedal").unwrap();
        let original = document.to_xml().unwrap();
        let mut draft = Draft::new(document, "Test".into(), true, 1).unwrap();
        frame(&ctx, &mut draft, vec![]);
        let (rect, _) = ctx.data(|d| {
            d.get_temp::<(egui::Rect, bool)>(egui::Id::new("control-expression-header"))
                .unwrap()
        });
        click(&ctx, &mut draft, rect);
        let rect = ctx.data(|d| {
            d.get_temp::<egui::Rect>(egui::Id::new((
                "expression-control",
                "Bank expression",
                "Tracking",
            )))
            .unwrap()
        });
        click(&ctx, &mut draft, rect);
        let rect = ctx.data(|d| {
            d.get_temp::<egui::Rect>(egui::Id::new((
                "expression-control",
                "Bank expression",
                "Tracking-0",
            )))
            .unwrap()
        });
        click(&ctx, &mut draft, rect);
        assert_eq!(
            draft
                .document
                .expression_pedal()
                .unwrap()
                .unwrap()
                .pedal_mode,
            Some(0)
        );
        assert!(draft.dirty());
        assert!(draft.can_undo());
        draft.undo();
        assert_eq!(draft.document.to_xml().unwrap(), original);
    }
}
