//! Catalog-aware, offline preset editor. Unknown models retain raw controls.
use crate::{session::Session, theme};
#[path = "annotation_view.rs"]
mod annotations;
#[path = "expression_view.rs"]
pub(crate) mod expression;
use eframe::egui::{self, RichText};
use openfuse_core::{
    model_catalog::{self, Model, ModuleCategory, Parameter},
    preset::{PresetModule, PresetParameter},
};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Inspector {
    #[default]
    Sound,
    Expression,
    Details,
}

pub fn preset(
    ui: &mut egui::Ui,
    session: &mut Session,
    selected: &mut String,
    error: &mut Option<String>,
    demo: bool,
    inspector: &mut Inspector,
) {
    let name = session.document.metadata().name;
    theme::eyebrow(
        ui,
        if session.is_storage_copy() {
            "STORAGE PRESET · LOCAL COPY"
        } else if session.is_converted() {
            "CONVERTED PRESET · LOCAL COPY"
        } else if session.source_label.is_some() {
            "DEVICE SNAPSHOT · LOCAL COPY"
        } else if demo {
            "EXAMPLE PRESET · OFFLINE"
        } else {
            "LOCAL PRESET · OFFLINE"
        },
    );
    if let Some(source) = &session.source_label {
        ui.add(
            egui::Label::new(
                RichText::new(source)
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
            )
            .wrap(),
        );
    }
    ui.horizontal(|ui| {
        ui.heading(if name.is_empty() {
            "Untitled preset"
        } else {
            &name
        });
        if session.dirty() {
            ui.label(RichText::new("• Unsaved").color(theme::accent(ui.ctx())));
        }
    });
    let product_id = session.document.product_id();
    let product = product_id.and_then(model_catalog::product);
    ui.label(
        RichText::new(product.map(|p| p.name.to_owned()).unwrap_or_else(|| {
            format!(
                "Unrecognized product {} · raw controls",
                product_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "(unspecified)".into())
            )
        }))
        .size(13.0)
        .color(theme::muted(ui.ctx())),
    );
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        for (view, label, key) in [
            (Inspector::Sound, "Sound", "tab-sound"),
            (Inspector::Expression, "Expression", "tab-expression"),
            (Inspector::Details, "Details", "tab-details"),
        ] {
            let r = ui.add_sized(
                [112.0, 36.0],
                egui::Button::new(label)
                    .corner_radius(7)
                    .selected(*inspector == view),
            );
            theme::remember(&r, key);
            if r.clicked() {
                *inspector = view;
            }
        }
    });
    ui.add_space(16.0);
    match inspector {
        Inspector::Sound => {
            let modules = session.document.modules();
            module_selector(
                ui,
                &modules,
                product_id,
                selected,
                session.document.pedal_colors().ok().flatten(),
            );
            ui.add_space(16.0);
            parameter_panel(
                ui,
                session,
                modules.iter().find(|m| m.key == *selected),
                error,
            );
        }
        Inspector::Expression => {
            ui.label(
                RichText::new("Make the pedal part of your sound")
                    .size(18.0)
                    .strong(),
            );
            ui.label(
                RichText::new("Choose what moves, then set the heel and toe positions.")
                    .color(theme::muted(ui.ctx())),
            );
            ui.add_space(16.0);
            expression::panel(ui, session, error, true);
        }
        Inspector::Details => {
            theme::surface(ui).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("Preset information").size(18.0).strong());
                ui.add_space(16.0);
                metadata(ui, session, error);
                ui.add_space(16.0);
                annotations::details(ui, session, error);
            });
            ui.add_space(16.0);
            egui::CollapsingHeader::new("Pedal colors")
                .default_open(true)
                .show(ui, |ui| annotations::colors(ui, session, error));
        }
    }
}

fn definition(product_id: Option<u32>, module: &PresetModule) -> Option<&'static Model> {
    model_catalog::model(
        product_id?,
        ModuleCategory::from_key(&module.key)?,
        module.model_id?,
    )
}

