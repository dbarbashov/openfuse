//! Independent FX/VOICE document history and bounded asynchronous library work.
use openfuse_core::{
    control_library::{ControlEntry, ControlLibrary},
    control_preset::ControlPreset,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};
pub const FOLDER_KEY: &str = "confuse.control_library.folder";

pub struct Draft {
    pub document: ControlPreset,
    pub source: String,
    pub id: u64,
    saved: String,
    undo: Vec<ControlPreset>,
    redo: Vec<ControlPreset>,
}
impl Draft {
    pub fn new(
        document: ControlPreset,
        source: String,
        saved: bool,
        id: u64,
    ) -> Result<Self, String> {
        let xml = document.to_xml().map_err(|e| e.to_string())?;
        Ok(Self {
            document,
            source,
            id,
            saved: if saved { xml } else { String::new() },
            undo: vec![],
            redo: vec![],
        })
    }
    pub fn dirty(&self) -> bool {
        self.document.to_xml().is_ok_and(|xml| xml != self.saved)
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn saved(&mut self, xml: String) {
        self.saved = xml;
    }
    pub fn edit(
        &mut self,
        change: impl FnOnce(&mut ControlPreset) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut candidate = self.document.clone();
        change(&mut candidate)?;
        if candidate.to_xml().map_err(|e| e.to_string())?
            != self.document.to_xml().map_err(|e| e.to_string())?
        {
            self.undo
                .push(std::mem::replace(&mut self.document, candidate));
            self.redo.clear();
            while self.undo.len() > 200
                || self
                    .undo
                    .iter()
                    .map(|doc| doc.to_xml().map_or(0, |xml| xml.len()))
                    .sum::<usize>()
                    > 16 * 1024 * 1024
            {
                self.undo.remove(0);
            }
        }
        Ok(())
    }
    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.document, previous));
        }
    }
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.document, next));
        }
    }
}
#[derive(Clone)]
pub enum Operation {
    Scan,
    Open(ControlEntry),
    Save(ControlPreset, u64),
    Duplicate(ControlEntry),
    Delete(ControlEntry),
    Recover(ControlEntry),
    Import(PathBuf, Option<(u16, u8)>),
    OpenFile(PathBuf, Option<(u16, u8)>),
    Export(ControlPreset, u64, PathBuf),
}
struct Completion {
    generation: u64,
    result: Result<Completed, String>,
}
struct Completed {
    entries: Vec<ControlEntry>,
    trash: Vec<ControlEntry>,
    diagnostics: Vec<String>,
    open: Option<(ControlPreset, String)>,
    saved: Option<(u64, String)>,
    notice: String,
}
#[derive(Default)]
pub struct Library {
    pub folder: Option<PathBuf>,
    pub entries: Vec<ControlEntry>,
    pub trash: Vec<ControlEntry>,
    pub diagnostics: Vec<String>,
    pub query: String,
    pub deleted: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub opened: Option<(ControlPreset, String)>,
    pub saved: Option<(u64, String)>,
    generation: u64,
    worker: Option<Receiver<Completion>>,
    rescan: bool,
}
impl Library {
    pub fn busy(&self) -> bool {
        self.worker.is_some() || self.rescan
    }
    pub fn set_folder(&mut self, folder: PathBuf) {
        self.folder = Some(folder);
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
        self.trash.clear();
        self.error = None;
        self.notice = None;
        self.opened = None;
        self.saved = None;
        self.rescan = true;
        self.poll();
    }
    pub fn start(&mut self, operation: Operation) {
        if self.busy() {
            self.error = Some("Wait for the current library operation".into());
            return;
        }
        let folder = self.folder.clone();
        let generation = self.generation;
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        self.error = None;
        std::thread::spawn(move || {
            let result = execute(folder, operation);
            let _ = tx.send(Completion { generation, result });
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
                                self.opened = done.open;
                                self.saved = done.saved;
                                self.notice = Some(done.notice);
                                self.error = None;
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.error = Some("Library worker stopped".into());
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
    let mut open = None;
    let mut saved = None;
    let library = folder
        .map(ControlLibrary::open)
        .transpose()
        .map_err(|e| e.to_string())?;
    let require = || {
        library
            .as_ref()
            .ok_or("Choose an FX/VOICE library folder first".to_string())
    };
    let notice = match operation {
        Operation::Scan => "Library refreshed".into(),
        Operation::Open(entry) => {
            let document = require()?.load(&entry).map_err(|e| e.to_string())?;
            open = Some((document, entry.name));
            "Opened library copy".into()
        }
        Operation::Save(document, id) => {
            let entry = require()?.save(&document).map_err(|e| e.to_string())?;
            saved = Some((id, document.to_xml().map_err(|e| e.to_string())?));
            format!("Saved {} in the library", entry.name)
        }
        Operation::Duplicate(entry) => {
            let entry = require()?.duplicate(&entry).map_err(|e| e.to_string())?;
            format!("Duplicated {}", entry.name)
        }
        Operation::Delete(entry) => {
            require()?.delete(&entry).map_err(|e| e.to_string())?;
            "Moved to recoverable trash".into()
        }
        Operation::Recover(entry) => {
            require()?.restore(&entry).map_err(|e| e.to_string())?;
            "Recovered library entry".into()
        }
        Operation::Import(path, profile) => {
            let entry = require()?
                .import_file(path, profile)
                .map_err(|e| e.to_string())?;
            format!("Imported {}", entry.name)
        }
        Operation::OpenFile(path, profile) => {
            let document = openfuse_core::control_library::read_document(&path, profile)
                .map_err(|e| e.to_string())?;
            open = Some((document, path.display().to_string()));
            "Opened local file".into()
        }
        Operation::Export(document, id, path) => {
            openfuse_core::control_library::export_document(&document, &path)
                .map_err(|e| e.to_string())?;
            saved = Some((id, document.to_xml().map_err(|e| e.to_string())?));
            format!("Exported {}", path.display())
        }
    };
    let (entries, trash, diagnostics) = if let Some(library) = library {
        let scan = library.scan().map_err(|e| e.to_string())?;
        let trash = library.trash().map_err(|e| e.to_string())?;
        let mut diagnostics = scan.diagnostics;
        diagnostics.extend(trash.diagnostics);
        if scan.truncated || trash.truncated {
            diagnostics.push("Library scan reached its bounded limit; narrow the folder".into());
        }
        (scan.entries, trash.entries, diagnostics)
    } else {
        (vec![], vec![], vec![])
    };
    Ok(Completed {
        entries,
        trash,
        diagnostics,
        open,
        saved,
        notice,
    })
}
