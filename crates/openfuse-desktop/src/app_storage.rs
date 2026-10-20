use super::{Editor, Pending};
use crate::{
    storage_state::{Operation, ROOT_KEY, VOLUME_KEY},
    storage_view::{self, Action},
};
use eframe::egui;
use openfuse_core::mounted_storage::Volume;
use std::path::PathBuf;
impl Editor {
    pub(super) fn open_storage(&mut self) {
        self.storage_ui.open = true;
        if self.storage_ui.root.is_some() && !self.storage_ui.busy() {
            self.storage_ui.start(Operation::Scan);
        }
    }
    pub(super) fn storage_update(&mut self, ctx: &egui::Context) {
        self.storage_ui.poll();
        if self.storage_ui.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if let Some((document, label)) = self.storage_ui.loaded.take() {
            self.storage_ui.open = false;
            self.request(Pending::OpenStorage(document, label), ctx);
        }
        if !self.media_ui.library.busy()
            && let Some(paths) = self.storage_ui.pending_media.take()
        {
            self.media_ui
                .library
                .start(crate::media_state::Operation::ImportVerified(paths));
            self.storage_ui.open = false;
            self.open_media();
        }
        if self.storage_ui.close_window_when_finished && !self.storage_ui.busy() {
            self.storage_ui.close_window_when_finished = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.storage_demo_update();
    }
    pub(super) fn storage_dialog(&mut self, ctx: &egui::Context) {
        if self.storage_ui.open
            && self.pending.is_none()
            && let Some(action) = storage_view::show(ctx, &mut self.storage_ui)
        {
            self.storage_action(action);
        }
    }
    pub(super) fn storage_action(&mut self, action: Action) {
        match action {
            Action::ChooseRoot => {
                if let Some(root) = rfd::FileDialog::new()
                    .set_title("Choose the explicit storage root (mounted or extracted)")
                    .pick_folder()
                {
                    self.storage_ui.set_root(root, self.storage_ui.volume);
                }
            }
            Action::Volume(volume) => {
                if let Some(root) = self.storage_ui.root.clone() {
                    self.storage_ui.set_root(root, volume);
                } else {
                    self.storage_ui.volume = volume;
                }
            }
            Action::Refresh => self.storage_ui.start(Operation::Scan),
            Action::Upload => {
                if let Some(paths) = rfd::FileDialog::new()
                    .set_title("Select files to copy into the chosen storage root")
                    .add_filter("G-DEC media and SD presets", &["wav", "mp3", "mid", "fuse"])
                    .pick_files()
                {
                    self.storage_ui
                        .start(Operation::Upload(paths, self.storage_ui.policy()));
                }
            }
            Action::Download => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Choose the local folder for verified copies")
                    .pick_folder()
                {
                    self.storage_ui.start(Operation::Download(
                        self.storage_ui.selected_entries(),
                        folder,
                        self.storage_ui.policy(),
                    ));
                }
            }
            Action::OpenPreset => {
                if let Some(entry) = self.storage_ui.selected_entries().into_iter().next() {
                    self.storage_ui
                        .start(Operation::OpenPreset(Box::new(entry)));
                }
            }
            Action::AddMedia => self.storage_add_media(),
            Action::Back => {
                self.storage_ui.preview = None;
                self.storage_ui.error = None;
            }
            Action::Apply => self.storage_ui.apply(),
            Action::Cancel => self.storage_ui.cancel(),
            Action::Close => {
                if self.storage_ui.busy() {
                    self.storage_ui.cancel();
                    self.storage_ui.close_when_finished = true;
                } else {
                    self.storage_ui.open = false;
                }
            }
        }
    }
    pub(super) fn storage_add_media(&mut self) {
        let Some(receipt) = &self.storage_ui.receipt else {
            return;
        };
        if !matches!(
            receipt.direction,
            openfuse_core::mounted_storage::Direction::Download
        ) {
            return;
        }
        let paths: Vec<(PathBuf, String)> = receipt
            .result
            .completed
            .iter()
            .filter(|item| storage_view::is_media(&item.destination))
            .map(|item| (item.destination.clone(), item.output_sha256.clone()))
            .collect();
        if paths.is_empty() {
            return;
        }
        if self.media_ui.library.folder.is_none() {
            let Some(folder) = rfd::FileDialog::new()
                .set_title("Choose the backing-track library folder")
                .pick_folder()
            else {
                return;
            };
            self.media_folder(folder);
        }
        self.storage_ui.pending_media = Some(paths);
    }
    pub(super) fn storage_cancel_close(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.storage_ui.cancel();
        self.storage_ui.open = true;
        self.storage_ui.close_window_when_finished = true;
    }
    pub(super) fn storage_restore_preferences(&mut self, storage: Option<&dyn eframe::Storage>) {
        let Some(storage) = storage else {
            return;
        };
        if self.storage_ui.root.is_none() {
            self.storage_ui.root = storage.get_string(ROOT_KEY).map(PathBuf::from);
            self.storage_ui.volume =
                if storage.get_string(VOLUME_KEY).as_deref() == Some("internal") {
                    Volume::Internal
                } else {
                    Volume::Sd
                };
        }
    }
    pub(super) fn storage_save_preferences(&self, storage: &mut dyn eframe::Storage) {
        if self.persist_library
            && let Some(root) = self.storage_ui.root.as_ref().and_then(|p| p.to_str())
        {
            storage.set_string(ROOT_KEY, root.to_owned());
            storage.set_string(
                VOLUME_KEY,
                if self.storage_ui.volume == Volume::Internal {
                    "internal"
                } else {
                    "sd"
                }
                .into(),
            );
        }
    }
}
#[cfg(test)]
#[path = "app_storage_tests.rs"]
mod tests;
