use crate::{
    Options,
    library_state::{FOLDER_KEY, LibraryState},
    session::Session,
    theme,
};
use openfuse_core::library::{LibraryEntry, PresetLibrary};
#[path = "app_archives.rs"]
mod archives;
#[path = "app_audio.rs"]
mod audio;
#[path = "app_audition.rs"]
mod audition;
#[path = "app_auxiliary.rs"]
mod auxiliary;
#[path = "app_bank.rs"]
mod bank;
#[path = "app_control_editor.rs"]
mod control_editor;
#[path = "app_control_editor_demo.rs"]
mod control_editor_demo;
#[path = "app_controls.rs"]
mod controls;
#[path = "app_conversion.rs"]
mod conversion;
#[path = "app_device.rs"]
mod device;
#[path = "app_device_read.rs"]
mod device_read;
#[path = "app_dialogs.rs"]
mod dialogs;
#[path = "app_documents.rs"]
mod documents;
#[path = "app_import.rs"]
mod import;
#[path = "app_library.rs"]
mod library;
#[path = "app_live.rs"]
mod live;
#[path = "app_media.rs"]
mod media;
#[path = "app_media_demo.rs"]
mod media_demo;
#[path = "app_shell.rs"]
mod shell;
#[path = "app_storage.rs"]
mod storage;
#[path = "app_storage_demo.rs"]
mod storage_demo;
#[path = "app_whole_restore.rs"]
mod whole_restore;
use eframe::egui::{self, RichText};
use std::path::PathBuf;

#[derive(Clone)]
enum Pending {
    Open(PathBuf),
    OpenIndexed(PresetLibrary, Box<LibraryEntry>),
    New(u32),
    Demo,
    OpenDevice(Box<crate::device_read::ReadDraft>),
    OpenArchive(openfuse_core::preset::PresetDocument, String),
    OpenConverted(openfuse_core::preset::PresetDocument, String),
    OpenStorage(openfuse_core::preset::PresetDocument, String),
    Close,
}

pub struct Editor {
    session: Option<Session>,
    tour: Option<crate::tour::Capture>,
    recent: Vec<PathBuf>,
    selected: String,
    inspector: crate::views::Inspector,
    appearance: egui::ThemePreference,
    pending: Option<Pending>,
    error: Option<String>,
    status: String,
    allow_close: bool,
    demo: bool,
    screenshot: Option<PathBuf>,
    wait_for_document_open: bool,
    frames: usize,
    new_dialog: bool,
    new_product: u32,
    library: LibraryState,
    archive_pending: Option<Vec<LibraryEntry>>,
    persist_library: bool,
    capture_requested: bool,
    capture_ready_since: Option<f64>,
    show_details: bool,
    show_pedals: bool,
    import_draft: Option<crate::module_import_view::ImportDraft>,
    last_import: Option<crate::module_import_view::ImportReceipt>,
    device: crate::device_state::DeviceState,
    device_dialog: bool,
    read_draft: Option<crate::device_read::ReadDraft>,
    device_origin: Option<crate::device_read::DeviceOrigin>,
    live_requested: bool,
    usb_open_demo: bool,
    usb_live_demo: bool,
    usb_audition_demo: bool,
    usb_audition_applied_demo: bool,
    audition_draft: Option<crate::audition_view::AuditionDraft>,
    operation_source: Option<String>,
    operation_seen: openfuse_device::OperationStatus,
    slot_preview_pending: bool,
    bank_ui: crate::bank_view::BankUi,
    controls_ui: crate::controls_view::ControlsUi,
    archives: crate::archive_state::ArchiveState,
    archive_ui: crate::archive_view::ArchiveUi,
    auxiliary_ui: crate::auxiliary_view::AuxiliaryUi,
    control_ui: crate::control_editor_view::State,
    conversion_draft: Option<crate::conversion_view::Draft>,
    media_ui: crate::media_view::State,
    audio_ui: crate::audio_state::State,
    storage_ui: crate::storage_state::State,
}

