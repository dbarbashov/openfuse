//! Native amplifier controls. Preview data is owned, and rendering does no I/O.
use crate::{audition_view::remember, device_state::DeviceState, theme};
use eframe::egui::{self, RichText};
use openfuse_core::{controls, protocol::Report64};
use openfuse_device::{LiveBinding, WireCapture};

#[derive(Default)]
pub struct ControlsUi {
    pub open: bool,
    pub draft: Option<SettingsDraft>,
    pub preview: Option<SettingsPreview>,
    pub mute: bool,
    pub demo: u8,
    pub demo_started: bool,
}
impl ControlsUi {
    pub fn modal_open(&self) -> bool {
        self.open || self.preview.is_some()
    }
}
pub struct SettingsDraft {
    pub binding: LiveBinding,
    pub pid: u16,
    pub name: String,
    pub reports: Vec<Report64>,
    pub current: controls::Settings,
    pub patch: controls::SettingsPatch,
}
impl SettingsDraft {
    pub fn prepare(capture: &WireCapture) -> Result<Self, String> {
        let reports = capture
            .reports
            .iter()
            .map(|raw| Report64::try_from(raw.as_slice()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            binding: LiveBinding {
                device_id: capture.device.id.clone(),
                generation: capture.generation,
            },
            pid: capture.device.product_id,
            name: capture.device.name.clone(),
            current: controls::settings(capture.device.product_id, &reports)
                .map_err(|e| e.to_string())?,
            reports,
            patch: Default::default(),
        })
    }
    pub fn preview(&self) -> Result<SettingsPreview, String> {
        let plan = controls::settings_plan(self.pid, &self.reports, &self.patch)
            .map_err(|e| e.to_string())?;
        Ok(SettingsPreview {
            binding: self.binding.clone(),
            name: self.name.clone(),
            patch: self.patch.clone(),
            warnings: plan.warnings().to_vec(),
            changes: changes(&self.current, &self.patch),
        })
    }
}
pub struct SettingsPreview {
    pub binding: LiveBinding,
    pub name: String,
    pub patch: controls::SettingsPatch,
    pub changes: Vec<(String, String, String)>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Copy)]