fn module_selector(
    ui: &mut egui::Ui,
    modules: &[PresetModule],
    product_id: Option<u32>,
    selected: &mut String,
    colors: Option<[i32; 4]>,
) {
    theme::eyebrow(ui, "Amplifier & effects");
    ui.add_space(6.0);
    let width = ((ui.available_width() - 8.0 * (modules.len().saturating_sub(1)) as f32)
        / modules.len().max(1) as f32)
        .max(108.0);
    ui.horizontal_wrapped(|ui| {
        for module in modules {
            let name = definition(product_id, module)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| format!("Model 0x{:04X}", module.model_id.unwrap_or_default()));
            let active = module.active.unwrap_or(true) && module.model_id != Some(0);
            let chosen = *selected == module.key;
            let text_color = if chosen {
                theme::accent(ui.ctx())
            } else {
                theme::ink(ui.ctx())
            };
            let response = ui.add_sized(
                [width, 94.0],
                egui::Button::new("")
                    .fill(theme::card(ui.ctx()))
                    .stroke(egui::Stroke::new(
                        if chosen { 1.5_f32 } else { 1.0_f32 },
                        if chosen {
                            theme::accent(ui.ctx())
                        } else {
                            theme::line(ui.ctx())
                        },
                    ))
                    .corner_radius(10),
            );
            let r = response.rect;
            let painter = ui.painter();
            painter.text(
                r.left_top() + egui::vec2(12.0, 15.0),
                egui::Align2::LEFT_TOP,
                &module.label,
                egui::FontId::proportional(12.0),
                text_color,
            );
            let galley = painter.layout(
                name,
                egui::FontId::proportional(13.0),
                theme::ink(ui.ctx()),
                width - 24.0,
            );
            painter.galley(
                r.left_top() + egui::vec2(12.0, 37.0),
                galley,
                theme::ink(ui.ctx()),
            );
            let state = if module.key == "Amplifier" {
                "AMP"
            } else if active {
                "ON"
            } else {
                "BYPASS"
            };
            painter.text(
                r.left_bottom() + egui::vec2(12.0, -12.0),
                egui::Align2::LEFT_BOTTOM,
                state,
                egui::FontId::proportional(9.0),
                theme::muted(ui.ctx()),
            );
            let color =
                annotations::color_for(&module.key, colors).unwrap_or(theme::accent(ui.ctx()));
            painter.circle_filled(
                r.right_bottom() + egui::vec2(-14.0, -17.0),
                3.5,
                if active {
                    color
                } else {
                    theme::muted(ui.ctx()).gamma_multiply(0.5)
                },
            );
            theme::remember(&response, &format!("module-{}", module.key));
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    true,
                    chosen,
                    &module.label,
                )
            });
            if response.clicked() {
                *selected = module.key.clone();
            }
        }
    });
}

