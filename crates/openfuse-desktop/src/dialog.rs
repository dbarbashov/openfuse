//! Shared modal surfaces. Content stays bounded even for long paths and diagnostics.
use crate::theme;
use eframe::egui::{self, RichText};

pub fn show<R>(
    ctx: &egui::Context,
    id: &str,
    preferred_width: f32,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::ModalResponse<R> {
    let width = preferred_width.min((ctx.screen_rect().width() - 104.0).max(240.0));
    let frame = egui::Frame::new()
        .fill(theme::card(ctx))
        .inner_margin(egui::Margin::symmetric(28, 24))
        .corner_radius(16)
        .stroke(egui::Stroke::new(1.0_f32, theme::line(ctx)))
        .shadow(ctx.style().visuals.window_shadow);
    #[cfg(test)]
    let mut inner_rect = egui::Rect::NOTHING;
    let response = egui::Modal::new(egui::Id::new(id))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
            ui.spacing_mut().button_padding = egui::vec2(14.0, 9.0);
            ui.spacing_mut().interact_size.y = 36.0;
            ui.style_mut()
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(21.0));
            let value = content(ui);
            #[cfg(test)]
            {
                inner_rect = ui.min_rect();
            }
            value
        });
    #[cfg(test)]
    ctx.data_mut(|d| {
        d.insert_temp(
            egui::Id::new(("dialog-bounds", id)),
            (response.response.rect, inner_rect),
        )
    });
    response
}

pub fn header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.heading(title);
    if !subtitle.is_empty() {
        hint(ui, subtitle);
    }
    ui.add_space(10.0);
}

pub fn hint(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(text.into()).color(theme::muted(ui.ctx()))).wrap());
}

pub fn section<R>(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = ui.available_width();
    let result = egui::Frame::new()
        .fill(theme::bg(ui.ctx()))
        .corner_radius(10)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width((width - 32.0).max(100.0));
            if !title.is_empty() {
                ui.label(RichText::new(title).strong());
                ui.add_space(4.0);
            }
            content(ui)
        })
        .inner;
    ui.add_space(4.0);
    result
}

pub fn footer<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    ui.horizontal_wrapped(content).inner
}

pub fn primary(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).color(theme::bg(ui.ctx())))
            .fill(theme::accent(ui.ctx()))
            .min_size(egui::vec2(100.0, 40.0)),
    )
}

/// Present established protocol notes in user language, retaining every original
/// note on demand. Unknown warnings remain visible verbatim.
pub fn review_notes(ui: &mut egui::Ui, notes: &[String]) {
    if notes.is_empty() {
        return;
    }
    section(ui, "Before you apply", |ui| {
        let mut shown = Vec::new();
        for note in notes {
            let summary = if note.starts_with("Only sound state is sent;") {
                "Preset metadata stays in the local file.".to_owned()
            } else if note.contains("continuous parameters lose their low 8 bits") {
                format!(
                    "{}: some values use lower precision on the amplifier.",
                    note.split(':').next().unwrap_or("Parameters")
                )
            } else if note.starts_with("This operation replaces only the temporary working preset")
            {
                "Temporary sound only, followed by a readback check. Hardware testing is still pending.".to_owned()
            } else if note.starts_with("Preset-only restore overwrites this slot") {
                "The restored sound becomes active. Global pedal and system/control-bank settings stay unchanged.".to_owned()
            } else if note.starts_with("Preset archive only:") {
                "This archive contains presets, without system settings or control banks."
                    .to_owned()
            } else {
                note.clone()
            };
            if !shown.contains(&summary) {
                ui.label(&summary);
                shown.push(summary);
            }
        }
        egui::CollapsingHeader::new(format!("Technical notes ({})", notes.len())).show(ui, |ui| {
            for note in notes {
                hint(ui, note);
            }
        });
    });
}
