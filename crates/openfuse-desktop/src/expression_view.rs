//! Stored expression assignments. Device movement and global volume ranges are separate.
use crate::{session::Session, theme};
use eframe::egui::{self, RichText};
use openfuse_core::{
    model_catalog::{self, ModuleCategory},
    preset::{
        PresetDocument,
        expression::{self, ExpressionPedal, PedalSlot},
    },
};

pub fn panel(ui: &mut egui::Ui, session: &mut Session, error: &mut Option<String>, reveal: bool) {
    let header = egui::CollapsingHeader::new("Expression pedals").id_salt("expression-settings");
    let header = if reveal {
        header.open(Some(true))
    } else {
        header
    };
    let response = header.show(ui, |ui| {
        let capability = session.document.product_id().and_then(expression::capabilities);
        let slots = [
            (PedalSlot::First, "Pedal 1", capability.as_ref().is_some_and(|c| c.first)),
            (PedalSlot::Second, "Pedal 2", capability.as_ref().is_some_and(|c| c.second)),
        ];
        for (slot, label, supported) in slots {
            let current = match session.document.expression_pedal(slot) {
                Ok(value) => value,
                Err(issue) => {
                    ui.colored_label(theme::accent(ui.ctx()), format!("{label}: {issue}"));
                    continue;
                }
            };
            if !supported && current.is_none() { continue; }
            ui.push_id(label, |ui| {
                egui::Frame::new().fill(theme::card(ui.ctx())).corner_radius(12).inner_margin(18)
                    .show(ui, |ui| {
                        ui.heading(label);
                        let Some(mut values) = current else {
                            ui.label("No assignment is stored in this preset.");
                            let add = ui.button("Add pedal settings");
                            remember(&add, label, "add");
                            if add.clicked() && let Err(issue) = session.expression(slot, &ExpressionPedal::defaults(slot)) {
                                *error = Some(issue);
                            }
                            return;
                        };
                        if !supported {
                            ui.label(RichText::new("Stored settings are preserved. This product has no editable pedal profile.").color(theme::muted(ui.ctx())));
                        }
                        let changed = ui.add_enabled_ui(supported, |ui| {
                            controls(ui, &session.document, slot, label, &mut values)
                        }).inner;
                        if changed && let Err(issue) = session.expression(slot, &values) {
                            *error = Some(issue);
                        }
                    });
            });
            ui.add_space(12.0);
        }
        if !capability.as_ref().is_some_and(|c| c.first || c.second) {
            ui.label(RichText::new("Expression pedal editing is unavailable for this amplifier family.").color(theme::muted(ui.ctx())));
        }
    });
    let _ = response;
}