impl Editor {
    pub fn new(cc: &eframe::CreationContext<'_>, mut options: Options) -> Self {
        theme::install(&cc.egui_ctx);
        if options.appearance.is_none()
            && options.screenshot.is_none()
            && options.tour_capture.is_none()
        {
            options.appearance = cc
                .storage
                .and_then(|s| s.get_string(theme::APPEARANCE_KEY))
                .and_then(|s| match s.as_str() {
                    "light" => Some(egui::ThemePreference::Light),
                    "dark" => Some(egui::ThemePreference::Dark),
                    "system" => Some(egui::ThemePreference::System),
                    _ => None,
                });
        }
        let folder = cc
            .storage
            .and_then(|storage| storage.get_string(FOLDER_KEY))
            .map(PathBuf::from);
        let archive_folder = cc
            .storage
            .and_then(|storage| storage.get_string(crate::archive_state::FOLDER_KEY))
            .map(PathBuf::from);
        let control_folder = cc
            .storage
            .and_then(|storage| storage.get_string(crate::control_editor_state::FOLDER_KEY))
            .map(PathBuf::from);
        let media_folder = cc
            .storage
            .and_then(|storage| storage.get_string(crate::media_state::FOLDER_KEY))
            .map(PathBuf::from);
        let mut app = Self::from_options(options, &cc.egui_ctx, folder);
        app.audio_restore_preferences(cc.storage);
        if app.persist_library {
            app.storage_restore_preferences(cc.storage);
        }
        if app.media_ui.library.folder.is_none()
            && app.persist_library
            && let Some(folder) = media_folder
        {
            app.media_folder(folder);
        }
        if app.control_ui.library.folder.is_none()
            && app.persist_library
            && let Some(folder) = control_folder
        {
            app.control_ui.library.set_folder(folder);
        }
        if app.archives.folder.is_none()
            && app.persist_library
            && let Some(folder) = archive_folder
        {
            app.archives.set_folder(folder);
        }
        app
    }

    fn from_options(options: Options, ctx: &egui::Context, stored_folder: Option<PathBuf>) -> Self {
        let persist_library = options.screenshot.is_none() && options.tour_capture.is_none();
        if let Some(mode) = options.appearance {
            ctx.set_theme(mode);
        }
        let folder = options
            .library
            .clone()
            .or(if persist_library { stored_folder } else { None });
        let mut app = Self {
            appearance: ctx.options(|o| o.theme_preference),
            session: None,
            tour: options
                .tour_capture
                .map(|folder| crate::tour::Capture::new(folder, &options.tour_scene)),
            audio_ui: crate::audio_state::State::with_demo(options.audio_settings_demo),
            storage_ui: crate::storage_state::State::with_demo(options.storage_demo),
            media_ui: crate::media_view::State {
                demo: options.media_demo,
                ..Default::default()
            },
            conversion_draft: None,
            recent: Vec::new(),
            selected: String::new(),
            inspector: if options.details {
                crate::views::Inspector::Details
            } else if options.pedals {
                crate::views::Inspector::Expression
            } else {
                Default::default()
            },
            pending: None,
            error: None,
            status: "Ready to open a local preset".into(),
            allow_close: false,
            demo: false,
            screenshot: options.screenshot,
            wait_for_document_open: options.wait_for_document_open,
            frames: 0,
            new_dialog: false,
            new_product: 13,
            library: LibraryState::default(),
            archive_pending: None,
            persist_library,
            capture_requested: false,
            capture_ready_since: None,
            show_details: options.details,
            show_pedals: options.pedals,
            import_draft: None,
            last_import: None,
            device: crate::device_state::DeviceState::with_model(
                options.usb_demo,
                options.controls_demo > 0,
                options.auxiliary_demo == 4 || options.control_editor_demo == 3,
            ),
            device_dialog: options.usb_demo && !options.usb_open_demo,
            read_draft: None,
            device_origin: None,
            live_requested: false,
            usb_open_demo: options.usb_open_demo,
            usb_live_demo: options.usb_live_demo,
            usb_audition_demo: options.usb_audition_demo,
            usb_audition_applied_demo: options.usb_audition_applied_demo,
            audition_draft: None,
            operation_source: None,
            operation_seen: openfuse_device::OperationStatus::default(),
            slot_preview_pending: false,
            archives: Default::default(),
            control_ui: crate::control_editor_view::State {
                demo: options.control_editor_demo,
                ..Default::default()
            },
            auxiliary_ui: crate::auxiliary_view::AuxiliaryUi {
                demo: options.auxiliary_demo,
                ..Default::default()
            },
            archive_ui: crate::archive_view::ArchiveUi {
                demo: options.archive_demo,
                ..Default::default()
            },
            controls_ui: crate::controls_view::ControlsUi {
                demo: options.controls_demo,
                ..Default::default()
            },
            bank_ui: crate::bank_view::BankUi::new(
                options.usb_bank_demo,
                options.usb_bank_save_demo,
                options.usb_bank_restore_demo,
            ),
        };
        if let Some(path) = options.acceptance_session {
            match openfuse_device::trace::read_bounded(&path)
                .and_then(|text| openfuse_device::acceptance::Acceptance::parse(&text))
            {
                Ok(bundle) => app.device.acceptance = Some(bundle),
                Err(error) => app.device.acceptance_message = Some(error),
            }
            app.device_dialog = true;
        }
        if let Some(folder) = folder {
            app.library.set_folder(folder);
        }
        if let Some(folder) = options.media_library {
            app.media_folder(folder);
        }
        if let Some(root) = options.storage_root {
            app.storage_ui.set_root(
                root,
                if options.storage_internal {
                    openfuse_core::mounted_storage::Volume::Internal
                } else {
                    openfuse_core::mounted_storage::Volume::Sd
                },
            );
            app.storage_ui.open = true;
        }
        if let Some(folder) = options.control_library {
            app.control_ui.library.set_folder(folder);
        }
        if let Some(folder) = options.archive_folder {
            app.archives.set_folder(folder);
        }
        if let Some(path) = options.preset {
            app.perform(Pending::Open(path), ctx);
        } else if options.demo {
            app.perform(Pending::Demo, ctx);
        }
        if let Some(path) = options.import_source {
            match Session::open(&path) {
                Ok(source) => app.prepare_module_import(source.document, path),
                Err(error) => app.error = Some(error),
            }
        }
        if options.conversion_demo > 0 {
            app.conversion_demo(options.conversion_demo, ctx);
        }
        app
    }

