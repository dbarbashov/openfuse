//! Audio output preferences. Rendering never enumerates devices or starts playback.
use crate::{audio_state::State, controls_view::button, theme};
use eframe::egui::{self, RichText};
use openfuse_audio::settings::{OutputConfiguration, OutputSettings};
pub enum Action {
    Refresh,
    Apply,
    Reset,
    Revert,
    SystemPanel,
    Close,
}

pub fn show(
    ctx: &egui::Context,
    state: &mut State,
    applied: &OutputSettings,
    active: Option<&OutputConfiguration>,
) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "audio-settings", 600.0, |ui| {
        theme::eyebrow(
            ui,
            if state.demo {
                "SYNTHETIC AUDIO DEMO"
            } else {
                "COMPUTER AUDIO"
            },
        );
        crate::dialog::header(ui, "Audio output", "Choose where your backing tracks play.");
        egui::ScrollArea::vertical().id_salt("audio-settings-scroll")
            .max_height((ctx.screen_rect().height() - 290.0).clamp(220.0, 530.0)).show(ui, |ui| {
            if let Some(warning) = &state.storage_warning { ui.colored_label(theme::warning(ui.ctx()), warning); }
            crate::dialog::section(ui, "Output device", |ui| {
                let selected = state.draft.device.as_ref().map(|key| format!("{} · {}", key.name, key.host)).unwrap_or_else(|| "System default".into());
                egui::ComboBox::from_id_salt("audio-output-device").width(ui.available_width()).selected_text(selected).show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.draft.device, None, "System default at playback time");
                    if let Some(inventory) = &state.inventory {
                        for device in &inventory.devices { ui.selectable_value(&mut state.draft.device, Some(device.key.clone()), format!("{} · {}", device.key.name, device.key.host)); }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if button(ui, "audio-refresh", if state.busy() { "Working…" } else { "Refresh devices" }, !state.busy()) { action = Some(Action::Refresh); }
                    if button(ui, "audio-system-panel", "System audio panel…", !state.busy()) { action = Some(Action::SystemPanel); }
                });
                if state.inventory.is_none() { crate::dialog::hint(ui, "Refresh to list devices. A disconnected saved output is kept."); }
            });
            crate::dialog::section(ui, "Playback format", |ui| {
                ui.columns(2, |columns| {
                    optional_number(&mut columns[0], "Sample rate", &mut state.draft.sample_rate, 48000, 8000..=384000, " Hz");
                    optional_number(&mut columns[1], "Buffer size", &mut state.draft.buffer_frames, 256, 1..=65536, " frames");
                });
                if let Some(inventory) = &state.inventory {
                    match openfuse_audio::settings::plan_output(&state.draft, inventory) {
                        Ok(plan) => { crate::dialog::hint(ui, format!("{} Hz · {} channels · {}", plan.sample_rate, plan.channels, plan.format)); }
                        Err(error) => { ui.colored_label(theme::danger(ui.ctx()), error); }
                    }
                    let device = match &state.draft.device { Some(key) => inventory.devices.iter().find(|device| device.key == *key), None => inventory.system_default.as_ref() };
                    if let Some(device) = device {
                        if let Some(error) = &device.error { ui.colored_label(theme::danger(ui.ctx()), error); }
                        egui::CollapsingHeader::new("Reported device capabilities").show(ui, |ui| {
                            for range in device.supported.iter().take(24) {
                                let buffer = match (range.min_buffer_frames, range.max_buffer_frames) { (Some(min), Some(max)) => format!("{min}–{max} frames"), _ => "buffer range not reported".into() };
                                ui.label(format!("{} channels · {}–{} Hz · {} · {}", range.channels, range.min_sample_rate, range.max_sample_rate, range.format, buffer));
                            }
                            if device.supported.len() > 24 { ui.label(format!("{} additional ranges", device.supported.len() - 24)); }
                        });
                    }
                }
            });
            if let Some(output) = active { crate::dialog::hint(ui, format!("Playing output: {} · {} Hz · {} channels", output.device.name, output.sample_rate, output.channels)); }
            else { crate::dialog::hint(ui, "No output stream is open"); }
            egui::CollapsingHeader::new("About these settings").show(ui, |ui| {
                crate::dialog::hint(ui, "Refresh reads capabilities without opening a stream. These settings affect the local player, not amplifier USB recording or ASIO drivers. System audio panel: Cmd/Ctrl+Shift+A. Default draft also requires Apply.");
            });
            if let Some(notice) = &state.notice { ui.label(notice); }
            if let Some(error) = &state.error { ui.colored_label(theme::danger(ui.ctx()), error); }
        });
        crate::dialog::hint(
            ui,
            "Apply pauses playback and keeps your position. Press Play to resume.",
        );
        crate::dialog::footer(ui, |ui| {
            if button(ui, "audio-defaults", "Defaults", !state.busy()) {
                action = Some(Action::Reset);
            }
            if button(ui, "audio-revert", "Revert", state.draft != *applied) {
                action = Some(Action::Revert);
            }
            if button(ui, "audio-close", "Close", true) {
                action = Some(Action::Close);
            }
            let apply = crate::dialog::primary(
                ui,
                "Apply & pause",
                !state.busy() && state.validate_draft().is_ok(),
            );
            crate::audition_view::remember(&apply, "audio-apply");
            if apply.clicked() {
                action = Some(Action::Apply);
            }
        });
    });
    action
}

fn optional_number(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<u32>,
    default: u32,
    range: std::ops::RangeInclusive<u32>,
    suffix: &str,
) {
    ui.label(RichText::new(label).strong());
    let mut automatic = value.is_none();
    if ui.checkbox(&mut automatic, "Use device default").changed() {
        *value = if automatic { None } else { Some(default) };
    }
    if let Some(value) = value {
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .clamp_existing_to_range(false)
                .suffix(suffix),
        );
    }
}
