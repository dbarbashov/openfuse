//! Read-only import of historical FUSE backup directories, preserving every file.
//! No raw USB replies are fabricated from these XML-only archives.
use crate::preset::{MAX_PRESET_BYTES, PresetDocument};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt, path::Path};
mod directory;
mod inventory;

pub const MAX_LEGACY_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_LEGACY_FILES: usize = 4096;
pub const MAX_LEGACY_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyError(String);
impl fmt::Display for LegacyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for LegacyError {}
pub type Result<T> = std::result::Result<T, LegacyError>;
fn error(value: impl fmt::Display) -> LegacyError {
    LegacyError(value.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Description,
    Preset,
    FuseMetadata,
    EffectBank,
    VoiceBank,
    SystemSettings,
    BandTrack,
    Unknown,
}

/// Metadata is descriptive; only parsed Presets are eligible for extraction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EntryMetadata {
    pub name: Option<String>,
    pub amplifier: Option<String>,
    pub xml_root: Option<String>,
    pub slot: Option<u16>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct LegacyEntry {
    path: String,
    bytes: Vec<u8>,
    sha256: String,
    kind: EntryKind,
    metadata: EntryMetadata,
}
impl LegacyEntry {
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn kind(&self) -> EntryKind {
        self.kind
    }
    pub fn metadata(&self) -> &EntryMetadata {
        &self.metadata
    }
}
#[derive(Debug, Clone)]
pub struct LegacyPreset {
    path: String,
    slot: u16,
    document: PresetDocument,
}
impl LegacyPreset {
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn slot(&self) -> u16 {
        self.slot
    }
    pub fn document(&self) -> &PresetDocument {
        &self.document
    }
}
#[derive(Debug, Clone)]
pub struct LegacyBackup {
    entries: Vec<LegacyEntry>,
    presets: Vec<LegacyPreset>,
    description: Option<String>,
    source_name: Option<String>,
    warnings: Vec<String>,
}
impl LegacyBackup {
    pub fn read_directory(root: &Path) -> Result<Self> {
        directory::read(root)
    }
    /// Paths are archive-relative POSIX paths. Every input byte survives unchanged.
    pub fn from_entries(mut inputs: Vec<(String, Vec<u8>)>) -> Result<Self> {
        if inputs.is_empty() || inputs.len() > MAX_LEGACY_FILES {
            return Err(error("Legacy backup requires 1..4096 files"));
        }
        inputs.sort_by(|a, b| a.0.cmp(&b.0));
        let mut paths = BTreeSet::new();
        let mut slots = BTreeSet::new();
        let mut total = 0usize;
        let mut entries = Vec::new();
        let mut presets = Vec::new();
        let mut description = None;
        let mut warnings = Vec::new();
        for (path, bytes) in inputs {
            validate_path(&path)?;
            if !paths.insert(path.to_ascii_lowercase()) {
                return Err(error("Duplicate or case-colliding legacy path"));
            }
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| error("Legacy backup size overflow"))?;
            if total > MAX_LEGACY_BYTES {
                return Err(error("Legacy backup exceeds 64 MiB"));
            }
            let (kind, mut metadata) = inventory::inspect(&path, &bytes);
            if kind == EntryKind::Preset {
                if bytes.len() > MAX_PRESET_BYTES {
                    return Err(error(format!("Preset {path} exceeds 2 MiB")));
                }
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| error(format!("Preset {path} is not UTF-8")))?;
                let document = PresetDocument::parse(text)
                    .map_err(|e| error(format!("Preset {path}: {e}")))?;
                let slot = metadata.slot.expect("classified preset has slot");
                if !slots.insert(slot) {
                    return Err(error(format!("Duplicate preset slot {slot}")));
                }
                metadata.name = Some(document.metadata().name);
                if let Some((_, filename_name)) = path
                    .rsplit('/')
                    .next()
                    .and_then(|name| name.strip_suffix(".fuse"))
                    .and_then(|stem| stem.split_once('_'))
                    && filename_name != document.metadata().name
                {
                    metadata.warnings.push("Filename title differs from embedded metadata; numeric slot and embedded name are retained separately".into());
                }
                if ![4, 5, 0x10, 0x14, 0x16, 0x12, 0xa, 0xe, 0x26]
                    .into_iter()
                    .any(|pid| crate::wire_preset::product_for_pid(pid) == document.product_id())
                {
                    metadata.warnings.push("No implemented USB bank profile for this preset family; preservation/extraction only".into());
                }

                if document.product_id().is_none() {
                    metadata.warnings.push(
                        "Preset has no ProductId; device compatibility cannot be inferred".into(),
                    );
                }
                presets.push(LegacyPreset {
                    path: path.clone(),
                    slot,
                    document,
                });
            } else if kind == EntryKind::Description {
                let text = decode_description(&bytes)
                    .map_err(|e| error(format!("Description {path}: {e}")))?;
                if description.is_some() {
                    return Err(error(
                        "Multiple backup descriptions make the archive ambiguous",
                    ));
                }
                description = Some(text);
            }
            for warning in &metadata.warnings {
                warnings.push(format!("{path}: {warning}"));
            }
            let sha256 = format!("{:x}", Sha256::digest(&bytes));
            entries.push(LegacyEntry {
                path,
                bytes,
                sha256,
                kind,
                metadata,
            });
        }
        let has_evidence = entries.iter().any(|e| {
            matches!(
                e.kind,
                EntryKind::Description
                    | EntryKind::Preset
                    | EntryKind::SystemSettings
                    | EntryKind::VoiceBank
                    | EntryKind::EffectBank
            )
        });
        if !has_evidence {
            return Err(error(
                "Directory contains no recognizable FUSE backup entries",
            ));
        }
        for path in &paths {
            let mut prefix = String::new();
            for part in path.split('/').take(path.split('/').count() - 1) {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                if paths.contains(&prefix) {
                    return Err(error("Legacy file path is also a directory prefix"));
                }
            }
        }
        presets.sort_by_key(LegacyPreset::slot);
        if description.is_none() {
            warnings.push("No BackupName.fuse description; files are preserved without inventing a backup title".into());
        }
        if presets.is_empty() {
            warnings.push(
                "No complete presets: auxiliary banks/settings are preserved for inspection only"
                    .into(),
            );
        }
        if entries
            .iter()
            .any(|e| !matches!(e.kind, EntryKind::Preset | EntryKind::Description))
        {
            warnings.push("Auxiliary files are preserved; whole restore requires complete exact-profile coverage, and media remains local".into());
        }
        let families = presets
            .iter()
            .filter_map(|p| p.document.product_id())
            .collect::<BTreeSet<_>>();
        if families.len() > 1 {
            warnings.push("Archive contains multiple preset families; compatibility must be checked for each selected preset".into());
        }
        Ok(Self {
            entries,
            presets,
            description,
            source_name: None,
            warnings,
        })
    }
    pub fn entries(&self) -> &[LegacyEntry] {
        &self.entries
    }
    pub fn presets(&self) -> &[LegacyPreset] {
        &self.presets
    }
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    /// Original directory label; date interpretation is not required to import.
    pub fn source_name(&self) -> Option<&str> {
        self.source_name.as_deref()
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn total_bytes(&self) -> usize {
        self.entries.iter().map(|e| e.bytes.len()).sum()
    }
}
fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 1024
        || path.contains(['\\', '\0', ':'])
        || path.chars().any(char::is_control)
    {
        return Err(error("Invalid legacy relative path"));
    }
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() > MAX_LEGACY_DEPTH
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return Err(error("Unsafe or overly deep legacy path"));
    }
    Ok(())
}
fn decode_description(bytes: &[u8]) -> Result<String> {
    if bytes.len() > 64 * 1024 {
        return Err(error("Backup description exceeds 64 KiB"));
    }
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err(error("Truncated UTF-16 description"));
        }
        let le = bytes[0] == 0xff;
        let values = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if le {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect::<Vec<_>>();
        return String::from_utf16(&values).map_err(error);
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    std::str::from_utf8(bytes).map(str::to_owned).map_err(error)
}