pub enum Action {
    Close,
    Reload,
    Review,
    Apply,
    Back,
    Tuner(bool),
    Cancel,
}
pub fn button(ui: &mut egui::Ui, id: &str, label: &str, enabled: bool) -> bool {
    let r = ui.add_enabled(
        enabled,
        egui::Button::new(label).min_size(egui::vec2(120.0, 40.0)),
    );
    remember(&r, id);
    r.clicked()
}
fn raw(value: Option<u8>) -> String {
    value.map_or_else(|| "Not reported".into(), |v| v.to_string())
}
fn changes(
    settings: &controls::Settings,
    patch: &controls::SettingsPatch,
) -> Vec<(String, String, String)> {
    let mut rows = vec![];
    if let Some(value) = patch.usb_gain {
        rows.push((
            "USB recording level".into(),
            raw(settings.usb_gain),
            value.to_string(),
        ));
    }
    if let Some(value) = patch.fx_loop_bypassed {
        rows.push((
            "FX loop state".into(),
            settings.fx_loop_bypassed.map_or_else(
                || "Not reported".into(),
                |v| {
                    if v {
                        "Active".into()
                    } else {
                        "Inactive".into()
                    }
                },
            ),
            if value {
                "Active".into()
            } else {
                "Inactive".into()
            },
        ));
    }
    if let Some(value) = &patch.global_volume {
        let before = settings.global_volume.as_ref().map_or_else(
            || "Not reported".into(),
            |v| format!("{} / {}", v.heel, v.toe),
        );
        rows.push((
            "Global volume: heel / toe".into(),
            before,
            format!("{} / {}", value.heel, value.toe),
        ));
    }
    for (label, current, value) in [
        (
            "First pedal mode",
            settings.first_pedal_mode,
            patch.first_pedal_mode,
        ),
        (
            "Second pedal mode",
            settings.second_pedal_mode,
            patch.second_pedal_mode,
        ),
    ] {
        if let Some(value) = value {
            rows.push((
                label.into(),
                current.map_or_else(|| "Not reported".into(), mode),
                mode(value),
            ));
        }
    }
    rows
}
pub fn preview(ctx: &egui::Context, draft: &SettingsPreview, valid: bool) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "controls-settings-preview", 610.0, |ui| {
        theme::eyebrow(ui, "AMPLIFIER SETTINGS · REVIEW");
        ui.heading("Apply these device settings");
        ui.label(format!(
            "{} · read {}",
            draft.name, draft.binding.generation
        ));
        ui.add_space(14.0);
        egui::Grid::new("settings-before-after")
            .num_columns(3)
            .spacing([24.0, 12.0])
            .show(ui, |ui| {
                ui.strong("Setting");
                ui.strong("Current read");
                ui.strong("Apply");
                ui.end_row();
                for (name, before, after) in &draft.changes {
                    ui.label(name);
                    ui.monospace(before);
                    ui.label(
                        RichText::new(after)
                            .monospace()
                            .color(theme::accent(ui.ctx())),
                    );
                    ui.end_row();
                }
            });
        ui.add_space(14.0);
        ui.label("The amplifier receives only the selected settings. A fresh readback checks the result; your local preset and undo history stay intact.");
        for warning in &draft.warnings {
            ui.label(
                RichText::new(warning)
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
            );
        }
        if !valid {
            ui.colored_label(
                theme::accent(ui.ctx()),
                "This read is no longer current. Go back and reload device settings.",
            );
        }
        ui.add_space(12.0);
        crate::dialog::footer(ui, |ui| {
            if button(ui, "controls-apply", "Apply + verify", valid) {
                action = Some(Action::Apply);
            }
            if button(ui, "controls-back", "Back", true) {
                action = Some(Action::Back);
            }
        });
    });
    action
}
pub fn tuner(ui: &mut egui::Ui, state: &DeviceState, mute: &mut bool, action: &mut Option<Action>) {
    let status = &state.tuner;
    theme::eyebrow(ui, "TUNER");
    let active = status.enabled || status.pending || status.error.is_some();
    ui.horizontal(|ui| {
        if button(
            ui,
            "tuner-toggle",
            if active { "Stop tuner" } else { "Start tuner" },
            active || (state.current_binding().is_some() && !state.operation_busy()),
        ) {
            *action = Some(Action::Tuner(!active));
        }
        let r = ui.add_enabled(
            !status.pending,
            egui::Checkbox::new(mute, "Mute while tuning"),
        );
        remember(&r, "tuner-mute");
        if r.changed() && status.enabled {
            *action = Some(Action::Tuner(true));
        }
    });
    if !active {
        ui.label(
            RichText::new("Start the tuner to receive pitch readings from the amplifier.")
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
        return;
    }
    let display = tuner_display(status);
    egui::Frame::new()
        .fill(theme::bg(ui.ctx()))
        .corner_radius(8)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_sized(
                    [155.0, 72.0],
                    egui::Label::new(RichText::new(&display.0).monospace().size(48.0).color(
                        if status.stale {
                            theme::muted(ui.ctx())
                        } else {
                            theme::accent(ui.ctx())
                        },
                    )),
                );
                ui.vertical(|ui| {
                    ui.label(RichText::new(&display.1).monospace().size(20.0));
                    ui.label(
                        RichText::new(&display.2)
                            .size(12.0)
                            .color(theme::muted(ui.ctx())),
                    );
                });
            });
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::hover());
            let left = rect.left() + 18.0;
            let right = rect.right() - 18.0;
            let y = rect.center().y;
            ui.painter().line_segment(
                [egui::pos2(left, y), egui::pos2(right, y)],
                egui::Stroke::new(2.0_f32, theme::muted(ui.ctx())),
            );
            for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let x = egui::lerp(left..=right, fraction);
                ui.painter().line_segment(
                    [egui::pos2(x, y - 5.0), egui::pos2(x, y + 5.0)],
                    egui::Stroke::new(1.0_f32, theme::muted(ui.ctx())),
                );
            }
            if status.enabled
                && !status.stale
                && let Some(reading) = status.reading.filter(|r| r.note.is_pitch())
            {
                let offset = if (-3..=3).contains(&reading.offset) {
                    0
                } else {
                    reading.offset
                };
                let x = egui::lerp(
                    left..=right,
                    (f32::from(offset).clamp(-50.0, 50.0) + 50.0) / 100.0,
                );
                let color = if offset == 0 {
                    egui::Color32::from_rgb(126, 194, 153)
                } else {
                    theme::accent(ui.ctx())
                };
                ui.painter().circle_filled(egui::pos2(x, y), 6.0, color);
            }
            ui.horizontal(|ui| {
                ui.monospace("−50");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.monospace("+50");
                });
            });
        });
    ui.label(
        RichText::new(&status.detail)
            .size(12.0)
            .color(theme::muted(ui.ctx())),
    );
    if let Some(error) = &status.error {
        ui.colored_label(theme::accent(ui.ctx()), error);
    }
}
pub fn tuner_display(status: &openfuse_device::TunerStatus) -> (String, String, String) {
    if status.error.is_some() {
        return (
            "—".into(),
            "Tuner state unknown".into(),
            "Stop again to confirm amplifier state".into(),
        );
    }
    if status.pending {
        return (
            "—".into(),
            if status.enabled {
                "Starting tuner…"
            } else {
                "Stopping tuner…"
            }
            .into(),
            "Waiting for amplifier confirmation".into(),
        );
    }
    if !status.enabled {
        return (
            "—".into(),
            "Tuner off".into(),
            "Start to receive amplifier pitch readings".into(),
        );
    }
    if status.stale {
        return (
            "—".into(),
            "Waiting for signal".into(),
            "No fresh pitch reading".into(),
        );
    }
    match &status.reading {
        Some(reading) => {
            let note = if reading.note.is_pitch() {
                reading.note.label()
            } else if reading.tone == 12 {
                "—"
            } else {
                "?"
            };
            let detail = if reading.tone == 12 {
                "No stable note".into()
            } else if reading.tone > 12 {
                format!("Unknown tone code {}", reading.tone)
            } else if !(-50..=50).contains(&reading.offset) {
                format!("Out of range ({:+})", reading.offset)
            } else {
                format!("Raw deviation {:+}", reading.offset)
            };
            (
                note.into(),
                detail,
                if status.confirmed {
                    "Amplifier state confirmed"
                } else {
                    "Pitch received · mute state unconfirmed"
                }
                .into(),
            )
        }
        None => (
            "—".into(),
            "Waiting for signal".into(),
            "No pitch reading yet".into(),
        ),
    }
}

