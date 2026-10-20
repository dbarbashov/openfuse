use super::Editor;
use crate::{media_state::Operation, media_view::Action};
impl Editor {
    pub(super) fn media_demo_update(&mut self) {
        let kind = self.media_ui.demo;
        if kind == 0 {
            return;
        }
        if self.media_ui.demo_folder.is_none() {
            let result = (|| {
                let folder = tempfile::tempdir().map_err(|e| e.to_string())?;
                let inputs = folder.path().join("originals");
                std::fs::create_dir(&inputs).map_err(|e| e.to_string())?;
                let audio = inputs.join("Evening groove.wav");
                std::fs::write(&audio, demo_wave()).map_err(|e| e.to_string())?;
                let other = inputs.join("Slow shuffle.wav");
                std::fs::write(&other, demo_wave()).map_err(|e| e.to_string())?;
                let midi = inputs.join("Practice pattern.mid");
                std::fs::write(
                    &midi,
                    b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk\0\0\0\x04\0\xff\x2f\0",
                )
                .map_err(|e| e.to_string())?;
                let library = folder.path().join("library");
                std::fs::create_dir(&library).map_err(|e| e.to_string())?;
                self.media_folder(library);
                self.media_ui.demo_folder = Some(folder);
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                self.media_ui.error = Some(error);
                self.media_ui.demo = 0;
            }
            return;
        }
        if self.media_ui.library.busy() {
            return;
        }
        if self.media_ui.library.entries.is_empty() {
            let input = self
                .media_ui
                .demo_folder
                .as_ref()
                .unwrap()
                .path()
                .join("originals");
            self.media_ui.library.start(Operation::Import(vec![
                input.join("Evening groove.wav"),
                input.join("Slow shuffle.wav"),
                input.join("Practice pattern.mid"),
            ]));
            return;
        }
        if self.media_ui.loaded.is_none() {
            let Some(entry) = self
                .media_ui
                .library
                .entries
                .iter()
                .find(|e| e.name == "Evening groove.wav")
                .cloned()
            else {
                return;
            };
            self.media_ui.library.selected = Some(entry.id.clone());
            self.media_ui.library.start(Operation::Load(entry));
            return;
        }
        if self.media_ui.player.snapshot().phase == openfuse_audio::Phase::Loading {
            return;
        }
        if kind == 4
            && self.media_ui.library.trash.is_empty()
            && let Some(entry) = self
                .media_ui
                .library
                .entries
                .iter()
                .find(|entry| entry.name == "Slow shuffle.wav")
                .cloned()
        {
            self.media_ui.library.start(Operation::Archive(entry));
            return;
        }
        self.media_ui.open = true;
        self.media_ui.show_band = matches!(kind, 2 | 3);
        if kind == 4 {
            self.media_ui.library.deleted = true;
            self.media_ui.library.selected = self
                .media_ui
                .library
                .trash
                .first()
                .map(|entry| entry.id.clone());
        }
        if kind == 2 {
            if let (Some(session), Some(entry)) = (&self.session, self.media_ui.loaded.clone()) {
                let source = session.document.to_xml().unwrap();
                let band = session.document.band().unwrap().unwrap_or_default();
                self.media_action(Action::Link(entry, source, band));
            }
        } else if kind == 3 {
            let mut session = crate::session::Session::new_preset(10).unwrap();
            let mut band = openfuse_core::preset::band::Band::default();
            band.set_filename("Practice pattern.mid").unwrap();
            band.location = 4;
            band.repeat = 1;
            band.tempo = 116;
            band.transpose = -2;
            band.count_in = 2;
            band.drum_solo = true;
            session.band(&band).unwrap();
            self.session = Some(session);
        }
        self.media_ui.demo = 0;
    }
}
pub(super) fn demo_wave() -> Vec<u8> {
    let rate = 22050u32;
    let frames = rate * 8;
    let length = frames * 2;
    let mut bytes = Vec::with_capacity(length as usize + 44);
    bytes.extend(b"RIFF");
    bytes.extend((length + 36).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(length.to_le_bytes());
    for i in 0..frames {
        let time = i as f32 / rate as f32;
        let envelope = (1.0 - (time * 2.0).fract()).powi(3);
        let sample =
            (0.15 * envelope * (std::f32::consts::TAU * 220.0 * time).sin() * i16::MAX as f32)
                as i16;
        bytes.extend(sample.to_le_bytes());
    }
    bytes
}
