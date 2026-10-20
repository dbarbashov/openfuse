use super::*;
use cap_std::{
    ambient_authority,
    fs::{Dir, Metadata},
};
use std::io::Read;

fn stamp(metadata: &Metadata) -> String {
    #[cfg(unix)]
    let identity = {
        use cap_std::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", metadata.created());
    format!(
        "{identity}:{}:{:?}:{}:{}:{}",
        metadata.len(),
        metadata.modified(),
        metadata.is_file(),
        metadata.is_dir(),
        metadata.file_type().is_symlink()
    )
}
fn stable(expected: &str, metadata: &Metadata) -> Result<()> {
    if expected != stamp(metadata) {
        return Err(error(
            "Legacy source changed during import; retry from a stable copy",
        ));
    }
    Ok(())
}
pub(super) fn read(root: &Path) -> Result<LegacyBackup> {
    let metadata = Metadata::from_just_metadata(std::fs::symlink_metadata(root).map_err(error)?);
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(error(
            "Legacy backup must be a real directory, not a symlink",
        ));
    }
    let root_stamp = stamp(&metadata);
    let dir = Dir::open_ambient_dir(root, ambient_authority()).map_err(error)?;
    stable(&root_stamp, &dir.dir_metadata().map_err(error)?)?;
    let mut inputs = Vec::new();
    let mut pending = vec![(String::new(), root_stamp.clone())];
    let mut snapshots = Vec::new();
    let mut total = 0usize;
    let mut directories = 0usize;
    while let Some((relative, expected)) = pending.pop() {
        directories += 1;
        if directories > MAX_LEGACY_FILES {
            return Err(error("Too many legacy directories"));
        }
        let opened = if relative.is_empty() {
            dir.try_clone()
        } else {
            dir.open_dir(&relative)
        }
        .map_err(error)?;
        stable(&expected, &opened.dir_metadata().map_err(error)?)?;
        for entry in opened.entries().map_err(error)? {
            let entry = entry.map_err(error)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| error("Legacy filename is not UTF-8"))?;
            let path = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            validate_path(&path)?;
            let metadata = dir.symlink_metadata(&path).map_err(error)?;
            if metadata.file_type().is_symlink() {
                return Err(error(format!("Legacy symlink rejected: {path}")));
            }
            let before = stamp(&metadata);
            if metadata.is_dir() {
                if directories + pending.len() >= MAX_LEGACY_FILES {
                    return Err(error("Too many legacy directories"));
                }
                pending.push((path, before));
                continue;
            }
            if !metadata.is_file() {
                return Err(error(format!("Legacy special file rejected: {path}")));
            }
            if inputs.len() >= MAX_LEGACY_FILES {
                return Err(error("Too many legacy files"));
            }
            let available = MAX_LEGACY_BYTES - total;
            if metadata.len() > available as u64 {
                return Err(error("Legacy backup exceeds 64 MiB"));
            }
            let mut file = dir.open(&path).map_err(error)?;
            stable(&before, &file.metadata().map_err(error)?)?;
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(available as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(error)?;
            if bytes.len() > available {
                return Err(error("Legacy backup grew beyond 64 MiB"));
            }
            stable(&before, &file.metadata().map_err(error)?)?;
            stable(&before, &dir.symlink_metadata(&path).map_err(error)?)?;
            if bytes.len() as u64 != metadata.len() {
                return Err(error("Legacy source length changed during read"));
            }
            total += bytes.len();
            snapshots.push((path.clone(), before));
            inputs.push((path, bytes));
        }
        stable(&expected, &opened.dir_metadata().map_err(error)?)?;
        snapshots.push((relative, expected));
    }
    // Detect changes to earlier files or directory membership while later files
    // were read, including additions, renames and replacement of the selected root.
    for (path, expected) in snapshots {
        let metadata = if path.is_empty() {
            dir.dir_metadata()
        } else {
            dir.symlink_metadata(path)
        }
        .map_err(error)?;
        stable(&expected, &metadata)?;
    }
    stable(
        &root_stamp,
        &Metadata::from_just_metadata(std::fs::symlink_metadata(root).map_err(error)?),
    )?;
    let mut result = LegacyBackup::from_entries(inputs)?;
    result.source_name = root.file_name().and_then(|s| s.to_str()).map(str::to_owned);
    Ok(result)
}
