use super::*;
use roxmltree::{Document, ParsingOptions};
fn slot_name(name: &str) -> Option<u16> {
    let stem = name.strip_suffix(".fuse")?;
    let slot = stem.split_once('_').map_or(stem, |v| v.0);
    if slot.len() != 2 || !slot.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    slot.parse().ok()
}
pub(super) fn inspect(path: &str, bytes: &[u8]) -> (EntryKind, EntryMetadata) {
    let lower = path.to_ascii_lowercase();
    let parts = lower.split('/').collect::<Vec<_>>();
    let leaf = *parts.last().unwrap();
    let slot = slot_name(leaf);
    let kind = if parts.len() == 1
        && matches!(
            leaf,
            "backupname.fuse"
                | "gd_backupname.fuse"
                | "mu_backupname.fuse"
                | "m2_backupname.fuse"
                | "br_backupname.fuse"
                | "sc_backupname.fuse"
                | "pm_backupname.fuse"
        ) {
        EntryKind::Description
    } else if parts.len() == 2 && parts[0] == "presets" && slot.is_some() {
        EntryKind::Preset
    } else if parts.len() == 2 && parts[0] == "fuse" && leaf.ends_with(".fuse") {
        EntryKind::FuseMetadata
    } else if parts.len() == 1 && leaf == "systemsettings.fuse" {
        EntryKind::SystemSettings
    } else if parts.len() == 2 && parts[0] == "voice" && leaf.ends_with(".fuse") {
        EntryKind::VoiceBank
    } else if ((parts.len() == 3
        && parts[0] == "effects"
        && matches!(parts[1], "mod" | "dlyrvb" | "fx" | "effects"))
        || (parts.len() == 2 && parts[0] == "fx"))
        && leaf.ends_with(".fuse")
    {
        EntryKind::EffectBank
    } else if parts.len() >= 3
        && matches!(parts[0], "hd" | "sd")
        && matches!(parts[1], "audio" | "midi")
    {
        EntryKind::BandTrack
    } else {
        EntryKind::Unknown
    };
    let mut metadata = EntryMetadata {
        slot,
        ..Default::default()
    };
    if matches!(
        kind,
        EntryKind::Preset
            | EntryKind::FuseMetadata
            | EntryKind::VoiceBank
            | EntryKind::EffectBank
            | EntryKind::SystemSettings
    ) {
        if bytes.len() > MAX_PRESET_BYTES {
            metadata
                .warnings
                .push("Auxiliary XML exceeds inspection size; original bytes retained".into());
        } else {
            let doc = std::str::from_utf8(bytes).ok().and_then(|s| {
                Document::parse_with_options(
                    s,
                    ParsingOptions {
                        allow_dtd: false,
                        nodes_limit: 65536,
                        ..Default::default()
                    },
                )
                .ok()
            });
            if let Some(doc) = doc {
                let root = doc.root_element();
                metadata.xml_root = Some(root.tag_name().name().into());
                metadata.name = root.attribute("Name").map(str::to_owned);
                metadata.amplifier = root.attribute("amplifier").map(str::to_owned);
                let expected = match kind {
                    EntryKind::Preset => "Preset",
                    EntryKind::SystemSettings => "System",
                    EntryKind::VoiceBank => "VOICE",
                    EntryKind::EffectBank => "FX",
                    _ => "FUSE",
                };
                if root.tag_name().name() != expected || root.tag_name().namespace().is_some() {
                    metadata.warnings.push(format!(
                        "Expected {expected} XML root; preserved without hardware interpretation"
                    ));
                }
                if doc
                    .descendants()
                    .any(|n| n.ancestors().take(66).count() > 65)
                {
                    metadata.warnings.push(
                        "XML exceeds inspection depth; retained without deeper interpretation"
                            .into(),
                    );
                }
            } else {
                metadata
                    .warnings
                    .push("Malformed or unsupported auxiliary XML; original bytes retained".into());
            }
        }
    }
    if kind == EntryKind::Unknown {
        metadata
            .warnings
            .push("Unrecognized entry preserved unchanged".into());
    }
    (kind, metadata)
}
