//! Window-independent library state. Filesystem work runs on one bounded worker.
use openfuse_core::library::{ArchiveToken, LibraryEntry, LibraryScan, PresetLibrary};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

pub const FOLDER_KEY: &str = "confuse.library.folder";
const REFRESH_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    Author,
    Product,
    File,
}
impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Author => "Author",
            Self::Product => "Amplifier family",
            Self::File => "File name",
        }
    }
}

pub enum Operation {
    Scan,
    Import(Vec<PathBuf>),
    Duplicate(Vec<LibraryEntry>),
    Archive(Vec<LibraryEntry>),
    Restore(ArchiveToken),
}
struct Snapshot {
    scan: LibraryScan,
    archives: Vec<ArchiveToken>,
    notices: Vec<String>,
}
struct Completion {
    generation: u64,
    result: Result<Snapshot, String>,
}

pub struct LibraryState {
    pub folder: Option<PathBuf>,
    pub scan: Option<LibraryScan>,
    pub archives: Vec<ArchiveToken>,
    pub query: String,
    pub sort: Sort,
    pub notices: Vec<String>,
    pub error: Option<String>,
    pub selected: Option<PathBuf>,
    pub marked: BTreeSet<PathBuf>,
    worker: Option<Receiver<Completion>>,
    generation: u64,
    scan_requested: bool,
    next_scan: Instant,
}
impl Default for LibraryState {
    fn default() -> Self {
        Self {
            folder: None,
            scan: None,
            archives: Vec::new(),
            query: String::new(),
            sort: Sort::Name,
            notices: Vec::new(),
            error: None,
            selected: None,
            marked: BTreeSet::new(),
            worker: None,
            generation: 0,
            scan_requested: false,
            next_scan: Instant::now(),
        }
    }
}
impl LibraryState {
    pub fn set_folder(&mut self, folder: PathBuf) {
        self.generation = self.generation.wrapping_add(1);
        self.folder = Some(if folder.is_absolute() {
            folder
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&folder))
                .unwrap_or(folder)
        });
        self.scan = None;
        self.archives.clear();
        self.selected = None;
        self.marked.clear();
        self.error = None;
        self.notices.clear();
        self.scan_requested = true;
        self.poll();
    }
    pub fn busy(&self) -> bool {
        self.worker.is_some() || self.scan_requested
    }
    pub fn refresh(&mut self) {
        self.scan_requested = self.folder.is_some();
        self.poll();
    }
    pub fn entries(&self) -> Vec<&LibraryEntry> {
        let mut entries: Vec<_> = self
            .scan
            .as_ref()
            .into_iter()
            .flat_map(|scan| &scan.entries)
            .filter(|e| e.matches(&self.query))
            .collect();
        entries.sort_by(|a, b| {
            let order = match self.sort {
                Sort::Name => a
                    .metadata
                    .name
                    .to_lowercase()
                    .cmp(&b.metadata.name.to_lowercase()),
                Sort::Author => a
                    .metadata
                    .author
                    .to_lowercase()
                    .cmp(&b.metadata.author.to_lowercase()),
                Sort::Product => a.product_id.cmp(&b.product_id),
                Sort::File => a.relative_path.cmp(&b.relative_path),
            };
            order.then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        entries
    }
    pub fn marked_entries(&self) -> Vec<LibraryEntry> {
        self.scan
            .as_ref()
            .into_iter()
            .flat_map(|scan| &scan.entries)
            .filter(|entry| self.marked.contains(&entry.path))
            .cloned()
            .collect()
    }
    pub fn start(&mut self, operation: Operation) {
        let batch_size = match &operation {
            Operation::Import(paths) => paths.len(),
            Operation::Duplicate(entries) | Operation::Archive(entries) => entries.len(),
            _ => 1,
        };
        if batch_size > 10_000 {
            self.error = Some("Select at most 10,000 files per library operation.".into());
            return;
        }
        if self.worker.is_some() {
            self.error = Some("Library is busy. Try again when indexing finishes.".into());
            return;
        }
        let Some(folder) = self.folder.clone() else {
            return;
        };
        self.error = None;
        let generation = self.generation;
        let (sender, receiver) = mpsc::channel();
        self.worker = Some(receiver);
        self.scan_requested = false;
        std::thread::spawn(move || {
            let result = execute(&folder, operation);
            let _ = sender.send(Completion { generation, result });
        });
    }
    pub fn poll(&mut self) {
        let completed = self
            .worker
            .as_ref()
            .and_then(|worker| match worker.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Completion {
                    generation: self.generation,
                    result: Err("Library worker stopped unexpectedly".into()),
                }),
            });
        if let Some(completed) = completed {
            self.worker = None;
            if completed.generation == self.generation {
                self.next_scan = Instant::now() + REFRESH_INTERVAL;
                match completed.result {
                    Ok(snapshot) => {
                        let indexed: BTreeSet<_> = snapshot
                            .scan
                            .entries
                            .iter()
                            .map(|entry| entry.path.as_path())
                            .collect();
                        self.marked.retain(|path| indexed.contains(path.as_path()));
                        self.scan = Some(snapshot.scan);
                        self.archives = snapshot.archives;
                        if !snapshot.notices.is_empty() {
                            self.notices = snapshot.notices;
                        }
                        self.error = None;
                    }
                    Err(error) => {
                        self.notices = vec![error.clone()];
                        self.error = Some(error);
                    }
                }
            }
        }
        if self.worker.is_none()
            && self.folder.is_some()
            && (self.scan_requested || Instant::now() >= self.next_scan)
        {
            self.start(Operation::Scan);
        }
    }
}
fn execute(folder: &Path, operation: Operation) -> Result<Snapshot, String> {
    let library = PresetLibrary::open(folder).map_err(|e| e.to_string())?;
    let mut notices = Vec::new();
    match operation {
        Operation::Scan => {}
        Operation::Import(paths) => {
            for path in paths {
                notices.push(match library.import(&path) {
                    Ok(entry) => format!("Imported {}", entry.relative_path.display()),
                    Err(error) => format!("Import failed for {}: {error}", path.display()),
                });
            }
        }
        Operation::Duplicate(entries) => {
            for source in entries {
                notices.push(match library.duplicate(&source) {
                    Ok(entry) => format!(
                        "Duplicated {} as {}",
                        source.relative_path.display(),
                        entry.relative_path.display()
                    ),
                    Err(error) => format!(
                        "Duplicate failed for {}: {error}",
                        source.relative_path.display()
                    ),
                });
            }
        }
        Operation::Archive(entries) => {
            for entry in entries {
                notices.push(match library.archive(&entry) {
                    Ok(archive) => format!(
                        "Archived {}. Restore it from Archived presets.",
                        archive.original_relative_path.display()
                    ),
                    Err(error) => format!(
                        "Archive failed for {}: {error}",
                        entry.relative_path.display()
                    ),
                });
            }
        }
        Operation::Restore(token) => {
            let entry = library.restore(&token).map_err(|e| e.to_string())?;
            notices.push(format!("Restored {}", entry.relative_path.display()));
        }
    }
    let scan = library.scan().map_err(|e| e.to_string())?;
    let archives = match library.archives() {
        Ok(archives) => archives,
        Err(error) => {
            notices.push(format!("Cannot read archive: {error}"));
            Vec::new()
        }
    };
    Ok(Snapshot {
        scan,
        archives,
        notices,
    })
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || a.canonicalize()
            .ok()
            .zip(b.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn settle(state: &mut LibraryState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.busy() && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!state.busy(), "library worker did not complete");
        assert!(state.error.is_none(), "{:?}", state.error);
    }
    fn write_preset(path: &Path, name: &str) {
        std::fs::write(path, format!("<Preset ProductId=\"13\"><FUSE><Info name=\"{name}\" author=\"A player\" tags=\"clean\"/></FUSE></Preset>")).unwrap();
    }

    #[test]
    fn asynchronous_library_import_duplicate_archive_restore_and_external_refresh() {
        let folder = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("source.fuse");
        write_preset(&path, "Warm room");
        let original = std::fs::read(&path).unwrap();
        let mut state = LibraryState::default();
        state.set_folder(folder.path().to_owned());
        settle(&mut state);
        let invalid = source.path().join("invalid.fuse");
        std::fs::write(&invalid, "<not-a-preset/>").unwrap();
        state.start(Operation::Import(vec![path.clone(), invalid]));
        settle(&mut state);
        assert_eq!(state.entries().len(), 1);
        assert!(
            state
                .notices
                .iter()
                .any(|notice| notice.contains("Imported"))
        );
        assert!(
            state
                .notices
                .iter()
                .any(|notice| notice.contains("Import failed"))
        );
        let entry = state.entries()[0].clone();
        state.start(Operation::Duplicate(vec![entry]));
        settle(&mut state);
        assert_eq!(state.entries().len(), 2);
        let entry = state.entries()[0].clone();
        state.start(Operation::Archive(vec![entry]));
        settle(&mut state);
        assert_eq!(state.entries().len(), 1);
        assert_eq!(state.archives.len(), 1);
        let token = state.archives[0].clone();
        state.start(Operation::Restore(token));
        settle(&mut state);
        assert_eq!(state.entries().len(), 2);
        assert!(state.archives.is_empty());
        write_preset(&folder.path().join("external.fuse"), "Another sound");
        state.next_scan = Instant::now();
        state.poll();
        settle(&mut state);
        assert_eq!(state.entries().len(), 3);
        state.query = "another".into();
        assert_eq!(state.entries().len(), 1);
        state.query = "clean".into();
        state.sort = Sort::Name;
        assert_eq!(state.entries().len(), 3);
        assert_eq!(state.entries()[0].metadata.name, "Another sound");
        let selected = state.entries().into_iter().cloned().collect();
        state.start(Operation::Duplicate(selected));
        settle(&mut state);
        assert_eq!(state.entries().len(), 6);
        assert_eq!(
            state
                .notices
                .iter()
                .filter(|notice| notice.starts_with("Duplicated "))
                .count(),
            3
        );
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn folder_switch_discards_old_worker_results() {
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        write_preset(&old.path().join("old.fuse"), "Old");
        write_preset(&new.path().join("new.fuse"), "New");
        let old_snapshot = execute(old.path(), Operation::Scan).unwrap();
        let mut state = LibraryState::default();
        let (sender, receiver) = mpsc::channel();
        state.worker = Some(receiver);
        state.set_folder(new.path().to_owned());
        sender
            .send(Completion {
                generation: 0,
                result: Ok(old_snapshot),
            })
            .unwrap();
        settle(&mut state);
        assert_eq!(state.entries().len(), 1);
        assert_eq!(state.entries()[0].metadata.name, "New");
    }
}