fn parameter_panel(
    ui: &mut egui::Ui,
    session: &mut Session,
    module: Option<&PresetModule>,
    error: &mut Option<String>,
) {
    theme::surface(ui).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        let Some(module) = module else {
            ui.label("Select an amplifier or effect to edit its controls.");
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(&module.label).size(18.0).strong());
            if module.key != "Amplifier" && module.model_id != Some(0) {
                if let Some(mut active) = module.active {
                    if {
                        let r = ui.checkbox(&mut active, "Enabled");
                        theme::remember(&r, "effect-enabled");
                        r.changed()
                    } && let Err(issue) = session.active(&module.key, active)
                    {
                        *error = Some(issue);
                    }
                } else {
                    ui.label(
                        RichText::new("State not stored")
                            .size(12.0)
                            .color(theme::muted(ui.ctx())),
                    );
                    for (label, active) in [("Enable", true), ("Bypass", false)] {
                        if ui.button(label).clicked()
                            && let Err(issue) = session.active(&module.key, active)
                        {
                            *error = Some(issue);
                        }
                    }
                }
            }
        });
        if choose_model(ui, session, module, error) {
            return;
        }
        let definition = definition(session.document.product_id(), module);
        ui.add_space(14.0);
        if module.parameters.is_empty() {
            ui.label("This module has no stored parameters.");
        }
        let primary_parameter = |parameter: &&PresetParameter| {
            if module.key == "Amplifier" && definition.is_some() {
                definition
                    .and_then(|m| m.parameters.iter().find(|p| p.index == parameter.index))
                    .is_some_and(|p| {
                        matches!(
                            p.name.as_str(),
                            "Volume"
                                | "Amp Volume"
                                | "Amp Type"
                                | "Gain"
                                | "Treble"
                                | "Middle"
                                | "Bass"
                                | "Presence"
                        )
                    })
            } else {
                module
                    .parameters
                    .iter()
                    .take(6)
                    .any(|p| p.index == parameter.index)
            }
        };
        let (primary, advanced): (Vec<_>, Vec<_>) =
            module.parameters.iter().partition(primary_parameter);
        parameter_grid(ui, session, module, definition, &primary, error, "primary");
        if !advanced.is_empty() {
            ui.add_space(8.0);
            egui::CollapsingHeader::new("More controls").show(ui, |ui| {
                parameter_grid(
                    ui, session, module, definition, &advanced, error, "advanced",
                );
            });
        }
        if module.key != "Amplifier" {
            ui.add_space(14.0);
            routing(ui, session, module, error);
        }
        egui::CollapsingHeader::new("Raw file values").show(ui, |ui| {
            egui::Grid::new(("raw-values", &module.key)).show(ui, |ui| {
                for parameter in &module.parameters {
                    ui.label(format!("Parameter {}", parameter.index));
                    let mut value = parameter.value;
                    if ui
                        .add(
                            egui::DragValue::new(&mut value)
                                .range(0..=u16::MAX)
                                .speed(1),
                        )
                        .changed()
                        && let Err(issue) = session.parameter(&module.key, parameter.index, value)
                    {
                        *error = Some(issue);
                    }
                    ui.end_row();
                }
            });
        });
    });
}

