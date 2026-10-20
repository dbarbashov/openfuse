use super::*;
use cap_std::{
    ambient_authority,
    fs::{Dir, Metadata, OpenOptions},
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Debug, Clone)]
pub(super) struct Folder {
    path: PathBuf,
    dir: Arc<Dir>,
    identity: String,
    parent: Option<Arc<Folder>>,
    name: Option<String>,
}
#[derive(Debug, Clone)]
pub(super) struct Source {
    pub folder: Folder,
    pub name: String,
    pub stamp: String,
    pub hash: String,
    pub size: u64,
}
pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn identity(meta: &Metadata) -> String {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        format!("{}:{}", meta.dev(), meta.ino())
    }
    #[cfg(not(unix))]
    {
        format!("{:?}", meta.created())
    }
}
fn stamp(meta: &Metadata) -> String {
    #[cfg(unix)]
    let change = {
        use cap_std::fs::MetadataExt;
        format!("{}:{}", meta.ctime(), meta.ctime_nsec())
    };
    #[cfg(not(unix))]
    let change = String::new();
    format!(
        "{}:{change}:{}:{:?}",
        identity(meta),
        meta.len(),
        meta.modified()
    )
}
pub(super) fn validate_name(name: &str) -> Result<()> {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|s| s.len() == 1 && matches!(s.as_bytes()[0], b'1'..=b'9'));
    if name.is_empty()
        || name.len() > 240
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| c < ' ' || "/\\:*?\"<>|".contains(c))
        || reserved
    {
        return Err(error("Filename is not portable to G-DEC FAT storage"));
    }
    Ok(())
}
impl Folder {
    pub fn open(path: &Path) -> Result<Self> {
        let meta = std::fs::symlink_metadata(path).map_err(error)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(error("Select a real directory without symbolic links"));
        }
        let path = path.canonicalize().map_err(error)?;
        let dir = Dir::open_ambient_dir(&path, ambient_authority()).map_err(error)?;
        let identity = identity(&dir.dir_metadata().map_err(error)?);
        if identity != self::identity(&Metadata::from_just_metadata(meta)) {
            return Err(error("Selected directory changed while opening"));
        }
        let result = Self {
            path,
            dir: Arc::new(dir),
            identity,
            parent: None,
            name: None,
        };
        result.ensure()?;
        Ok(result)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn identity(&self) -> String {
        self.identity.clone()
    }
    pub fn ensure(&self) -> Result<()> {
        let meta = if let Some(parent) = &self.parent {
            parent.ensure()?;
            parent
                .dir
                .symlink_metadata(self.name.as_ref().unwrap())
                .map_err(error)?
        } else {
            Metadata::from_just_metadata(std::fs::symlink_metadata(&self.path).map_err(error)?)
        };
        if !meta.is_dir() || meta.file_type().is_symlink() || identity(&meta) != self.identity {
            return Err(error(
                "Selected directory moved, changed or became a symbolic link",
            ));
        }
        Ok(())
    }
    pub fn membership_stamp(&self) -> Result<String> {
        self.ensure()?;
        Ok(stamp(&self.dir.dir_metadata().map_err(error)?))
    }
    pub fn names(&self) -> Result<(Vec<String>, bool)> {
        self.ensure()?;
        let mut names = Vec::new();
        for (i, e) in self.dir.entries().map_err(error)?.enumerate() {
            if i >= MAX_ENTRIES {
                return Ok((names, true));
            }
            names.push(
                e.map_err(error)?
                    .file_name()
                    .into_string()
                    .map_err(|_| error("Filename is not UTF-8"))?,
            );
        }
        names.sort();
        Ok((names, false))
    }
    pub fn child(&self, name: &str) -> Result<Self> {
        validate_name(name)?;
        self.ensure()?;
        let before = self.dir.symlink_metadata(name).map_err(error)?;
        if !before.is_dir() || before.file_type().is_symlink() {
            return Err(error("Storage folder must be a real directory"));
        }
        let dir = self.dir.open_dir(name).map_err(error)?;
        if identity(&before) != identity(&dir.dir_metadata().map_err(error)?) {
            return Err(error("Storage folder changed while opening"));
        }
        let result = Self {
            path: self.path.join(name),
            dir: Arc::new(dir),
            identity: identity(&before),
            parent: Some(Arc::new(self.clone())),
            name: Some(name.into()),
        };
        result.ensure()?;
        Ok(result)
    }
    pub fn find_child(&self, expected: &str) -> Result<Option<Self>> {
        let (names, truncated) = self.names()?;
        if truncated {
            return Err(error("Root exceeds directory scan limit"));
        }
        let found = names
            .iter()
            .filter(|n| n.eq_ignore_ascii_case(expected))
            .collect::<Vec<_>>();
        match found.as_slice() {
            [] => Ok(None),
            [name] => self.child(name).map(Some),
            _ => Err(error("Case-colliding storage folders")),
        }
    }
    pub fn create_child(&self, name: &str) -> Result<Self> {
        validate_name(name)?;
        self.ensure()?;
        if self.find_child(name)?.is_some() {
            return Err(error("Destination folder appeared after preview"));
        }
        self.dir.create_dir(name).map_err(error)?;
        self.child(name)
    }
    pub fn inspect(&self, name: &str, limit: u64) -> Result<(Source, Vec<u8>)> {
        validate_name(name)?;
        self.ensure()?;
        let meta = self.dir.symlink_metadata(name).map_err(error)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit {
            return Err(error("Expected bounded regular file without symlinks"));
        }
        let before = stamp(&meta);
        let mut file = self.dir.open(name).map_err(error)?;
        if stamp(&file.metadata().map_err(error)?) != before {
            return Err(error("Source changed while opening"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() as u64 > limit
            || bytes.len() as u64 != meta.len()
            || stamp(&file.metadata().map_err(error)?) != before
            || stamp(&self.dir.symlink_metadata(name).map_err(error)?) != before
        {
            return Err(error("Source changed during read"));
        }
        self.ensure()?;
        Ok((
            Source {
                folder: self.clone(),
                name: name.into(),
                stamp: before,
                hash: digest(&bytes),
                size: bytes.len() as u64,
            },
            bytes,
        ))
    }
    pub fn absent(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let (names, truncated) = self.names()?;
        if truncated {
            return Err(error("Destination directory exceeds scan limit"));
        }
        if names
            .iter()
            .any(|n| n.to_lowercase() == name.to_lowercase())
        {
            return Err(error("Destination exists or collides ignoring case"));
        }
        Ok(())
    }
    pub fn write_new(
        &self,
        name: &str,
        bytes: &[u8],
        cancel: &AtomicBool,
        mut created: impl FnMut(),
    ) -> Result<()> {
        self.absent(name)?;
        if cancel.load(Ordering::Acquire) {
            return Err(error("Cancelled before file creation"));
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = self.dir.open_with(name, &options).map_err(error)?;
        created();
        // Portable FAT publication: an interrupted newly created file is retained
        // and reported, never silently deleted or counted as verified.
        for chunk in bytes.chunks(64 * 1024) {
            if cancel.load(Ordering::Acquire) {
                return Err(error("Cancelled; partial new file retained"));
            }
            self.ensure()?;
            file.write_all(chunk).map_err(error)?;
        }
        file.sync_all().map_err(error)?;
        drop(file);
        self.ensure()?;
        let (_, readback) = self.inspect(name, MAX_FILE_BYTES)?;
        let (names, truncated) = self.names()?;
        if truncated
            || names
                .iter()
                .filter(|n| n.to_lowercase() == name.to_lowercase())
                .count()
                != 1
        {
            return Err(error(
                "Destination case collision appeared during publication",
            ));
        }
        if readback != bytes {
            return Err(error("Destination readback differs from copied bytes"));
        }
        Ok(())
    }
}
impl Source {
    pub fn check(&self) -> Result<()> {
        self.folder.ensure()?;
        let meta = self
            .folder
            .dir
            .symlink_metadata(&self.name)
            .map_err(error)?;
        if !meta.is_file() || meta.file_type().is_symlink() || stamp(&meta) != self.stamp {
            return Err(error("Source changed since inventory"));
        }
        Ok(())
    }
    pub fn path(&self) -> PathBuf {
        self.folder.path.join(&self.name)
    }
    pub fn read(&self) -> Result<Vec<u8>> {
        let (actual, bytes) = self.folder.inspect(&self.name, MAX_FILE_BYTES)?;
        if actual.stamp != self.stamp || actual.hash != self.hash || actual.size != self.size {
            return Err(error("Source changed since preview; prepare a new plan"));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_after_exclusive_creation_retains_unverified_new_file() {
        let temp = tempfile::tempdir().unwrap();
        let folder = Folder::open(temp.path()).unwrap();
        let cancelled = AtomicBool::new(false);
        let result = folder.write_new("partial.wav", b"new payload", &cancelled, || {
            cancelled.store(true, Ordering::Release);
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(temp.path().join("partial.wav")).unwrap(), b"");
        let mut created = false;
        cancelled.store(false, Ordering::Release);
        assert!(
            folder
                .write_new("partial.wav", b"replacement", &cancelled, || created = true)
                .is_err()
        );
        assert!(!created);
        assert_eq!(std::fs::read(temp.path().join("partial.wav")).unwrap(), b"");
    }
}
