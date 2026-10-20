//! Native USB controls return explicit actions; rendering never sends a command.
use crate::{device_state::DeviceState, theme};
use eframe::egui::{self, RichText};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Close,
    Refresh,
    Connect,
    Disconnect,
    ReadCurrent,
    OpenRead,
    OpenCapture,
    ExportCapture,
    SelectSlot(u16),
}

pub fn panel(ui: &mut egui::Ui, state: &DeviceState) -> Option<Action> {
    let mut action = None;
    theme::eyebrow(
        ui,
        if state.simulated {
            "SIMULATED AMPLIFIER"
        } else {
            "AMPLIFIER"
        },
    );
    ui.label(
        RichText::new(state.phase())
            .strong()
            .color(theme::accent(ui.ctx())),
    );
    if let Some(version) = &state.status.version {
        ui.label(
            RichText::new(format!("Firmware {version}"))
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
    }
    if let Some(snapshot) = &state.status.snapshot {
        ui.label(snapshot.name.as_deref().unwrap_or("Last read preset"));
        ui.label(
            RichText::new(format!("{} modules received", snapshot.modules))
                .size(12.0)
                .color(theme::muted(ui.ctx())),
        );
    } else {
        ui.label(
            RichText::new(if state.has_connection() {
                "Reading amplifier state…"
            } else {
                "Connect to inspect the amplifier’s current preset."
            })
            .size(12.0)
            .color(theme::muted(ui.ctx())),
        );
    }
    let open = ui.add_sized(
        [ui.available_width(), 42.0],
        egui::Button::new("USB devices…"),
    );
    remember(&open, "open");
    if open.clicked() {
        action = Some(Action::Open);
    }
    if state.status.can_sync {
        let read = ui.add_sized(
            [ui.available_width(), 42.0],
            egui::Button::new("Read current preset"),
        );
        remember(&read, "sidebar-read");
        if read.clicked() {
            action = Some(Action::ReadCurrent);
        }
    }
    if state.capture.is_some() {
        let open = ui.add_sized(
            [ui.available_width(), 42.0],
            egui::Button::new("Open read preset…"),
        );
        remember(&open, "sidebar-open-read");
        if open.clicked() {
            action = Some(Action::OpenRead);
        }
    }
    if let Some((events, dropped)) = state.recorder().progress() {
        ui.label(
            RichText::new(format!(
                "Trace recording · {events} events · {dropped} dropped"
            ))
            .small()
            .color(theme::accent(ui.ctx())),
        );
    }
    if state.error.is_some() {
        ui.label(
            RichText::new("Connection needs attention")
                .size(12.0)
                .color(theme::accent(ui.ctx())),
        );
    }
    action
}

pub fn dialog(ctx: &egui::Context, state: &mut DeviceState) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "usb-devices", 660.0, |ui| {
        egui::ScrollArea::vertical().id_salt("usb-dialog-content").max_height((ctx.screen_rect().height() - 330.0).clamp(220.0, 530.0)).show(ui, |ui| {
        theme::eyebrow(ui, if state.simulated { "SIMULATED DEVICE · NO HID ACCESS" } else { "AMPLIFIER CONNECTION" });
        ui.heading(if state.simulated { "USB simulator" } else { "USB devices" });
        ui.label("Read the amplifier’s state alongside your local preset editor.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let refresh = ui.add_enabled(!state.enumerating, egui::Button::new("Refresh devices"));
            remember(&refresh, "refresh");
            if refresh.clicked() { action = Some(Action::Refresh); }
            if state.enumerating { ui.spinner(); ui.label("Looking for amplifiers…"); }
            else if state.enumerated { ui.label(RichText::new(format!("{} found", state.devices.len())).color(theme::muted(ui.ctx()))); }
        });
        if state.devices.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(if state.enumerated { "No amplifiers found. Connect the USB cable, then refresh." } else { "Refresh to list connected amplifiers." }).color(theme::muted(ui.ctx())));
        } else {
            egui::ScrollArea::vertical().id_salt("usb-device-list").max_height(220.0).show(ui, |ui| {
                for device in &state.devices {
                    let selected = state.selected.as_ref() == Some(&device.id);
                    egui::Frame::new().fill(if selected { theme::accent(ui.ctx()).gamma_multiply(0.12) } else { theme::panel(ui.ctx()) }).corner_radius(10).inner_margin(10).show(ui, |ui| {
                        let choice = ui.add_sized([ui.available_width(), 40.0], egui::Button::new(&device.name).selected(selected).frame(false));
                        remember(&choice, &format!("device-{}", device.id));
                        if choice.clicked() { state.selected = Some(device.id.clone()); }
                        ui.label(RichText::new(format!("USB {:04X} · interface {}{}", device.product_id, device.interface_number,
                            device.serial.as_ref().map(|serial| format!(" · serial {serial}")).unwrap_or_default())).size(12.0).color(theme::muted(ui.ctx())));
                        if let Some(reason) = &device.unavailable_reason { ui.label(RichText::new(reason).size(12.0).color(theme::accent(ui.ctx()))); }
                    });
                }
            });
        }
        ui.add_space(12.0);

        ui.add_space(12.0);
        egui::Frame::new().fill(theme::panel(ui.ctx())).corner_radius(12).inner_margin(16).show(ui, |ui| {
            ui.set_width((ui.available_width()-32.0).max(240.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(state.phase()).size(18.0).strong().color(theme::accent(ui.ctx())));
                if state.status.reconnecting { ui.spinner(); }
            });
            if !state.status.detail.is_empty() { ui.label(&state.status.detail); }
            if let Some(version) = &state.status.version { ui.label(RichText::new(format!("Firmware {version}")).monospace().size(13.0)); }
            if let Some(snapshot) = &state.status.snapshot {
                ui.add_space(8.0);
                theme::eyebrow(ui, "LAST READ PRESET · SNAPSHOT");
                ui.label(RichText::new(snapshot.name.as_deref().unwrap_or("Unnamed device preset")).strong());
                let slot = snapshot.slot.map(|slot| format!("Slot {slot} · ")).unwrap_or_default();
                ui.label(RichText::new(format!("{slot}{} modules", snapshot.modules)).monospace().size(13.0));
                egui::CollapsingHeader::new("Read details").show(ui, |ui| {
                    ui.label(RichText::new(format!("Generation {} · {} reports", snapshot.generation, snapshot.reports)).monospace().size(12.0));
                });
            }
            if let Some(capture) = &state.capture {
                match openfuse_core::audition::slot_count(capture.device.product_id) {
                    Ok(count) if count > 0 => {
                        ui.add_space(10.0);
                        theme::eyebrow(ui, "SELECT STORED AMPLIFIER SLOT");
                        ui.label("Loads an amplifier sound into its working preset, then reads it back. Local edits stay here.");
                        ui.horizontal(|ui| {
                            state.selected_slot = state.selected_slot.min(count - 1);
                            ui.label("Slot index");
                            ui.add(egui::DragValue::new(&mut state.selected_slot).range(0..=count-1));
                            ui.label(format!("0–{}", count-1));
                            let select = ui.add_enabled(state.status.can_sync && !state.operation_busy(), egui::Button::new("Select and read"));
                            remember(&select, "select-slot");
                            if select.clicked() { action = Some(Action::SelectSlot(state.selected_slot)); }
                        });
                    }
                    Ok(_) => { ui.label(RichText::new("This amplifier has no selectable stored slots.").color(theme::muted(ui.ctx()))); }
                    Err(error) => { ui.label(RichText::new(error.to_string()).color(theme::muted(ui.ctx()))); }
                }
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Snapshot summary. Your local edits and undo history stay in the editor.").size(12.0).color(theme::muted(ui.ctx())));
        });
        crate::acceptance_view::panel(ui, state);
        if let Some(error) = &state.error {
            ui.add_space(6.0);
            ui.label(RichText::new(error).color(theme::danger(ui.ctx())));
        }
        });
        ui.add_space(12.0);
        let open = ui.add_enabled(
            state.capture.is_some(),
            egui::Button::new("Open read preset…"),
        );
        remember(&open, "open-read");
        if open.clicked() {
            action = Some(Action::OpenRead);
        }
        egui::CollapsingHeader::new("Capture files").show(ui, |ui| {
            if ui.button("Open capture file…").clicked() {
                action = Some(Action::OpenCapture);
            }
            if ui
                .add_enabled(
                    state.capture.is_some(),
                    egui::Button::new("Export raw capture…"),
                )
                .clicked()
            {
                action = Some(Action::ExportCapture);
            }
        });
        crate::dialog::footer(ui, |ui| {
            for (title, name, enabled, next) in [
                ("Connect", "connect", state.can_connect(), Action::Connect),
                (
                    "Disconnect",
                    "disconnect",
                    state.has_connection(),
                    Action::Disconnect,
                ),
                (
                    "Read current",
                    "read",
                    state.status.can_sync,
                    Action::ReadCurrent,
                ),
            ] {
                let button = ui.add_enabled(
                    enabled,
                    egui::Button::new(title).min_size(egui::vec2(80.0, 40.0)),
                );
                remember(&button, name);
                if button.clicked() {
                    action = Some(next);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.add_sized([76.0, 40.0], egui::Button::new("Close"));
                remember(&close, "close");
                if close.clicked() {
                    action = Some(Action::Close);
                }
            });
        });
    });
    action
}
fn remember(response: &egui::Response, name: &str) {
    #[cfg(test)]
    response.ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new(("usb-control", name)),
            (response.rect, response.enabled()),
        )
    });
    #[cfg(not(test))]
    let _ = (response, name);
}
