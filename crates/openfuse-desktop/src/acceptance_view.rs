//! Recording and reviewing evidence never sends a device command.
use crate::{device_state::DeviceState, theme};
use eframe::egui::{self, RichText};
use openfuse_device::{
    acceptance::{Acceptance, CHECKS, Outcome},
    trace,
};

pub fn panel(ui: &mut egui::Ui, state: &mut DeviceState) {
    let recorder = state.recorder();
    egui::CollapsingHeader::new("Acceptance workbench · traces and owner checks")
        .id_salt("acceptance-workbench").default_open(state.acceptance.is_some() || state.acceptance_message.is_some()).show(ui, |ui| {
        ui.label("Start recording before Connect. Continue using the editor, then stop and export the session.");
        ui.label(RichText::new("Payloads can contain preset names. Serial numbers and device paths are omitted.").small().color(theme::muted(ui.ctx())));
        if let Some((count, dropped)) = recorder.progress() {
            ui.label(RichText::new(format!("RECORDING · {count} events · {dropped} dropped")).monospace().color(theme::accent(ui.ctx())));
            if ui.add_sized([210.0, 40.0], egui::Button::new("Stop and review trace")).clicked()
                && let Some(trace) = recorder.stop() {
                    match Acceptance::new(trace) {
                        Ok(bundle) => { state.acceptance = Some(bundle); state.acceptance_dirty = true; state.acceptance_message = None; }
                        Err(error) => state.acceptance_message = Some(error),
                    }
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!state.has_connection() && state.acceptance.is_none(), egui::Button::new("Start trace recording").min_size(egui::vec2(180.0, 40.0))).clicked() {
                    state.acceptance_message = recorder.start().err();
                }
                if ui.add_enabled(state.acceptance.is_none(), egui::Button::new("Open session JSON…").min_size(egui::vec2(170.0, 40.0))).clicked()
                    && let Some(path) = rfd::FileDialog::new().add_filter("Acceptance session", &["json"]).pick_file() {
                        match trace::read_bounded(&path).and_then(|text| Acceptance::parse(&text)) {
                            Ok(bundle) => { state.acceptance = Some(bundle); state.acceptance_dirty = false; state.acceptance_message = None; }
                            Err(error) => state.acceptance_message = Some(error),
                        }
                }
            });
            if state.has_connection() && state.acceptance.is_none() { ui.label("Disconnect to arm a complete trace, then connect explicitly."); }
        }
        let mut clear = false;
        if let Some(bundle) = &mut state.acceptance {
            ui.add_space(10.0);
            theme::eyebrow(ui, if bundle.trace.origin == trace::Origin::Simulator { "SIMULATOR · HARDWARE CHECKS PENDING" } else { "RECORDED TRANSPORT · OWNER CHECKS SEPARATE" });
            ui.label(format!("{} events · {} dropped · {} / {}", bundle.trace.entries.len(), bundle.trace.dropped_events, bundle.trace.os, bundle.trace.architecture));
            egui::CollapsingHeader::new("Recorded report bytes (last 128 events)").show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(200.0).id_salt("trace-bytes").show(ui, |ui| {
                    for (index, entry) in bundle.trace.entries.iter().enumerate().skip(bundle.trace.entries.len().saturating_sub(128)) {
                        let description = match &entry.event {
                            trace::Event::Tx { bytes, succeeded } => format!("TX {} {}", if *succeeded { "OK" } else { "FAILED" }, openfuse_core::hex::encode(bytes)),
                            trace::Event::Rx { bytes } => format!("RX {}", openfuse_core::hex::encode(bytes)),
                            event => format!("{event:?}"),
                        };
                        ui.label(RichText::new(format!("#{index} +{} ms {description}", entry.at_ms)).monospace().small());
                    }
                });
            });
            // Replay only on explicit request; never run O(trace) work each frame.
            if ui.button("Verify bootstrap replay offline").clicked() {
                state.acceptance_message = Some(match trace::replay_bootstrap(&bundle.trace) {
                    Ok(r) => format!("PASS · USB {:04X} · {} · {} TX / {} RX · {} later events not replayed. Hardware acceptance remains separate.", r.product_id, r.firmware, r.tx_verified, r.rx_consumed, r.events_not_replayed),
                    Err(e) => format!("Replay did not pass: {e}"),
                });
            }
            let mut changed = false;
            egui::CollapsingHeader::new("Model, setup and owner observations").show(ui, |ui| {
                changed |= field(ui, "Amplifier model / revision", &mut bundle.amplifier_label);
                changed |= field(ui, "OS version / USB setup / audio outputs", &mut bundle.environment);
                changed |= field(ui, "Observer", &mut bundle.observer);
                for (observation, (_, title, instruction)) in bundle.observations.iter_mut().zip(CHECKS) {
                    ui.separator();
                    ui.label(RichText::new(title).strong());
                    ui.label(RichText::new(instruction).small().color(theme::muted(ui.ctx())));
                    egui::ComboBox::from_id_salt((&observation.id, "outcome")).selected_text(observation.outcome.label()).show_ui(ui, |ui| {
                        for outcome in [Outcome::Pending, Outcome::Pass, Outcome::Fail, Outcome::NotApplicable] {
                            ui.add_enabled_ui(outcome != Outcome::Pass || bundle.trace.origin != trace::Origin::Simulator, |ui| {
                                changed |= ui.selectable_value(&mut observation.outcome, outcome, outcome.label()).changed();
                            });
                        }
                    });
                    changed |= ui.add(egui::TextEdit::multiline(&mut observation.notes).desired_rows(2).desired_width(f32::INFINITY).char_limit(1024)).changed();
                }
            });
            state.acceptance_dirty |= changed;
            ui.horizontal_wrapped(|ui| {
                for (label, extension) in [("Export session JSON…", "json"), ("Export acceptance HTML…", "html")] {
                    if ui.add_sized([215.0, 40.0], egui::Button::new(label)).clicked() {
                        let bytes = if extension == "json" { bundle.json() } else { bundle.html().map(String::into_bytes) };
                        match bytes {
                            Err(error) => state.acceptance_message = Some(error),
                            Ok(bytes) => if let Some(path) = rfd::FileDialog::new().add_filter("Acceptance", &[extension]).set_file_name(format!("openfuse-acceptance.{extension}")).save_file() {
                                state.acceptance_message = Some(match trace::write_new(&path, &bytes) { Ok(()) => { if extension == "json" { state.acceptance_dirty = false; } format!("Saved {}", path.display()) }, Err(error) => error });
                            },
                        }
                    }
                }
            });
            egui::CollapsingHeader::new("Discard this in-memory review").show(ui, |ui| {
                ui.label("Export first to keep the trace and notes. Saved files are unaffected.");
                clear = ui.button("Discard trace and notes").clicked();
            });
        }
        if clear { state.acceptance = None; state.acceptance_dirty = false; state.acceptance_message = None; }
        if let Some(message) = &state.acceptance_message { ui.label(RichText::new(message).color(theme::accent(ui.ctx()))); }
    });
}
fn field(ui: &mut egui::Ui, title: &str, value: &mut String) -> bool {
    ui.label(title);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .char_limit(1024),
    )
    .changed()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_and_arming_do_not_enumerate_or_connect() {
        let mut state = DeviceState::new(false);
        state.recorder().start().unwrap();
        let context = egui::Context::default();
        for _ in 0..3 {
            let _ = context.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panel(ui, &mut state));
            });
            state.poll();
        }
        assert!(!state.enumerated);
        assert!(!state.has_connection());
        assert_eq!(state.recorder().progress(), Some((0, 0)));
        assert!(state.recorder().stop().unwrap().entries.is_empty());
    }
    #[test]
    fn reviewing_a_saved_trace_does_not_connect_or_arm_recording() {
        let mut state = DeviceState::new(false);
        state.recorder().start().unwrap();
        state.acceptance = Some(Acceptance::new(state.recorder().stop().unwrap()).unwrap());
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| panel(ui, &mut state));
        });
        state.poll();
        assert!(!state.has_connection());
        assert!(!state.enumerated);
        assert!(state.recorder().progress().is_none());
        assert_eq!(state.acceptance.as_ref().unwrap().observations.len(), 8);
    }
}
