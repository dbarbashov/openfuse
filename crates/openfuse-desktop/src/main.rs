mod acceptance_view;
mod app;
mod archive_state;
mod archive_view;
mod audio_state;
mod audio_view;
mod audition_view;
mod auxiliary_view;
mod bank_view;
mod control_editor_state;
mod control_editor_view;
mod controls_view;
mod conversion_view;
mod device_read;
mod device_state;
mod device_view;
mod dialog;
mod document_events;
mod library_state;
mod library_view;
mod media_state;
mod media_view;
mod module_import_view;
mod session;
mod storage_state;
mod storage_view;
mod theme;
mod tour;
mod views;
mod whole_restore_view;

use std::path::PathBuf;

#[derive(Default)]
pub struct Options {
    pub preset: Option<PathBuf>,
    pub appearance: Option<eframe::egui::ThemePreference>,
    pub tour_capture: Option<PathBuf>,
    pub tour_scene: String,
    pub acceptance_session: Option<PathBuf>,
    pub demo: bool,
    pub conversion_demo: u8,
    pub usb_demo: bool,
    pub usb_open_demo: bool,
    pub usb_live_demo: bool,
    pub usb_audition_demo: bool,
    pub usb_audition_applied_demo: bool,
    pub usb_bank_demo: bool,
    pub usb_bank_save_demo: bool,
    pub usb_bank_restore_demo: bool,
    pub controls_demo: u8,
    pub archive_demo: u8,
    pub auxiliary_demo: u8,
    pub control_editor_demo: u8,
    pub control_library: Option<PathBuf>,
    pub media_library: Option<PathBuf>,
    pub media_demo: u8,
    pub audio_settings_demo: bool,
    pub storage_demo: u8,
    pub storage_root: Option<PathBuf>,
    pub storage_internal: bool,
    pub archive_folder: Option<PathBuf>,
    pub details: bool,
    pub pedals: bool,
    pub import_source: Option<PathBuf>,
    pub library: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub screenshot_size: Option<[f32; 2]>,
    pub wait_for_document_open: bool,
}