pub fn dialog(
    ctx: &egui::Context,
    state: &DeviceState,
    controls: &mut ControlsUi,
) -> Option<Action> {
    let mut action = None;
    let valid = controls
        .draft
        .as_ref()
        .is_some_and(|draft| state.matches_capture(&draft.binding))
        && !state.operation_busy();
    crate::dialog::show(ctx, "device-controls", 660.0, |ui| {
        theme::eyebrow(
            ui,
            if state.simulated {
                "SIMULATOR · AMPLIFIER CONTROLS"
            } else {
                "AMPLIFIER CONTROLS"
            },
        );
        ui.heading("Tune & configure");
        egui::ScrollArea::vertical().id_salt("controls-scroll").max_height((ctx.screen_rect().height()-305.0).clamp(250.0,650.0)).show(ui,|ui| {
            let pid=controls.draft.as_ref().map(|d|d.pid).or_else(||state.capture.as_ref().map(|c|c.device.product_id));
            if pid.and_then(|pid|controls::capabilities(pid).ok()).is_some_and(|cap|cap.tuner) {
                tuner(ui,state,&mut controls.mute,&mut action);ui.add_space(14.0);ui.separator();
            } else {ui.label("This amplifier has no supported tuner control.");}
            ui.add_space(8.0);theme::eyebrow(ui,"DEVICE SETTINGS");
            ui.label("Choose the settings to change, then review them before sending.");
            if let Some(draft)=&mut controls.draft {
                ui.label(RichText::new(format!("{} · read {}",draft.name,draft.binding.generation)).size(12.0).color(theme::muted(ui.ctx())));
                for warning in &draft.current.warnings {ui.label(RichText::new(warning).size(12.0).color(theme::accent(ui.ctx())));}
                if !state.tuner.exclusive() {ui.add_enabled_ui(valid,|ui|settings_editor(ui,draft));}
            } else {ui.label("Read the amplifier to inspect its settings.");}
            if !(valid || state.tuner.enabled || state.tuner.pending) {ui.colored_label(theme::accent(ui.ctx()),"Reload current settings before preparing another change.");}
            if state.tuner.enabled || state.tuner.pending {ui.label(RichText::new("Stop the tuner before changing settings or using other amplifier operations.").color(theme::muted(ui.ctx())));}
            if state.operation.kind=="Change device settings" && state.operation.phase!=openfuse_device::OperationPhase::Idle {
                ui.add_space(10.0);ui.label(RichText::new(format!("Settings · {:?}",state.operation.phase)).color(theme::accent(ui.ctx())));ui.label(&state.operation.detail);
                if state.operation.busy() && button(ui,"controls-cancel","Cancel operation",true) {action=Some(Action::Cancel);}
            }
            if let Some(error)=&state.error {ui.colored_label(theme::accent(ui.ctx()),error);}
        });
        ui.add_space(12.0);
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "controls-review",
                "Review changes…",
                valid && controls.draft.as_ref().is_some_and(|d| !d.patch.is_empty()),
            ) {
                action = Some(Action::Review);
            }
            if button(
                ui,
                "controls-reload",
                "Reload settings",
                !state.operation_busy(),
            ) {
                action = Some(Action::Reload);
            }
            if button(ui, "controls-close", "Close", true) {
                action = Some(Action::Close);
            }
        });
    });
    action
}
fn settings_editor(ui: &mut egui::Ui, draft: &mut SettingsDraft) {
    let Ok(cap) = controls::capabilities(draft.pid) else {
        return;
    };
    if cap.usb_gain {
        let mut edit = draft.patch.usb_gain.is_some();
        let r = ui.add_enabled(
            draft.current.usb_gain.is_some(),
            egui::Checkbox::new(&mut edit, "USB recording level"),
        );
        remember(&r, "settings-usb-enable");
        if edit {
            let value = draft
                .patch
                .usb_gain
                .get_or_insert(u16::from(draft.current.usb_gain.unwrap_or(0)));
            let r = ui.add(
                egui::Slider::new(value, 0..=255)
                    .text("raw")
                    .show_value(true),
            );
            remember(&r, "settings-usb-value");
            ui.label(
                RichText::new(format!(
                    "{:.0}% · current {} / 255",
                    f32::from(*value) * 100.0 / 255.0,
                    raw(draft.current.usb_gain)
                ))
                .size(12.0)
                .color(theme::muted(ui.ctx())),
            );
        } else {
            draft.patch.usb_gain = None;
            ui.label(
                RichText::new(draft.current.usb_gain.map_or_else(
                    || "Not reported".into(),
                    |v| {
                        format!(
                            "Current: {:.0}% · raw {v} / 255",
                            f32::from(v) * 100.0 / 255.0
                        )
                    },
                ))
                .size(12.0)
                .color(theme::muted(ui.ctx())),
            );
        }
    }
    if cap.fx_loop {
        let mut edit = draft.patch.fx_loop_bypassed.is_some();
        let r = ui.add_enabled(
            draft.current.fx_loop_bypassed.is_some(),
            egui::Checkbox::new(&mut edit, "Change FX loop"),
        );
        remember(&r, "settings-loop-enable");
        if edit {
            let value = draft
                .patch
                .fx_loop_bypassed
                .get_or_insert(draft.current.fx_loop_bypassed.unwrap_or(false));
            let r = ui.checkbox(value, "FX loop active");
            remember(&r, "settings-loop-value");
        } else {
            draft.patch.fx_loop_bypassed = None;
            ui.label(
                RichText::new(draft.current.fx_loop_bypassed.map_or("Not reported", |v| {
                    if v {
                        "Current: Active"
                    } else {
                        "Current: Inactive"
                    }
                }))
                .size(12.0)
                .color(theme::muted(ui.ctx())),
            );
        }
    }
    if cap.global_volume {
        let mut edit = draft.patch.global_volume.is_some();
        let r = ui.add_enabled(
            draft.current.global_volume.is_some(),
            egui::Checkbox::new(&mut edit, "Global pedal volume range"),
        );
        remember(&r, "settings-volume-enable");
        if edit {
            if let Some(current) = draft.current.global_volume {
                let value = draft.patch.global_volume.get_or_insert(current);
                for (id, label, raw) in [
                    ("settings-volume-heel", "Heel", &mut value.heel),
                    ("settings-volume-toe", "Toe", &mut value.toe),
                ] {
                    let r = ui.add(egui::Slider::new(raw, 0..=65535).text(label));
                    remember(&r, id);
                }
                ui.label(RichText::new("Global range is shared by expression pedals. Device values use 256-step precision.").size(12.0).color(theme::muted(ui.ctx())));
            } else {
                ui.colored_label(
                    theme::accent(ui.ctx()),
                    "Volume endpoints were not reported; read current before editing.",
                );
            }
        } else {
            draft.patch.global_volume = None;
            ui.label(
                RichText::new(draft.current.global_volume.map_or_else(
                    || "Not reported".into(),
                    |v| {
                        format!(
                            "Current: heel {:.1}% / toe {:.1}%",
                            f32::from(v.heel) * 100.0 / 65535.0,
                            f32::from(v.toe) * 100.0 / 65535.0
                        )
                    },
                ))
                .size(12.0)
                .color(theme::muted(ui.ctx())),
            );
        }
    }
    for (supported, id, label, current, value) in [
        (
            cap.first_pedal,
            "first",
            "First pedal mode",
            draft.current.first_pedal_mode,
            &mut draft.patch.first_pedal_mode,
        ),
        (
            cap.second_pedal,
            "second",
            "Second pedal mode",
            draft.current.second_pedal_mode,
            &mut draft.patch.second_pedal_mode,
        ),
    ] {
        if !supported {
            continue;
        }
        let mut edit = value.is_some();
        let r = ui.add_enabled(current.is_some(), egui::Checkbox::new(&mut edit, label));
        remember(&r, &format!("settings-{id}-enable"));
        if edit {
            let selected = value.get_or_insert(current.unwrap_or(0));
            egui::ComboBox::from_id_salt(format!("settings-{id}-mode"))
                .selected_text(mode(*selected))
                .show_ui(ui, |ui| {
                    ui.selectable_value(selected, 0, "Capture");
                    ui.selectable_value(selected, 1, "Live");
                });
        } else {
            *value = None;
        }
        if let Some(current) = current {
            ui.label(
                RichText::new(format!("Current: {}", mode(current)))
                    .size(12.0)
                    .color(theme::muted(ui.ctx())),
            );
        }
    }
}
fn mode(value: u8) -> String {
    match value {
        0 => "Capture".into(),
        1 => "Live".into(),
        other => format!("Unknown ({other})"),
    }
}
