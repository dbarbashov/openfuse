//! Window chrome and navigation, separate from document and device actions.
use super::{Editor, Pending};
use crate::{session::Session, theme};
use eframe::egui::{self, RichText};

impl Editor {
    pub(super) fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::panel(ctx))
                    .inner_margin(egui::Margin::symmetric(20, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(40.0);
                    ui.add_sized(
                        [194.0, 40.0],
                        egui::Label::new(RichText::new("OpenFUSE").size(17.0).strong()),
                    );
                    ui.separator();
                    let title = self
                        .session
                        .as_ref()
                        .map(|s| s.document.metadata().name)
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Preset editor".into());
                    ui.add_sized(
                        [180.0, 32.0],
                        egui::Label::new(RichText::new(&title).strong()).truncate(),
                    );
                    if self.dirty() {
                        ui.label(RichText::new("Edited").small().color(theme::muted(ctx)));
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                        "{title} — OpenFUSE"
                    )));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save = ui.add_enabled(
                            self.session.is_some(),
                            egui::Button::new(RichText::new("Save").color(theme::bg(ctx)))
                                .fill(theme::accent(ctx))
                                .min_size(egui::vec2(64.0, 36.0)),
                        );
                        theme::remember(&save, "save");
                        if save.on_hover_text("Save preset · ⌘S / Ctrl+S").clicked() {
                            self.save();
                        }
                        ui.menu_button("More", |ui| {
                            if ui.button("New preset…").clicked() {
                                self.new_dialog = true;
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(self.session.is_some(), egui::Button::new("Save as…"))
                                .clicked()
                            {
                                self.save_as();
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    self.session.is_some(),
                                    egui::Button::new("Import modules…"),
                                )
                                .clicked()
                            {
                                self.choose_module_import();
                                ui.close_menu();
                            }
                            if ui.button("Convert / repair file…").clicked() {
                                self.choose_conversion();
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    self.session.is_some(),
                                    egui::Button::new("Convert current preset…"),
                                )
                                .clicked()
                            {
                                self.prepare_current_conversion();
                                ui.close_menu();
                            }
                            ui.separator();
                            ui.label(
                                RichText::new("Appearance")
                                    .small()
                                    .color(theme::muted(ui.ctx())),
                            );
                            let preference = ctx.options(|o| o.theme_preference);
                            for (mode, label) in [
                                (egui::ThemePreference::System, "System"),
                                (egui::ThemePreference::Light, "Light"),
                                (egui::ThemePreference::Dark, "Dark"),
                            ] {
                                if ui.selectable_label(preference == mode, label).clicked() {
                                    ctx.set_theme(mode);
                                    ui.close_menu();
                                }
                            }
                        });
                        let open = ui.button("Open…");
                        theme::remember(&open, "open");
                        if open
                            .on_hover_text("Open a local preset · ⌘O / Ctrl+O")
                            .clicked()
                        {
                            self.choose_open(ctx);
                        }
                        ui.separator();
                        let redo = ui.add_enabled(
                            self.session.as_ref().is_some_and(Session::can_redo),
                            egui::Button::new("Redo"),
                        );
                        theme::remember(&redo, "redo");
                        if redo.clicked() {
                            self.history(false);
                        }
                        let undo = ui.add_enabled(
                            self.session.as_ref().is_some_and(Session::can_undo),
                            egui::Button::new("Undo"),
                        );
                        theme::remember(&undo, "undo");
                        if undo.clicked() {
                            self.history(true);
                        }
                    });
                });
            });
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("library")
            .default_width(244.0)
            .width_range(228.0..=320.0)
            .frame(egui::Frame::new().fill(theme::panel(ctx)).inner_margin(14))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("library-panel")
                    .show(ui, |ui| {
                        group(ui, "Workspace");
                        if row(
                            ui,
                            "Presets",
                            Icon::Sliders,
                            !self.media_ui.open && !self.audio_ui.open,
                            "nav-presets",
                        )
                        .clicked()
                        {
                            self.inspector = crate::views::Inspector::Sound;
                        }
                        if row(
                            ui,
                            "Backing tracks",
                            Icon::Music,
                            self.media_ui.open,
                            "nav-tracks",
                        )
                        .clicked()
                        {
                            self.open_media();
                        }
                        if row(
                            ui,
                            "Archives",
                            Icon::Archive,
                            self.archive_ui.modal_open(),
                            "nav-archives",
                        )
                        .clicked()
                        {
                            self.open_archives();
                        }
                        group(ui, "Amplifier");
                        if row(
                            ui,
                            if self.device.simulated {
                                "USB simulator"
                            } else {
                                "Connection"
                            },
                            Icon::Connection,
                            self.device_dialog,
                            "nav-device",
                        )
                        .clicked()
                        {
                            self.device_action(crate::device_view::Action::Open);
                        }
                        if row(
                            ui,
                            "FX & voice banks",
                            Icon::Sliders,
                            self.control_ui.open,
                            "nav-banks",
                        )
                        .clicked()
                        {
                            self.open_control_editor();
                        }
                        if row(
                            ui,
                            "G-DEC storage",
                            Icon::Folder,
                            self.storage_ui.open,
                            "nav-storage",
                        )
                        .clicked()
                        {
                            self.open_storage();
                        }
                        group(ui, "Library");
                        let active = self.session.as_ref().and_then(|s| s.path.as_deref());
                        if let Some(action) = crate::library_view::panel(
                            ui,
                            &mut self.library,
                            active,
                            self.session.is_some(),
                        ) {
                            self.library_action(action, ctx);
                        }
                        if self.library.folder.is_none()
                            && row(
                                ui,
                                "Warm room · Example",
                                Icon::Preset,
                                self.demo,
                                "nav-example",
                            )
                            .clicked()
                        {
                            self.request(Pending::Demo, ctx);
                        }
                        if !self.recent.is_empty() {
                            egui::CollapsingHeader::new("Recent presets").show(ui, |ui| {
                                for path in self.recent.clone() {
                                    if ui
                                        .selectable_label(
                                            false,
                                            path.file_name().unwrap_or_default().to_string_lossy(),
                                        )
                                        .clicked()
                                    {
                                        self.request(Pending::Open(path), ctx);
                                    }
                                }
                            });
                        }
                        ui.add_space(16.0);
                        ui.separator();
                        if row(
                            ui,
                            "Audio settings",
                            Icon::Sliders,
                            self.audio_ui.open,
                            "nav-audio",
                        )
                        .clicked()
                        {
                            self.open_audio();
                        }
                        ui.horizontal(|ui| {
                            let color = if self.device.has_connection() {
                                theme::accent(ui.ctx())
                            } else {
                                theme::muted(ui.ctx())
                            };
                            let (r, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 16.0), egui::Sense::hover());
                            ui.painter().circle_filled(r.center(), 3.0, color);
                            ui.label(
                                RichText::new(if self.device.simulated {
                                    "Simulator · No hardware"
                                } else if self.device.has_connection() {
                                    "Amplifier connected"
                                } else {
                                    "Offline · Local editing"
                                })
                                .size(11.0)
                                .color(theme::muted(ui.ctx())),
                            );
                        });
                    });
            });
    }
}
fn group(ui: &mut egui::Ui, label: &str) {
    ui.add_space(12.0);
    ui.label(
        RichText::new(label)
            .size(11.0)
            .strong()
            .color(theme::muted(ui.ctx())),
    );
    ui.add_space(2.0);
}
#[derive(Clone, Copy)]
enum Icon {
    Sliders,
    Music,
    Archive,
    Connection,
    Folder,
    Preset,
}
fn row(ui: &mut egui::Ui, label: &str, icon: Icon, selected: bool, key: &str) -> egui::Response {
    let response = ui.add_sized(
        [ui.available_width(), 36.0],
        egui::Button::new("")
            .frame(false)
            .selected(selected)
            .corner_radius(6)
            .stroke(egui::Stroke::NONE)
            .frame(false),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    let p = ui.painter();
    p.text(
        response.rect.left_center() + egui::vec2(38.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        theme::ink(ui.ctx()),
    );
    let c = response.rect.left_center() + egui::vec2(16.0, 0.0);
    let color = if selected {
        theme::accent(ui.ctx())
    } else {
        theme::muted(ui.ctx())
    };
    let stroke = egui::Stroke::new(1.5_f32, color);
    let line = |a: egui::Vec2, b: egui::Vec2| p.line_segment([c + a, c + b], stroke);
    match icon {
        Icon::Sliders => {
            for (x, y) in [(-6.0, -3.0), (0.0, 4.0), (6.0, -1.0)] {
                line(egui::vec2(x, -7.0), egui::vec2(x, 7.0));
                p.circle_filled(c + egui::vec2(x, y), 2.5, color);
            }
        }
        Icon::Music => {
            line(egui::vec2(-3.0, 4.0), egui::vec2(-3.0, -6.0));
            line(egui::vec2(-3.0, -6.0), egui::vec2(6.0, -8.0));
            line(egui::vec2(6.0, -8.0), egui::vec2(6.0, 2.0));
            p.circle_filled(c + egui::vec2(-5.0, 5.0), 3.0, color);
            p.circle_filled(c + egui::vec2(4.0, 3.0), 3.0, color);
        }
        Icon::Connection => {
            p.circle_stroke(c, 7.0, stroke);
            line(egui::vec2(0.0, -4.0), egui::vec2(0.0, 4.0));
            line(egui::vec2(-4.0, 0.0), egui::vec2(4.0, 0.0));
        }
        Icon::Archive | Icon::Folder | Icon::Preset => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(16.0, 13.0)),
                2,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(egui::vec2(-5.0, -2.0), egui::vec2(5.0, -2.0));
            if matches!(icon, Icon::Preset) {
                line(egui::vec2(-5.0, 2.0), egui::vec2(3.0, 2.0));
            }
        }
    }
    theme::remember(&response, key);
    response
}
