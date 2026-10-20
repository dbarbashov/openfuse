use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CollisionPolicy {
    Reject,
    KeepBoth,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Direction {
    Upload,
    Download,
}
#[derive(Debug, Clone, Serialize)]
pub struct TransferItem {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub size: u64,
    pub source_sha256: String,
    pub output_sha256: String,
    pub changes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct TransferReceipt {
    pub completed: Vec<TransferItem>,
    pub interrupted: Option<PathBuf>,
    pub cancelled: bool,
    pub error: Option<String>,
}
#[derive(Debug, Clone)]
struct Destination {
    root: fs::Folder,
    subfolder: Option<String>,
    folder: Option<fs::Folder>,
}
impl Destination {
    fn check(&self) -> Result<()> {
        self.root.ensure()?;
        if let Some(folder) = &self.folder {
            folder.ensure()?;
        } else if let Some(name) = &self.subfolder
            && self.root.find_child(name)?.is_some()
        {
            return Err(error("Destination folder appeared after preview"));
        }
        Ok(())
    }
    fn path(&self) -> PathBuf {
        self.folder
            .as_ref()
            .map(|f| f.path().into())
            .unwrap_or_else(|| {
                self.root
                    .path()
                    .join(self.subfolder.as_deref().unwrap_or(""))
            })
    }
    fn names(&self) -> Result<Vec<String>> {
        self.check()?;
        if let Some(folder) = &self.folder {
            let (names, truncated) = folder.names()?;
            if truncated {
                return Err(error("Destination scan limit exceeded"));
            }
            Ok(names)
        } else {
            Ok(vec![])
        }
    }
}
struct Input {
    source: fs::Source,
    target: Destination,
    name: String,
    bytes: Vec<u8>,
    changes: Vec<String>,
}
#[derive(Debug, Clone)]
struct Copy {
    source: fs::Source,
    target: Destination,
    name: String,
    bytes: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct TransferPlan {
    direction: Direction,
    items: Vec<TransferItem>,
    copies: Vec<Copy>,
}
impl TransferPlan {
    pub fn direction(&self) -> Direction {
        self.direction
    }
    pub fn items(&self) -> &[TransferItem] {
        &self.items
    }
    pub fn upload(storage: &MountedStorage, files: &[PathBuf]) -> Result<Self> {
        Self::upload_with_policy(storage, files, CollisionPolicy::Reject)
    }
    pub fn upload_keep_both(storage: &MountedStorage, files: &[PathBuf]) -> Result<Self> {
        Self::upload_with_policy(storage, files, CollisionPolicy::KeepBoth)
    }
    pub fn upload_with_policy(
        storage: &MountedStorage,
        files: &[PathBuf],
        policy: CollisionPolicy,
    ) -> Result<Self> {
        validate_count(files.len())?;
        storage.root.ensure()?;
        let mut inputs = Vec::new();
        let mut total = 0;
        for path in files {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| error("Source filename must be UTF-8"))?;
            fs::validate_name(name)?;
            let kind = FileKind::from_name(name)
                .ok_or_else(|| error("Upload supports .wav/.mp3/.mid/.fuse"))?;
            if kind == FileKind::Preset && storage.volume == Volume::Internal {
                return Err(error(
                    "Original mounted transfer has no internal preset folder",
                ));
            }
            let absolute = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir().map_err(error)?.join(path)
            };
            let folder = fs::Folder::open(
                absolute
                    .parent()
                    .ok_or_else(|| error("Missing source directory"))?,
            )?;
            let limit = if kind == FileKind::Preset {
                crate::preset::MAX_PRESET_BYTES as u64
            } else {
                MAX_FILE_BYTES
            };
            let (source, mut bytes) = folder.inspect(name, limit.min(MAX_BATCH_BYTES - total))?;
            total += bytes.len() as u64;
            let mut changes = Vec::new();
            if kind == FileKind::Preset {
                let xml = std::str::from_utf8(&bytes).map_err(error)?;
                let (fixed, notes) = fix_gdec_tag(xml)?;
                let doc = crate::preset::PresetDocument::parse(&fixed).map_err(error)?;
                if doc.product_id() != Some(10) {
                    return Err(error(
                        "SD preset upload requires family 10; convert an independent copy first",
                    ));
                }
                bytes = fixed.into_bytes();
                changes = notes;
            }
            let target = Destination {
                root: storage.root.clone(),
                subfolder: Some(kind.folder().into()),
                folder: storage.root.find_child(kind.folder())?,
            };
            inputs.push(Input {
                source,
                target,
                name: name.to_owned(),
                bytes,
                changes,
            });
        }
        Self::build(Direction::Upload, inputs, policy)
    }
    pub fn download(
        storage: &MountedStorage,
        entries: &[Entry],
        directory: impl AsRef<Path>,
    ) -> Result<Self> {
        Self::download_with_policy(storage, entries, directory, CollisionPolicy::Reject)
    }
    pub fn download_keep_both(
        storage: &MountedStorage,
        entries: &[Entry],
        directory: impl AsRef<Path>,
    ) -> Result<Self> {
        Self::download_with_policy(storage, entries, directory, CollisionPolicy::KeepBoth)
    }
    pub fn download_with_policy(
        storage: &MountedStorage,
        entries: &[Entry],
        directory: impl AsRef<Path>,
        policy: CollisionPolicy,
    ) -> Result<Self> {
        validate_count(entries.len())?;
        let root = fs::Folder::open(directory.as_ref())?;
        let target = Destination {
            folder: Some(root.clone()),
            root,
            subfolder: None,
        };
        let mut inputs = Vec::new();
        let mut total = 0u64;
        for entry in entries {
            total = total
                .checked_add(entry.size)
                .ok_or_else(|| error("Transfer size overflow"))?;
            if total > MAX_BATCH_BYTES {
                return Err(error("Transfer exceeds 512 MiB"));
            }
            let bytes = storage.read(entry)?;
            inputs.push(Input {
                source: entry.source.clone(),
                target: target.clone(),
                name: entry.name.clone(),
                bytes,
                changes: Vec::new(),
            });
        }
        Self::build(Direction::Download, inputs, policy)
    }
    fn build(direction: Direction, inputs: Vec<Input>, policy: CollisionPolicy) -> Result<Self> {
        let mut items = Vec::new();
        let mut copies = Vec::new();
        let mut reserved = BTreeMap::<PathBuf, BTreeSet<String>>::new();
        let mut sources = BTreeSet::new();
        let mut total = 0u64;
        for Input {
            source,
            target,
            name: original,
            bytes,
            mut changes,
        } in inputs
        {
            total += bytes.len() as u64;
            if total > MAX_BATCH_BYTES {
                return Err(error("Transfer exceeds 512 MiB"));
            }
            if !sources.insert(source.path()) {
                return Err(error("Duplicate source in batch"));
            }
            let directory = target.path();
            if !reserved.contains_key(&directory) {
                reserved.insert(
                    directory.clone(),
                    target
                        .names()?
                        .into_iter()
                        .map(|s| s.to_lowercase())
                        .collect(),
                );
            }
            let names = reserved.get_mut(&directory).unwrap();
            let name = choose_name(&original, names, policy)?;
            if name != original {
                changes.push(format!(
                    "Keep both: destination filename {original} to {name}"
                ));
            }
            names.insert(name.to_lowercase());
            items.push(TransferItem {
                source: source.path(),
                destination: directory.join(&name),
                size: bytes.len() as u64,
                source_sha256: source.hash.clone(),
                output_sha256: fs::digest(&bytes),
                changes,
            });
            copies.push(Copy {
                source,
                target,
                name,
                bytes,
            });
        }
        let plan = Self {
            direction,
            items,
            copies,
        };
        plan.preflight()?;
        Ok(plan)
    }
    fn preflight(&self) -> Result<()> {
        for copy in &self.copies {
            copy.source.read()?;
            copy.target.check()?;
            if let Some(folder) = &copy.target.folder {
                folder.absent(&copy.name)?;
            }
        }
        Ok(())
    }
    pub fn execute(&self, cancel: &AtomicBool) -> TransferReceipt {
        self.execute_with_progress(cancel, |_, _| {})
    }
    pub fn execute_with_progress(
        &self,
        cancel: &AtomicBool,
        mut progress: impl FnMut(usize, usize),
    ) -> TransferReceipt {
        let mut result = TransferReceipt::default();
        if !cfg!(unix) {
            result.error = Some("Mounted writes require stable Unix file identities; this platform remains read-only".into());
            return result;
        }
        if cancel.load(Ordering::Acquire) {
            result.cancelled = true;
            return result;
        }
        if let Err(e) = self.preflight() {
            result.error = Some(e.to_string());
            return result;
        }
        let mut created = BTreeMap::<PathBuf, fs::Folder>::new();
        for (copy, item) in self.copies.iter().zip(&self.items) {
            if cancel.load(Ordering::Acquire) {
                result.cancelled = true;
                break;
            }
            let operation: Result<()> = (|| {
                copy.source.read()?;
                copy.target.root.ensure()?;
                let folder = if let Some(folder) = &copy.target.folder {
                    folder.ensure()?;
                    folder.clone()
                } else if let Some(folder) = created.get(&copy.target.path()) {
                    folder.ensure()?;
                    folder.clone()
                } else {
                    copy.target.check()?;
                    let folder = copy
                        .target
                        .root
                        .create_child(copy.target.subfolder.as_deref().unwrap())?;
                    created.insert(copy.target.path(), folder.clone());
                    folder
                };
                folder.absent(&copy.name)?;
                folder.write_new(&copy.name, &copy.bytes, cancel, || {
                    result.interrupted = Some(item.destination.clone());
                })?;
                result.interrupted = None;
                Ok(())
            })();
            if let Err(e) = operation {
                result.error = Some(e.to_string());
                result.cancelled = cancel.load(Ordering::Acquire);
                break;
            }
            result.completed.push(item.clone());
            progress(result.completed.len(), self.items.len());
        }
        result
    }
}
fn validate_count(count: usize) -> Result<()> {
    if count == 0 || count > 100 {
        return Err(error("Choose 1..100 files per transfer"));
    }
    Ok(())
}
fn choose_name(
    original: &str,
    names: &BTreeSet<String>,
    policy: CollisionPolicy,
) -> Result<String> {
    if !names.contains(&original.to_lowercase()) {
        return Ok(original.into());
    }
    if policy == CollisionPolicy::Reject {
        return Err(error(format!(
            "Destination already contains {original}; choose Keep both or another folder"
        )));
    }
    let path = Path::new(original);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| error("Invalid filename"))?;
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    for index in 2..=10000 {
        let suffix = if extension.is_empty() {
            String::new()
        } else {
            format!(".{extension}")
        };
        let name = format!("{stem} ({index}){suffix}");
        fs::validate_name(&name)?;
        if !names.contains(&name.to_lowercase()) {
            return Ok(name);
        }
    }
    Err(error("No available Keep both filename within limit"))
}