    fn dirty(&self) -> bool {
        self.session.as_ref().is_some_and(Session::dirty)
    }

    fn request(&mut self, action: Pending, ctx: &egui::Context) {
        if self.dirty() {
            self.pending = Some(action);
        } else {
            self.perform(action, ctx);
        }
    }

    fn perform(&mut self, action: Pending, ctx: &egui::Context) {
        let mut origin = None;
        let (result, demo) = match action {
            Pending::Close => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            Pending::Open(path) => match Session::open(&path) {
                Ok(session) => (Ok(session), false),
                Err(error) => {
                    self.prepare_file_conversion(path);
                    if let Some(draft) = &mut self.conversion_draft {
                        draft.error = Some(format!(
                            "Normal import failed: {error}. Choose explicit repair options and review again."
                        ));
                    }
                    return;
                }
            },
            Pending::OpenIndexed(library, entry) => {
                let session = library
                    .read(&entry)
                    .map_err(|error| error.to_string())
                    .and_then(|document| document.to_xml().map_err(|error| error.to_string()))
                    .and_then(|xml| Session::parse(&xml, Some(entry.path)));
                (session, false)
            }
            Pending::OpenDevice(draft) => {
                let result = Session::from_device(&draft.document, draft.source_label());
                origin = Some(crate::device_read::DeviceOrigin::from(*draft));
                (result, false)
            }
            Pending::OpenConverted(document, label) => {
                (Session::converted(&document, label), false)
            }
            Pending::OpenStorage(document, label) => {
                (Session::from_storage(&document, label), false)
            }
            Pending::OpenArchive(document, label) => {
                (Session::from_device(&document, label), false)
            }
            Pending::New(product) => (Session::new_preset(product), false),
            Pending::Demo => (Session::example(), true),
        };
        match result {
            Ok(session) => {
                self.disable_live();
                self.device_origin = origin;
                self.read_draft = None;
                self.audition_draft = None;
                if let Some(path) = &session.path {
                    self.remember(path.clone());
                }
                self.selected = session
                    .document
                    .modules()
                    .first()
                    .map(|m| m.key.clone())
                    .unwrap_or_default();
                let has_path = session.path.is_some();
                self.session = Some(session);
                self.import_draft = None;
                self.conversion_draft = None;
                self.last_import = None;
                self.demo = demo;
                self.error = None;
                self.status = if self.device_origin.is_some() {
                    "Device snapshot opened as an unsaved local preset"
                } else if demo {
                    "Original catalog-based example loaded"
                } else if has_path {
                    "Preset loaded from disk"
                } else {
                    "New preset created"
                }
                .into();
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn remember(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(12);
    }

    fn choose_open(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("FUSE preset", &["fuse", "FUSE", "xml"])
            .pick_file()
        {
            self.request(Pending::Open(path), ctx);
        }
    }

    fn save_as(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let filename = session
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "preset.fuse".into());
        let Some(path) = rfd::FileDialog::new()
            .add_filter("FUSE preset", &["fuse"])
            .set_file_name(filename)
            .save_file()
        else {
            return false;
        };
        match session.save_as(&path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.error = None;
                self.remember(path);
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    fn save(&mut self) -> bool {
        let Some(path) = self.session.as_ref().and_then(|s| s.path.clone()) else {
            return self.save_as();
        };
        match self
            .session
            .as_mut()
            .expect("session with a path")
            .save_as(&path)
        {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    fn history(&mut self, undo: bool) {
        if let Some(session) = &mut self.session {
            let result = if undo { session.undo() } else { session.redo() };
            if let Err(error) = result {
                self.error = Some(error);
            }
        }
    }

    fn editor(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.show_details) {
            self.inspector = crate::views::Inspector::Details;
        }
        let show_pedals = std::mem::take(&mut self.show_pedals);
        if show_pedals {
            self.inspector = crate::views::Inspector::Expression;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::bg(ctx)).inner_margin(28))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("editor-content")
                    .show(ui, |ui| {
                        if self.device.has_connection() {
                            egui::CollapsingHeader::new(if self.device.simulated {
                                "Amplifier workspace · Simulator"
                            } else {
                                "Amplifier workspace"
                            })
                            .show(ui, |ui| {
                                if let Some(action) = crate::device_view::panel(ui, &self.device) {
                                    self.device_action(action);
                                }
                                self.working_panel(ui);
                                self.bank_panel(ui);
                                self.controls_panel(ui);
                                self.live_panel(ui);
                            });
                            ui.add_space(12.0);
                        }
                        let Some(session) = &mut self.session else {
                            ui.add_space(90.0);
                            theme::eyebrow(ui, "A NEW HOME FOR YOUR PRESETS");
                            ui.heading("Keep your sounds within reach.");
                            ui.label(
                                "Open a .FUSE file to inspect and edit its stored parameters.",
                            );
                            ui.add_space(20.0);
                            if ui.button("Open a preset…").clicked() {
                                self.choose_open(ctx);
                            }
                            return;
                        };
                        if let Some(receipt) = &self.last_import {
                            crate::module_import_view::receipt(ui, receipt);
                        }
                        crate::views::preset(
                            ui,
                            session,
                            &mut self.selected,
                            &mut self.error,
                            self.demo,
                            &mut self.inspector,
                        );
                    });
            });
    }
}

impl eframe::App for Editor {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Some(tour) = &mut self.tour {
            tour.input(ctx, input);
        }
    }

    fn persist_egui_memory(&self) -> bool {
        self.persist_library
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.storage_save_preferences(storage);
        if self.persist_library {
            self.audio_ui.save(storage);
            storage.set_string(
                theme::APPEARANCE_KEY,
                match self.appearance {
                    egui::ThemePreference::Light => "light",
                    egui::ThemePreference::Dark => "dark",
                    egui::ThemePreference::System => "system",
                }
                .into(),
            );
        }
        if self.persist_library
            && let Some(folder) = self
                .media_ui
                .library
                .folder
                .as_ref()
                .and_then(|p| p.to_str())
        {
            storage.set_string(crate::media_state::FOLDER_KEY, folder.to_owned());
        }
        if self.persist_library
            && let Some(folder) = self
                .control_ui
                .library
                .folder
                .as_ref()
                .and_then(|p| p.to_str())
        {
            storage.set_string(crate::control_editor_state::FOLDER_KEY, folder.to_owned());
        }
        if self.persist_library
            && let Some(folder) = self.archives.folder.as_ref().and_then(|p| p.to_str())
        {
            storage.set_string(crate::archive_state::FOLDER_KEY, folder.to_owned());
        }
        if self.persist_library
            && let Some(folder) = self.library.folder.as_ref().and_then(|p| p.to_str())
        {
            storage.set_string(FOLDER_KEY, folder.to_owned());
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frames += 1;
        self.document_events(ctx);
        self.audio_update(ctx);
        self.media_update(ctx);
        self.storage_update(ctx);
        self.library.poll();
        self.device.poll();
        self.operation_update();
        self.reconcile_live();
        self.bank_demo_update();
        self.controls_update();
        self.archives_update(ctx);
        self.auxiliary_update();
        self.control_update(ctx);
        if self.usb_open_demo && self.device.capture_ready() {
            self.usb_open_demo = false;
            self.prepare_device_read();
            if let Some(draft) = self.read_draft.take() {
                self.request(Pending::OpenDevice(Box::new(draft)), ctx);
                if self.usb_live_demo && self.device.simulated && self.device_origin.is_some() {
                    self.enable_live();
                }
            }
        }
        if self.usb_audition_demo && !self.usb_open_demo {
            self.usb_audition_demo = false;
            if self.device.simulated && self.device_origin.is_some() {
                self.prepare_audition();
                if self.usb_audition_applied_demo {
                    self.apply_audition();
                }
            }
        }
        if self.device.needs_polling() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(if self.library.busy() {
            100
        } else {
            1000
        }));
        if ctx.input(|i| i.viewport().close_requested())
            && (self.device.acceptance_dirty || self.device.recorder().progress().is_some())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.device_dialog = true;
            self.device.acceptance_message = Some("Before quitting, stop recording and export the session JSON, or discard the in-memory review.".into());
        } else if ctx.input(|i| i.viewport().close_requested()) && self.storage_ui.executing() {
            self.storage_cancel_close(ctx);
        } else if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.control_ui.dirty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.control_ui.pending = Some(crate::control_editor_view::Pending::CloseWindow);
            self.open_control_editor();
        } else if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.dirty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }
        if self.pending.is_none()
            && !self.new_dialog
            && self.archive_pending.is_none()
            && self.import_draft.is_none()
            && self.conversion_draft.is_none()
            && !self.audio_ui.open
            && !self.media_ui.open
            && !self.storage_ui.open
            && !self.device_dialog
            && self.read_draft.is_none()
            && self.audition_draft.is_none()
            && !self.bank_ui.modal_open()
            && !self.controls_ui.modal_open()
            && !self.archive_ui.modal_open()
            && !self.auxiliary_ui.modal_open()
        {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
                self.choose_open(ctx);
            }
            if ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::S,
                )
            }) {
                self.save_as();
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                self.save();
            }
            if ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                )
            }) {
                self.history(false);
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.history(true);
            }
        }
        self.toolbar(ctx);
        self.appearance = ctx.options(|o| o.theme_preference);
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(theme::panel(ctx)).inner_margin(8))
            .show(ctx, |ui| {
                if let Some(error) = &self.error {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(theme::danger(ui.ctx()), error);
                    });
                } else {
                    ui.label(
                        RichText::new(&self.status)
                            .size(12.0)
                            .color(theme::muted(ui.ctx())),
                    );
                }
            });
        self.sidebar(ctx);
        self.editor(ctx);
        self.confirm(ctx);
        if self.audio_ui.open {
            self.audio_dialog(ctx);
        } else if self.storage_ui.open {
            self.storage_dialog(ctx);
        } else if self.media_ui.open {
            self.media_dialog(ctx);
        } else if self.conversion_draft.is_some() {
            self.conversion_dialog(ctx);
        } else {
            self.new_preset_dialog(ctx);
            self.archive_dialog(ctx);
            self.import_dialog(ctx);
            self.device_dialog(ctx);
            self.read_dialog(ctx);
            self.audition_dialog(ctx);
            self.bank_dialog(ctx);
            self.controls_dialog(ctx);
            self.archives_dialog(ctx);
            self.auxiliary_dialog(ctx);
        }
        self.process_device_edits();
        self.capture(ctx);
        if let Some(tour) = &mut self.tour
            && tour.capture(
                ctx,
                self.frames > 30 && !self.library.busy(),
                self.session.as_ref(),
            )
        {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
#[path = "app_modal_tests.rs"]
mod modal_tests;
