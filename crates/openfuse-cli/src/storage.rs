//! Mounted-folder inspection and plans only; this CLI never executes a transfer.
use openfuse_core::mounted_storage::{MountedStorage, TransferPlan, Volume};
use serde_json::json;
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

pub fn run(command: &str, args: Vec<OsString>) -> Result<(), String> {
    let mut options = BTreeMap::new();
    let mut files = Vec::new();
    let mut entries = Vec::new();
    let mut keep_both = false;
    let mut args = args.into_iter();
    while let Some(key) = args.next() {
        let key = key.to_str().ok_or("Invalid option")?.to_owned();
        if key == "--keep-both" && command != "storage-inspect" && !keep_both {
            keep_both = true;
            continue;
        }
        let allowed = matches!(key.as_str(), "--root" | "--kind")
            || command == "storage-upload-plan" && key == "--file"
            || command == "storage-download-plan" && matches!(key.as_str(), "--entry" | "--out");
        if !allowed {
            return Err(format!("Unknown option: {key}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {key}"))?;
        match key.as_str() {
            "--file" => files.push(PathBuf::from(value)),
            "--entry" => entries.push(PathBuf::from(value)),
            _ => {
                if options.insert(key.clone(), value).is_some() {
                    return Err(format!("Duplicate option: {key}"));
                }
            }
        }
    }
    let value = |key: &str| options.get(key).ok_or_else(|| format!("Missing {key}"));
    let volume = match value("--kind")?.to_str() {
        Some("sd") => Volume::Sd,
        Some("internal") => Volume::Internal,
        _ => return Err("--kind must be sd or internal".into()),
    };
    let storage =
        MountedStorage::open(PathBuf::from(value("--root")?), volume).map_err(|e| e.to_string())?;
    let data = match command {
        "storage-inspect" => serde_json::to_value(storage.scan().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
        "storage-upload-plan" => {
            if files.is_empty() {
                return Err("Upload plan requires at least one --file".into());
            }
            let plan = if keep_both {
                TransferPlan::upload_keep_both(&storage, &files)
            } else {
                TransferPlan::upload(&storage, &files)
            }
            .map_err(|e| e.to_string())?;
            json!({"direction":plan.direction(),"items":plan.items()})
        }
        "storage-download-plan" => {
            if entries.is_empty() {
                return Err("Download plan requires at least one --entry relative/path".into());
            }
            let inventory = storage.scan().map_err(|e| e.to_string())?;
            let selected = entries
                .iter()
                .map(|path| {
                    inventory
                        .entries
                        .iter()
                        .find(|entry| &entry.relative_path == path)
                        .cloned()
                        .ok_or_else(|| {
                            format!("Entry absent from storage inventory: {}", path.display())
                        })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let destination = PathBuf::from(value("--out")?);
            let plan = if keep_both {
                TransferPlan::download_keep_both(&storage, &selected, &destination)
            } else {
                TransferPlan::download(&storage, &selected, &destination)
            }
            .map_err(|e| e.to_string())?;
            json!({"direction":plan.direction(),"items":plan.items()})
        }
        _ => return Err("Unknown storage command".into()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "kind":command,"transmitted":false,"files_written":false,
            "usb_access":false,"hardware_validated":false,"data":data
        }))
        .map_err(|e| e.to_string())?
    );
    Ok(())
}
