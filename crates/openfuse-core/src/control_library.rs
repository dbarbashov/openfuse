//! Immutable local versions of editable physical-bank presets. Uses the archive
//! catalog's capability-based, checksummed, no-clobber storage and recovery.
use crate::{
    archive::{Archive, ArchiveCatalog, ArchiveKind, CatalogEntry, CatalogScan},
    control_preset::ControlPreset,
};
use std::{
    fmt,
    path::{Path, PathBuf},
};
pub const MAX_CONTROL_FILE_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Debug)]
pub struct LibraryError(String);
impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for LibraryError {}
pub type Result<T> = std::result::Result<T, LibraryError>;
fn error(e: impl fmt::Display) -> LibraryError {
    LibraryError(e.to_string())
}

#[derive(Debug, Clone)]
pub struct ControlEntry {
    pub catalog: CatalogEntry,
    pub pid: u16,
    pub control: u8,
    pub name: String,
}
impl ControlEntry {
    pub fn matches(&self, query: &str) -> bool {
        let profile = crate::catalog::find_device(0x1ed8, self.pid).map_or("", |d| d.name);
        let bank = match (self.pid, self.control) {
            (4 | 20, 1) => "MOD",
            (4 | 20, 2) => "DLY RVB",
            (14, 1) => "VOICE",
            _ => "FX",
        };
        let text = format!(
            "{} {} {profile} {bank} {:04x} {}",
            self.name, self.catalog.label, self.pid, self.control
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|term| text.contains(term))
    }
}
#[derive(Debug, Clone, Default)]
pub struct LibraryScan {
    pub entries: Vec<ControlEntry>,
    pub diagnostics: Vec<String>,
    pub truncated: bool,
}
#[derive(Debug, Clone)]
pub struct ControlLibrary {
    catalog: ArchiveCatalog,
}
impl ControlLibrary {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            catalog: ArchiveCatalog::open(root).map_err(error)?,
        })
    }
    pub fn root(&self) -> &Path {
        self.catalog.root()
    }
    pub fn scan(&self) -> Result<LibraryScan> {
        self.convert(self.catalog.scan().map_err(error)?)
    }
    pub fn trash(&self) -> Result<LibraryScan> {
        self.convert(self.catalog.trash().map_err(error)?)
    }
    fn convert(&self, scan: CatalogScan) -> Result<LibraryScan> {
        let mut result = LibraryScan {
            diagnostics: scan
                .diagnostics
                .into_iter()
                .map(|d| format!("{}: {}", d.path.display(), d.message))
                .collect(),
            truncated: scan.truncated,
            ..Default::default()
        };
        for entry in scan
            .entries
            .into_iter()
            .filter(|e| e.kind == ArchiveKind::ControlPreset)
        {
            match self.entry(entry.clone()) {
                Ok(entry) => result.entries.push(entry),
                Err(e) => result
                    .diagnostics
                    .push(format!("{}: {e}", entry.path.display())),
            }
        }
        result.entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.catalog.id.cmp(&b.catalog.id))
        });
        Ok(result)
    }
    fn entry(&self, catalog: CatalogEntry) -> Result<ControlEntry> {
        let archive = self.catalog.load(&catalog).map_err(error)?;
        let document = archive
            .control_preset()
            .ok_or_else(|| error("Catalog entry is not an individual control preset"))?;
        Ok(ControlEntry {
            pid: document.pid(),
            control: document.control(),
            name: document.name(),
            catalog,
        })
    }
    pub fn load(&self, entry: &ControlEntry) -> Result<ControlPreset> {
        let archive = self.catalog.load(&entry.catalog).map_err(error)?;
        let document = archive
            .control_preset()
            .ok_or_else(|| error("Not a control preset"))?;
        if document.pid() != entry.pid
            || document.control() != entry.control
            || document.name() != entry.name
        {
            return Err(error("Control preset identity changed; refresh library"));
        }
        Ok(document.clone())
    }
    /// Save a new immutable version. Identical documents reuse their existing
    /// entry; saving an identical deleted document recovers that entry.
    pub fn save(&self, document: &ControlPreset) -> Result<ControlEntry> {
        let mut entry = self
            .catalog
            .import_control_preset(document)
            .map_err(error)?;
        if entry.trashed {
            entry = self.catalog.restore(&entry).map_err(error)?;
        }
        self.entry(entry)
    }
    pub fn duplicate(&self, entry: &ControlEntry) -> Result<ControlEntry> {
        let document = self.load(entry)?;
        self.entry(
            self.catalog
                .duplicate_control_preset(&document)
                .map_err(error)?,
        )
    }
    pub fn delete(&self, entry: &ControlEntry) -> Result<ControlEntry> {
        self.load(entry)?;
        self.entry(self.catalog.delete(&entry.catalog).map_err(error)?)
    }
    pub fn restore(&self, entry: &ControlEntry) -> Result<ControlEntry> {
        self.load(entry)?;
        self.entry(self.catalog.restore(&entry.catalog).map_err(error)?)
    }
    pub fn import_file(
        &self,
        path: impl AsRef<Path>,
        profile: Option<(u16, u8)>,
    ) -> Result<ControlEntry> {
        self.save(&read_document(path, profile)?)
    }
    pub fn export(&self, entry: &ControlEntry, path: impl AsRef<Path>) -> Result<()> {
        export_document(&self.load(entry)?, path)
    }
}
fn file_parts(path: &Path) -> Result<(ArchiveCatalog, PathBuf)> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| error("File has no parent directory"))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| error("File has no filename"))?;
    Ok((
        ArchiveCatalog::open(parent).map_err(error)?,
        PathBuf::from(name),
    ))
}
pub fn read_document(path: impl AsRef<Path>, profile: Option<(u16, u8)>) -> Result<ControlPreset> {
    let path = path.as_ref();
    let (directory, name) = file_parts(path)?;
    let bytes = directory
        .read_file(&name, MAX_CONTROL_FILE_BYTES)
        .map_err(error)?;
    let text = std::str::from_utf8(&bytes).map_err(error)?;
    if text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with('<')
    {
        let (pid, control) = profile.ok_or_else(|| {
            error("Standalone FUSE XML requires an explicit PID and physical bank")
        })?;
        return ControlPreset::parse(pid, control, text).map_err(error);
    }
    if profile.is_some() {
        return Err(error("Native preset identity cannot be overridden"));
    }
    if name.to_string_lossy().ends_with(".confuse-archive.json") {
        let archive = Archive::read_path(path).map_err(error)?;
        return archive
            .control_preset()
            .cloned()
            .ok_or_else(|| error("Archive is not an individual control preset"));
    }
    ControlPreset::parse_json(text).map_err(error)
}
/// Publish one new file atomically. Existing destinations are never overwritten.
pub fn export_document(document: &ControlPreset, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = match extension.as_str() {
        "fuse" | "xml" => document.to_xml().map_err(error)?,
        "json" => document.to_json().map_err(error)?,
        _ => return Err(error("Export requires .fuse, .xml or .json extension")),
    };
    if text.len() as u64 > MAX_CONTROL_FILE_BYTES {
        return Err(error("Control preset file exceeds size limit"));
    }
    let (directory, name) = file_parts(path)?;
    directory.publish_new(&name, text.as_bytes()).map_err(error)
}
