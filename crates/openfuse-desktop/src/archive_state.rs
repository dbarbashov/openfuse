//! Bounded background catalog work; filesystem operations never run in drawing.
use openfuse_core::{
    archive::{Archive, ArchiveCatalog, CatalogEntry, CatalogScan},
    bank::Backup,
    library::PresetLibrary,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
};
pub const FOLDER_KEY: &str = "confuse.archive.folder";
pub enum Operation {
    Scan,
    ImportFile(PathBuf),
    ImportLegacy(PathBuf),
    Load(CatalogEntry),
    Delete(CatalogEntry),
    Restore(CatalogEntry),
    Extract(CatalogEntry, Vec<u16>, PathBuf),
    SaveCapture(Backup),
    SaveAuxiliary(openfuse_core::auxiliary::Snapshot),
}
struct Completion {
    generation: u64,
    result: Result<Snapshot, String>,
}
struct Snapshot {
    scan: CatalogScan,
    trash: CatalogScan,
    loaded: Option<(CatalogEntry, Archive)>,
    notice: Option<String>,
    extracted: bool,
}
#[derive(Default)]
pub struct ArchiveState {
    pub folder: Option<PathBuf>,
    pub entries: Vec<CatalogEntry>,
    pub trash: Vec<CatalogEntry>,
    pub diagnostics: Vec<String>,
    pub query: String,
    pub show_deleted: bool,
    pub loaded: Option<(CatalogEntry, Archive)>,
    pub marked: BTreeSet<u16>,
    pub targets: BTreeMap<u16, u16>,
    pub inspected: Option<u16>,
    pub legacy_profile: Option<u16>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub library_refresh: bool,
    worker: Option<Receiver<Completion>>,
    generation: u64,
    scan_pending: bool,
}
impl ArchiveState {
    pub fn busy(&self) -> bool {
        self.worker.is_some() || self.scan_pending
    }
    pub fn set_folder(&mut self, folder: PathBuf) {
        self.generation = self.generation.wrapping_add(1);
        self.folder = Some(folder);
        self.entries.clear();
        self.trash.clear();
        self.loaded = None;
        self.marked.clear();
        self.targets.clear();
        self.inspected = None;
        self.error = None;
        self.notice = None;
        self.legacy_profile = None;
        self.scan_pending = true;
        self.poll();
    }
    pub fn rows(&self) -> Vec<&CatalogEntry> {
        let mut entries: Vec<_> = if self.show_deleted {
            &self.trash
        } else {
            &self.entries
        }
        .iter()
        .filter(|e| e.matches(&self.query))
        .collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.imported_at_ms));
        entries
    }
    pub fn start(&mut self, operation: Operation) {
        if self.worker.is_some() {
            self.error = Some("Wait for the current archive operation".into());
            return;
        }
        let Some(folder) = self.folder.clone() else {
            self.error = Some("Choose an archive catalog folder first".into());
            return;
        };
        self.error = None;
        self.scan_pending = false;
        let generation = self.generation;
        let (sender, receiver) = mpsc::channel();
        self.worker = Some(receiver);
        std::thread::spawn(move || {
            let result = execute(folder, operation);
            let _ = sender.send(Completion { generation, result });
        });
    }
    pub fn poll(&mut self) {
        let done = self
            .worker
            .as_ref()
            .and_then(|worker| match worker.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Completion {
                    generation: self.generation,
                    result: Err("Archive worker stopped unexpectedly".into()),
                }),
            });
        if let Some(done) = done {
            self.worker = None;
            if done.generation == self.generation {
                match done.result {
                    Ok(snapshot) => {
                        self.library_refresh |= snapshot.extracted;
                        self.entries = snapshot.scan.entries;
                        self.trash = snapshot.trash.entries;
                        self.diagnostics = snapshot
                            .scan
                            .diagnostics
                            .into_iter()
                            .chain(snapshot.trash.diagnostics)
                            .map(|d| format!("{}: {}", d.path.display(), d.message))
                            .collect();
                        if snapshot.scan.truncated || snapshot.trash.truncated {
                            self.diagnostics
                                .push("Catalog listing reached its scan limit".into());
                        }
                        if let Some(loaded) = snapshot.loaded {
                            self.inspected = loaded.1.presets().first().map(|p| p.slot);
                            self.legacy_profile = None;
                            self.marked.clear();
                            self.targets.clear();
                            self.loaded = Some(loaded);
                        } else if self.loaded.as_ref().is_some_and(|(old, _)| {
                            !self.entries.iter().any(|entry| entry.id == old.id)
                        }) {
                            self.loaded = None;
                            self.marked.clear();
                            self.targets.clear();
                        }
                        if snapshot.notice.is_some() {
                            self.notice = snapshot.notice;
                        }
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        }
        if self.worker.is_none() && self.scan_pending {
            self.start(Operation::Scan);
        }
    }
}
fn execute(folder: PathBuf, operation: Operation) -> Result<Snapshot, String> {
    let catalog = ArchiveCatalog::open(&folder).map_err(|e| e.to_string())?;
    let mut loaded = None;
    let mut notice = None;
    let mut extracted = false;
    match operation {
        Operation::Scan => {}
        Operation::Load(entry) => {
            let archive = catalog.load(&entry).map_err(|e| e.to_string())?;
            loaded = Some((entry, archive));
        }
        Operation::ImportFile(path) => {
            let entry = catalog.import_file(path).map_err(|e| e.to_string())?;
            notice = Some(format!("Imported {}", entry.label));
            let archive = catalog.load(&entry).map_err(|e| e.to_string())?;
            loaded = Some((entry, archive));
        }
        Operation::ImportLegacy(path) => {
            let entry = catalog.import_legacy(path).map_err(|e| e.to_string())?;
            notice = Some(format!(
                "Imported {} with original files retained",
                entry.label
            ));
            let archive = catalog.load(&entry).map_err(|e| e.to_string())?;
            loaded = Some((entry, archive));
        }
        Operation::Delete(entry) => {
            let entry = catalog.delete(&entry).map_err(|e| e.to_string())?;
            notice = Some(format!(
                "Moved {} to recoverable deleted archives",
                entry.label
            ));
        }
        Operation::Restore(entry) => {
            let entry = catalog.restore(&entry).map_err(|e| e.to_string())?;
            notice = Some(format!("Recovered {}", entry.label));
        }
        Operation::Extract(entry, slots, folder) => {
            let library = PresetLibrary::open(folder).map_err(|e| e.to_string())?;
            let imported = catalog
                .extract(&entry, &slots, &library)
                .map_err(|e| e.to_string())?;
            notice = Some(format!(
                "Extracted {} preset(s) into the local library",
                imported.len()
            ));
            extracted = true;
        }
        Operation::SaveAuxiliary(snapshot) => {
            let entry = catalog
                .import_auxiliary(&snapshot, "System and control banks")
                .map_err(|e| e.to_string())?;
            let archive = catalog.load(&entry).map_err(|e| e.to_string())?;
            notice = Some(format!(
                "Stored {} system/control entries in the catalog",
                snapshot.entries().len()
            ));
            loaded = Some((entry, archive));
        }
        Operation::SaveCapture(backup) => {
            let entry = catalog
                .import_backup(
                    &backup,
                    if backup.is_complete() {
                        "Complete amplifier bank"
                    } else {
                        "Selected amplifier presets"
                    },
                )
                .map_err(|e| e.to_string())?;
            let archive = catalog.load(&entry).map_err(|e| e.to_string())?;
            notice = Some(format!(
                "Stored {} captured presets in the catalog",
                backup.slots().len()
            ));
            loaded = Some((entry, archive));
        }
    }
    Ok(Snapshot {
        scan: catalog.scan().map_err(|e| e.to_string())?,
        trash: catalog.trash().map_err(|e| e.to_string())?,
        loaded,
        notice,
        extracted,
    })
}
/// Calendar display in UTC; archives retain the original millisecond timestamp.
pub fn date(timestamp_ms: u64) -> String {
    let seconds = timestamp_ms / 1000;
    let days = (seconds / 86400) as i64 + 719468;
    let era = days.div_euclid(146097);
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3600 % 24,
        seconds / 60 % 60
    )
}
