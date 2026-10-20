//! Managed media work stays off the render thread. Folder generations bind completions.
use openfuse_core::media_library::{MediaEntry, MediaLibrary};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};
pub const FOLDER_KEY: &str = "confuse.media_library.folder";
#[derive(Clone)]
pub enum Operation {
    Scan,
    Import(Vec<PathBuf>),
    ImportVerified(Vec<(PathBuf, String)>),
    ImportFolder(PathBuf),
    Load(MediaEntry),
    Archive(MediaEntry),
    Restore(MediaEntry),
}
struct Completion {
    generation: u64,
    load_epoch: u64,
    result: Result<Completed, String>,
}
struct Completed {
    entries: Vec<MediaEntry>,
    trash: Vec<MediaEntry>,
    diagnostics: Vec<String>,
    loaded: Option<(MediaEntry, Vec<u8>)>,
    notice: String,
}
#[derive(Default)]
pub struct Library {
    pub folder: Option<PathBuf>,
    pub entries: Vec<MediaEntry>,
    pub trash: Vec<MediaEntry>,
    pub query: String,
    pub deleted: bool,
    pub selected: Option<String>,
    pub diagnostics: Vec<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub loaded: Option<(MediaEntry, Vec<u8>)>,
    generation: u64,
    load_epoch: u64,
    worker: Option<Receiver<Completion>>,
    rescan: bool,
}
impl Library {
    pub fn cancel_load(&mut self) {
        self.load_epoch = self.load_epoch.wrapping_add(1);
        self.loaded = None;
    }
    pub fn busy(&self) -> bool {
        self.worker.is_some() || self.rescan
    }
    pub fn set_folder(&mut self, folder: PathBuf) {
        self.generation = self.generation.wrapping_add(1);
        self.folder = Some(folder);
        self.entries.clear();
        self.trash.clear();
        self.selected = None;
        self.loaded = None;
        self.error = None;
        self.notice = None;
        self.diagnostics.clear();
        self.rescan = true;
        self.poll();
    }
    pub fn selected_entry(&self) -> Option<&MediaEntry> {
        let id = self.selected.as_ref()?;
        let entries = if self.deleted {
            &self.trash
        } else {
            &self.entries
        };
        entries.iter().find(|entry| &entry.id == id)
    }
    pub fn start(&mut self, operation: Operation) {
        if self.busy() {
            self.error = Some("Wait for the current media operation".into());
            return;
        }
        let folder = self.folder.clone();
        let generation = self.generation;
        let load_epoch = self.load_epoch;
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        self.error = None;
        std::thread::spawn(move || {
            let result = execute(folder, operation);
            let _ = tx.send(Completion {
                generation,
                load_epoch,
                result,
            });
        });
    }
    pub fn poll(&mut self) {
        if let Some(rx) = &self.worker {
            match rx.try_recv() {
                Ok(completion) => {
                    self.worker = None;
                    if completion.generation == self.generation {
                        match completion.result {
                            Ok(done) => {
                                self.entries = done.entries;
                                self.trash = done.trash;
                                self.diagnostics = done.diagnostics;
                                self.loaded = if completion.load_epoch == self.load_epoch {
                                    done.loaded
                                } else {
                                    None
                                };
                                self.notice = Some(done.notice);
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.error = Some("Media worker stopped".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.worker.is_none() && self.rescan {
            self.rescan = false;
            self.start(Operation::Scan);
        }
    }
}
fn execute(folder: Option<PathBuf>, operation: Operation) -> Result<Completed, String> {
    let library = MediaLibrary::open(folder.ok_or("Choose a backing-track library folder first")?)
        .map_err(|e| e.to_string())?;
    let mut loaded = None;
    let mut diagnostics = vec![];
    let notice = match operation {
        Operation::Scan => "Library refreshed".into(),
        Operation::Import(paths) => import_paths(
            &library,
            paths.into_iter().map(|path| (path, None)).collect(),
            &mut diagnostics,
        ),
        Operation::ImportVerified(paths) => import_paths(
            &library,
            paths
                .into_iter()
                .map(|(path, hash)| (path, Some(hash)))
                .collect(),
            &mut diagnostics,
        ),
        Operation::ImportFolder(path) => {
            let imported = library.import_directory(path).map_err(|e| e.to_string())?;
            diagnostics.extend(
                imported
                    .diagnostics
                    .into_iter()
                    .map(|d| format!("{}: {}", d.path.display(), d.message)),
            );
            if imported.truncated {
                diagnostics
                    .push("Folder import limit reached; import remaining files separately".into());
            }
            format!("Imported {} tracks from folder", imported.entries.len())
        }
        Operation::Load(entry) => {
            let bytes = library.read(&entry).map_err(|e| e.to_string())?;
            loaded = Some((entry, bytes));
            "Verified managed audio copy".into()
        }
        Operation::Archive(entry) => {
            library.delete(&entry).map_err(|e| e.to_string())?;
            "Track archived; recover it from Archived".into()
        }
        Operation::Restore(entry) => {
            library.restore(&entry).map_err(|e| e.to_string())?;
            "Track restored".into()
        }
    };
    let scan = library.scan().map_err(|e| e.to_string())?;
    let trash = library.trash().map_err(|e| e.to_string())?;
    diagnostics.extend(
        scan.diagnostics
            .into_iter()
            .chain(trash.diagnostics)
            .map(|d| format!("{}: {}", d.path.display(), d.message)),
    );
    if scan.truncated || trash.truncated {
        diagnostics.push("Library results were limited; narrow the folder contents".into());
    }
    Ok(Completed {
        entries: scan.entries,
        trash: trash.entries,
        diagnostics,
        loaded,
        notice,
    })
}

fn import_paths(
    library: &MediaLibrary,
    paths: Vec<(PathBuf, Option<String>)>,
    diagnostics: &mut Vec<String>,
) -> String {
    let mut imported = 0;
    let mut bytes = 0;
    if paths.len() > 100 {
        diagnostics.push(
            "Import is limited to 100 selected files; import the remainder separately".into(),
        );
    }
    for (path, expected) in paths.into_iter().take(100) {
        if bytes >= 512 * 1024 * 1024 {
            diagnostics.push("Import byte limit reached; import remaining files separately".into());
            break;
        }
        if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 512 * 1024 * 1024 - bytes)
        {
            diagnostics.push(format!(
                "{}: skipped; batch copy limit would be exceeded",
                path.display()
            ));
            continue;
        }
        match expected.as_deref().map_or_else(
            || library.import_file(&path),
            |hash| library.import_file_checked(&path, hash),
        ) {
            Ok(entry) => {
                imported += 1;
                bytes += entry.size;
            }
            Err(e) => diagnostics.push(format!("{}: {e}", path.display())),
        }
    }
    format!("Imported {imported} tracks · {} errors", diagnostics.len())
}
