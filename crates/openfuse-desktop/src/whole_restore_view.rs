//! Full-device replacement has its own frozen review, separate from mapped writes.
use crate::{audition_view::remember, controls_view::button, theme};
use eframe::egui::{self, RichText};
use openfuse_core::auxiliary::{Key, Snapshot, WholeRestore};
use openfuse_device::LiveBinding;

pub struct Preparation {
    pub receiver: std::sync::mpsc::Receiver<Result<Preview, String>>,
    pub cancelled: bool,
}
pub fn preparing(ctx: &egui::Context) -> bool {
    let mut cancel = false;
    crate::dialog::show(ctx, "whole-restore-preparing", 520.0, |ui| {
        theme::eyebrow(ui, "WHOLE-DEVICE RESTORE");
        ui.heading("Checking the complete archive");
        ui.add(egui::Label::new("Validating every preset, System field and control-bank entry against the frozen destination read. Nothing is being written.").wrap());
        ui.spinner();
        cancel = button(ui, "whole-cancel-preview", "Cancel review", true);
    });
    cancel
}

pub struct Preview {
    pub binding: LiveBinding,
    pub plan: WholeRestore,
    pub source_revision: u64,
    pub source_snapshot_fingerprint: String,
    pub source_label: String,
    pub destination_label: String,
    pub presets: Vec<Row>,
    pub banks: Vec<Row>,
    pub system: Vec<Row>,
    pub normalizations: Vec<Row>,
    pub normalization_page: usize,
    pub warnings: Vec<String>,
    pub confirmed: bool,
    pub tab: Tab,
}
pub struct Row {
    pub label: String,
    pub before: String,
    pub after: String,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Presets,
    Banks,
    System,
    Normalizations,
}
pub enum Action {
    Apply,
    Back,
}
impl Preview {
    pub fn rows(source: &Snapshot, destination: &Snapshot) -> (Vec<Row>, Vec<Row>, Vec<Row>) {
        let presets = source
            .presets()
            .iter()
            .map(|slot| Row {
                label: format!("Slot {}", slot.slot()),
                before: destination
                    .presets()
                    .iter()
                    .find(|p| p.slot() == slot.slot())
                    .map_or_else(|| "Missing".into(), |p| p.name()),
                after: slot.name(),
            })
            .collect();
        let banks = source
            .entries()
            .iter()
            .filter(|entry| entry.key() != Key::System)
            .map(|entry| Row {
                label: match entry.key() {
                    Key::Bank { control, slot } => format!("Bank {control} · slot {slot}"),
                    Key::System => unreachable!(),
                },
                before: destination
                    .entries()
                    .iter()
                    .find(|e| e.key() == entry.key())
                    .map_or_else(|| "Missing".into(), |e| e.label()),
                after: entry.label(),
            })
            .collect();
        let system = system_rows(source, destination);
        (presets, banks, system)
    }
}

fn system_rows(source: &Snapshot, destination: &Snapshot) -> Vec<Row> {
    let decode = |snapshot: &Snapshot| {
        let entry = snapshot
            .entries()
            .iter()
            .find(|entry| entry.key() == Key::System)?;
        openfuse_core::auxiliary::decode_system_xml(entry.xml()?).ok()
    };
    let (Some(source), Some(destination)) = (decode(source), decode(destination)) else {
        return vec![];
    };
    let mut rows = Vec::with_capacity(22);
    for (index, label) in [
        (32, "LCD contrast".into()),
        (33, "Memory lock".into()),
        (34, "2-button footswitch mode".into()),
        (35, "Expression volume heel".into()),
        (36, "Expression volume toe".into()),
        (37, "4-button footswitch mode".into()),
    ]
    .into_iter()
    .chain((0..16).map(|i| (16 + i, format!("QA {}", i + 1))))
    {
        rows.push(Row {
            label,
            before: destination.as_bytes()[index].to_string(),
            after: source.as_bytes()[index].to_string(),
        });
    }
    rows
}

