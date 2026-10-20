//! Local, immutable media copies and recoverable catalog removal. No USB writes.
//! File type is an extension inventory hint, never a claim of decodability.
use crate::archive::ArchiveCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
pub const MAX_MEDIA_BYTES: u64 = 128 * 1024 * 1024;
const MAX_MANIFEST: u64 = 16 * 1024;
const MAX_SCAN_FILES: usize = 4096;
#[derive(Debug)]
pub struct MediaError(String);
impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for MediaError {}
pub type Result<T> = std::result::Result<T, MediaError>;
fn error(e: impl fmt::Display) -> MediaError {
    MediaError(e.to_string())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub use crate::media_format::MediaFormat;
#[derive(Debug, Clone)]
pub struct MediaEntry {
    pub id: String,
    pub name: String,
    pub original_path: PathBuf,
    pub format: MediaFormat,
    pub size: u64,
    pub sha256: String,
    pub imported_at_ms: u64,
    pub trashed: bool,
    /// Managed payload path, informational only; use `read` for verified playback.
    pub path: PathBuf,
    fingerprint: String,
}
impl MediaEntry {
    pub fn matches_query(&self, query: &str) -> bool {
        let text = format!(
            "{} {} {:?}",
            self.name,
            self.original_path.display(),
            self.format
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|part| text.contains(part))
    }
}
#[derive(Debug, Clone)]
pub struct MediaDiagnostic {
    pub path: PathBuf,
    pub message: String,
}
#[derive(Debug, Default)]
pub struct MediaScan {
    pub entries: Vec<MediaEntry>,
    pub diagnostics: Vec<MediaDiagnostic>,
    pub truncated: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    name: String,
    original_path: PathBuf,
    format: MediaFormat,
    size: u64,
    sha256: String,
    imported_at_ms: u64,
}
pub struct MediaLibrary {
    directory: ArchiveCatalog,
}
impl MediaLibrary {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            directory: ArchiveCatalog::open(path).map_err(error)?,
        })
    }
    pub fn root(&self) -> &Path {
        self.directory.root()
    }
    /// Import a flat source folder; subdirectories are never traversed.
    /// Partial successes remain cataloged when another file fails. Limits apply
    /// to directory entries, track count and total copied bytes per operation.
    pub fn import_directory(&self, path: impl AsRef<Path>) -> Result<MediaScan> {
        let source = ArchiveCatalog::open(path).map_err(error)?;
        let (mut names, truncated) = source.bounded_names(MAX_SCAN_FILES).map_err(error)?;
        names.sort();
        let mut result = MediaScan {
            truncated,
            ..Default::default()
        };
        let owned = if source.root() == self.root() {
            self.owned_payloads()?
        } else {
            HashSet::new()
        };
        let mut bytes = 0;
        for name in names {
            if MediaFormat::from_path(&name).is_none() {
                continue;
            }
            if owned.contains(&name) {
                continue;
            }
            if result.entries.len() >= 100 || bytes >= 512 * 1024 * 1024 {
                result.truncated = true;
                break;
            }
            let path = source.root().join(&name);
            // Refuse a moved source directory between discovery and import.
            source.ensure_root().map_err(error)?;
            let remaining = 512 * 1024 * 1024 - bytes;
            if source
                .regular_file_size(&name)
                .is_ok_and(|size| size > remaining)
            {
                result.truncated = true;
                result.diagnostics.push(MediaDiagnostic {
                    path,
                    message: "Track exceeds remaining 512 MiB folder import budget".into(),
                });
                continue;
            }
            match self.import_file_bounded(&path, remaining.min(MAX_MEDIA_BYTES), Some(&owned)) {
                Ok(entry) => {
                    bytes += entry.size;
                    result.entries.push(entry);
                }
                Err(e) => result.diagnostics.push(MediaDiagnostic {
                    path,
                    message: e.to_string(),
                }),
            }
        }
        Ok(result)
    }
    pub fn import_file(&self, path: impl AsRef<Path>) -> Result<MediaEntry> {
        self.import_file_bounded(path.as_ref(), MAX_MEDIA_BYTES, None)
    }
    /// Import the exact verified download; refuse external edits before publishing.
    pub fn import_file_checked(
        &self,
        path: impl AsRef<Path>,
        expected_sha256: &str,
    ) -> Result<MediaEntry> {
        self.import_file_expected(path.as_ref(), MAX_MEDIA_BYTES, None, Some(expected_sha256))
    }
    fn import_file_bounded(
        &self,
        path: &Path,
        limit: u64,
        owned: Option<&HashSet<PathBuf>>,
    ) -> Result<MediaEntry> {
        self.import_file_expected(path, limit, owned, None)
    }
    fn import_file_expected(
        &self,
        path: &Path,
        limit: u64,
        owned: Option<&HashSet<PathBuf>>,
        expected: Option<&str>,
    ) -> Result<MediaEntry> {
        self.directory.ensure_root().map_err(error)?;
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_err(error)?.join(path)
        };
        let format = MediaFormat::from_path(&absolute)
            .ok_or_else(|| error("Choose WAV, MP3 or MIDI media"))?;
        let name = absolute
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| error("Media filename must be UTF-8"))?;
        if name.len() > 1024 || absolute.as_os_str().len() > 4096 {
            return Err(error("Media path exceeds catalog limits"));
        }
        let source = ArchiveCatalog::open(
            absolute
                .parent()
                .ok_or_else(|| error("Missing source directory"))?,
        )
        .map_err(error)?;
        let managed = if source.root() != self.root() {
            false
        } else if let Some(owned) = owned {
            owned.contains(Path::new(name))
        } else if looks_like_payload(Path::new(name)) {
            self.owned_payloads()?.contains(Path::new(name))
        } else {
            false
        };
        if managed {
            return Err(error(
                "This file is already a managed media copy; select its catalog entry",
            ));
        }
        let bytes = source.read_file(Path::new(name), limit).map_err(error)?;
        if bytes.is_empty() {
            return Err(error("Media file is empty"));
        }
        if expected.is_some_and(|hash| hash != digest(&bytes)) {
            return Err(error("Downloaded media changed since its verified receipt"));
        }
        let manifest = Manifest {
            version: 1,
            name: name.into(),
            original_path: absolute.clone(),
            format,
            size: bytes.len() as u64,
            sha256: digest(&bytes),
            imported_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64,
        };
        // Stable identity makes reimport of the same named source idempotent.
        let id = manifest_id(&manifest)?;
        let filename = manifest_name(&id);
        if self.directory.file_exists(&filename).map_err(error)? {
            let found = self.entry(&id)?;
            self.read(&found)?;
            return Ok(found);
        }
        let payload = payload_name(&manifest.sha256, format);
        if !self.directory.file_exists(&payload).map_err(error)?
            && let Err(publish_error) = self.directory.publish_new(&payload, &bytes)
        {
            let existing = self
                .directory
                .read_file(&payload, MAX_MEDIA_BYTES)
                .map_err(|_| error(publish_error))?;
            if existing != bytes {
                return Err(error("Managed media changed during import"));
            }
        }
        let retained = self
            .directory
            .read_file(&payload, MAX_MEDIA_BYTES)
            .map_err(error)?;
        if retained != bytes {
            return Err(error(
                "Managed media payload does not match imported source",
            ));
        }
        let json = serde_json::to_vec(&manifest).map_err(error)?;
        if json.len() as u64 > MAX_MANIFEST {
            return Err(error("Media manifest exceeds size limit"));
        }
        if let Err(publish_error) = self.directory.publish_new(&filename, &json) {
            return self.entry(&id).map_err(|_| error(publish_error));
        }
        self.entry(&id)
    }
    // Ownership requires a valid catalog manifest in this exact library. An
    // external file with a similar name remains an ordinary importable source.
    fn owned_payloads(&self) -> Result<HashSet<PathBuf>> {
        let (names, truncated) = self
            .directory
            .bounded_names(MAX_SCAN_FILES)
            .map_err(error)?;
        if truncated {
            return Err(error(
                "Catalog scan limit prevents verifying managed media ownership",
            ));
        }
        let mut owned = HashSet::new();
        for candidate in names {
            let Some(id) = candidate
                .to_str()
                .and_then(|s| s.strip_suffix(".confuse-media.json"))
            else {
                continue;
            };
            // Ownership is independent of a recovery marker. A corrupted marker
            // cannot turn a retained payload into a new importable track.
            let Ok(bytes) = self.directory.read_file(&candidate, MAX_MANIFEST) else {
                continue;
            };
            let Ok(data) = serde_json::from_slice::<Manifest>(&bytes) else {
                continue;
            };
            if data.version == 1
                && valid_hash(&data.sha256)
                && manifest_id(&data).is_ok_and(|actual| actual == id)
            {
                owned.insert(payload_name(&data.sha256, data.format));
            }
        }
        Ok(owned)
    }
    pub fn scan(&self) -> Result<MediaScan> {
        self.scan_state(false)
    }
    pub fn trash(&self) -> Result<MediaScan> {
        self.scan_state(true)
    }
    fn scan_state(&self, trashed: bool) -> Result<MediaScan> {
        let (names, truncated) = self
            .directory
            .bounded_names(MAX_SCAN_FILES)
            .map_err(error)?;
        let mut scan = MediaScan {
            truncated,
            ..Default::default()
        };
        for name in names {
            let Some(id) = name
                .to_str()
                .and_then(|s| s.strip_suffix(".confuse-media.json"))
            else {
                continue;
            };
            match self.entry(id) {
                Ok(entry) if entry.trashed == trashed => scan.entries.push(entry),
                Ok(_) => (),
                Err(e) => scan.diagnostics.push(MediaDiagnostic {
                    path: self.root().join(name),
                    message: e.to_string(),
                }),
            }
        }
        scan.entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        Ok(scan)
    }
    fn entry(&self, id: &str) -> Result<MediaEntry> {
        if !valid_hash(id) {
            return Err(error("Invalid media identity"));
        }
        let bytes = self
            .directory
            .read_file(&manifest_name(id), MAX_MANIFEST)
            .map_err(error)?;
        let data: Manifest = serde_json::from_slice(&bytes).map_err(error)?;
        if data.version != 1
            || data.name.is_empty()
            || data.name.len() > 1024
            || data.name.contains(['/', '\\', '\0'])
            || data.original_path.as_os_str().len() > 4096
            || !valid_hash(&data.sha256)
            || data.size == 0
            || data.size > MAX_MEDIA_BYTES
            || manifest_id(&data)? != id
            || MediaFormat::from_path(Path::new(&data.name)) != Some(data.format)
        {
            return Err(error("Invalid media manifest"));
        }
        let fingerprint = digest(&bytes);
        let marker = marker_name(id);
        let trashed = self.directory.file_exists(&marker).map_err(error)?;
        if trashed
            && self.directory.read_file(&marker, 64).map_err(error)? != fingerprint.as_bytes()
        {
            return Err(error("Recovery marker does not match media manifest"));
        }
        let path = self.root().join(payload_name(&data.sha256, data.format));
        Ok(MediaEntry {
            id: id.into(),
            name: data.name,
            original_path: data.original_path,
            format: data.format,
            size: data.size,
            sha256: data.sha256,
            imported_at_ms: data.imported_at_ms,
            trashed,
            path,
            fingerprint,
        })
    }
    fn checked(&self, entry: &MediaEntry) -> Result<MediaEntry> {
        let current = self.entry(&entry.id)?;
        if current.fingerprint != entry.fingerprint
            || current.trashed != entry.trashed
            || current.path != entry.path
        {
            return Err(error(
                "Media changed or belongs to another library; refresh",
            ));
        }
        Ok(current)
    }
    pub fn read(&self, entry: &MediaEntry) -> Result<Vec<u8>> {
        let entry = self.checked(entry)?;
        let bytes = self
            .directory
            .read_file(&payload_name(&entry.sha256, entry.format), MAX_MEDIA_BYTES)
            .map_err(error)?;
        if bytes.len() as u64 != entry.size || digest(&bytes) != entry.sha256 {
            return Err(error("Media payload changed; playback refused"));
        }
        Ok(bytes)
    }
    pub fn delete(&self, entry: &MediaEntry) -> Result<MediaEntry> {
        let entry = self.checked(entry)?;
        if entry.trashed {
            return Err(error("Media is already in recovery"));
        }
        self.directory
            .publish_new(&marker_name(&entry.id), entry.fingerprint.as_bytes())
            .map_err(error)?;
        self.entry(&entry.id)
    }
    pub fn restore(&self, entry: &MediaEntry) -> Result<MediaEntry> {
        let entry = self.checked(entry)?;
        if !entry.trashed {
            return Err(error("Media is not in recovery"));
        }
        self.directory
            .remove_file(&marker_name(&entry.id))
            .map_err(error)?;
        self.entry(&entry.id)
    }
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn manifest_id(data: &Manifest) -> Result<String> {
    Ok(digest(
        &serde_json::to_vec(&(
            &data.name,
            &data.original_path,
            data.format,
            data.size,
            &data.sha256,
        ))
        .map_err(error)?,
    ))
}
fn manifest_name(id: &str) -> PathBuf {
    PathBuf::from(format!("{id}.confuse-media.json"))
}
fn marker_name(id: &str) -> PathBuf {
    PathBuf::from(format!("{id}.media-deleted"))
}
fn payload_name(hash: &str, format: MediaFormat) -> PathBuf {
    PathBuf::from(format!("{hash}.media.{}", format.extension()))
}

fn looks_like_payload(name: &Path) -> bool {
    name.to_str()
        .and_then(|s| s.split_once(".media."))
        .is_some_and(|(hash, extension)| {
            valid_hash(hash) && matches!(extension, "wav" | "mp3" | "mid")
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remaining_byte_budget_refuses_before_any_publication() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let path = source.path().join("track.wav");
        std::fs::write(&path, b"123456789").unwrap();
        let library = MediaLibrary::open(root.path()).unwrap();
        assert!(library.import_file_bounded(&path, 8, None).is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        let imported = library.import_file_bounded(&path, 9, None).unwrap();
        assert_eq!(library.read(&imported).unwrap(), b"123456789");
    }
}
