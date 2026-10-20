//! Local metadata controls; no community request is made by editing these fields.
use crate::{session::Session, theme};
use eframe::egui::{self, Color32, RichText};
use openfuse_core::preset::annotations::{DEFAULT_PEDAL_COLORS, GENRES, PEDAL_COLORS};

pub fn color_for(key: &str, colors: Option<[i32; 4]>) -> Option<Color32> {
    let index = match key {
        "Stompbox" => 0,
        "Modulation" => 1,
        "Delay" => 2,
        "Reverb" => 3,
        _ => return None,
    };
    let value = colors?[index];
    let color = PEDAL_COLORS.iter().find(|c| c.id == value)?;
    Some(Color32::from_rgb(color.rgb[0], color.rgb[1], color.rgb[2]))
}

pub fn details(ui: &mut egui::Ui, session: &mut Session, error: &mut Option<String>) {
    let mut values = match session.document.annotations() {
        Ok(values) => values,
        Err(issue) => {
            ui.colored_label(
                theme::accent(ui.ctx()),
                format!("Cannot edit stored details: {issue}"),
            );
            return;
        }
    };
    let before = values.clone();
    egui::Grid::new("extended_metadata").num_columns(2).spacing([24.0,10.0]).show(ui,|ui| {
        for (index,label) in ["Main genre","Subgenre 1","Subgenre 2"].into_iter().enumerate() {
            ui.label(label);
            let enabled=index==0 || values.genres[index-1].is_some_and(|g|g != -1);
            ui.add_enabled_ui(enabled,|ui| {
                egui::ComboBox::from_id_salt(("genre",index)).width(260.0)
                    .selected_text(genre_label(values.genres[index])).show_ui(ui,|ui| {
                        ui.selectable_value(&mut values.genres[index],None,"Not stored");
                        let mut genres:Vec<_>=GENRES.iter().collect();genres.sort_by_key(|g|g.label.to_lowercase());
                        for genre in genres {ui.selectable_value(&mut values.genres[index],Some(genre.id),genre.label);}
                    });
            }).response.on_hover_text("Subgenres become editable after choosing the preceding genre. Existing values are preserved.");
            ui.end_row();
        }
        ui.label("Local rating");
        let mut rating=values.rating.clone().unwrap_or_default();
        if ui.add(egui::TextEdit::singleline(&mut rating).id(egui::Id::new("local-rating")).desired_width(160.0).hint_text("e.g. 4.5 or 4,5")).on_hover_text("Decimal text stored exactly in this file. Keep the original decimal separator. Does not submit a community rating.").changed() {
            values.rating=if rating.is_empty(){None}else{Some(rating)};
        }
        ui.end_row();
        ui.label("Fender preset ID");
        ui.horizontal(|ui| {
            if let Some(value)=values.fender_id.as_mut() {
                ui.add(egui::DragValue::new(value).speed(1));
                if ui.small_button("Clear").clicked(){values.fender_id=None;}
            } else {
                ui.label(RichText::new("Not stored").color(theme::muted(ui.ctx())));
                if ui.small_button("Add").clicked(){values.fender_id=Some(0);}
            }
        }).response.on_hover_text("Original file identifier, retained for archive compatibility.");
        ui.end_row();
    });
    if values != before {
        match session.annotations(&values) {
            Ok(()) => *error = None,
            Err(issue) => *error = Some(issue),
        }
    }
}

fn genre_label(value: Option<i32>) -> String {
    match value {
        None => "Not stored".into(),
        Some(id) => GENRES
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.label.to_owned())
            .unwrap_or_else(|| format!("Unknown genre ({id})")),
    }
}

pub fn colors(ui: &mut egui::Ui, session: &mut Session, error: &mut Option<String>) {
    let current = match session.document.pedal_colors() {
        Ok(colors) => colors,
        Err(issue) => {
            ui.colored_label(
                theme::accent(ui.ctx()),
                format!("Cannot edit stored colors: {issue}"),
            );
            return;
        }
    };
    let Some(mut values) = current else {
        ui.horizontal(|ui| {
            ui.label("This file has no pedal colors.");
            if ui.button("Add default colors").clicked()
                && let Err(issue) = session.pedal_colors(DEFAULT_PEDAL_COLORS)
            {
                *error = Some(issue);
            }
        });
        return;
    };
    let before = values;
    ui.horizontal_wrapped(|ui| {
        for (index, label) in ["Stompbox", "Modulation", "Delay", "Reverb"]
            .into_iter()
            .enumerate()
        {
            ui.vertical(|ui| {
                ui.label(label);
                let selected = PEDAL_COLORS
                    .iter()
                    .find(|c| c.id == values[index])
                    .map(|c| c.label.to_owned())
                    .unwrap_or_else(|| format!("Unknown ({})", values[index]));
                egui::ComboBox::from_id_salt(("pedal_color", index))
                    .width(145.0)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for color in PEDAL_COLORS {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(14.0, 14.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    2.0,
                                    Color32::from_rgb(color.rgb[0], color.rgb[1], color.rgb[2]),
                                );
                                ui.selectable_value(&mut values[index], color.id, color.label);
                            });
                        }
                    });
                if let Some(color) = color_for(label, Some(values)) {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(145.0, 4.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 1.0, color);
                }
            });
        }
    });
    if values != before
        && let Err(issue) = session.pedal_colors(values)
    {
        *error = Some(issue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_a_rating_changes_the_document_and_undo_restores_it() {
        let context = egui::Context::default();
        let mut session = Session::example().unwrap();
        let before = session.document.to_xml().unwrap();
        let mut error = None;
        let draw = |ctx: &egui::Context, session: &mut Session, error: &mut Option<String>| {
            egui::CentralPanel::default().show(ctx, |ui| details(ui, session, error));
        };
        let _ = context.run(egui::RawInput::default(), |ctx| {
            draw(ctx, &mut session, &mut error)
        });
        context.memory_mut(|m| m.request_focus(egui::Id::new("local-rating")));
        let input = egui::RawInput {
            events: vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text("4.125".into()),
            ],
            ..Default::default()
        };
        let _ = context.run(input, |ctx| draw(ctx, &mut session, &mut error));
        assert!(error.is_none(), "{error:?}");
        assert_eq!(
            session.document.annotations().unwrap().rating.as_deref(),
            Some("4.125")
        );
        assert!(session.dirty());
        session.undo().unwrap();
        assert_eq!(session.document.to_xml().unwrap(), before);
    }

    #[test]
    fn unknown_annotation_values_render_without_normalizing_or_dropping_them() {
        let mut session = Session::example().unwrap();
        let mut values = session.document.annotations().unwrap();
        values.genres = [Some(600), Some(-7), Some(i32::MAX)];
        values.rating = Some("4.1234567890123456789012345678".into());
        session.annotations(&values).unwrap();
        session.pedal_colors([-1, 500, 16, i32::MAX]).unwrap();
        let before = session.document.to_xml().unwrap();
        let context = egui::Context::default();
        let mut error = None;
        for _ in 0..2 {
            let _ = context.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    details(ui, &mut session, &mut error);
                    colors(ui, &mut session, &mut error);
                });
            });
        }
        assert!(error.is_none());
        assert_eq!(session.document.to_xml().unwrap(), before);
        assert_eq!(color_for("Stompbox", Some([-1, 500, 16, i32::MAX])), None);
    }
}
