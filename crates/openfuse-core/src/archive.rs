//! Local backup catalog. Original bytes remain in immutable checksummed bundles;
//! removal uses recoverable tombstones rather than deleting the source bundle.
use crate::{
    bank::{self, Backup, RestoreSelection, SlotMapping},
    hex,
    legacy_backup::LegacyBackup,
    library::{LibraryEntry, PresetLibrary},
    preset::PresetDocument,
};
use cap_std::{
    ambient_authority,
    fs::{Dir, Metadata, OpenOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
mod storage;
const MAX_ARCHIVE_BYTES: u64 = 136 * 1024 * 1024;
const MAX_SCAN_ENTRIES: usize = 1000;
const MAX_SCAN_BYTES: u64 = 512 * 1024 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[derive(Debug)]
pub struct ArchiveError(String);
impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ArchiveError {}
impl From<io::Error> for ArchiveError {
    fn from(value: io::Error) -> Self {
        Self(value.to_string())
    }
}
pub type Result<T> = std::result::Result<T, ArchiveError>;
fn error(value: impl fmt::Display) -> ArchiveError {
    ArchiveError(value.to_string())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveKind {
    Native,
    Auxiliary,
    ControlPreset,
    Legacy,
}
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub id: String,
    pub label: String,
    pub imported_at_ms: u64,
    pub pid: Option<u16>,
    pub families: Vec<u32>,
    pub preset_count: usize,
    pub original_bytes: u64,
    pub kind: ArchiveKind,
    pub names: Vec<String>,
    pub trashed: bool,
    pub path: PathBuf,
    fingerprint: String,
}
impl CatalogEntry {
    pub fn matches(&self, query: &str) -> bool {
        let model = self
            .pid
            .and_then(|pid| crate::catalog::find_device(0x1ed8, pid))
            .map(|d| d.name)
            .unwrap_or("");
        let families = self
            .families
            .iter()
            .filter_map(|id| crate::model_catalog::product(*id))
            .map(|p| p.name)
            .collect::<Vec<_>>()
            .join(" ");
        let text = format!(
            "{} {} {:?} {:?} {:?} {} {}",
            self.label,
            self.names.join(" "),
            self.families,
            self.pid,
            self.kind,
            model,
            families
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|term| text.contains(term))
    }
}
#[derive(Debug, Clone)]
pub struct CatalogDiagnostic {
    pub path: PathBuf,
    pub message: String,
}
#[derive(Debug, Clone, Default)]
pub struct CatalogScan {
    pub entries: Vec<CatalogEntry>,
    pub diagnostics: Vec<CatalogDiagnostic>,
    pub truncated: bool,
}
#[derive(Debug, Clone)]
pub struct ArchivePreset {
    pub slot: u16,
    pub document: PresetDocument,
}
#[derive(Debug, Clone)]
pub struct Archive {
    presets: Vec<ArchivePreset>,
    warnings: Vec<String>,
    native: Option<Backup>,
    legacy: Option<LegacyBackup>,
    auxiliary: Option<crate::auxiliary::Snapshot>,
    control_preset: Option<crate::control_preset::ControlPreset>,
}
impl Archive {
    /// Offline inspection without importing into a catalog or writing any files.
    pub fn read_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let meta = std::fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() {
            return Err(error("Archive source must not be a symbolic link"));
        }
        if meta.is_dir() {
            let legacy = LegacyBackup::read_directory(path).map_err(error)?;
            let presets = legacy
                .presets()
                .iter()
                .map(|p| ArchivePreset {
                    slot: p.slot(),
                    document: p.document().clone(),
                })
                .collect();
            let mut warnings = legacy.warnings().to_vec();
            warnings.push(
                "Exact USB PID is absent: explicitly select a compatible profile before restore"
                    .into(),
            );
            return Ok(Self {
                presets,
                warnings,
                native: None,
                legacy: Some(legacy),
                auxiliary: None,
                control_preset: None,
            });
        }
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let parent = absolute
            .parent()
            .ok_or_else(|| error("Archive file has no parent"))?;
        let name = absolute
            .file_name()
            .ok_or_else(|| error("Archive file has no name"))?;
        let directory = ArchiveCatalog::open(parent)?;
        let bytes = directory.read_file(Path::new(name), MAX_ARCHIVE_BYTES)?;
        if name.to_string_lossy().ends_with(".confuse-archive.json") {
            let bundle: Bundle = serde_json::from_slice(&bytes).map_err(error)?;
            if bundle.format != "confuse-archive"
                || bundle.version != 1
                || bundle.sha256 != bundle.checksum()?
            {
                return Err(error("Invalid catalog bundle format or checksum"));
            }
            return bundle.archive();
        }
        let text = std::str::from_utf8(&bytes).map_err(error)?;
        if serde_json::from_str::<serde_json::Value>(text)
            .map_err(error)?
            .get("format")
            .and_then(|v| v.as_str())
            == Some("confuse-auxiliary")
        {
            return Self::from_auxiliary(crate::auxiliary::Snapshot::parse(text).map_err(error)?);
        }
        if serde_json::from_str::<serde_json::Value>(text)
            .map_err(error)?
            .get("format")
            .and_then(|v| v.as_str())
            == Some("confuse-control-preset")
        {
            return Ok(Self::from_control_preset(
                crate::control_preset::ControlPreset::parse_json(text).map_err(error)?,
            ));
        }
        let backup = Backup::parse(std::str::from_utf8(&bytes).map_err(error)?).map_err(error)?;
        let presets = backup
            .slots()
            .iter()
            .map(|p| ArchivePreset {
                slot: p.slot(),
                document: p.document().clone(),
            })
            .collect();
        Ok(Self {
            presets,
            warnings: vec!["Preset-only backup; global settings are not restored".into()],
            native: Some(backup),
            legacy: None,
            auxiliary: None,
            control_preset: None,
        })
    }
    fn from_auxiliary(snapshot: crate::auxiliary::Snapshot) -> Result<Self> {
        let presets = snapshot
            .presets()
            .iter()
            .map(|p| ArchivePreset {
                slot: p.slot(),
                document: p.document().clone(),
            })
            .collect();
        Ok(Self {
            presets,
            warnings: snapshot.warnings().to_vec(),
            native: None,
            legacy: None,
            auxiliary: Some(snapshot),
            control_preset: None,
        })
    }
    fn from_control_preset(document: crate::control_preset::ControlPreset) -> Self {
        Self {
            presets: vec![],
            warnings: vec![],
            native: None,
            legacy: None,
            auxiliary: None,
            control_preset: Some(document),
        }
    }
    pub fn control_preset(&self) -> Option<&crate::control_preset::ControlPreset> {
        self.control_preset.as_ref()
    }
    pub fn auxiliary(&self) -> Option<&crate::auxiliary::Snapshot> {
        self.auxiliary.as_ref()
    }
    pub fn auxiliary_for_pid(&self, pid: u16) -> Result<crate::auxiliary::Snapshot> {
        if let Some(snapshot) = &self.auxiliary {
            if snapshot.pid() != pid {
                return Err(error("Archive PID must exactly match selected profile"));
            }
            return Ok(snapshot.clone());
        }
        crate::auxiliary::Snapshot::from_legacy(
            pid,
            self.legacy
                .as_ref()
                .ok_or_else(|| error("Archive has no auxiliary sections"))?,
        )
        .map_err(error)
    }
    pub fn presets(&self) -> &[ArchivePreset] {
        &self.presets
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn native(&self) -> Option<&Backup> {
        self.native.as_ref()
    }
    pub fn legacy(&self) -> Option<&LegacyBackup> {
        self.legacy.as_ref()
    }
    /// Native sources enforce their exact captured PID. Legacy folders have no
    /// proven USB PID; the caller must obtain an explicit compatible profile choice.
    pub fn restore_selection(
        &self,
        target_pid: u16,
        mappings: &[SlotMapping],
    ) -> Result<RestoreSelection> {
        if let Some(backup) = &self.native {
            return RestoreSelection::new(backup, target_pid, mappings).map_err(error);
        }
        if self
            .auxiliary
            .as_ref()
            .is_some_and(|s| s.pid() != target_pid)
        {
            return Err(error("Archive PID must exactly match destination"));
        }
        RestoreSelection::from_documents(
            target_pid,
            self.presets
                .iter()
                .map(|p| (p.slot, p.document.clone()))
                .collect(),
            target_pid,
            mappings,
        )
        .map_err(error)
    }
}
#[derive(Debug, Clone)]
pub struct ArchiveCatalog {
    root: PathBuf,
    directory: Arc<Dir>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFile {
    path: String,
    hex: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    format: String,
    version: u32,
    kind: ArchiveKind,
    label: String,
    imported_at_ms: u64,
    files: Vec<SourceFile>,
    sha256: String,
}
impl Bundle {
    fn checksum(&self) -> Result<String> {
        Ok(digest(
            &serde_json::to_vec(&(
                "confuse-archive",
                self.version,
                self.kind,
                &self.label,
                self.imported_at_ms,
                &self.files,
            ))
            .map_err(error)?,
        ))
    }
    fn id(&self) -> Result<String> {
        Ok(digest(
            &serde_json::to_vec(&(self.kind, &self.files)).map_err(error)?,
        ))
    }
    fn archive(&self) -> Result<Archive> {
        match self.kind {
            ArchiveKind::Native => {
                if self.files.len() != 1 {
                    return Err(error("Native archive requires exactly one original file"));
                }
                let bytes = source_bytes(&self.files[0].hex)?;
                let backup =
                    Backup::parse(std::str::from_utf8(&bytes).map_err(error)?).map_err(error)?;
                let presets = backup
                    .slots()
                    .iter()
                    .map(|p| ArchivePreset {
                        slot: p.slot(),
                        document: p.document().clone(),
                    })
                    .collect();
                Ok(Archive{presets,warnings:vec!["Preset archive only: global system settings and physical knob banks are not restored".into()],native:Some(backup),legacy:None,auxiliary:None,control_preset:None})
            }
            ArchiveKind::ControlPreset => {
                if self.files.len() != 1 {
                    return Err(error("Control preset archive requires one source file"));
                }
                let bytes = source_bytes(&self.files[0].hex)?;
                let document = crate::control_preset::ControlPreset::parse_json(
                    std::str::from_utf8(&bytes).map_err(error)?,
                )
                .map_err(error)?;
                Ok(Archive::from_control_preset(document))
            }
            ArchiveKind::Auxiliary => {
                if self.files.len() != 1 {
                    return Err(error(
                        "Auxiliary archive requires exactly one original file",
                    ));
                }
                let bytes = source_bytes(&self.files[0].hex)?;
                Archive::from_auxiliary(
                    crate::auxiliary::Snapshot::parse(std::str::from_utf8(&bytes).map_err(error)?)
                        .map_err(error)?,
                )
            }
            ArchiveKind::Legacy => {
                let entries = self
                    .files
                    .iter()
                    .map(|f| Ok((f.path.clone(), source_bytes(&f.hex)?)))
                    .collect::<Result<_>>()?;
                let legacy = LegacyBackup::from_entries(entries).map_err(error)?;
                let presets = legacy
                    .presets()
                    .iter()
                    .map(|p| ArchivePreset {
                        slot: p.slot(),
                        document: p.document().clone(),
                    })
                    .collect();
                let mut warnings = legacy.warnings().to_vec();
                warnings.push("Historical folders do not establish an exact USB PID. Select a compatible target profile explicitly; only selected preset XML is restorable".into());
                Ok(Archive {
                    presets,
                    warnings,
                    native: None,
                    legacy: Some(legacy),
                    auxiliary: None,
                    control_preset: None,
                })
            }
        }
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn source_hex(bytes: &[u8]) -> String {
    hex::encode(bytes).replace(' ', "")
}
fn source_bytes(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error("Archive source must use canonical hexadecimal bytes"));
    }
    if value.is_empty() {
        Ok(Vec::new())
    } else {
        hex::decode(value).map_err(error)
    }
}
