//! A bounded, offline preset library. All library I/O is relative to a directory
//! capability; links cannot grant access outside the selected folder. Files are
//! never overwritten, and removal retains both XML and a restoration manifest.

use std::{
    ffi::OsString,
    fmt,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use cap_std::{
    ambient_authority,
    fs::{Dir, Metadata, OpenOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::preset::{MAX_PRESET_BYTES, PresetDocument, PresetMetadata};

const TRASH: &str = ".confuse-trash";
const TEMP_PREFIX: &str = ".confuse-write-";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub const DEFAULT_SCAN_LIMITS: ScanLimits = ScanLimits {
    max_entries: 10_000,
    max_depth: 16,
    max_total_bytes: 256 * 1024 * 1024,
};

#[derive(Debug, Clone, Copy)]
pub struct ScanLimits {
    pub max_entries: usize,
    pub max_depth: usize,
    pub max_total_bytes: u64,
}

#[derive(Debug)]
pub struct LibraryError(String);
impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for LibraryError {}
impl From<io::Error> for LibraryError {
    fn from(value: io::Error) -> Self {
        Self(value.to_string())
    }
}
type Result<T> = std::result::Result<T, LibraryError>;
fn error(message: impl Into<String>) -> LibraryError {
    LibraryError(message.into())
}

#[derive(Debug, Clone)]
pub struct LibraryEntry {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub metadata: PresetMetadata,
    pub product_id: Option<u32>,
    pub fingerprint: String,
    identity: String,
}
impl LibraryEntry {
    /// Every whitespace-separated term must match somewhere in the metadata or path.
    pub fn matches(&self, query: &str) -> bool {
        let haystack = format!(
            "{} {} {} {} {}",
            self.metadata.name,
            self.metadata.author,
            self.metadata.tags,
            self.metadata.description,
            self.relative_path.display()
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|term| haystack.contains(term))
    }
}

#[derive(Debug, Clone)]
pub struct LibraryDiagnostic {
    pub path: PathBuf,
    pub message: String,
}
#[derive(Debug, Clone, Default)]
pub struct LibraryScan {
    pub entries: Vec<LibraryEntry>,
    pub diagnostics: Vec<LibraryDiagnostic>,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveToken {
    /// Absolute display path. Operations validate this against the selected root.
    pub path: PathBuf,
    pub original_relative_path: PathBuf,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct PresetLibrary {
    root: PathBuf,
    directory: Arc<Dir>,
}

impl PresetLibrary {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let metadata = std::fs::symlink_metadata(root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(error("Select a real directory, not a symbolic link"));
        }
        let root = root.canonicalize()?;
        let directory = Arc::new(Dir::open_ambient_dir(&root, ambient_authority())?);
        Ok(Self { root, directory })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn scan(&self) -> Result<LibraryScan> {
        self.scan_with_limits(DEFAULT_SCAN_LIMITS)
    }
    pub fn scan_with_limits(&self, limits: ScanLimits) -> Result<LibraryScan> {
        self.ensure_root()?;
        let mut result = LibraryScan::default();
        let mut pending = vec![(PathBuf::new(), 0)];
        let mut visited = 0usize;
        let mut bytes = 0u64;
        while let Some((relative, depth)) = pending.pop() {
            if !relative.as_os_str().is_empty()
                && let Err(reason) = self.check_components(&relative)
            {
                result.diagnostic(self.root.join(&relative), reason);
                continue;
            }
            let directory = if relative.as_os_str().is_empty() {
                self.directory.try_clone()
            } else {
                self.directory.open_dir(&relative)
            };
            let directory = match directory {
                Ok(directory) => directory,
                Err(reason) => {
                    result.diagnostic(self.root.join(&relative), reason);
                    continue;
                }
            };
            let entries = match directory.entries() {
                Ok(entries) => entries,
                Err(reason) => {
                    result.diagnostic(self.root.join(&relative), reason);
                    continue;
                }
            };
            for entry in entries {
                if visited >= limits.max_entries {
                    result.truncated = true;
                    break;
                }
                visited += 1;
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(reason) => {
                        result.diagnostic(self.root.join(&relative), reason);
                        continue;
                    }
                };
                let name = entry.file_name();
                if name == TRASH || name.to_string_lossy().starts_with(TEMP_PREFIX) {
                    continue;
                }
                let path = relative.join(name);
                let metadata = match self.directory.symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(reason) => {
                        result.diagnostic(self.root.join(&path), reason);
                        continue;
                    }
                };
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    if depth >= limits.max_depth {
                        result.truncated = true;
                    } else {
                        pending.push((path, depth + 1));
                    }
                    continue;
                }
                if !metadata.is_file() || !is_preset_path(&path) {
                    continue;
                }
                if metadata.len() > MAX_PRESET_BYTES as u64 {
                    result.diagnostic(self.root.join(&path), "Preset exceeds 2 MiB limit");
                    continue;
                }
                if bytes.saturating_add(metadata.len()) > limits.max_total_bytes {
                    result.truncated = true;
                    continue;
                }
                bytes += metadata.len();
                match self.load_with_limit(&path, limits.max_total_bytes - (bytes - metadata.len()))
                {
                    Ok((xml, entry)) => {
                        bytes = bytes - metadata.len() + xml.len() as u64;
                        result.entries.push(entry)
                    }
                    Err(reason) => result.diagnostic(self.root.join(&path), reason),
                }
            }
            if visited >= limits.max_entries && !pending.is_empty() {
                result.truncated = true;
                break;
            }
        }
        result
            .entries
            .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        result.diagnostics.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(result)
    }

    pub fn read(&self, entry: &LibraryEntry) -> Result<PresetDocument> {
        let (xml, _) = self.checked(entry)?;
        PresetDocument::parse(&xml).map_err(|e| error(e.to_string()))
    }
    pub fn import(&self, source: impl AsRef<Path>) -> Result<LibraryEntry> {
        self.ensure_root()?;
        let source = source.as_ref();
        let metadata = std::fs::symlink_metadata(source)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(error("Import requires a regular file, not a symbolic link"));
        }
        if !is_preset_path(source) {
            return Err(error("Import requires a .fuse or .xml file"));
        }
        // Resolve the selected parent, never the final file. Resolving the whole
        // path could follow a concurrently substituted symlink before confinement.
        let source = if source.is_absolute() {
            source.to_path_buf()
        } else {
            std::env::current_dir()?.join(source)
        };
        let parent = source
            .parent()
            .ok_or_else(|| error("Import has no parent directory"))?;
        let source_library = Self::open(parent)?;
        let name = source
            .file_name()
            .ok_or_else(|| error("Import has no filename"))?;
        let (xml, _) = source_library.load(Path::new(name))?;
        self.write_unique(Path::new(name), xml.as_bytes())
    }
    pub fn duplicate(&self, entry: &LibraryEntry) -> Result<LibraryEntry> {
        let (xml, _) = self.checked(entry)?;
        let path = suffixed(&entry.relative_path, "-copy");
        self.write_unique(&path, xml.as_bytes())
    }
    /// Import an independently parsed preset from an archive, without a staging
    /// path or overwriting an existing file. The preferred name is one filename.
    pub fn import_document(
        &self,
        document: &PresetDocument,
        preferred_name: &str,
    ) -> Result<LibraryEntry> {
        self.ensure_root()?;
        let path = Path::new(preferred_name);
        validate_relative(path)?;
        if path.components().count() != 1 || !is_preset_path(path) {
            return Err(error(
                "Archive extraction requires one .fuse or .xml filename",
            ));
        }
        let xml = document.to_xml().map_err(|e| error(e.to_string()))?;
        if xml.len() > MAX_PRESET_BYTES {
            return Err(error("Preset exceeds 2 MiB limit"));
        }
        self.write_unique(path, xml.as_bytes())
    }
    pub fn archive(&self, entry: &LibraryEntry) -> Result<ArchiveToken> {
        self.checked(entry)?;
        match self.directory.create_dir(TRASH) {
            Ok(()) => (),
            Err(reason) if reason.kind() == io::ErrorKind::AlreadyExists => (),
            Err(reason) => return Err(reason.into()),
        }
        self.check_components(Path::new(TRASH))?;
        let folder = self.unique_directory(Path::new(TRASH))?;
        let name = entry
            .relative_path
            .file_name()
            .ok_or_else(|| error("Missing source filename"))?;
        let archived = folder.join(name);
        let token = ArchiveToken {
            path: self.root.join(&archived),
            original_relative_path: entry.relative_path.clone(),
            fingerprint: entry.fingerprint.clone(),
        };
        let mut record = token.clone();
        record.path = archived.clone();
        let manifest = serde_json::to_vec(&record).map_err(|e| error(e.to_string()))?;
        self.write_exclusive(&folder.join("record.json"), &manifest)?;
        // Revalidate after preparing the recovery record, immediately before moving.
        self.checked(entry)?;
        self.directory
            .rename(&entry.relative_path, &self.directory, &archived)?;
        let (_, moved) = self.load(&archived)?;
        if moved.fingerprint != entry.fingerprint || moved.identity != entry.identity {
            return Err(error(format!(
                "Source changed during archive; retained safely at {}",
                token.path.display()
            )));
        }
        Ok(token)
    }
    pub fn archives(&self) -> Result<Vec<ArchiveToken>> {
        self.ensure_root()?;
        let metadata = match self.directory.symlink_metadata(TRASH) {
            Ok(metadata) => metadata,
            Err(reason) if reason.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(reason) => return Err(reason.into()),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(error("Archive directory is not a regular directory"));
        }
        let mut tokens = Vec::new();
        for (index, item) in self.directory.open_dir(TRASH)?.entries()?.enumerate() {
            if index >= DEFAULT_SCAN_LIMITS.max_entries {
                return Err(error("Archive count exceeds scan limit"));
            }
            let item = item?;
            let folder = Path::new(TRASH).join(item.file_name());
            self.check_components(&folder)?;
            if !self.directory.symlink_metadata(&folder)?.is_dir() {
                continue;
            }
            let manifest = folder.join("record.json");
            self.check_components(&manifest)?;
            if !self.directory.symlink_metadata(&manifest)?.is_file() {
                return Err(error("Archive manifest must be a regular file"));
            }
            let mut bytes = Vec::new();
            self.directory
                .open(&manifest)?
                .take(64 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 64 * 1024 {
                return Err(error("Archive manifest exceeds limit"));
            }
            let mut token: ArchiveToken = serde_json::from_slice(&bytes)
                .map_err(|e| error(format!("Invalid archive manifest: {e}")))?;
            // Rebase records onto the actual archive folder so a moved library
            // remains recoverable. Only the original filename is used from disk.
            let name = token
                .path
                .file_name()
                .ok_or_else(|| error("Archive record has no filename"))?;
            if !is_preset_path(Path::new(name)) {
                return Err(error("Archive record must identify a preset file"));
            }
            token.path = self.root.join(folder.join(name));
            let archived = self.archive_relative(&token)?;
            if archived.parent() != Some(folder.as_path()) {
                return Err(error("Archive record points outside its directory"));
            }
            let marker = folder.join("restored.json");
            match self.directory.symlink_metadata(&marker) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    continue;
                }
                Ok(_) => return Err(error("Archive restoration marker must be a regular file")),
                Err(reason) if reason.kind() == io::ErrorKind::NotFound => (),
                Err(reason) => return Err(reason.into()),
            }
            // A crash before moving leaves a harmless empty record.
            if self.directory.symlink_metadata(&archived).is_ok() {
                tokens.push(token);
            }
        }
        tokens.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(tokens)
    }
    pub fn restore(&self, token: &ArchiveToken) -> Result<LibraryEntry> {
        self.ensure_root()?;
        let archived = self.archive_relative(token)?;
        let marker = archived
            .parent()
            .ok_or_else(|| error("Invalid archive directory"))?
            .join("restored.json");
        match self.directory.symlink_metadata(&marker) {
            Ok(_) => return Err(error("This archive entry has already been restored")),
            Err(reason) if reason.kind() == io::ErrorKind::NotFound => (),
            Err(reason) => return Err(reason.into()),
        }
        let (xml, entry) = self.load(&archived)?;
        if entry.fingerprint != token.fingerprint {
            return Err(error(
                "Archived preset changed; inspect it before restoring",
            ));
        }
        validate_relative(&token.original_relative_path)?;
        if token.original_relative_path.starts_with(TRASH) {
            return Err(error("Invalid restoration destination"));
        }
        let parent = token
            .original_relative_path
            .parent()
            .unwrap_or(Path::new(""));
        if !parent.as_os_str().is_empty() {
            self.check_components(parent)?;
        }
        let restored = self.write_unique(&token.original_relative_path, xml.as_bytes())?;
        // Retain recovery bytes. A compare followed by unlink cannot rule out a
        // concurrent replacement between those operations. Marking completion
        // hides the recovery entry without deleting any concurrently changed data.
        self.write_exclusive(
            &marker,
            &serde_json::to_vec(&restored.relative_path).map_err(|e| error(e.to_string()))?,
        )?;
        Ok(restored)
    }
    fn archive_relative(&self, token: &ArchiveToken) -> Result<PathBuf> {
        let path = token
            .path
            .strip_prefix(&self.root)
            .map_err(|_| error("Archive belongs to another library"))?
            .to_path_buf();
        validate_relative(&path)?;
        if !path.starts_with(TRASH) || path.components().count() != 3 {
            return Err(error("Invalid archive path"));
        }
        Ok(path)
    }
    fn checked(&self, expected: &LibraryEntry) -> Result<(String, LibraryEntry)> {
        self.ensure_root()?;
        if expected.path != self.root.join(&expected.relative_path) {
            return Err(error("Entry belongs to another library"));
        }
        let result = self.load(&expected.relative_path)?;
        if result.1.fingerprint != expected.fingerprint || result.1.identity != expected.identity {
            return Err(error(
                "Preset changed on disk; refresh the library before continuing",
            ));
        }
        Ok(result)
    }
    fn load(&self, path: &Path) -> Result<(String, LibraryEntry)> {
        self.load_with_limit(path, MAX_PRESET_BYTES as u64)
    }
    fn ensure_root(&self) -> Result<()> {
        let metadata = std::fs::symlink_metadata(&self.root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(error("Library folder changed; select it again"));
        }
        let current = Dir::open_ambient_dir(&self.root, ambient_authority())?;
        if file_identity(&current.dir_metadata()?) != file_identity(&self.directory.dir_metadata()?)
        {
            return Err(error(
                "Library folder was moved or replaced; select it again",
            ));
        }
        Ok(())
    }
    fn load_with_limit(&self, path: &Path, byte_limit: u64) -> Result<(String, LibraryEntry)> {
        self.check_components(path)?;
        if !self.directory.symlink_metadata(path)?.is_file() {
            return Err(error("Preset must be a regular file"));
        }
        let mut file = self.directory.open(path)?;
        let before = file.metadata()?;
        let byte_limit = byte_limit.min(MAX_PRESET_BYTES as u64);
        if !before.is_file() || before.len() > byte_limit {
            return Err(error("Preset must be a regular file of at most 2 MiB"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(byte_limit + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > byte_limit {
            return Err(error("Preset exceeds 2 MiB limit"));
        }
        let after = file.metadata()?;
        if identity(&before) != identity(&after) {
            return Err(error("Preset changed while being read"));
        }
        let fingerprint = format!("{:x}", Sha256::digest(&bytes));
        let xml = String::from_utf8(bytes).map_err(|_| error("Preset is not UTF-8 XML"))?;
        let document = PresetDocument::parse(&xml).map_err(|e| error(e.to_string()))?;
        let entry = LibraryEntry {
            path: self.root.join(path),
            relative_path: path.into(),
            metadata: document.metadata(),
            product_id: document.product_id(),
            fingerprint,
            identity: identity(&after),
        };
        Ok((xml, entry))
    }
    fn check_components(&self, path: &Path) -> Result<()> {
        validate_relative(path)?;
        let mut partial = PathBuf::new();
        for component in path.components() {
            partial.push(component.as_os_str());
            if self
                .directory
                .symlink_metadata(&partial)?
                .file_type()
                .is_symlink()
            {
                return Err(error("Library paths cannot traverse symbolic links"));
            }
        }
        Ok(())
    }
    fn unique_directory(&self, parent: &Path) -> Result<PathBuf> {
        for _ in 0..128 {
            let path = parent.join(unique_id());
            match self.directory.create_dir(&path) {
                Ok(()) => return Ok(path),
                Err(reason) if reason.kind() == io::ErrorKind::AlreadyExists => (),
                Err(reason) => return Err(reason.into()),
            }
        }
        Err(error("Could not reserve an archive directory"))
    }
    fn write_exclusive(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        validate_relative(path)?;
        let mut file = self
            .directory
            .open_with(path, OpenOptions::new().write(true).create_new(true))?;
        let result = file.write_all(bytes).and_then(|()| file.sync_all());
        if result.is_err() {
            drop(file);
            let _ = self.directory.remove_file(path);
        }
        result.map_err(Into::into)
    }
    fn write_unique(&self, preferred: &Path, bytes: &[u8]) -> Result<LibraryEntry> {
        validate_relative(preferred)?;
        let parent = preferred.parent().unwrap_or(Path::new(""));
        if !parent.as_os_str().is_empty() {
            self.check_components(parent)?;
        }
        let temporary = parent.join(format!("{TEMP_PREFIX}{}", unique_id()));
        self.write_exclusive(&temporary, bytes)?;
        let result = (|| {
            for suffix in 0..10_000 {
                let destination = if suffix == 0 {
                    preferred.to_path_buf()
                } else {
                    suffixed(preferred, &format!("-{suffix}"))
                };
                // A hard link atomically publishes fully written bytes, without replacing
                // any existing file or symlink. The temporary is on the same filesystem.
                match self
                    .directory
                    .hard_link(&temporary, &self.directory, &destination)
                {
                    Ok(()) => return self.load(&destination).map(|(_, entry)| entry),
                    Err(reason) if reason.kind() == io::ErrorKind::AlreadyExists => (),
                    Err(reason) => return Err(reason.into()),
                }
            }
            Err(error("Too many filename collisions"))
        })();
        let _ = self.directory.remove_file(&temporary);
        result
    }
}
impl LibraryScan {
    fn diagnostic(&mut self, path: PathBuf, message: impl fmt::Display) {
        self.diagnostics.push(LibraryDiagnostic {
            path,
            message: message.to_string(),
        });
    }
}
fn validate_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(error(
            "Expected a nonempty relative path without parent traversal",
        ));
    }
    Ok(())
}
fn is_preset_path(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("fuse") || s.eq_ignore_ascii_case("xml"))
}
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.file_stem().unwrap_or_default());
    name.push(suffix);
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    path.with_file_name(name)
}
fn unique_id() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
fn identity(metadata: &Metadata) -> String {
    format!(
        "{}:{}:{:?}",
        file_identity(metadata),
        metadata.len(),
        metadata.modified()
    )
}
fn file_identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    let platform = {
        use cap_std::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let platform = format!("{:?}", metadata.created());
    platform
}