fn routing(
    ui: &mut egui::Ui,
    session: &mut Session,
    module: &PresetModule,
    error: &mut Option<String>,
) {
    if module.model_id == Some(0) {
        ui.label(
            RichText::new("Choose an effect to set its signal position.")
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
        return;
    }
    let product = session
        .document
        .product_id()
        .and_then(model_catalog::product);
    let Some(product) = product else {
        ui.label(
            RichText::new("Routing is unavailable for this unknown product.")
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
        return;
    };
    if product.id == 2259 {
        ui.label(
            RichText::new("Super Champ routing requires its live amplifier mode.")
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
        return;
    }
    let label = |position: u16| {
        if product.orderable {
            format!(
                "{} · slot {}",
                if position < 4 {
                    "Before amplifier"
                } else {
                    "After amplifier"
                },
                position % 4 + 1
            )
        } else if position == 0 {
            "Before amplifier".to_owned()
        } else {
            "After amplifier".to_owned()
        }
    };
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.label("Signal position");
        egui::ComboBox::from_id_salt("routing")
            .width(260.0)
            .selected_text(
                module
                    .position
                    .map(label)
                    .unwrap_or_else(|| "Unspecified".into()),
            )
            .show_ui(ui, |ui| {
                for position in 0..if product.orderable { 8 } else { 2 } {
                    let after = if product.orderable {
                        position >= 4
                    } else {
                        position != 0
                    };
                    if module.key == "Stompbox" && after && !product.post_amp_stomp {
                        continue;
                    }
                    if ui
                        .selectable_label(module.position == Some(position), label(position))
                        .clicked()
                        && module.position != Some(position)
                    {
                        chosen = Some(position);
                    }
                }
            });
    });
    if let Some(position) = chosen
        && let Err(issue) = session.move_module(&module.key, position)
    {
        *error = Some(issue);
    }
}

fn choose_model(
    ui: &mut egui::Ui,
    session: &mut Session,
    module: &PresetModule,
    error: &mut Option<String>,
) -> bool {
    let Some(product_id) = session.document.product_id() else {
        return false;
    };
    let Some(category) = ModuleCategory::from_key(&module.key) else {
        return false;
    };
    let current = definition(Some(product_id), module);
    let models: Vec<_> = model_catalog::models(product_id, category).collect();
    if models.is_empty() {
        ui.label(
            RichText::new("Unknown model family · all values preserved as raw data")
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
        return false;
    }
    let mut chosen = None;
    let model_picker = egui::ComboBox::from_id_salt(("model", &module.key))
        .width(ui.available_width().min(440.0))
        .selected_text(
            current.map(|m| m.name.clone()).unwrap_or_else(|| {
                format!("Unknown model {}", module.model_id.unwrap_or_default())
            }),
        )
        .show_ui(ui, |ui| {
            let mut demo_choice = false;
            for model in models {
                let option = ui.selectable_label(module.model_id == Some(model.id), &model.name);
                theme::remember(&option, &format!("model-choice-{}", model.id));
                if !demo_choice && model.id != 0 && module.model_id != Some(model.id) {
                    theme::remember(&option, "model-choice-demo");
                    demo_choice = true;
                }
                if option.clicked() && module.model_id != Some(model.id) {
                    chosen = Some(model);
                }
            }
        });
    theme::remember(&model_picker.response, "model-picker");
    if let Some(model) = chosen {
        let parameters: Vec<_> = model
            .parameters
            .iter()
            .map(|p| PresetParameter {
                index: p.index,
                value: p.default_raw,
            })
            .collect();
        if let Err(issue) =
            session.model(&module.key, model.id, &parameters, Some(model.bypass_mode))
        {
            *error = Some(issue);
        }
        return true;
    }
    model_picker.response.on_hover_text(
        "Changing models loads their defaults. Undo restores your previous settings.",
    );
    false
}

fn parameter_grid(
    ui: &mut egui::Ui,
    session: &mut Session,
    module: &PresetModule,
    model: Option<&Model>,
    parameters: &[&PresetParameter],
    error: &mut Option<String>,
    id: &str,
) {
    ui.push_id(("parameters", &module.key, id), |ui| {
        let columns = if ui.available_width() >= 620.0 { 2 } else { 1 };
        for row in parameters.chunks(columns) {
            ui.columns(columns, |cols| {
                for (ui, parameter) in cols.iter_mut().zip(row) {
                    let control = model.and_then(|m| {
                        m.parameters
                            .iter()
                            .find(|p| p.index == parameter.index && p.name != "NULL")
                    });
                    ui.label(
                        RichText::new(
                            control
                                .map(|p| p.name.clone())
                                .unwrap_or_else(|| format!("Parameter {:02}", parameter.index)),
                        )
                        .strong(),
                    );
                    let mut value = parameter.value;
                    ui.push_id((&module.key, parameter.index), |ui| {
                        if parameter_control(ui, &mut value, control)
                            && let Err(issue) =
                                session.parameter(&module.key, parameter.index, value)
                        {
                            *error = Some(issue);
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        }
    });
}

pub(crate) fn parameter_control(
    ui: &mut egui::Ui,
    value: &mut u16,
    control: Option<&Parameter>,
) -> bool {
    let before = *value;
    let choices = control.map(Parameter::choices).unwrap_or_default();
    if !choices.is_empty() {
        egui::ComboBox::from_id_salt("value")
            .width(260.0)
            .selected_text(
                choices
                    .iter()
                    .find(|c| c.value == *value)
                    .map(|c| c.label.clone())
                    .unwrap_or_else(|| format!("Unknown value {}", *value)),
            )
            .show_ui(ui, |ui| {
                for choice in choices {
                    ui.selectable_value(value, choice.value, choice.label);
                }
            });
    } else {
        ui.scope(|ui| {
            ui.spacing_mut().interact_size.y = 24.0;
            ui.visuals_mut().selection.bg_fill = theme::accent(ui.ctx());
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 88.0).clamp(100.0, 460.0);
                let range = control.map(Parameter::raw_range).unwrap_or(0..=u16::MAX);
                let slider = ui.add(
                    egui::Slider::new(value, range)
                        .clamping(egui::SliderClamping::Never)
                        .show_value(false),
                );
                if let Some(control) = control {
                    theme::remember(&slider, &format!("parameter-{}", control.name));
                }
                let label = control
                    .map(|p| p.format_value(*value))
                    .unwrap_or_else(|| value.to_string());
                if control.is_some() {
                    ui.add_sized(
                        [72.0, 28.0],
                        egui::Label::new(RichText::new(label).monospace()),
                    );
                }
                if control.is_none() {
                    ui.add(egui::DragValue::new(value).range(0..=u16::MAX).speed(1))
                        .on_hover_text("Exact raw file value");
                }
            });
        });
    }
    ui.response().on_hover_text(format!(
        "Stored value: {} · XML parameter index {}",
        *value,
        control
            .map(|p| p.index.to_string())
            .unwrap_or_else(|| "unknown".into())
    ));
    *value != before
}

fn metadata(ui: &mut egui::Ui, session: &mut Session, error: &mut Option<String>) {
    let mut metadata = session.document.metadata();
    let mut changed = false;
    egui::Grid::new("metadata")
        .num_columns(2)
        .spacing([24.0, 10.0])
        .show(ui, |ui| {
            for (label, field) in [
                ("Name", &mut metadata.name),
                ("Author", &mut metadata.author),
                ("Tags", &mut metadata.tags),
            ] {
                ui.label(label);
                changed |= ui
                    .add_sized(
                        [ui.available_width().min(580.0), 40.0],
                        egui::TextEdit::singleline(field)
                            .id_salt(label)
                            .background_color(theme::card(ui.ctx()))
                            .margin(egui::vec2(12.0, 10.0)),
                    )
                    .changed();
                ui.end_row();
            }
            ui.label("Description");
            changed |= ui
                .add_sized(
                    [ui.available_width().min(580.0), 70.0],
                    egui::TextEdit::multiline(&mut metadata.description)
                        .id_salt("description")
                        .background_color(theme::card(ui.ctx()))
                        .margin(egui::vec2(12.0, 10.0)),
                )
                .changed();
            ui.end_row();
        });
    if changed && let Err(issue) = session.metadata(&metadata) {
        *error = Some(issue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_profiles_does_not_clamp_or_modify_stored_values() {
        for product in model_catalog::products() {
            let mut session = Session::new_preset(product.id).unwrap();
            let before = session.document.to_xml().unwrap();
            let context = egui::Context::default();
            let mut selected = "Amplifier".to_owned();
            let mut error = None;
            theme::install(&context);
            for mode in [egui::Theme::Light, egui::Theme::Dark] {
                context.set_theme(mode);
                for mut inspector in [Inspector::Sound, Inspector::Expression, Inspector::Details] {
                    for width in [640.0, 980.0] {
                        let input = egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 760.0),
                            )),
                            ..Default::default()
                        };
                        let _ = context.run(input, |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                preset(
                                    ui,
                                    &mut session,
                                    &mut selected,
                                    &mut error,
                                    false,
                                    &mut inspector,
                                );
                            });
                        });
                    }
                }
            }
            assert!(error.is_none());
            assert_eq!(session.document.to_xml().unwrap(), before);
            assert!(!session.can_undo());
        }
    }
}
