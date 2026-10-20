//! Bounded raw preset inputs and atomic publication to a new file.
//! A legacy input is read before parsing so repairs remain explicit.
use crate::{archive::ArchiveCatalog, preset::PresetDocument};
use std::{
    fmt,
    path::{Path, PathBuf},
};

pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
#[derive(Debug)]
pub struct FileError(String);
impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for FileError {}
pub type Result<T> = std::result::Result<T, FileError>;
fn error(value: impl fmt::Display) -> FileError {
    FileError(value.to_string())
}

fn location(path: &Path) -> Result<(ArchiveCatalog, PathBuf)> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| error("Missing preset directory"))?;
    // Reading a missing source must not create its parent directory.
    if !parent.is_dir() {
        return Err(error("Preset directory does not exist"));
    }
    let name = absolute
        .file_name()
        .ok_or_else(|| error("Missing preset filename"))?;
    Ok((
        ArchiveCatalog::open(parent).map_err(error)?,
        PathBuf::from(name),
    ))
}
pub fn read(path: impl AsRef<Path>) -> Result<String> {
    let (directory, name) = location(path.as_ref())?;
    let bytes = directory.read_file(&name, MAX_BYTES).map_err(error)?;
    String::from_utf8(bytes).map_err(error)
}
pub fn write_new(document: &PresetDocument, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("fuse") || s.eq_ignore_ascii_case("xml"))
    {
        return Err(error("Preset export requires .fuse or .xml extension"));
    }
    let xml = document.to_xml().map_err(error)?;
    if xml.len() as u64 > MAX_BYTES {
        return Err(error("Preset exceeds 2 MiB limit"));
    }
    let (directory, name) = location(path)?;
    directory.publish_new(&name, xml.as_bytes()).map_err(error)
}
