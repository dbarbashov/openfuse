use super::*;
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct LegacyDelay {
    pub normalized: u16,
    pub milliseconds: u16,
    pub was_dirty: bool,
}
/// Original FixLegacyDelayTimesInput followed by AsFileOutput's millisecond
/// projection. Endpoints and unchecked ushort wrap intentionally match CIL.
pub fn legacy_delay_value(type_id: u16, input: u16) -> Result<LegacyDelay> {
    let length = match type_id {
        136 => 1111u32,
        137 => 556,
        138 => 371,
        _ => return Err(error("Not an evidenced G-DEC delay-time type")),
    };
    let inside = (30..=length).contains(&u32::from(input));
    let normalized = if inside {
        (((f64::from(input) - 29.0) / f64::from(length - 30) * 65535.0) as u32) as u16
    } else {
        input
    };
    let milliseconds = (30.0 + f64::from(length - 31) * (f64::from(normalized) / 65535.0)) as u16;
    Ok(LegacyDelay {
        normalized,
        milliseconds,
        was_dirty: !inside,
    })
}
pub fn repair_double_encoded_xml(source: &str) -> Result<String> {
    repair(source, &mut vec![])
}
pub(super) fn repair(source: &str, changes: &mut Vec<Change>) -> Result<String> {
    if source.len() > preset::MAX_PRESET_BYTES {
        return Err(error("Preset XML exceeds 2 MiB"));
    }
    let mut result = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("&amp;") {
        let begin = cursor + relative;
        result.push_str(&source[cursor..begin]);
        let content = begin + 5;
        let Some(length) = source[content..].find(';') else {
            result.push_str(&source[begin..]);
            return Ok(result);
        };
        let end = content + length + 1;
        let entity = &source[content..end - 1];
        let numeric = entity
            .strip_prefix('#')
            .map(|s| s.strip_prefix('x').unwrap_or(s))
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()));
        if matches!(entity, "amp" | "quot" | "apos" | "lt" | "gt") || numeric {
            if changes.len() >= MAX_CHANGES {
                return Err(error("Legacy repair exceeds 4096 reported changes"));
            }
            let value = if entity == "apos" {
                "'".into()
            } else {
                format!("&{entity};")
            };
            change(
                changes,
                ChangeKind::Repair,
                format!("XML bytes {begin}..{end}"),
                Some(source[begin..end].into()),
                Some(value.clone()),
                "One original FUSE double-entity repair pass",
            );
            result.push_str(&value);
            cursor = end;
        } else {
            result.push_str("&amp;");
            cursor = content;
        }
    }
    result.push_str(&source[cursor..]);
    Ok(result)
}