fn main() -> eframe::Result {
    let mut options = Options::default();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--screenshot-size" {
            options.screenshot_size =
                args.next()
                    .and_then(|v| v.into_string().ok())
                    .and_then(|v| {
                        let (w, h) = v.split_once('x')?;
                        let (w, h) = (w.parse::<u16>().ok()?, h.parse::<u16>().ok()?);
                        ((940..=3840).contains(&w) && (680..=2160).contains(&h))
                            .then_some([f32::from(w), f32::from(h)])
                    });
            if options.screenshot_size.is_none() {
                eprintln!("--screenshot-size requires WIDTHxHEIGHT within 940x680–3840x2160");
                std::process::exit(2);
            }
        } else if arg == "--appearance" {
            options.appearance = match args.next().and_then(|s| s.into_string().ok()).as_deref() {
                Some("light") => Some(eframe::egui::ThemePreference::Light),
                Some("dark") => Some(eframe::egui::ThemePreference::Dark),
                Some("system") => Some(eframe::egui::ThemePreference::System),
                _ => {
                    eprintln!("--appearance requires light, dark or system");
                    std::process::exit(2);
                }
            };
        } else if arg == "--tour-capture" {
            options.tour_capture = args.next().map(PathBuf::from);
            if options.tour_capture.is_none() {
                eprintln!("--tour-capture requires a directory");
                std::process::exit(2);
            }
        } else if arg == "--tour-scene" {
            options.tour_scene = args
                .next()
                .and_then(|s| s.into_string().ok())
                .unwrap_or_default();
            if !matches!(
                options.tour_scene.as_str(),
                "edit" | "expression" | "library"
            ) {
                eprintln!("--tour-scene requires edit, expression or library");
                std::process::exit(2);
            }
        } else if arg == "--audio-settings-demo" {
            options.audio_settings_demo = true;
            options.demo = true;
        } else if arg == "--acceptance-session" {
            options.acceptance_session = args.next().map(PathBuf::from);
            if options.acceptance_session.is_none() {
                eprintln!("--acceptance-session requires a JSON path");
                std::process::exit(2);
            }
        } else if arg == "--conversion-demo"
            || arg == "--conversion-repair-demo"
            || arg == "--conversion-applied-demo"
        {
            options.demo = true;
            options.conversion_demo = if arg == "--conversion-repair-demo" {
                2
            } else if arg == "--conversion-applied-demo" {
                3
            } else {
                1
            };
        } else if arg == "--demo" {
            options.demo = true;
        } else if arg == "--usb-demo" {
            options.usb_demo = true;
        } else if arg == "--usb-open-demo" {
            options.usb_demo = true;
            options.usb_open_demo = true;
        } else if arg == "--usb-live-demo" {
            options.usb_demo = true;
            options.usb_open_demo = true;
            options.usb_live_demo = true;
        } else if arg == "--usb-audition-demo" {
            options.usb_demo = true;
            options.usb_open_demo = true;
            options.usb_audition_demo = true;
        } else if arg == "--usb-audition-applied-demo" {
            options.usb_demo = true;
            options.usb_open_demo = true;
            options.usb_audition_demo = true;
            options.usb_audition_applied_demo = true;
        } else if arg == "--usb-bank-demo"
            || arg == "--usb-bank-save-demo"
            || arg == "--usb-bank-restore-demo"
        {
            options.usb_demo = true;
            options.usb_open_demo = true;
            options.usb_bank_demo = true;
            options.usb_bank_save_demo = arg == "--usb-bank-save-demo";
            options.usb_bank_restore_demo = arg == "--usb-bank-restore-demo";
        } else if matches!(
            arg.to_str(),
            Some(
                "--usb-control-editor-demo"
                    | "--usb-control-write-demo"
                    | "--usb-voice-editor-demo"
                    | "--control-library-demo"
            )
        ) {
            options.usb_demo = true;
            options.usb_open_demo = true;
            options.control_editor_demo = match arg.to_str().unwrap() {
                "--usb-control-write-demo" => 2,
                "--usb-voice-editor-demo" => 3,
                "--control-library-demo" => 4,
                _ => 1,
            };
        } else if matches!(
            arg.to_str(),
            Some(
                "--gdec-storage-demo"
                    | "--gdec-storage-preview-demo"
                    | "--gdec-storage-copied-demo"
            )
        ) {
            options.demo = true;
            options.storage_demo = match arg.to_str().unwrap() {
                "--gdec-storage-preview-demo" => 2,
                "--gdec-storage-copied-demo" => 3,
                _ => 1,
            };
        } else if arg == "--gdec-storage-root" || arg == "--gdec-internal-root" {
            options.storage_internal = arg == "--gdec-internal-root";
            options.storage_root = args.next().map(PathBuf::from);
            if options.storage_root.is_none() {
                eprintln!("Storage root flag requires a folder path");
                std::process::exit(2);
            }
        } else if arg == "--media-demo"
            || arg == "--media-linked-demo"
            || arg == "--media-gdec-demo"
            || arg == "--media-archive-demo"
        {
            options.demo = true;
            options.media_demo = if arg == "--media-archive-demo" {
                4
            } else if arg == "--media-linked-demo" {
                2
            } else if arg == "--media-gdec-demo" {
                3
            } else {
                1
            };
        } else if arg == "--media-library" {
            options.media_library = args.next().map(PathBuf::from);
            if options.media_library.is_none() {
                eprintln!("--media-library requires a folder path");
                std::process::exit(2);
            }
        } else if arg == "--control-library" {
            options.control_library = args.next().map(PathBuf::from);
        } else if arg == "--usb-auxiliary-demo"
            || arg == "--usb-auxiliary-restore-demo"
            || arg == "--usb-auxiliary-applied-demo"
            || arg == "--usb-superchamp-demo"
            || arg == "--usb-whole-restore-demo"
            || arg == "--usb-whole-restore-applied-demo"
        {
            options.usb_demo = true;
            options.usb_open_demo = true;
            options.auxiliary_demo = if arg == "--usb-whole-restore-demo" {
                5
            } else if arg == "--usb-whole-restore-applied-demo" {
                6
            } else if arg == "--usb-auxiliary-restore-demo" {
                2
            } else if arg == "--usb-auxiliary-applied-demo" {
                3
            } else if arg == "--usb-superchamp-demo" {
                4
            } else {
                1
            };
        } else if arg == "--usb-controls-demo"
            || arg == "--usb-controls-applied-demo"
            || arg == "--usb-tuner-demo"
        {
            options.usb_demo = true;
            options.usb_open_demo = true;
            options.controls_demo = if arg == "--usb-tuner-demo" {
                3
            } else if arg == "--usb-controls-applied-demo" {
                2
            } else {
                1
            };
        } else if arg == "--usb-archive-demo"
            || arg == "--usb-archive-restore-demo"
            || arg == "--usb-archive-legacy-demo"
        {
            options.usb_demo = true;
            options.usb_open_demo = true;
            options.archive_demo = if arg == "--usb-archive-legacy-demo" {
                3
            } else if arg == "--usb-archive-restore-demo" {
                2
            } else {
                1
            };
        } else if arg == "--archive-folder" {
            options.archive_folder = args.next().map(PathBuf::from);
            if options.archive_folder.is_none() {
                eprintln!("--archive-folder requires a folder path");
                std::process::exit(2);
            }
        } else if arg == "--pedals" {
            options.pedals = true;
        } else if arg == "--import-source" {
            options.import_source = args.next().map(PathBuf::from);
            if options.import_source.is_none() {
                eprintln!("--import-source requires a preset path");
                std::process::exit(2);
            }
        } else if arg == "--details" {
            options.details = true;
        } else if arg == "--library" {
            options.library = args.next().map(PathBuf::from);
            if options.library.is_none() {
                eprintln!("--library requires a folder path");
                std::process::exit(2);
            }
        } else if arg == "--wait-for-document-open" {
            options.wait_for_document_open = true;
        } else if arg == "--screenshot" {
            options.screenshot = args.next().map(PathBuf::from);
            if options.screenshot.is_none() {
                eprintln!("--screenshot requires a PNG path");
                std::process::exit(2);
            }
        } else if arg == "--help" || arg == "-h" {
            println!(
                "openfuse-desktop [preset.fuse] [--acceptance-session SESSION.json] [--demo] [--conversion-demo|--conversion-repair-demo|--conversion-applied-demo] [--usb-demo|--usb-open-demo|--usb-live-demo|--usb-audition-demo|--usb-audition-applied-demo] [--usb-bank-demo|--usb-bank-save-demo|--usb-bank-restore-demo] [--usb-controls-demo|--usb-controls-applied-demo|--usb-tuner-demo] [--usb-archive-demo|--usb-archive-restore-demo|--usb-archive-legacy-demo] [--usb-auxiliary-demo|--usb-auxiliary-restore-demo|--usb-auxiliary-applied-demo|--usb-superchamp-demo] [--usb-whole-restore-demo|--usb-whole-restore-applied-demo] [--archive-folder path] [--control-library path] [--usb-control-editor-demo|--usb-control-write-demo|--usb-voice-editor-demo|--control-library-demo] [--details|--pedals] [--import-source preset.fuse] [--library folder] [--audio-settings-demo] [--media-library folder] [--media-demo|--media-linked-demo|--media-gdec-demo|--media-archive-demo] [--gdec-storage-root folder|--gdec-internal-root folder] [--gdec-storage-demo|--gdec-storage-preview-demo|--gdec-storage-copied-demo] [--screenshot output.png] [--wait-for-document-open]\n--screenshot-size WIDTHxHEIGHT sets an isolated capture viewport (requires --screenshot).\n--appearance light|dark|system selects the UI appearance. --tour-capture DIRECTORY --tour-scene edit|expression|library records 240 native offline interaction frames.\n--wait-for-document-open delays a requested screenshot until an OS or drop preset-open event is admitted; normal startup and playback are unchanged.\nStarts offline. USB enumeration and connection require explicit actions and the hid build feature.\n--demo loads an original catalog-based example. --usb-demo runs a simulated USB read session without HID access. --usb-open-demo also opens its decoded preset locally. --usb-live-demo additionally enables simulated live parameters. --usb-audition-demo opens a simulator audition preview. --usb-audition-applied-demo also applies it and waits for simulated readback. --usb-bank-demo reads simulator names; --usb-bank-save-demo previews a simulated slot overwrite; --usb-bank-restore-demo reads the simulated bank and previews restore. Bank demo flags never use HID. Controls and tuner demo flags use a simulated Floor amplifier and never use HID. Archive demos capture three simulated slots into a temporary catalog and optionally show a mapped restore preview. Auxiliary demos read simulated system/control banks; restore and applied variants preview or verify a mapped bank write. Whole-restore demos validate a complete simulated snapshot and preview or verify persistent System, presets and control banks. The Super Champ demo displays its voice and effect banks. All auxiliary and control-editor demos use temporary catalogs/libraries and never use HID."
            );
            return Ok(());
        } else if options.preset.is_none() && !arg.to_string_lossy().starts_with('-') {
            options.preset = Some(arg.into());
        } else {
            eprintln!("Unknown argument: {}", arg.to_string_lossy());
            std::process::exit(2);
        }
    }
    if let Err(error) = document_events::install() {
        eprintln!("Document opening: {error}");
    }
    if options.tour_capture.is_some() {
        // The recorder is deliberately limited to local example workflows.
        if options.preset.is_some()
            || options.usb_demo
            || options.acceptance_session.is_some()
            || options.import_source.is_some()
            || options.storage_root.is_some()
            || options.archive_folder.is_some()
        {
            eprintln!(
                "Tour capture only accepts offline examples; no amplifier, external preset or storage input"
            );
            std::process::exit(2);
        }
        options.demo = true;
    }
    if options.screenshot_size.is_some() && options.screenshot.is_none() {
        eprintln!("--screenshot-size requires --screenshot");
        std::process::exit(2);
    }
    eframe::run_native(
        "OpenFUSE — Preset Editor",
        eframe::NativeOptions {
            persist_window: options.screenshot.is_none() && options.tour_capture.is_none(),
            // Keep existing preferences across the project rename.
            persistence_path: eframe::storage_dir("Confuse — Preset Editor")
                .map(|directory| directory.join("app.ron")),
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size(options.screenshot_size.unwrap_or([1280.0, 860.0]))
                .with_min_inner_size([940.0, 680.0]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(app::Editor::new(cc, options)))),
    )
}
