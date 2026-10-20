use super::*;
use eframe::egui;

#[test]
fn modal_frames_keep_insets_and_fit_minimum_window_in_both_themes() {
    for mode in [egui::Theme::Light, egui::Theme::Dark] {
        for (id, case) in [
            ("audio-settings", 0),
            ("audition-preview", 1),
            ("backing-tracks", 2),
            ("archive-restore-preview", 3),
            ("device-controls", 4),
            ("preset-conversion", 5),
            ("gdec-storage-manager", 6),
            ("archive-catalog", 7),
            ("bank-browser", 8),
            ("bank-save-preview", 9),
            ("bank-restore-preview", 10),
            ("auxiliary-browser", 11),
            ("auxiliary-restore-preview", 12),
            ("whole-device-restore-preview", 13),
            ("control-editor", 14),
            ("gdec-storage-preview", 15),
        ] {
            let ctx = egui::Context::default();
            theme::install(&ctx);
            // Exercise the portable fallback, including on macOS hosts.
            ctx.set_fonts(egui::FontDefinitions::default());
            ctx.set_theme(mode);
            let mut options = Options {
                demo: true,
                ..Default::default()
            };
            match case {
                0 => options.audio_settings_demo = true,
                1 => {
                    options.usb_demo = true;
                    options.usb_open_demo = true;
                    options.usb_audition_demo = true;
                }
                2 => options.media_demo = 2,
                3 => {
                    options.usb_demo = true;
                    options.usb_open_demo = true;
                    options.archive_demo = 2;
                }
                4 => {
                    options.usb_demo = true;
                    options.usb_open_demo = true;
                    options.controls_demo = 3;
                }
                5 => options.conversion_demo = 1,
                6 => options.storage_demo = 1,
                7 => {
                    options.usb_demo = true;
                    options.usb_open_demo = true;
                    options.archive_demo = 1;
                }
                8..=10 => {
                    options.usb_demo = true;
                    options.usb_open_demo = true;
                    options.usb_bank_demo = true;
                    options.usb_bank_save_demo = case == 9;
                    options.usb_bank_restore_demo = case == 10;
                }
                11..=13 => {
                    options.usb_demo = true;
                    options.usb_open_demo = true;
                    options.auxiliary_demo = if case == 13 { 5 } else { case - 10 };
                }
                14 => {
                    options.usb_demo = true;
                    options.usb_open_demo = true;
                    options.control_editor_demo = 1;
                }
                15 => options.storage_demo = 2,
                _ => unreachable!(),
            }
            let mut app = Editor::from_options(options, &ctx, None);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(940.0, 680.0));
            let mut settled = 0;
            for _ in 0..1000 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ctx| {
                        eframe::App::update(&mut app, ctx, &mut eframe::Frame::_new_kittest());
                    },
                );
                if ctx
                    .data(|d| {
                        d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new(("dialog-bounds", id)))
                    })
                    .is_some()
                {
                    settled += 1;
                    if settled > 8 {
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let (outer, inner) = ctx
                .data(|d| {
                    d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new(("dialog-bounds", id)))
                })
                .unwrap_or_else(|| panic!("{id} did not open: {:?}", app.error));
            assert!(
                screen.contains_rect(outer),
                "{id} {mode:?}: modal {outer:?} escapes {screen:?}"
            );
            assert!(
                inner.left() - outer.left() >= 27.0 && outer.right() - inner.right() >= 27.0,
                "{id}: horizontal inset {outer:?} / {inner:?}"
            );
            assert!(
                inner.top() - outer.top() >= 23.0 && outer.bottom() - inner.bottom() >= 23.0,
                "{id}: vertical inset {outer:?} / {inner:?}"
            );
        }
    }
}
