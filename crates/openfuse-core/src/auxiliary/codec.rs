use super::*;
use crate::model_catalog;
use roxmltree::{Document, Node};
const SYSTEM_FIELDS: [&str; 6] = [
    "LCDContrast",
    "MemoryLock",
    "Foot2BtnMode",
    "ExpPedalVolumeHeel",
    "ExpPedalVolumeToe",
    "Foot4BtnMode",
];
fn escape(text: &str) -> Result<String> {
    let mut result = String::new();
    for c in text.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            '>' => result.push_str("&gt;"),
            c if c < ' ' => return Err(error("Unrepresentable XML control character")),
            c => result.push(c),
        }
    }
    Ok(result)
}
pub(super) fn check(node: Node<'_, '_>, attrs: &[&str], children: &[&str]) -> Result<()> {
    if !children.is_empty()
        && node
            .children()
            .any(|n| n.is_text() && !n.text().unwrap_or("").trim().is_empty())
    {
        return Err(error("Unexpected mixed XML text"));
    }
    if node.tag_name().namespace().is_some()
        || node
            .attributes()
            .any(|a| a.namespace().is_some() || !attrs.contains(&a.name()))
        || node
            .children()
            .filter(|n| n.is_element())
            .any(|n| n.tag_name().namespace().is_some() || !children.contains(&n.tag_name().name()))
    {
        return Err(error(format!(
            "Unknown {} XML extension; original retained",
            node.tag_name().name()
        )));
    }
    Ok(())
}
pub(super) fn scalar(node: Node<'_, '_>) -> String {
    node.children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>()
}
pub(super) fn number(node: Node<'_, '_>, name: &str) -> Result<u16> {
    node.attribute(name)
        .ok_or_else(|| error(format!("Missing {name}")))?
        .parse()
        .map_err(error)
}
pub(super) fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Option<Node<'a, 'i>>> {
    let mut children = node.children().filter(|n| n.has_tag_name(name));
    let value = children.next();
    if children.next().is_some() {
        return Err(error(format!("Duplicate {name}")));
    }
    Ok(value)
}
pub(super) fn frame(kind: u8, control: u8, slot: u16) -> [u8; 64] {
    let mut b = [0; 64];
    b[0] = 0x1c;
    b[1] = 3;
    b[2] = kind;
    b[3] = control;
    b[4..6].copy_from_slice(&slot.to_le_bytes());
    b
}
fn system_xml(raw: &Report64) -> String {
    let b = raw.as_bytes();
    let mut xml = "<System>".to_string();
    for value in &b[16..32] {
        xml.push_str(&format!("<QA>{value}</QA>"));
    }
    for (i, name) in SYSTEM_FIELDS.iter().enumerate() {
        xml.push_str(&format!("<{name}>{}</{name}>", b[32 + i]));
    }
    xml.push_str("</System>");
    xml
}
/// Offline reproduction of the original system XML decoder. This is not an
/// observed report and is never included in a selective persistent-write plan.
pub fn decode_system_xml(xml: &str) -> Result<Report64> {
    if xml.len() > 65536 {
        return Err(error("System XML exceeds 64 KiB"));
    }
    let doc = Document::parse(xml).map_err(error)?;
    let root = doc.root_element();
    if !root.has_tag_name("System") {
        return Err(error("Expected System root"));
    }
    let mut names = SYSTEM_FIELDS.to_vec();
    names.push("QA");
    check(root, &[], &names)?;
    let mut b = frame(11, 0, 0);
    b[1] = 1;
    b[6] = 1;
    b[7] = 1;
    let qa = root
        .children()
        .filter(|n| n.has_tag_name("QA"))
        .collect::<Vec<_>>();
    if qa.len() != 16 {
        return Err(error("System requires sixteen QA values"));
    }
    let value = |n: Node<'_, '_>| -> Result<u8> {
        check(n, &[], &[])?;
        scalar(n).trim().parse().map_err(error)
    };
    for (i, node) in qa.into_iter().enumerate() {
        b[16 + i] = value(node)?;
    }
    for (i, name) in SYSTEM_FIELDS.iter().enumerate() {
        b[32 + i] = match child(root, name)? {
            Some(node) => value(node)?,
            None if i == 5 => 0,
            None => return Err(error(format!("Missing {name}"))),
        };
    }
    Ok(Report64::from_bytes(b))
}
fn module_xml(pid: u16, report: &Report64) -> Result<String> {
    let b = report.as_bytes();
    let (kind, key, category) = wire_preset::CATEGORIES
        .iter()
        .find(|(k, _, _)| *k == b[2])
        .ok_or_else(|| error("Unknown module type"))?;
    let product = wire_preset::product_for_pid(pid).unwrap();
    let model = model_catalog::model(product, *category, u16::from_le_bytes([b[16], b[17]]))
        .ok_or_else(|| error("Unknown model retained in raw capture"))?;
    let pos = if *kind == 5 {
        0
    } else if pid == 14 && b[18] == 4 {
        5
    } else {
        b[18]
    };
    let active = if *kind == 5 {
        b[52] & 64 == 0
    } else {
        b[22] == 0
    };
    let bypass = if *kind == 5 { model.bypass_mode } else { b[21] };
    let mut xml = format!(
        "<{key} ID=\"{}\"><Module ID=\"{}\" POS=\"{pos}\" BypassState=\"{}\" BypassMode=\"{bypass}\">",
        kind - 5,
        model.id,
        u8::from(active)
    );
    let defaults = model
        .parameters
        .iter()
        .map(|p| (p.index, p.default_raw))
        .collect::<Vec<_>>();
    let values = model
        .parameters
        .iter()
        .map(|p| {
            (
                p.index,
                if p.type_id < 128 {
                    u16::from(b[32 + usize::from(p.index)]) * 256
                } else {
                    u16::from(b[32 + usize::from(p.index)])
                },
            )
        })
        .collect::<Vec<_>>();
    for (index, value) in
        model_catalog::load_wire_parameters(product, *category, &defaults, &values)
    {
        xml.push_str(&format!("<Param ControlIndex=\"{index}\">{value}</Param>"));
    }
    xml.push_str(&format!("</Module></{key}>"));
    Ok(xml)
}
fn project(pid: u16, key: Key, raw: &[Report64]) -> Result<String> {
    let Key::Bank { control, .. } = key else {
        return Ok(system_xml(&raw[0]));
    };
    let name = raw[0].as_bytes()[16..]
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| char::from(c))
        .collect::<String>();
    let tag = if pid == 14 && control == 1 {
        "VOICE"
    } else {
        "FX"
    };
    let mut xml = format!("<{tag} Name=\"{}\"", escape(name.trim_end_matches(' '))?);
    if pid == 14 {
        xml.push_str(" amplifier=\"SuperChampX2\"");
    }
    xml.push('>');
    for report in &raw[1..raw.len() - 1] {
        let b = report.as_bytes();
        match b[2] {
            5..=9 => xml.push_str(&module_xml(pid, report)?),
            13 => xml.push_str(&format!("<UsbGain>{}</UsbGain>", b[16])),
            10 => {
                let parameter = raw
                    .iter()
                    .find(|r| {
                        (3..=5).contains(&b[20]) && b[20].checked_add(4) == Some(r.as_bytes()[2])
                    })
                    .map(|r| r.as_bytes()[19])
                    .unwrap_or(0);
                xml.push_str(&format!("<FirstExpressionPedal VolumeModeBehavior=\"{}\" ExpressionModeBehavior=\"{}\" ParameterIndex=\"{}\" HeelSetting=\"{}\" ToeSetting=\"{}\" PedalMode=\"{}\" BypassEffectWhenVolumeMode=\"{}\" VolumeSwitchRevert=\"{}\" DefaultPedalState=\"{}\" PedalOverrideState=\"{}\"/>",b[16],b[20],parameter,u16::from(b[22])*256,u16::from(b[23])*256,b[24],b[19],b[26],b[27],b[28]));
            }
            _ => return Err(error("Unknown auxiliary record retained in raw capture")),
        }
    }
    xml.push_str(&format!("</{tag}>"));
    Ok(xml)
}
pub(super) fn entry(pid: u16, key: Key, raw: Vec<Report64>) -> Entry {
    let label = match key {
        Key::System => "System settings".into(),
        Key::Bank { control, slot } => format!("{} {}", bank_label(pid, control), slot),
    };
    let projected = project(pid, key, &raw);
    let (xml, restore_error) = match projected {
        Err(e) => (None, Some(e.to_string())),
        Ok(xml) => {
            let validation = if key == Key::System {
                Err(error(
                    "Isolated System writes are unavailable; System is eligible only inside validated whole-device restore",
                ))
            } else {
                plan_xml(pid, key, &xml).and_then(|reports| {
                    if storage::canonical(&reports) != storage::canonical(&raw) {
                        Err(error(
                            "Raw fields cannot be reproduced losslessly by the evidenced writer",
                        ))
                    } else {
                        Ok(())
                    }
                })
            };
            (Some(xml), validation.err().map(|e| e.to_string()))
        }
    };
    let label = if key != Key::System {
        let name = raw[0].as_bytes()[16..]
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| char::from(c))
            .collect::<String>();
        format!("{label} · {}", name.trim_end_matches(' '))
    } else {
        label
    };
    Entry {
        key,
        label,
        xml,
        raw,
        restore_error,
    }
}
pub(super) fn legacy(pid: u16, legacy: &crate::legacy_backup::LegacyBackup) -> Result<Snapshot> {
    capabilities(pid)?;
    let mut entries = BTreeMap::new();
    let mut warnings = legacy.warnings().to_vec();
    for source in legacy.entries() {
        let path = source.path().to_ascii_lowercase();
        if path != "systemsettings.fuse" {
            let parts = path.split('/').collect::<Vec<_>>();
            let filename = parts.last().copied().unwrap_or("");
            let base = filename.strip_suffix(".fuse").unwrap_or("");
            let valid_name = base.len() >= 2
                && base.as_bytes()[..2].iter().all(u8::is_ascii_digit)
                && (base.len() == 2 || base.as_bytes()[2] == b'_');
            if !valid_name || !(parts.len() == 2 || parts.len() == 3 && parts[0] == "effects") {
                continue;
            }
        }
        let control = if path.starts_with("voice/") && pid == 14 {
            Some(1)
        } else if path.starts_with("fx/") && pid == 14 {
            Some(2)
        } else if path.starts_with("effects/mod/") && matches!(pid, 4 | 20) {
            Some(1)
        } else if path.starts_with("effects/dlyrvb/") && matches!(pid, 4 | 20)
            || path.starts_with("effects/fx/") && matches!(pid, 10 | 16)
            || path.starts_with("effects/effects/") && matches!(pid, 16 | 38)
        {
            Some(2)
        } else {
            None
        };
        let key = if path == "systemsettings.fuse" {
            Key::System
        } else if let Some(control) = control {
            let filename = path.rsplit('/').next().unwrap();
            let prefix = filename
                .get(..2)
                .ok_or_else(|| error("Missing auxiliary slot"))?;
            let slot = prefix.parse::<u16>().map_err(error)?;
            Key::Bank { control, slot }
        } else {
            continue;
        };
        let xml = std::str::from_utf8(source.bytes())
            .map_err(error)?
            .trim_start_matches('\u{feff}')
            .to_string();
        let validation = if key == Key::System {
            decode_system_xml(&xml).map(|_| ())
        } else {
            plan_xml(pid, key, &xml).map(|_| ())
        };
        let restore_error = if key == Key::System {
            Some("Isolated System writes are unavailable; System is eligible only inside validated whole-device restore".into())
        } else {
            validation.as_ref().err().map(ToString::to_string)
        };
        if let Err(e) = validation {
            warnings.push(format!("{}: {e}", source.path()));
        }
        if entries
            .insert(
                key,
                Entry {
                    key,
                    label: source.path().into(),
                    xml: Some(xml),
                    raw: vec![],
                    restore_error,
                },
            )
            .is_some()
        {
            return Err(error("Duplicate legacy auxiliary slot"));
        }
    }
    if entries.is_empty() {
        return Err(error("No auxiliary entries match the selected profile"));
    }
    Ok(Snapshot {
        pid,
        entries: entries.into_values().collect(),
        presets: vec![],
        raw: vec![],
        warnings,
        legacy: true,
    })
}
pub(super) fn restore(pid: u16, source: &Entry, target: &Entry) -> Result<(Vec<Report64>, Entry)> {
    let xml = source
        .xml
        .as_deref()
        .ok_or_else(|| error("Missing restorable auxiliary XML"))?;
    let reports = plan_xml(pid, target.key, xml)?;
    let mut expected = entry(pid, target.key, reports.clone());
    expected.restore_error = None;
    Ok((reports, expected))
}
