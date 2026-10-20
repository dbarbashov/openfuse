use super::Choice;
use std::ops::RangeInclusive;

pub(super) fn choices(type_id: u16) -> Vec<Choice> {
    include_str!("../data/choices.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let kind: u16 = fields.next()?.parse().ok()?;
            let value = fields.next()?.parse().ok()?;
            let label = fields.next()?.to_owned();
            (kind == type_id).then_some(Choice { value, label })
        })
        .collect()
}
pub(super) fn range(type_id: u16) -> RangeInclusive<u16> {
    match type_id {
        136 => 30..=1110,
        137 => 30..=555,
        138 => 30..=370,
        141 => 0..=9999,
        _ => {
            let choices = choices(type_id);
            match (choices.first(), choices.last()) {
                (Some(first), Some(last)) => first.value..=last.value,
                _ => 0..=65535,
            }
        }
    }
}

// The hundredths scales are established numerically by DisplayStrings. Formatting
// uses explicit units and keeps raw values available in the editor; it is not a
// byte-for-byte recreation of the original's width-limited display strings.
fn scaled(value: u16, low: u32, high: u32, unit: &str) -> String {
    let n = u64::from(low) + u64::from(value) * u64::from(high - low) / 65535;
    format!("{}.{:02}{}", n / 100, n % 100, unit)
}
pub(super) fn format(type_id: u16, value: u16) -> String {
    let choices = choices(type_id);
    if !choices.is_empty() {
        return choices
            .iter()
            .find(|c| c.value == value)
            .map(|c| c.label.clone())
            .unwrap_or_else(|| format!("Custom ({value})"));
    }
    match type_id {
        1 => scaled(value, 0, 10000, " %"),
        4 => scaled(value, 200, 100000, ":1"),
        6 => scaled(value, 3000, 111000, " ms"),
        7 => scaled(value, 3000, 55500, " ms"),
        8 => scaled(value, 3000, 37000, " ms"),
        12 => scaled(value, 100, 1000, ""),
        13 => format!("{:+.0} %", f64::from(value) / 65535.0 * 100.0 - 50.0),
        14 => scaled(value, 130, 1000, " Hz"),
        15 => scaled(value, 67, 567, " Hz"),
        16 => scaled(value, 8, 1000, " Hz"),
        18 => format!("{} %", (-100 + i32::from(value) * 200 / 65535).abs()),
        19 => {
            let cents = -2400 + i32::from(value >> 8) * 240 / 255 * 20;
            format!("{:+.2} semitones", f64::from(cents) / 100.0)
        }
        136..=138 => format!("{value} ms"),
        141 => value.to_string(),
        _ => format!("Raw {value}"),
    }
}
