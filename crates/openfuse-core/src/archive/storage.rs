use super::*;
impl ArchiveCatalog {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let meta = std::fs::symlink_metadata(root)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(error(
                "Select a real catalog directory, not a symbolic link",
            ));
        }
        let root = root.canonicalize()?;
        let directory = Arc::new(Dir::open_ambient_dir(&root, ambient_authority())?);
        Ok(Self { root, directory })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn ensure_root(&self) -> Result<()> {
        let metadata = std::fs::symlink_metadata(&self.root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(error("Catalog directory changed; open it again"));
        }
        let current = Dir::open_ambient_dir(&self.root, ambient_authority())?;
        if identity(&current.dir_metadata()?) != identity(&self.directory.dir_metadata()?) {
            return Err(error("Catalog directory moved or was replaced"));
        }
        Ok(())
    }
    pub fn import_file(&self, path: impl AsRef<Path>) -> Result<CatalogEntry> {
        self.ensure_root()?;
        let path = path.as_ref();
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let parent = path
            .parent()
            .ok_or_else(|| error("Source file has no parent"))?;
        let filename = path
            .file_name()
            .ok_or_else(|| error("Source file has no name"))?;
        let source = Self::open(parent)?;
        let bytes = source.read_file(Path::new(filename), bank::MAX_BACKUP_BYTES as u64)?;
        let text = std::str::from_utf8(&bytes).map_err(error)?;
        let format = serde_json::from_str::<serde_json::Value>(text).map_err(error)?;
        let kind = match format.get("format").and_then(|v| v.as_str()) {
            Some("confuse-auxiliary") => {
                crate::auxiliary::Snapshot::parse(text).map_err(error)?;
                ArchiveKind::Auxiliary
            }
            Some("confuse-control-preset") => {
                crate::control_preset::ControlPreset::parse_json(text).map_err(error)?;
                ArchiveKind::ControlPreset
            }
            _ => {
                Backup::parse(text).map_err(error)?;
                ArchiveKind::Native
            }
        };
        let name = filename.to_string_lossy().to_string();
        self.import_sources(
            kind,
            &name,
            vec![SourceFile {
                path: name.clone(),
                hex: source_hex(&bytes),
            }],
        )
    }
    pub fn import_backup(&self, backup: &Backup, label: &str) -> Result<CatalogEntry> {
        let bytes = backup.to_json().map_err(error)?.into_bytes();
        self.import_sources(
            ArchiveKind::Native,
            label,
            vec![SourceFile {
                path: "backup.json".into(),
                hex: source_hex(&bytes),
            }],
        )
    }
    pub fn import_auxiliary(
        &self,
        snapshot: &crate::auxiliary::Snapshot,
        label: &str,
    ) -> Result<CatalogEntry> {
        let bytes = snapshot.to_json().map_err(error)?.into_bytes();
        self.import_sources(
            ArchiveKind::Auxiliary,
            label,
            vec![SourceFile {
                path: "auxiliary.json".into(),
                hex: source_hex(&bytes),
            }],
        )
    }
    pub fn import_control_preset(
        &self,
        document: &crate::control_preset::ControlPreset,
    ) -> Result<CatalogEntry> {
        self.import_control_source(document, "preset.json")
    }
    pub(crate) fn duplicate_control_preset(
        &self,
        document: &crate::control_preset::ControlPreset,
    ) -> Result<CatalogEntry> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        self.import_control_source(
            document,
            &format!("copy-{}-{stamp}-{sequence}.json", std::process::id()),
        )
    }
    fn import_control_source(
        &self,
        document: &crate::control_preset::ControlPreset,
        filename: &str,
    ) -> Result<CatalogEntry> {
        let text = document.to_json().map_err(error)?;
        self.import_sources(
            ArchiveKind::ControlPreset,
            &document.name(),
            vec![SourceFile {
                path: filename.into(),
                hex: source_hex(text.as_bytes()),
            }],
        )
    }
    pub fn import_legacy(&self, path: impl AsRef<Path>) -> Result<CatalogEntry> {
        self.ensure_root()?;
        let path = path.as_ref();
        let legacy = LegacyBackup::read_directory(path).map_err(error)?;
        let label = path
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or("Historical FUSE backup");
        let files = legacy
            .entries()
            .iter()
            .map(|e| SourceFile {
                path: e.path().to_owned(),
                hex: source_hex(e.bytes()),
            })
            .collect();
        self.import_sources(ArchiveKind::Legacy, label, files)
    }
    fn import_sources(
        &self,
        kind: ArchiveKind,
        label: &str,
        files: Vec<SourceFile>,
    ) -> Result<CatalogEntry> {
        self.ensure_root()?;
        if label.len() > 1024 || label.contains('\0') {
            return Err(error("Invalid archive label"));
        }
        let mut bundle = Bundle {
            format: "confuse-archive".into(),
            version: 1,
            kind,
            label: label.into(),
            imported_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64,
            files,
            sha256: String::new(),
        };
        bundle.archive()?;
        bundle.sha256 = bundle.checksum()?;
        let id = bundle.id()?;
        let path = PathBuf::from(format!("{id}.confuse-archive.json"));
        if self.directory.symlink_metadata(&path).is_ok() {
            return self.read_bundle(&id).map(|(_, entry)| entry);
        }
        let bytes = serde_json::to_vec(&bundle).map_err(error)?;
        if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
            return Err(error("Catalog bundle exceeds 136 MiB limit"));
        }
        if let Err(reason) = self.publish_new(&path, &bytes) {
            // A concurrent identical import may have published this content ID.
            // Accept only a complete, verified bundle; never replace the winner.
            return self
                .read_bundle(&id)
                .map(|(_, entry)| entry)
                .map_err(|_| reason);
        }
        self.read_bundle(&id).map(|(_, entry)| entry)
    }
    pub fn scan(&self) -> Result<CatalogScan> {
        self.scan_kind(false)
    }
    pub fn trash(&self) -> Result<CatalogScan> {
        self.scan_kind(true)
    }
    fn scan_kind(&self, trashed: bool) -> Result<CatalogScan> {
        self.ensure_root()?;
        let mut result = CatalogScan::default();
        let mut bytes = 0u64;
        for (index, item) in self.directory.entries()?.enumerate() {
            if index >= MAX_SCAN_ENTRIES {
                result.truncated = true;
                break;
            }
            let item = match item {
                Ok(item) => item,
                Err(e) => {
                    result.diagnostics.push(CatalogDiagnostic {
                        path: self.root.clone(),
                        message: e.to_string(),
                    });
                    continue;
                }
            };
            let name = item.file_name();
            let name = name.to_string_lossy();
            let Some(id) = name.strip_suffix(".confuse-archive.json") else {
                continue;
            };
            let path = self.root.join(name.as_ref());
            let metadata = self.directory.symlink_metadata(name.as_ref())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                result.diagnostics.push(CatalogDiagnostic {
                    path,
                    message: "Catalog entry must be a regular file".into(),
                });
                continue;
            }
            if bytes.saturating_add(metadata.len()) > MAX_SCAN_BYTES {
                result.truncated = true;
                continue;
            }
            bytes += metadata.len();
            match self.read_bundle(id) {
                Ok((_, entry)) => {
                    if entry.trashed == trashed {
                        result.entries.push(entry);
                    }
                }
                Err(e) => result.diagnostics.push(CatalogDiagnostic {
                    path,
                    message: e.to_string(),
                }),
            }
        }
        result.entries.sort_by(|a, b| {
            b.imported_at_ms
                .cmp(&a.imported_at_ms)
                .then(a.id.cmp(&b.id))
        });
        Ok(result)
    }
    pub fn load(&self, entry: &CatalogEntry) -> Result<Archive> {
        let (bundle, _) = self.checked(entry)?;
        bundle.archive()
    }
    pub fn delete(&self, entry: &CatalogEntry) -> Result<CatalogEntry> {
        let (_, actual) = self.checked(entry)?;
        if actual.trashed {
            return Err(error("Archive is already in recovery"));
        }
        self.publish_new(
            Path::new(&format!("{}.deleted", entry.id)),
            entry.fingerprint.as_bytes(),
        )?;
        self.read_bundle(&entry.id).map(|(_, entry)| entry)
    }
    pub fn restore(&self, entry: &CatalogEntry) -> Result<CatalogEntry> {
        let (_, actual) = self.checked(entry)?;
        if !actual.trashed {
            return Err(error("Archive is not in recovery"));
        }
        self.directory
            .remove_file(format!("{}.deleted", entry.id))?;
        self.read_bundle(&entry.id).map(|(_, entry)| entry)
    }
    pub fn extract(
        &self,
        entry: &CatalogEntry,
        slots: &[u16],
        library: &PresetLibrary,
    ) -> Result<Vec<LibraryEntry>> {
        let archive = self.load(entry)?;
        let mut selected = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        if slots.is_empty() {
            return Err(error("Choose at least one preset to extract"));
        }
        for slot in slots {
            if !seen.insert(*slot) {
                return Err(error("Duplicate extraction slot"));
            }
            let preset = archive
                .presets
                .iter()
                .find(|p| p.slot == *slot)
                .ok_or_else(|| error("Selected source slot is absent"))?;
            let name = preset
                .document
                .metadata()
                .name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .take(64)
                .collect::<String>();
            selected.push((format!("{:03}_{}.fuse", slot, name), &preset.document));
        }
        let mut imported = Vec::new();
        for (name, document) in selected {
            match library.import_document(document, &name) {
                Ok(entry) => imported.push(entry),
                Err(e) => {
                    return Err(error(format!(
                        "Extraction stopped after {} successful files (retained in library): {e}",
                        imported.len()
                    )));
                }
            }
        }
        Ok(imported)
    }
    fn checked(&self, entry: &CatalogEntry) -> Result<(Bundle, CatalogEntry)> {
        self.ensure_root()?;
        if entry.path != self.root.join(format!("{}.confuse-archive.json", entry.id)) {
            return Err(error("Entry belongs to another catalog"));
        }
        let result = self.read_bundle(&entry.id)?;
        if result.1.fingerprint != entry.fingerprint || result.1.trashed != entry.trashed {
            return Err(error("Archive changed; refresh the catalog"));
        }
        Ok(result)
    }
    fn read_bundle(&self, id: &str) -> Result<(Bundle, CatalogEntry)> {
        if id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(error("Invalid archive identity"));
        }
        let path = PathBuf::from(format!("{id}.confuse-archive.json"));
        let bytes = self.read_file(&path, MAX_ARCHIVE_BYTES)?;
        let bundle: Bundle = serde_json::from_slice(&bytes).map_err(error)?;
        if bundle.format != "confuse-archive"
            || bundle.version != 1
            || bundle.label.len() > 1024
            || bundle.sha256 != bundle.checksum()?
            || bundle.id()? != id
        {
            return Err(error("Invalid archive format or checksum"));
        }
        let archive = bundle.archive()?;
        let fingerprint = digest(&bytes);
        let marker = PathBuf::from(format!("{id}.deleted"));
        let trashed = match self.directory.symlink_metadata(&marker) {
            Ok(_) => {
                let value = self.read_file(&marker, 64)?;
                if value != fingerprint.as_bytes() {
                    return Err(error(
                        "Archive recovery marker does not match the retained bundle",
                    ));
                }
                true
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        let mut families = archive
            .presets
            .iter()
            .filter_map(|p| p.document.product_id())
            .collect::<Vec<_>>();
        families.sort();
        families.dedup();
        let entry = CatalogEntry {
            id: id.into(),
            label: bundle.label.clone(),
            imported_at_ms: bundle.imported_at_ms,
            pid: archive
                .native
                .as_ref()
                .map(Backup::pid)
                .or_else(|| {
                    archive
                        .auxiliary
                        .as_ref()
                        .map(crate::auxiliary::Snapshot::pid)
                })
                .or_else(|| archive.control_preset.as_ref().map(|d| d.pid())),
            families,
            preset_count: archive.presets.len(),
            original_bytes: bundle.files.iter().map(|f| (f.hex.len() / 2) as u64).sum(),
            kind: bundle.kind,
            names: archive
                .presets
                .iter()
                .map(|p| p.document.metadata().name)
                .chain(archive.control_preset.iter().map(|d| d.name()))
                .collect(),
            trashed,
            path: self.root.join(path),
            fingerprint,
        };
        Ok((bundle, entry))
    }
    pub(crate) fn read_file(&self, path: &Path, limit: u64) -> Result<Vec<u8>> {
        self.ensure_root()?;
        let metadata = self.directory.symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
            return Err(error(
                "Expected a bounded regular file without symbolic links",
            ));
        }
        let mut file = self.directory.open(path)?;
        let before = file.metadata()?;
        if !before.is_file()
            || before.len() > limit
            || identity(&metadata) != identity(&before)
            || metadata.len() != before.len()
            || metadata.modified().ok() != before.modified().ok()
        {
            return Err(error("Source changed or exceeds size limit"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit + 1)
            .read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        if bytes.len() as u64 > limit
            || identity(&before) != identity(&after)
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err(error("File changed while being read or exceeds size limit"));
        }
        Ok(bytes)
    }
    pub(crate) fn regular_file_size(&self, path: &Path) -> Result<u64> {
        self.ensure_root()?;
        let metadata = self.directory.symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(error("Expected a regular file without symbolic links"));
        }
        Ok(metadata.len())
    }
    pub(crate) fn bounded_names(&self, limit: usize) -> Result<(Vec<PathBuf>, bool)> {
        self.ensure_root()?;
        let mut names = Vec::new();
        for (index, entry) in self.directory.entries()?.enumerate() {
            if index == limit {
                return Ok((names, true));
            }
            names.push(PathBuf::from(entry?.file_name()));
        }
        Ok((names, false))
    }
    pub(crate) fn file_exists(&self, path: &Path) -> Result<bool> {
        self.ensure_root()?;
        match self.directory.symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
    pub(crate) fn remove_file(&self, path: &Path) -> Result<()> {
        self.ensure_root()?;
        self.directory.remove_file(path)?;
        Ok(())
    }
    pub(crate) fn publish_new(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.ensure_root()?;
        let tmp = format!(
            ".confuse-archive-tmp-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = self.directory.open_with(&tmp, &options)?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            self.directory.hard_link(&tmp, &self.directory, path)
        })()
        .map_err(ArchiveError::from);
        drop(file);
        let _ = self.directory.remove_file(tmp);
        result
    }
}
fn identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    }
    #[cfg(not(unix))]
    {
        format!("{:?}", metadata.created())
    }
}
