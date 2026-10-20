use super::Editor;
use crate::{session::Session, storage_state::Operation};
use openfuse_core::mounted_storage::{CollisionPolicy, Volume};
impl Editor {
    pub(super) fn storage_demo_update(&mut self) {
        if self.storage_ui.demo == 0 {
            return;
        }
        if self.storage_ui.demo_directory.is_none() {
            let result = (|| {
                let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
                let root = directory.path().join("G-DEC SD workspace");
                for folder in ["Audio", "Midi", "Phrase", "Presets"] {
                    std::fs::create_dir_all(root.join(folder)).map_err(|e| e.to_string())?;
                }
                let local = directory.path().join("Local tracks");
                std::fs::create_dir(&local).map_err(|e| e.to_string())?;
                std::fs::write(
                    root.join("Audio/Evening groove.wav"),
                    super::media_demo::demo_wave(),
                )
                .map_err(|e| e.to_string())?;
                std::fs::write(
                    local.join("Evening groove.wav"),
                    super::media_demo::demo_wave(),
                )
                .map_err(|e| e.to_string())?;
                std::fs::write(
                    root.join("Phrase/Practice take.wav"),
                    super::media_demo::demo_wave(),
                )
                .map_err(|e| e.to_string())?;
                std::fs::write(
                    root.join("Midi/Practice pattern.mid"),
                    b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk\0\0\0\x04\0\xff\x2f\0",
                )
                .map_err(|e| e.to_string())?;
                let mut preset = Session::new_preset(10)?.document;
                let mut metadata = preset.metadata();
                metadata.name = "Evening practice".into();
                preset.set_metadata(&metadata).map_err(|e| e.to_string())?;
                std::fs::write(
                    root.join("Presets/Evening practice.fuse"),
                    preset.to_xml().map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                std::fs::write(
                    local.join("New practice.fuse"),
                    preset.to_xml().map_err(|e| e.to_string())?.replacen(
                        "<Preset ",
                        "<Preset amplifier=\"G-DEC 3.0\" ",
                        1,
                    ),
                )
                .map_err(|e| e.to_string())?;
                self.storage_ui.set_root(root, Volume::Sd);
                self.storage_ui.demo_directory = Some(directory);
                self.storage_ui.open = true;
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                self.storage_ui.error = Some(error);
                self.storage_ui.demo = 0;
            }
            return;
        }
        if self.storage_ui.busy() {
            return;
        }
        match self.storage_ui.demo_stage {
            0 => {
                if self.storage_ui.demo == 1 {
                    self.storage_ui.demo = 0;
                    self.frames = 0;
                    return;
                }
                let local = self
                    .storage_ui
                    .demo_directory
                    .as_ref()
                    .unwrap()
                    .path()
                    .join("Local tracks");
                self.storage_ui.keep_both = true;
                self.storage_ui.start(Operation::Upload(
                    vec![
                        local.join("Evening groove.wav"),
                        local.join("New practice.fuse"),
                    ],
                    CollisionPolicy::KeepBoth,
                ));
                self.storage_ui.demo_stage = 1;
            }
            1 if self.storage_ui.demo == 3 && self.storage_ui.preview.is_some() => {
                self.storage_ui.preview.as_mut().unwrap().confirmed = true;
                self.storage_ui.apply();
                self.storage_ui.demo_stage = 2;
            }
            _ => {
                self.storage_ui.demo = 0;
                self.frames = 0;
            }
        }
        if self.storage_ui.error.is_some() {
            self.storage_ui.demo = 0;
            self.frames = 0;
        }
    }
}
