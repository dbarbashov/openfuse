//! Whole-preset conversion is a frozen, local preview, independent of USB.
use crate::{controls_view::button, theme};
use eframe::egui::{self, RichText};
use openfuse_core::{
    conversion::{Options, Plan},
    model_catalog,
};

pub struct Draft {
    pub source: String,
    pub source_product: Option<u32>,
    pub label: String,
    pub current: bool,
    pub options: Options,
    pub plan: Option<Plan>,
    pub reviewed: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub page: usize,
}
impl Draft {
    pub fn new(source: String, label: String, current: bool) -> Self {
        let source_product = openfuse_core::preset::PresetDocument::parse(&source)
            .ok()
            .and_then(|d| d.product_id());
        let target = source_product
            .filter(|id| model_catalog::product(*id).is_some())
            .unwrap_or(13);
        Self {
            source,
            source_product,
            label,
            current,
            options: Options {
                target_product: target,
                repair_double_entities: false,
                normalize_legacy_delays: false,
                assume_missing_gdec: false,
            },
            plan: None,
            reviewed: false,
            error: None,
            notice: None,
            page: 0,
        }
    }
    pub fn invalidate(&mut self) {
        self.plan = None;
        self.reviewed = false;
        self.error = None;
        self.notice = None;
        self.page = 0;
    }
    pub fn review(&mut self) {
        self.invalidate();
        match Plan::preview(&self.source, self.options) {
            Ok(plan) => self.plan = Some(plan),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    pub fn result(&self) -> Result<openfuse_core::preset::PresetDocument, String> {
        if !self.reviewed {
            return Err("Review the conversion changes first".into());
        }
        self.plan
            .as_ref()
            .ok_or("Build a conversion preview first")?
            .apply(&self.source)
            .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy)]
pub enum Action {
    Cancel,
    Apply,
    Export,
}
fn family(id: Option<u32>) -> String {
    id.map(|id| {
        model_catalog::product(id)
            .map(|p| p.name.to_owned())
            .unwrap_or_else(|| format!("Unknown family {id}"))
    })
    .unwrap_or_else(|| "Legacy / unspecified".into())
}
pub fn show(ctx: &egui::Context, draft: &mut Draft, source_valid: bool) -> Option<Action> {
    let mut action = None;
    let height = ctx.screen_rect().height();
    crate::dialog::show(ctx, "preset-conversion", 800.0, |ui| {
        ui.label(
            RichText::new("LOCAL PRESET CONVERSION")
                .size(11.0)
                .color(theme::accent(ui.ctx())),
        );
        ui.heading("Make a compatible copy");
        ui.add(egui::Label::new(RichText::new(&draft.label).color(theme::muted(ui.ctx()))).wrap());
        egui::ScrollArea::vertical().id_salt("conversion-content").max_height((height - 300.0).max(180.0)).show(ui, |ui| {
            settings(ui, draft);
            if button(ui, "conversion-review", "Review conversion", true) { draft.review(); }
            ui.separator();
            report(ui, draft, (height - 650.0).clamp(100.0, 240.0));
            if !source_valid { ui.add(egui::Label::new(RichText::new("The editor changed since this preview opened. Cancel and create a fresh preview.").color(theme::accent(ui.ctx()))).wrap()); }
            if let Some(error) = &draft.error { ui.add(egui::Label::new(RichText::new(error).color(theme::danger(ui.ctx()))).wrap()); }
            if let Some(notice) = &draft.notice { ui.add(egui::Label::new(notice).wrap()); }
        });
        ui.add_space(8.0);
        crate::dialog::footer(ui, |ui| {
            let ready = draft.plan.is_some() && draft.reviewed && source_valid;
            if button(ui, "conversion-apply", "Open converted copy", ready) {
                action = Some(Action::Apply);
            }
            if button(ui, "conversion-export", "Export new file…", ready) {
                action = Some(Action::Export);
            }
            if button(ui, "conversion-cancel", "Cancel", true) {
                action = Some(Action::Cancel);
            }
        });
        ui.add(egui::Label::new(RichText::new(if draft.current {
            "Opening keeps the source in Undo and starts an unsaved copy. Live editing is turned off."
        } else {
            "Opening creates an unsaved local preset. Existing editor changes are protected by the save prompt."
        }).small().color(theme::muted(ui.ctx()))).wrap());
    });
    action
}
fn settings(ui: &mut egui::Ui, draft: &mut Draft) {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Target family");
        egui::ComboBox::from_id_salt("conversion-family")
            .width(270.0)
            .selected_text(family(Some(draft.options.target_product)))
            .show_ui(ui, |ui| {
                for product in model_catalog::products() {
                    let supported = if draft.options.assume_missing_gdec {
                        product.id == 10
                    } else {
                        draft.source_product.is_none_or(|source| {
                            openfuse_core::conversion::supported_targets(source)
                                .contains(&product.id)
                        })
                    };
                    if supported {
                        changed |= ui
                            .selectable_value(
                                &mut draft.options.target_product,
                                product.id,
                                product.name,
                            )
                            .changed();
                    }
                }
            });
    });
    let repair = ui.checkbox(
        &mut draft.options.repair_double_entities,
        "Repair legacy double-encoded XML entities",
    );
    crate::audition_view::remember(&repair, "conversion-repair");
    changed |= repair.changed();
    changed |= ui
        .checkbox(
            &mut draft.options.normalize_legacy_delays,
            "Normalize legacy G-DEC delay times",
        )
        .changed();
    let assume = ui.checkbox(
        &mut draft.options.assume_missing_gdec,
        "Interpret missing or invalid ProductId as G-DEC 3",
    );
    crate::audition_view::remember(&assume, "conversion-assume-gdec");
    if assume.changed() {
        changed = true;
        if draft.options.assume_missing_gdec {
            draft.options.target_product = 10;
        }
    }
    if changed {
        draft.invalidate();
    }
    ui.add(egui::Label::new("Repairs are opt-in. Review substitutions, normalized parameters and losses before opening the copy. The original file stays intact.").wrap());
}
fn report(ui: &mut egui::Ui, draft: &mut Draft, height: f32) {
    let Some(plan) = &draft.plan else {
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Choose a target and build the preview. Your editor has not changed.",
                )
                .color(theme::muted(ui.ctx())),
            )
            .wrap(),
        );
        return;
    };
    use openfuse_core::conversion::ChangeKind;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "{} to {}",
                family(plan.source_product()),
                family(Some(plan.target_product()))
            ))
            .strong(),
        );
        ui.label(
            RichText::new(format!("{} changes", plan.changes().len()))
                .color(theme::muted(ui.ctx())),
        );
    });
    let count = |kind| {
        plan.changes()
            .iter()
            .filter(|change| change.kind == kind)
            .count()
    };
    ui.add(
        egui::Label::new(
            RichText::new(format!(
                "Substitutions: {} · Losses: {} · Normalizations: {} · Repairs: {}",
                count(ChangeKind::Replacement),
                count(ChangeKind::Loss),
                count(ChangeKind::Normalization),
                count(ChangeKind::Repair)
            ))
            .color(theme::muted(ui.ctx())),
        )
        .wrap(),
    );
    let mut changes: Vec<_> = plan.changes().iter().collect();
    changes.sort_by_key(|change| match change.kind {
        ChangeKind::Replacement => 0,
        ChangeKind::Repair => 1,
        ChangeKind::Loss => 2,
        ChangeKind::Normalization => 3,
        ChangeKind::Metadata => 4,
    });
    let pages = plan.changes().len().div_ceil(40).max(1);
    draft.page = draft.page.min(pages - 1);
    if pages > 1 {
        ui.horizontal(|ui| {
            if button(ui, "conversion-prev", "Previous changes", draft.page > 0) {
                draft.page -= 1;
            }
            ui.label(format!(
                "Page {} of {pages} · 40 changes per page",
                draft.page + 1
            ));
            if button(
                ui,
                "conversion-next",
                "Next changes",
                draft.page + 1 < pages,
            ) {
                draft.page += 1;
            }
        });
    }
    egui::ScrollArea::vertical()
        .id_salt(("conversion-changes", draft.page))
        .max_height(height)
        .show(ui, |ui| {
            if plan.changes().is_empty() {
                ui.label("No conversion changes required.");
            }
            for change in changes.iter().skip(draft.page * 40).take(40) {
                egui::Frame::new()
                    .fill(theme::panel(ui.ctx()))
                    .inner_margin(10)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("{:?} · {}", change.kind, change.path))
                                    .strong(),
                            )
                            .wrap(),
                        );
                        ui.add(egui::Label::new(&change.message).wrap());
                        ui.add(
                            egui::Label::new(format!(
                                "Before: {}",
                                change_value(change, plan.source_product(), true)
                            ))
                            .wrap(),
                        );
                        ui.add(
                            egui::Label::new(format!(
                                "After: {}",
                                change_value(change, Some(plan.target_product()), false)
                            ))
                            .wrap(),
                        );
                    });
                ui.add_space(4.0);
            }
        });
    let response = ui.checkbox(
        &mut draft.reviewed,
        "I reviewed these changes and any losses",
    );
    crate::audition_view::remember(&response, "conversion-ack");
}

