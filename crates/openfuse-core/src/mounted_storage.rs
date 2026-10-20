//! Explicit mounted G-DEC storage. No USB mode switch or implicit mount access.
use serde::Serialize;
use std::{
    fmt,
    path::{Path, PathBuf},
};
mod fs;
mod inventory;
mod plan;
mod preset;
pub use plan::{CollisionPolicy, Direction, TransferItem, TransferPlan, TransferReceipt};
pub use preset::fix_gdec_tag;
pub const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_BATCH_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 4096;
#[derive(Debug)]
pub struct StorageError(String);
impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for StorageError {}
pub type Result<T> = std::result::Result<T, StorageError>;
fn error(e: impl fmt::Display) -> StorageError {
    StorageError(e.to_string())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Volume {
    Internal,
    Sd,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FileKind {
    Audio,
    Phrase,
    Midi,
    Preset,
}
impl FileKind {
    pub fn folder(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Phrase => "Phrase",
            Self::Midi => "Midi",
            Self::Preset => "Presets",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        match Path::new(name)
            .extension()?
            .to_str()?
            .to_ascii_lowercase()
            .as_str()
        {
            "wav" | "mp3" => Some(Self::Audio),
            "mid" => Some(Self::Midi),
            "fuse" => Some(Self::Preset),
            _ => None,
        }
    }
}
/// Original BaseDevice.GetStorageLocation transfer subset; root/firmware paths excluded.
pub fn resolve_location(location: u8) -> Result<(Volume, &'static str)> {
    match location {
        0 => Ok((Volume::Internal, "Audio")),
        1 => Ok((Volume::Sd, "Audio")),
        2 => Ok((Volume::Internal, "Phrase")),
        3 => Ok((Volume::Sd, "Phrase")),
        4 => Ok((Volume::Internal, "Midi")),
        5 => Ok((Volume::Sd, "Midi")),
        7 => Ok((Volume::Sd, "Presets")),
        _ => Err(error(
            "Location is outside the supported mounted-file transfer scope",
        )),
    }
}
#[derive(Debug, Clone)]
pub struct MountedStorage {
    root: fs::Folder,
    volume: Volume,
}
impl MountedStorage {
    pub fn open(root: impl AsRef<Path>, volume: Volume) -> Result<Self> {
        Ok(Self {
            root: fs::Folder::open(root.as_ref())?,
            volume,
        })
    }
    pub fn root(&self) -> &Path {
        self.root.path()
    }
    pub fn volume(&self) -> Volume {
        self.volume
    }
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>> {
        self.root.ensure()?;
        if entry.root_identity != self.root.identity()
            || entry.root_path != self.root.path()
            || entry.volume != self.volume
        {
            return Err(error("Entry belongs to another mounted root"));
        }
        if entry.name != entry.source.name
            || entry.size != entry.source.size
            || entry.relative_path
                != entry
                    .source
                    .path()
                    .strip_prefix(self.root.path())
                    .map_err(error)?
        {
            return Err(error("Entry metadata changed since inventory"));
        }
        let bytes = entry.source.read()?;
        if fs::digest(&bytes) != entry.sha256 {
            return Err(error("Mounted file changed; refresh inventory"));
        }
        Ok(bytes)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub relative_path: PathBuf,
    pub name: String,
    pub kind: FileKind,
    pub size: u64,
    pub sha256: String,
    pub preset_name: Option<String>,
    #[serde(skip)]
    source: fs::Source,
    #[serde(skip)]
    root_identity: String,
    #[serde(skip)]
    root_path: PathBuf,
    #[serde(skip)]
    volume: Volume,
}
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub message: String,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Inventory {
    pub entries: Vec<Entry>,
    pub diagnostics: Vec<Diagnostic>,
    pub truncated: bool,
}