pub(crate) fn controls(
    ui: &mut egui::Ui,
    document: &PresetDocument,
    slot: PedalSlot,
    label: &str,
    values: &mut ExpressionPedal,
) -> bool {
    let mut changed = false;
    egui::Grid::new("pedal-fields")
        .num_columns(2)
        .spacing([20.0, 10.0])
        .show(ui, |ui| {
            ui.label("Expression target");
            let before = values.expression_mode_behavior;
            let targets = [
                (0, "Off"),
                (1, "Amplifier"),
                (2, "Stompbox"),
                (3, "Modulation"),
                (4, "Delay"),
                (5, "Reverb"),
                (6, "Volume"),
            ];
            let target_response = egui::ComboBox::from_id_salt("target")
                .width(300.0)
                .selected_text(enum_label(values.expression_mode_behavior, &targets))
                .show_ui(ui, |ui| {
                    for (value, name) in targets {
                        let available = value == 0
                            || value == 6
                            || !parameters(document, Some(value)).is_empty();
                        let response = ui.add_enabled(
                            available,
                            egui::Button::new(name)
                                .selected(values.expression_mode_behavior == Some(value)),
                        );
                        remember(&response, label, &format!("target-{value}"));
                        if response.clicked() {
                            values.expression_mode_behavior = Some(value);
                            ui.close_menu();
                        }
                    }
                })
                .response;
            remember(&target_response, label, "target");
            if values.expression_mode_behavior != before {
                values.parameter_index =
                    Some(default_parameter(document, values.expression_mode_behavior));
                changed = true;
            }
            ui.end_row();
            ui.label("Parameter");
            let choices = parameters(document, values.expression_mode_behavior);
            let selected = values
                .parameter_index
                .and_then(|index| choices.iter().find(|p| p.index == index))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| {
                    values
                        .parameter_index
                        .map(|index| format!("Stored parameter {index}"))
                        .unwrap_or_else(|| "Not stored".into())
                });
            let response = ui
                .add_enabled_ui(!choices.is_empty(), |ui| {
                    egui::ComboBox::from_id_salt("parameter")
                        .width(300.0)
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for parameter in &choices {
                                if ui
                                    .selectable_value(
                                        &mut values.parameter_index,
                                        Some(parameter.index),
                                        &parameter.name,
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            }
                        })
                        .response
                })
                .inner;
            remember(&response, label, "parameter");
            ui.end_row();
            changed |= range_row(ui, label, "Heel", &mut values.heel_setting, 0);
            changed |= range_row(ui, label, "Toe", &mut values.toe_setting, u16::MAX);
            changed |= enum_row(
                ui,
                label,
                "Tracking",
                &mut values.pedal_mode,
                &[(0, "Capture"), (1, "Live")],
            );
            if matches!(slot, PedalSlot::First) {
                changed |= enum_row(
                    ui,
                    label,
                    "Volume mode",
                    &mut values.volume_mode_behavior,
                    &[(0, "Off"), (1, "Volume")],
                );
                changed |= enum_row(
                    ui,
                    label,
                    "Preset override",
                    &mut values.pedal_override_state,
                    &[(0, "Override"), (1, "Do not override")],
                );
            }
            changed |= enum_row(
                ui,
                label,
                "Bypass effect in volume mode",
                &mut values.bypass_effect_when_volume_mode,
                &[(0, "No"), (1, "Yes")],
            );
            changed |= enum_row(
                ui,
                label,
                "Revert volume on switch",
                &mut values.volume_switch_revert,
                &[(0, "No"), (1, "Yes")],
            );
            changed |= enum_row(
                ui,
                label,
                "Initial mode",
                &mut values.default_pedal_state,
                &[(0, "Volume"), (1, "Expression")],
            );
        });
    changed
}