fn change_value(
    change: &openfuse_core::conversion::Change,
    product: Option<u32>,
    before: bool,
) -> String {
    let value = if before {
        change.before.as_deref().unwrap_or("absent")
    } else {
        change.after.as_deref().unwrap_or("removed")
    };
    if change.path.ends_with("/Module/@ID") {
        let category = change
            .path
            .split('/')
            .next()
            .and_then(model_catalog::ModuleCategory::from_key);
        if let (Some(product), Some(category), Ok(id)) = (product, category, value.parse::<u16>())
            && let Some(model) = model_catalog::model(product, category, id)
        {
            return format!("{} (ID {id})", model.name);
        }
    }
    bounded_value(value)
}

fn bounded_value(value: &str) -> String {
    const LIMIT: usize = 600;
    if let Some((end, _)) = value.char_indices().nth(LIMIT) {
        format!(
            "{}… [preview shortened; {} bytes total, full XML retained in the source]",
            &value[..end],
            value.len()
        )
    } else {
        value.to_owned()
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn large_loss_values_have_a_bounded_unicode_safe_explicit_preview() {
        let change = openfuse_core::conversion::Change {
            kind: openfuse_core::conversion::ChangeKind::Replacement,
            path: "Amplifier/Module/@ID".into(),
            before: Some("249".into()),
            after: Some("124".into()),
            message: String::new(),
        };
        assert!(super::change_value(&change, Some(13), true).contains("Thrift"));
        assert!(super::change_value(&change, Some(1), false).contains("Champ"));
        let source = "𐐷".repeat(400_000);
        let preview = super::bounded_value(&source);
        assert!(preview.len() < 2600);
        assert!(preview.contains("1600000 bytes total"));
        assert!(preview.contains("full XML retained"));
        assert_eq!(super::bounded_value("unchanged"), "unchanged");
    }
}
