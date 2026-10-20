use openfuse_core::{
    media_library::{MediaEntry, MediaLibrary, MediaScan},
    preset::PresetDocument,
    preset_file,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
fn entry(e: &MediaEntry) -> Value {
    json!({"id":e.id,"name":e.name,"original_path":e.original_path,"format":e.format,"size":e.size,"sha256":e.sha256,"imported_at_ms":e.imported_at_ms,"trashed":e.trashed,"managed_path":e.path,"local_playable_format":e.format!=openfuse_core::media_library::MediaFormat::Midi})
}
fn scan(s: MediaScan) -> Value {
    json!({"entries":s.entries.iter().map(entry).collect::<Vec<_>>(),"diagnostics":s.diagnostics.iter().map(|d|json!({"path":d.path,"message":d.message})).collect::<Vec<_>>(),"truncated":s.truncated})
}
pub fn run(command: &str, args: Vec<OsString>) -> Result<(), String> {
    let mut values = BTreeMap::new();
    let mut trash = false;
    let mut args = args.into_iter();
    while let Some(key) = args.next() {
        let key = key.to_str().ok_or("Invalid option")?.to_owned();
        if key == "--trash" && command == "media-list" && !trash {
            trash = true;
            continue;
        }
        let allowed = match command {
            "media-list" => key == "--library",
            "media-import" => matches!(key.as_str(), "--library" | "--path"),
            "band-inspect" => key == "--file",
            _ => false,
        };
        if !allowed || values.contains_key(&key) {
            return Err(format!("Unknown or duplicate option: {key}"));
        }
        values.insert(key, args.next().ok_or("Missing option value")?);
    }
    let path = |key: &str| {
        values
            .get(key)
            .map(PathBuf::from)
            .ok_or_else(|| format!("Missing {key}"))
    };
    let data = if command == "band-inspect" {
        let raw = preset_file::read(path("--file")?).map_err(|e| e.to_string())?;
        let doc = PresetDocument::parse(&raw).map_err(|e| e.to_string())?;
        json!({"product_id":doc.product_id(),"band":doc.band().map_err(|e|e.to_string())?})
    } else {
        let root = path("--library")?;
        if command == "media-list" && !root.is_dir() {
            return Err("Media library does not exist".into());
        }
        let library = MediaLibrary::open(root).map_err(|e| e.to_string())?;
        if command == "media-import" {
            let source = path("--path")?;
            if source.is_dir() {
                scan(
                    library
                        .import_directory(source)
                        .map_err(|e| e.to_string())?,
                )
            } else {
                json!({"imported":entry(&library.import_file(source).map_err(|e|e.to_string())?)})
            }
        } else {
            scan(
                if trash {
                    library.trash()
                } else {
                    library.scan()
                }
                .map_err(|e| e.to_string())?,
            )
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"kind":command,"device_access":false,"playback_started":false,"data":data})
        )
        .map_err(|e| e.to_string())?
    );
    Ok(())
}