fn parameters(
    document: &PresetDocument,
    target: Option<u16>,
) -> Vec<&'static model_catalog::Parameter> {
    let Some(key) = target_key(target) else {
        return Vec::new();
    };
    let Some(product) = document.product_id() else {
        return Vec::new();
    };
    let Some(module) = document.modules().into_iter().find(|m| m.key == key) else {
        return Vec::new();
    };
    module
        .model_id
        .map(|id| expression::controllable_parameters(product, key, id))
        .unwrap_or_default()
        .into_iter()
        .filter(|parameter| {
            module
                .parameters
                .iter()
                .any(|stored| stored.index == parameter.index)
        })
        .collect()
}
fn target_key(target: Option<u16>) -> Option<&'static str> {
    match target {
        Some(1) => Some("Amplifier"),
        Some(2) => Some("Stompbox"),
        Some(3) => Some("Modulation"),
        Some(4) => Some("Delay"),
        Some(5) => Some("Reverb"),
        _ => None,
    }
}
fn default_parameter(document: &PresetDocument, target: Option<u16>) -> u8 {
    let choices = parameters(document, target);
    let default = target_key(target)
        .and_then(|key| {
            document
                .modules()
                .into_iter()
                .find(|m| m.key == key)
                .and_then(|m| {
                    model_catalog::model(
                        document.product_id()?,
                        ModuleCategory::from_key(key)?,
                        m.model_id?,
                    )
                })
        })
        .map(|model| model.expression_pedal_index);
    default
        .filter(|index| choices.iter().any(|p| p.index == *index))
        .or_else(|| choices.first().map(|p| p.index))
        .unwrap_or(0)
}
fn enum_label(value: Option<u16>, choices: &[(u16, &str)]) -> String {
    value
        .map(|value| {
            choices
                .iter()
                .find(|(id, _)| *id == value)
                .map(|(_, label)| (*label).into())
                .unwrap_or_else(|| format!("Unknown ({value})"))
        })
        .unwrap_or_else(|| "Not stored".into())
}
fn enum_row(
    ui: &mut egui::Ui,
    pedal: &str,
    label: &str,
    value: &mut Option<u16>,
    choices: &[(u16, &str)],
) -> bool {
    let before = *value;
    ui.label(label);
    let response = egui::ComboBox::from_id_salt(label)
        .width(300.0)
        .selected_text(enum_label(*value, choices))
        .show_ui(ui, |ui| {
            for (id, name) in choices {
                let choice = ui.selectable_value(value, Some(*id), *name);
                remember(&choice, pedal, &format!("{label}-{id}"));
            }
        })
        .response;
    remember(&response, pedal, label);
    ui.end_row();
    *value != before
}
fn range_row(
    ui: &mut egui::Ui,
    pedal: &str,
    label: &str,
    value: &mut Option<u16>,
    default: u16,
) -> bool {
    ui.label(label);
    let before = *value;
    if let Some(value) = value {
        ui.spacing_mut().slider_width = 260.0;
        let response = ui.add(
            egui::Slider::new(value, 0..=u16::MAX)
                .clamping(egui::SliderClamping::Never)
                .custom_formatter(|value, _| format!("{:.1}%", value / 65535.0 * 100.0))
                .custom_parser(|text| {
                    text.trim()
                        .trim_end_matches('%')
                        .parse::<f64>()
                        .ok()
                        .filter(|value| (0.0..=100.0).contains(value))
                        .map(|value| value / 100.0 * 65535.0)
                }),
        );
        remember(&response, pedal, label);
    } else {
        let response = ui.button(format!("Set to {}%", if default == 0 { 0 } else { 100 }));
        remember(&response, pedal, label);
        if response.clicked() {
            *value = Some(default);
        }
    }
    ui.end_row();
    *value != before
}
fn remember(response: &egui::Response, pedal: &str, key: &str) {
    theme::remember(response, &format!("expression-{pedal}-{key}"));
    #[cfg(test)]
    response.ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new(("expression-control", pedal, key)),
            response.rect,
        )
    });
    #[cfg(not(test))]
    let _ = (response, pedal, key);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        session: &mut Session,
        events: Vec<egui::Event>,
    ) -> Option<String> {
        let mut error = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 1100.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panel(ui, session, &mut error, true));
            },
        );
        error
    }
    fn click(ctx: &egui::Context, session: &mut Session, key: &str) {
        let rect: egui::Rect = ctx.data(|data| {
            data.get_temp(egui::Id::new(("expression-control", "Pedal 1", key)))
                .unwrap()
        });
        let pos = rect.center();
        for pressed in [true, false] {
            let error = frame(
                ctx,
                session,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            assert!(error.is_none(), "{error:?}");
        }
        for _ in 0..2 {
            frame(ctx, session, Vec::new());
        }
    }
    #[test]
    fn pointer_controls_create_assignment_rebind_target_and_change_tracking_with_undo() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut session = Session::example().unwrap();
        for _ in 0..2 {
            frame(&ctx, &mut session, Vec::new());
        }
        click(&ctx, &mut session, "add");
        let defaults = session
            .document
            .expression_pedal(PedalSlot::First)
            .unwrap()
            .unwrap();
        assert_eq!(defaults.expression_mode_behavior, Some(6));
        frame(&ctx, &mut session, Vec::new());
        click(&ctx, &mut session, "target");
        click(&ctx, &mut session, "target-1");
        let assignment = session
            .document
            .expression_pedal(PedalSlot::First)
            .unwrap()
            .unwrap();
        assert_eq!(assignment.expression_mode_behavior, Some(1));
        assert!(
            parameters(&session.document, Some(1))
                .iter()
                .any(|p| Some(p.index) == assignment.parameter_index)
        );
        let before = session.document.to_xml().unwrap();
        frame(&ctx, &mut session, Vec::new());
        click(&ctx, &mut session, "Tracking");
        click(&ctx, &mut session, "Tracking-1");
        assert_eq!(
            session
                .document
                .expression_pedal(PedalSlot::First)
                .unwrap()
                .unwrap()
                .pedal_mode,
            Some(1)
        );
        session.undo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), before);
    }
    #[test]
    fn rendering_unknown_modes_and_reversed_range_keeps_original_xml() {
        let ctx = egui::Context::default();
        let mut session = Session::example().unwrap();
        let mut values = ExpressionPedal::defaults(PedalSlot::First);
        values.heel_setting = Some(65535);
        values.toe_setting = Some(0);
        session.expression(PedalSlot::First, &values).unwrap();
        let xml = session
            .document
            .to_xml()
            .unwrap()
            .replace("PedalMode=\"0\"", "PedalMode=\"60000\"");
        let mut session = Session::parse(&xml, None).unwrap();
        for _ in 0..2 {
            assert!(frame(&ctx, &mut session, Vec::new()).is_none());
        }
        assert_eq!(session.document.to_xml().unwrap(), xml);
        assert!(!session.dirty());
        assert!(!session.can_undo());
    }
}
