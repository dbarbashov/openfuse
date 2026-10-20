use super::*;
/// Original FixGDECXmlTag condition, applied to a copy. Other inputs retain bytes.
pub fn fix_gdec_tag(xml: &str) -> Result<(String, Vec<String>)> {
    if xml.len() > crate::preset::MAX_PRESET_BYTES {
        return Err(error("Preset exceeds 2 MiB"));
    }
    let repaired = crate::conversion::repair_double_encoded_xml(xml).map_err(error)?;
    let doc = roxmltree::Document::parse(&repaired).map_err(error)?;
    let root = doc.root_element();
    if !root.has_tag_name("Preset") {
        return Err(error("Expected Preset root"));
    }
    if root
        .attribute("ProductId")
        .and_then(|s| s.trim().parse::<u16>().ok())
        != Some(10)
        || root.attribute("amplifier") != Some("G-DEC 3.0")
    {
        return Ok((xml.into(), Vec::new()));
    }
    let attr = root.attributes().find(|a| a.name() == "amplifier").unwrap();
    let range = attr.range_value();
    let mut output = repaired.clone();
    output.replace_range(range, "G-DEC3");
    let mut changes = vec!["Preset amplifier alias G-DEC 3.0 to G-DEC3 in destination copy".into()];
    if repaired != xml {
        changes
            .push("Original single-pass double-entity repair applied to destination copy".into());
    }
    Ok((output, changes))
}
