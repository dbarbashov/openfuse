use super::Editor;
use crate::{
    media_state::Operation,
    media_view::{self, Action, BandInput},
};
use eframe::egui;
use openfuse_core::preset::band::Band;
use std::path::PathBuf;
impl Editor {
    pub(super) fn open_media(&mut self) {
        self.media_ui.open = true;
    }
    pub(super) fn media_folder(&mut self, folder: PathBuf) {
        self.media_ui.player.clear();
        self.media_ui.loaded = None;
        self.media_ui.error = None;
        self.media_ui.library.set_folder(folder);
    }
    pub(super) fn media_update(&mut self, ctx: &egui::Context) {
        self.media_ui.library.poll();
        if self.media_ui.open
            && self.media_ui.library.folder.is_some()
            && !self.media_ui.library.busy()
            && self.media_ui.last_refresh.elapsed() >= std::time::Duration::from_secs(3)
        {
            self.media_ui.last_refresh = std::time::Instant::now();
            self.media_ui.library.start(Operation::Scan);
        }
        if self.media_ui.loaded.as_ref().is_some_and(|loaded| {
            !self
                .media_ui
                .library
                .entries
                .iter()
                .any(|entry| entry.id == loaded.id)
        }) {
            self.media_ui.player.clear();
            self.media_ui.loaded = None;
        }
        self.media_ui.player.poll();
        if let Some((entry, bytes)) = self.media_ui.library.loaded.take() {
            match self
                .media_ui
                .player
                .load(entry.id.clone(), bytes, entry.format.extension())
            {
                Ok(_) => {
                    self.media_ui.loaded = Some(entry);
                    self.media_ui.error = None;
                }
                Err(error) => self.media_ui.error = Some(error),
            }
        }
        if self.media_ui.library.busy()
            || matches!(
                self.media_ui.player.snapshot().phase,
                openfuse_audio::Phase::Loading | openfuse_audio::Phase::Playing
            )
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.media_demo_update();
    }
    pub(super) fn media_dialog(&mut self, ctx: &egui::Context) {
        if !self.media_ui.open || self.pending.is_some() {
            return;
        }
        let band = self
            .session
            .as_ref()
            .map(|session| {
                let source = session.document.to_xml().map_err(|e| e.to_string())?;
                let band = session
                    .document
                    .band()
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                Ok::<_, String>(BandInput {
                    source,
                    band,
                    product: session.document.product_id().unwrap_or(0),
                })
            })
            .transpose();
        let band = match band {
            Ok(band) => band,
            Err(error) => {
                self.media_ui.error = Some(error);
                None
            }
        };
        if let Some(action) = media_view::show(ctx, &mut self.media_ui, band) {
            self.media_action(action);
        }
    }
    pub(super) fn media_action(&mut self, action: Action) {
        self.media_ui.error = None;
        match action {
            Action::AudioSettings => self.open_audio(),
            Action::Close => {
                self.media_ui.library.cancel_load();
                self.media_ui.player.stop();
                self.media_ui.open = false;
            }
            Action::Folder => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Choose backing-track library folder")
                    .pick_folder()
                {
                    self.media_folder(folder);
                }
            }
            Action::Refresh => self.media_ui.library.start(Operation::Scan),
            Action::Import => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Backing tracks", &["wav", "mp3", "mid", "midi"])
                    .pick_files()
                {
                    self.media_ui.library.start(Operation::Import(paths));
                }
            }
            Action::ImportFolder => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Import backing tracks from a folder")
                    .pick_folder()
                {
                    self.media_ui.library.start(Operation::ImportFolder(folder));
                }
            }
            Action::FindLinked(name) => {
                self.media_ui.library.query = name.clone();
                self.media_ui.library.deleted = false;
                let entries: Vec<_> = self
                    .media_ui
                    .library
                    .entries
                    .iter()
                    .filter(|entry| entry.name == name)
                    .collect();
                self.media_ui.library.selected = if entries.len() == 1 {
                    Some(entries[0].id.clone())
                } else {
                    None
                };
                self.media_ui.library.notice=Some(match entries.len(){0=>"Linked file is not in this library. Import or locate it explicitly.".into(),1=>"Linked track found. Load it explicitly to play.".into(),_=>"Several tracks share this filename. Inspect their original paths and select one.".into()});
            }
            Action::Select(id) => self.media_ui.library.selected = Some(id),
            Action::Load(entry) => {
                if self.media_ui.library.busy() {
                    self.media_ui.error = Some("Wait for the current media operation".into());
                    return;
                }
                self.media_ui.library.cancel_load();
                self.media_ui.player.clear();
                self.media_ui.loaded = None;
                self.media_ui.library.start(Operation::Load(entry));
            }
            Action::Archive(entry) => {
                if self
                    .media_ui
                    .loaded
                    .as_ref()
                    .is_some_and(|loaded| loaded.id == entry.id)
                {
                    self.media_ui.player.clear();
                    self.media_ui.loaded = None;
                }
                self.media_ui.library.start(Operation::Archive(entry));
            }
            Action::Restore(entry) => self.media_ui.library.start(Operation::Restore(entry)),
            Action::Play => {
                if let Err(error) = self.media_ui.player.play() {
                    self.media_ui.error = Some(error);
                }
            }
            Action::Pause => self.media_ui.player.pause(),
            Action::Stop => {
                self.media_ui.library.cancel_load();
                self.media_ui.player.stop();
            }
            Action::Seek(seconds) => {
                if let Err(error) = self.media_ui.player.seek(seconds) {
                    self.media_ui.error = Some(error);
                }
            }
            Action::Volume(value) => {
                if let Err(error) = self.media_ui.player.set_volume(value) {
                    self.media_ui.error = Some(error);
                }
            }
            Action::Balance(value) => {
                if let Err(error) = self.media_ui.player.set_balance(value) {
                    self.media_ui.error = Some(error);
                }
            }
            Action::Repeat(value) => self.media_ui.player.set_repeat(value),
            Action::Link(entry, source, mut band) => match band.link_pc_audio(&entry.name) {
                Ok(()) => self.apply_band(&source, band),
                Err(error) => self.media_ui.error = Some(error.to_string()),
            },
            Action::ClearBand(source, mut band) => {
                band.clear();
                self.apply_band(&source, band);
            }
            Action::SetBand(source, band) => self.apply_band(&source, band),
            Action::SetFilename(source, name, mut band) => match band.set_filename(&name) {
                Ok(()) => self.apply_band(&source, band),
                Err(error) => self.media_ui.error = Some(error.to_string()),
            },
            Action::UsePreset(source, band) => {
                let matches_source = self
                    .session
                    .as_ref()
                    .and_then(|session| session.document.to_xml().ok())
                    .as_deref()
                    == Some(&source);
                let loaded = self.media_ui.loaded.as_ref().is_some_and(|entry| {
                    entry.name == band.filename
                        && self
                            .media_ui
                            .library
                            .entries
                            .iter()
                            .any(|candidate| candidate.id == entry.id)
                });
                let unique = self
                    .media_ui
                    .library
                    .entries
                    .iter()
                    .filter(|entry| entry.name == band.filename)
                    .count()
                    == 1;
                if !matches_source || !loaded || !unique || band.location != 9 {
                    self.media_ui.error = Some(
                        "Load the unambiguous linked track and review the current preset first"
                            .into(),
                    );
                    return;
                }
                self.media_ui.player.set_repeat(band.repeat == 1);
                let balance = if band.audio_mix == 1 {
                    2.0 * band.balance as f32 / 65535.0 - 1.0
                } else {
                    0.0
                };
                if let Err(error) = self.media_ui.player.set_balance(balance) {
                    self.media_ui.error = Some(error);
                }
            }
        }
    }
    pub(super) fn apply_band(&mut self, source: &str, band: Band) {
        let result = (|| {
            let session = self
                .session
                .as_mut()
                .ok_or("Open a preset before editing its Band settings")?;
            if session.document.to_xml().map_err(|e| e.to_string())? != source {
                return Err(
                    "The preset changed; review its current track link before applying".into(),
                );
            }
            session.band(&band)
        })();
        match result {
            Ok(()) => {
                self.disable_live();
                self.device_origin = None;
                self.status = "Preset Band settings updated locally · Undo is available".into()
            }
            Err(error) => self.media_ui.error = Some(error),
        }
    }
}
#[cfg(test)]
#[path = "app_media_tests.rs"]
mod tests;
