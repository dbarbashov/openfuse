//! Shared semantic appearance for the native editor. No Apple assets are bundled.
use eframe::egui::{self, Color32, FontId, TextStyle};

pub fn bg(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(25, 26, 29)
    } else {
        Color32::from_rgb(246, 246, 248)
    }
}
pub fn panel(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(34, 35, 39)
    } else {
        Color32::from_rgb(237, 238, 241)
    }
}
pub fn card(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(43, 44, 48)
    } else {
        Color32::WHITE
    }
}
pub fn ink(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(242, 242, 247)
    } else {
        Color32::from_rgb(28, 28, 30)
    }
}
pub fn muted(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(174, 175, 183)
    } else {
        Color32::from_rgb(101, 103, 113)
    }
}
pub fn accent(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(85, 159, 255)
    } else {
        Color32::from_rgb(0, 99, 218)
    }
}
pub fn line(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_white_alpha(24)
    } else {
        Color32::from_black_alpha(20)
    }
}

pub fn danger(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(255, 133, 128)
    } else {
        Color32::from_rgb(184, 37, 31)
    }
}
pub fn warning(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(255, 198, 90)
    } else {
        Color32::from_rgb(134, 83, 0)
    }
}
pub const APPEARANCE_KEY: &str = "openfuse.appearance.v1";

pub fn install(ctx: &egui::Context) {
    // Use the installed platform font on macOS; retain egui's portable fallback.
    #[cfg(target_os = "macos")]
    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("system-ui".into(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "system-ui".into());
        ctx.set_fonts(fonts);
    }
    for mode in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_theme(mode);
        ctx.set_visuals(match mode {
            egui::Theme::Light => egui::Visuals::light(),
            egui::Theme::Dark => egui::Visuals::dark(),
        });
        let mut style = (*ctx.style()).clone();
        style.visuals.panel_fill = panel(ctx);
        style.visuals.window_fill = card(ctx);
        style.visuals.extreme_bg_color = bg(ctx);
        style.visuals.override_text_color = Some(ink(ctx));
        style.visuals.selection.bg_fill = accent(ctx).gamma_multiply(0.15);
        style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, accent(ctx));
        style.visuals.widgets.inactive.bg_fill = if mode == egui::Theme::Dark {
            Color32::from_rgb(83, 85, 92)
        } else {
            Color32::from_rgb(217, 219, 224)
        };
        style.visuals.widgets.inactive.weak_bg_fill = card(ctx);
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, line(ctx));
        style.visuals.widgets.hovered.bg_fill = panel(ctx);
        style.visuals.widgets.hovered.weak_bg_fill = panel(ctx);
        style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, accent(ctx));
        style.visuals.widgets.active.bg_fill = accent(ctx).gamma_multiply(0.18);
        style.visuals.widgets.active.weak_bg_fill = accent(ctx).gamma_multiply(0.18);
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, accent(ctx));
        style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, line(ctx));
        for visual in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            visual.corner_radius = egui::CornerRadius::same(6);
        }
        style.visuals.window_corner_radius = egui::CornerRadius::same(12);
        style.visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(40),
        };
        style.visuals.window_stroke = egui::Stroke::new(1.0_f32, line(ctx));
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size.y = 32.0;
        style.spacing.slider_width = 240.0;
        style.spacing.slider_rail_height = 4.0;
        style.visuals.slider_trailing_fill = true;
        style.spacing.window_margin = egui::Margin::same(22);
        style.animation_time = 0.15;
        for (kind, size) in [
            (TextStyle::Body, 13.0),
            (TextStyle::Button, 13.0),
            (TextStyle::Heading, 22.0),
            (TextStyle::Small, 11.0),
        ] {
            style.text_styles.insert(kind, FontId::proportional(size));
        }
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.0));
        ctx.set_style_of(mode, style);
    }
    ctx.set_theme(egui::ThemePreference::System);
}

pub fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(11.0)
            .strong()
            .color(muted(ui.ctx())),
    );
}

pub fn surface(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(card(ui.ctx()))
        .corner_radius(12)
        .inner_margin(22)
        .stroke(egui::Stroke::new(1.0_f32, line(ui.ctx())))
}

/// Capture semantic widget bounds for native input automation; never drives state.
pub fn remember(response: &egui::Response, key: &str) {
    response
        .ctx
        .data_mut(|d| d.insert_temp(egui::Id::new(("tour-control", key)), response.rect));
}