pub fn dialog(
    ctx: &egui::Context,
    draft: &mut Preview,
    valid: bool,
    error: Option<&str>,
) -> Option<Action> {
    let mut action = None;
    crate::dialog::show(ctx, "whole-device-restore-preview", 860.0, |ui| {
        theme::eyebrow(ui, "WHOLE-DEVICE RESTORE · PERSISTENT");
        ui.heading("Replace all stored amplifier settings");
        ui.add(
            egui::Label::new(format!(
                "{} to {} · USB PID {:04X} · destination read {}",
                draft.source_label,
                draft.destination_label,
                draft.plan.pid(),
                draft.binding.generation
            ))
            .wrap(),
        );
        ui.label(format!(
            "{} presets · {} control-bank entries · System {}",
            draft.plan.preset_count(),
            draft.plan.bank_count(),
            if draft.plan.has_system() {
                "included"
            } else {
                "not present on this profile"
            }
        ));
        if !draft.normalizations.is_empty() {
            ui.add(egui::Label::new(RichText::new(format!("{} original-loader value normalizations are included. Review the Normalizations tab before confirming.",draft.normalizations.len())).color(theme::accent(ui.ctx()))).wrap());
        }
        ui.horizontal_wrapped(|ui| {
            for (tab, label, count) in [
                (Tab::Presets, "Presets", draft.presets.len()),
                (Tab::Banks, "Control banks", draft.banks.len()),
                (Tab::System, "System", draft.system.len()),
                (
                    Tab::Normalizations,
                    "Normalizations",
                    draft.normalizations.len(),
                ),
            ] {
                let response = ui.add_enabled(
                    count > 0,
                    egui::Button::new(format!("{label} ({count})")).selected(draft.tab == tab),
                );
                remember(&response, &format!("whole-tab-{label}"));
                if response.clicked() {
                    draft.tab = tab;
                }
            }
        });
        egui::ScrollArea::vertical().id_salt("whole-device-rows").max_height((ctx.screen_rect().height()-520.0).clamp(100.0,430.0)).show(ui, |ui| {
            if draft.tab==Tab::System {
                ui.add(egui::Label::new("Original numeric System values. QA and footswitch modes are shown without inferred assignments or enum labels.").wrap());
            }
            if draft.tab==Tab::Normalizations {
                ui.add(egui::Label::new("Original loader cascades and packed wire values can change stored bytes. Every allowed change is listed; unknown bytes are refused.").wrap());
                let pages=draft.normalizations.len().div_ceil(40).max(1);
                ui.horizontal(|ui| {
                    if button(ui,"whole-normalizations-prev","Previous",draft.normalization_page>0){draft.normalization_page-=1;}
                    ui.monospace(format!("Page {} / {pages}",draft.normalization_page+1));
                    if button(ui,"whole-normalizations-next","Next",draft.normalization_page+1<pages){draft.normalization_page+=1;}
                });
            }
            let rows = match draft.tab {Tab::Presets=>&draft.presets[..],Tab::Banks=>&draft.banks[..],Tab::System=>&draft.system[..],Tab::Normalizations=>{
                let start=(draft.normalization_page*40).min(draft.normalizations.len());
                &draft.normalizations[start..(start+40).min(draft.normalizations.len())]
            }};
            egui::Grid::new(("whole-overwrites",draft.tab as u8)).num_columns(3).striped(true).spacing([16.0,10.0]).max_col_width((ui.available_width()-40.0)/3.0).show(ui, |ui| {
                ui.strong(if draft.tab==Tab::Normalizations{"Source field"}else{"Destination"});ui.strong(if draft.tab==Tab::Normalizations{"Archive value"}else{"Currently stored"});ui.strong(if draft.tab==Tab::Normalizations{"Will be written"}else{"Will be replaced by"});ui.end_row();
                for row in rows {
                    ui.add(egui::Label::new(&row.label).wrap());
                    ui.add(egui::Label::new(&row.before).wrap());
                    ui.add(egui::Label::new(RichText::new(&row.after).color(if row.before==row.after{theme::muted(ui.ctx())}else{theme::accent(ui.ctx())})).wrap());ui.end_row();
                }
            });
            for warning in &draft.warnings {ui.add(egui::Label::new(RichText::new(warning).size(12.0).color(theme::muted(ui.ctx()))).wrap());}
        });
        ui.add(egui::Label::new("Every listed section is overwritten, including unchanged entries. The device is read again before writing and after completion. Cancel stops future writes; changes already sent remain. There is no automatic rollback.").wrap());
        ui.add(egui::Label::new("Your local preset and Undo history remain independent. Host metadata and media files are not sent to the amplifier.").wrap());
        let response = ui.checkbox(
            &mut draft.confirmed,
            "Overwrite every listed section, including disclosed value normalizations",
        );
        remember(&response, "whole-confirm");
        if !valid {
            ui.add(egui::Label::new(RichText::new("Source or destination changed. Go back, read the destination and review again.").color(theme::accent(ui.ctx()))).wrap());
        }
        if let Some(error) = error {
            ui.add(egui::Label::new(RichText::new(error).color(theme::accent(ui.ctx()))).wrap());
        }
        crate::dialog::footer(ui, |ui| {
            if button(
                ui,
                "whole-apply",
                "Restore whole device",
                valid && draft.confirmed,
            ) {
                action = Some(Action::Apply);
            }
            if button(ui, "whole-back", "Back", true) {
                action = Some(Action::Back);
            }
        });
    });
    action
}
