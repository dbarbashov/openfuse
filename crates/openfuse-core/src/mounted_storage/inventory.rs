use super::*;
use std::collections::BTreeSet;
impl MountedStorage {
    pub fn scan(&self) -> Result<Inventory> {
        self.root.ensure()?;
        let root_stamp = self.root.membership_stamp()?;
        let mut inventory = Inventory::default();
        let mut total = 0u64;
        let mut visited = 0usize;
        let mut folders = Vec::new();
        for kind in [
            FileKind::Audio,
            FileKind::Phrase,
            FileKind::Midi,
            FileKind::Preset,
        ] {
            if kind == FileKind::Preset && self.volume == Volume::Internal {
                continue;
            }
            let folder = match self.root.find_child(kind.folder()) {
                Ok(Some(folder)) => folder,
                Ok(None) => continue,
                Err(e) => {
                    inventory.diagnostics.push(Diagnostic {
                        path: self.root.path().join(kind.folder()),
                        message: e.to_string(),
                    });
                    continue;
                }
            };
            let before = folder.membership_stamp()?;
            let (names, truncated) = folder.names()?;
            inventory.truncated |= truncated;
            let mut seen = BTreeSet::new();
            let mut collisions = BTreeSet::new();
            for name in &names {
                if !seen.insert(name.to_lowercase()) {
                    collisions.insert(name.to_lowercase());
                }
            }
            for name in names {
                visited += 1;
                if visited > MAX_ENTRIES {
                    inventory.truncated = true;
                    break;
                }
                let path = folder.path().join(&name);
                if collisions.contains(&name.to_lowercase()) {
                    inventory.diagnostics.push(Diagnostic {
                        path,
                        message: "Case-colliding filenames cannot be transferred safely".into(),
                    });
                    continue;
                }
                let found = FileKind::from_name(&name);
                let supported = found == Some(kind) || kind == FileKind::Phrase;
                if !supported {
                    inventory.diagnostics.push(Diagnostic {
                        path,
                        message:
                            "Unrecognized file retained without interpreting or transferring it"
                                .into(),
                    });
                    continue;
                }
                if total >= MAX_BATCH_BYTES {
                    inventory.truncated = true;
                    break;
                }
                match folder.inspect(&name, MAX_FILE_BYTES.min(MAX_BATCH_BYTES - total)) {
                    Ok((source, bytes)) => {
                        total += source.size;
                        let preset_name = if kind == FileKind::Preset {
                            std::str::from_utf8(&bytes)
                                .ok()
                                .and_then(|s| crate::preset::PresetDocument::parse(s).ok())
                                .map(|d| d.metadata().name)
                        } else {
                            None
                        };
                        inventory.entries.push(Entry {
                            relative_path: path
                                .strip_prefix(self.root.path())
                                .map_err(error)?
                                .into(),
                            name,
                            kind,
                            size: source.size,
                            sha256: source.hash.clone(),
                            preset_name,
                            source,
                            root_identity: self.root.identity(),
                            root_path: self.root.path().into(),
                            volume: self.volume,
                        });
                    }
                    Err(e) => {
                        if MAX_BATCH_BYTES - total < MAX_FILE_BYTES {
                            inventory.truncated = true;
                        }
                        inventory.diagnostics.push(Diagnostic {
                            path,
                            message: e.to_string(),
                        });
                    }
                }
            }
            if folder.membership_stamp()? != before {
                return Err(error("Mounted directory changed during inventory"));
            }
            folders.push((folder, before));
        }
        if self.root.membership_stamp()? != root_stamp {
            return Err(error("Mounted root changed during inventory"));
        }
        for (folder, before) in folders {
            if folder.membership_stamp()? != before {
                return Err(error("Mounted directory changed during inventory"));
            }
        }
        // Check all earlier file identities after later files have been read.
        for entry in &inventory.entries {
            entry.source.check()?;
        }
        inventory
            .entries
            .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(inventory)
    }
}
