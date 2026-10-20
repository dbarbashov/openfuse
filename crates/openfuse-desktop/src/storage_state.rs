//! Explicit mounted/extracted roots. Reads, hashing and copies run off the render thread.
use openfuse_core::{
    mounted_storage::{
        CollisionPolicy, Direction, Entry, FileKind, Inventory, MountedStorage, TransferPlan,
        TransferReceipt, Volume,
    },
    preset::PresetDocument,
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};
pub const ROOT_KEY: &str = "confuse.gdec_storage.root";
pub const VOLUME_KEY: &str = "confuse.gdec_storage.volume";
pub enum Operation {
    Scan,
    Upload(Vec<PathBuf>, CollisionPolicy),
    Download(Vec<Entry>, PathBuf, CollisionPolicy),
    OpenPreset(Box<Entry>),
    Execute(TransferPlan),
}

pub struct Preview {
    pub plan: TransferPlan,
    pub generation: u64,
    pub confirmed: bool,
    pub page: usize,
}
pub struct Receipt {
    pub direction: Direction,
    pub result: TransferReceipt,
}
enum Completion {
    Inventory(Inventory),
    Preview(TransferPlan),
    Preset(PresetDocument, String),
    Receipt(Direction, TransferReceipt),
}
enum Event {
    Progress(usize, usize),
    Done(Result<Completion, String>),
}
struct Worker {
    generation: u64,
    receiver: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    executing: bool,
}
pub struct State {
    pub open: bool,
    pub root: Option<PathBuf>,
    pub volume: Volume,
    pub entries: Vec<Entry>,
    pub diagnostics: Vec<String>,
    pub truncated: bool,
    pub query: String,
    pub filter: Option<FileKind>,
    pub selected: BTreeSet<PathBuf>,
    pub keep_both: bool,
    pub preview: Option<Preview>,
    pub receipt: Option<Receipt>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub progress: (usize, usize),
    pub phase: String,
    pub loaded: Option<(PresetDocument, String)>,
    pub pending_media: Option<Vec<(PathBuf, String)>>,
    pub close_when_finished: bool,
    pub close_window_when_finished: bool,
    pub demo: u8,
    pub demo_stage: u8,
    pub demo_directory: Option<tempfile::TempDir>,
    generation: u64,
    worker: Option<Worker>,
    rescan: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            root: None,
            volume: Volume::Sd,
            entries: vec![],
            diagnostics: vec![],
            truncated: false,
            query: String::new(),
            filter: None,
            selected: BTreeSet::new(),
            keep_both: false,
            preview: None,
            receipt: None,
            error: None,
            notice: None,
            progress: (0, 0),
            phase: String::new(),
            loaded: None,
            pending_media: None,
            close_when_finished: false,
            close_window_when_finished: false,
            demo: 0,
            demo_stage: 0,
            demo_directory: None,
            generation: 0,
            worker: None,
            rescan: false,
        }
    }
}
impl State {
    pub fn with_demo(demo: u8) -> Self {
        Self {
            demo,
            ..Default::default()
        }
    }
    pub fn busy(&self) -> bool {
        self.worker.is_some() || self.rescan
    }
    pub fn executing(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| w.executing)
    }
    pub fn policy(&self) -> CollisionPolicy {
        if self.keep_both {
            CollisionPolicy::KeepBoth
        } else {
            CollisionPolicy::Reject
        }
    }
    pub fn selected_entries(&self) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| self.selected.contains(&e.relative_path))
            .cloned()
            .collect()
    }
    pub fn set_root(&mut self, root: PathBuf, volume: Volume) {
        if self.executing() {
            self.error = Some(
                "Cancel the current copy and wait for its receipt before changing the storage root"
                    .into(),
            );
            return;
        }
        self.cancel();
        self.generation = self.generation.wrapping_add(1);
        self.root = Some(root);
        self.volume = volume;
        self.entries.clear();
        self.diagnostics.clear();
        self.selected.clear();
        self.preview = None;
        self.receipt = None;
        self.loaded = None;
        self.error = None;
        self.notice = None;
        self.rescan = true;
        self.poll();
    }
    pub fn cancel(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::Release);
            self.phase = "Cancelling; completed copies remain".into();
        }
    }
    pub fn start(&mut self, operation: Operation) {
        if self.busy() {
            self.error = Some("Wait for the current storage operation to finish".into());
            return;
        }
        let Some(root) = self.root.clone() else {
            self.error = Some("Choose an explicit storage root first".into());
            return;
        };
        let volume = self.volume;
        let generation = self.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(16);
        let executing = matches!(operation, Operation::Execute(_));
        self.phase = match &operation {
            Operation::Scan => "Reading folder inventory",
            Operation::Upload(..) | Operation::Download(..) => "Preparing verified copy preview",
            Operation::OpenPreset(_) => "Reading verified preset copy",
            Operation::Execute(_) => "Copying reviewed files",
        }
        .into();
        self.progress = match &operation {
            Operation::Execute(plan) => (0, plan.items().len()),
            _ => (0, 0),
        };
        self.error = None;
        self.loaded = None;
        self.worker = Some(Worker {
            generation,
            receiver,
            cancel,
            executing,
        });
        std::thread::spawn(move || {
            let result = (|| {
                let storage = MountedStorage::open(&root, volume).map_err(|e| e.to_string())?;
                if worker_cancel.load(Ordering::Acquire) {
                    return Err("Storage operation cancelled".into());
                }
                match operation {
                    Operation::Scan => storage
                        .scan()
                        .map(Completion::Inventory)
                        .map_err(|e| e.to_string()),
                    Operation::Upload(paths, policy) => {
                        TransferPlan::upload_with_policy(&storage, &paths, policy)
                            .map(Completion::Preview)
                            .map_err(|e| e.to_string())
                    }
                    Operation::Download(entries, folder, policy) => {
                        TransferPlan::download_with_policy(&storage, &entries, &folder, policy)
                            .map(Completion::Preview)
                            .map_err(|e| e.to_string())
                    }
                    Operation::OpenPreset(entry) => {
                        let bytes = storage.read(&entry).map_err(|e| e.to_string())?;
                        let xml = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
                        let document = PresetDocument::parse(xml).map_err(|e| e.to_string())?;
                        Ok(Completion::Preset(
                            document,
                            format!(
                                "Storage copy · {}",
                                root.join(entry.relative_path).display()
                            ),
                        ))
                    }
                    Operation::Execute(plan) => {
                        let direction = plan.direction();
                        let receipt = plan.execute_with_progress(&worker_cancel, |done, total| {
                            let _ = sender.try_send(Event::Progress(done, total));
                        });
                        Ok(Completion::Receipt(direction, receipt))
                    }
                }
            })();
            let _ = sender.send(Event::Done(result));
        });
    }
    pub fn apply(&mut self) {
        let Some(preview) = &self.preview else {
            return;
        };
        if self.busy() || preview.generation != self.generation {
            self.error = Some("Copy preview is stale; choose the root and review again".into());
            return;
        }
        if !preview.confirmed {
            self.error = Some("Confirm the listed copies first".into());
            return;
        }
        let preview = self.preview.take().expect("checked preview");
        self.receipt = None;
        self.start(Operation::Execute(preview.plan));
    }
    pub fn poll(&mut self) {
        while let Some(worker) = &self.worker {
            let event = match worker.receiver.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    Event::Done(Err("Storage worker stopped unexpectedly".into()))
                }
            };
            let current = worker.generation == self.generation;
            let cancelled = worker.cancel.load(Ordering::Acquire);
            match event {
                Event::Progress(done, total) => {
                    if current {
                        self.progress = (done, total);
                    }
                }
                Event::Done(result) => {
                    self.worker = None;
                    if current {
                        match result {
                            Ok(Completion::Receipt(direction, receipt)) => {
                                self.progress.0 = receipt.completed.len();
                                self.notice = Some(format!(
                                    "{} files copied and verified{}",
                                    receipt.completed.len(),
                                    if receipt.cancelled {
                                        "; remaining copies cancelled"
                                    } else {
                                        ""
                                    }
                                ));
                                self.error = receipt.error.clone();
                                self.receipt = Some(Receipt {
                                    direction,
                                    result: receipt,
                                });
                                self.rescan = true;
                            }
                            _ if cancelled => {
                                self.notice = Some(
                                    "Storage operation cancelled; no late preview will be opened"
                                        .into(),
                                )
                            }
                            Ok(Completion::Inventory(inventory)) => {
                                self.entries = inventory.entries;
                                self.diagnostics = inventory
                                    .diagnostics
                                    .into_iter()
                                    .map(|d| format!("{}: {}", d.path.display(), d.message))
                                    .collect();
                                self.truncated = inventory.truncated;
                                self.selected.retain(|path| {
                                    self.entries.iter().any(|e| &e.relative_path == path)
                                });
                            }
                            Ok(Completion::Preview(plan)) => {
                                self.preview = Some(Preview {
                                    plan,
                                    generation: self.generation,
                                    confirmed: false,
                                    page: 0,
                                })
                            }
                            Ok(Completion::Preset(document, label)) => {
                                self.loaded = Some((document, label))
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                }
            }
        }
        if self.worker.is_none() && self.rescan {
            self.rescan = false;
            self.start(Operation::Scan);
        }
        if !self.busy() && self.close_when_finished {
            self.open = false;
            self.close_when_finished = false;
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn partial_cancelled_receipt_survives_automatic_inventory_refresh() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().join("storage");
        let local = folder.path().join("local");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&local).unwrap();
        let sources = vec![local.join("one.wav"), local.join("two.wav")];
        for path in &sources {
            std::fs::write(path, b"original audio bytes").unwrap();
        }
        let storage = MountedStorage::open(&root, Volume::Sd).unwrap();
        let plan = TransferPlan::upload(&storage, &sources).unwrap();
        let cancel = AtomicBool::new(false);
        let receipt =
            plan.execute_with_progress(&cancel, |_, _| cancel.store(true, Ordering::Release));
        assert_eq!(receipt.completed.len(), 1);
        assert!(receipt.cancelled);
        let mut state = State {
            root: Some(root.clone()),
            ..Default::default()
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        state.worker = Some(Worker {
            generation: 0,
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
            executing: true,
        });
        sender
            .send(Event::Done(Ok(Completion::Receipt(
                Direction::Upload,
                receipt,
            ))))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            state.poll();
            if !state.busy() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let receipt = &state.receipt.as_ref().unwrap().result;
        assert_eq!(receipt.completed.len(), 1);
        assert!(receipt.cancelled);
        assert_eq!(state.entries.len(), 1);
        assert!(root.join("Audio/one.wav").exists());
        assert!(!root.join("Audio/two.wav").exists());
    }
}
