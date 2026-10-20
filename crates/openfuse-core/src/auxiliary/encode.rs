use super::codec::{check, child, frame, number, scalar};
use super::*;
use crate::model_catalog;
use roxmltree::Document;

/// Offline packet construction for an evidenced direct physical-bank save.
/// No USB access, restore envelope or system write is performed.
pub fn plan_xml(pid: u16, key: Key, xml: &str) -> Result<Vec<Report64>> {
    if xml.len() > 2 * 1024 * 1024 {
        return Err(error("Auxiliary XML exceeds 2 MiB"));
    }
    let Key::Bank { control, slot } = key else {
        return Err(error("Selective system writes have not been established"));
    };
    let caps = capabilities(pid)?;
    if slot
        >= *caps
            .banks
            .get(&control)
            .ok_or_else(|| error("Unsupported auxiliary bank"))?
    {
        return Err(error("Auxiliary target slot out of range"));
    }
    let required = kinds(pid, control).unwrap();
    let doc = Document::parse(xml).map_err(error)?;
    let root = doc.root_element();
    let tag = if pid == 14 && control == 1 {
        "VOICE"
    } else {
        "FX"
    };
    if !root.has_tag_name(tag) {
        return Err(error("Auxiliary XML root does not match bank"));
    }
    let mut allowed = wire_preset::CATEGORIES
        .iter()
        .filter(|(k, _, _)| required.contains(k))
        .map(|(_, key, _)| *key)
        .collect::<Vec<_>>();
    if required.contains(&13) {
        allowed.push("UsbGain");
    }
    if required.contains(&10) {
        allowed.push("FirstExpressionPedal");
    }
    check(root, &["Name", "amplifier"], &allowed)?;
    if root
        .attribute("amplifier")
        .is_some_and(|s| s != "SuperChampX2")
        || pid != 14 && root.attribute("amplifier").is_some()
    {
        return Err(error("Auxiliary amplifier family does not match"));
    }
    let name = root
        .attribute("Name")
        .ok_or_else(|| error("Missing auxiliary Name"))?;
    let mut n = frame(4, control, slot);
    n[1] = 1;
    n[6] = 1;
    n[7] = 1;
    for (i, c) in name.chars().enumerate() {
        if i >= 48 || c == '\0' || u32::from(c) > 255 {
            return Err(error("Auxiliary name must fit 48 non-NUL Latin bytes"));
        }
        n[16 + i] = c as u8;
    }
    let mut out = vec![Report64::from_bytes(n)];
    let product = wire_preset::product_for_pid(pid).unwrap();
    let pedal = child(root, "FirstExpressionPedal")?;
    if let Some(p) = pedal {
        if p.children()
            .any(|n| n.is_text() && !n.text().unwrap_or("").trim().is_empty())
        {
            return Err(error("Unexpected expression XML text"));
        }
        check(
            p,
            &[
                "VolumeModeBehavior",
                "ExpressionModeBehavior",
                "ParameterIndex",
                "HeelSetting",
                "ToeSetting",
                "PedalMode",
                "BypassEffectWhenVolumeMode",
                "VolumeSwitchRevert",
                "DefaultPedalState",
                "PedalOverrideState",
            ],
            &[],
        )?;
    }
    for &(kind, key, category) in &wire_preset::CATEGORIES {
        if !required.contains(&kind) {
            continue;
        }
        let Some(wrapper) = child(root, key)? else {
            if pid == 14 && control == 1 && slot == 16 && kind == 6 {
                continue;
            }
            return Err(error(format!("Missing {key}")));
        };
        check(wrapper, &["ID"], &["Module"])?;
        if wrapper.attribute("ID").is_some() && number(wrapper, "ID")? != u16::from(kind - 5) {
            return Err(error("Category ID differs from XML category"));
        }
        let m = child(wrapper, "Module")?.ok_or_else(|| error("Missing Module"))?;
        check(m, &["ID", "POS", "BypassState", "BypassMode"], &["Param"])?;
        let model = model_catalog::model(product, category, number(m, "ID")?)
            .ok_or_else(|| error("Unknown model cannot be written"))?;
        let mut values = BTreeMap::new();
        for p in m.children().filter(|n| n.has_tag_name("Param")) {
            check(p, &["ControlIndex"], &[])?;
            let index = number(p, "ControlIndex")?;
            let value = scalar(p).trim().parse::<u16>().map_err(error)?;
            if values.insert(index, value).is_some() {
                return Err(error("Duplicate parameter"));
            }
        }
        if values.len() != model.parameters.len()
            || model
                .parameters
                .iter()
                .any(|p| !values.contains_key(&u16::from(p.index)))
        {
            return Err(error(
                "Module must contain its complete known parameter set",
            ));
        }
        let mut b = frame(kind, control, slot);
        b[7] = 1;
        b[16..18].copy_from_slice(&model.id.to_le_bytes());
        for p in &model.parameters {
            let value = values[&u16::from(p.index)];
            b[32 + usize::from(p.index)] = if p.type_id < 128 {
                (value >> 8) as u8
            } else {
                u8::try_from(value).map_err(error)?
            };
        }
        let expression = if let Some(p) = pedal {
            if number(p, "ExpressionModeBehavior")? == u16::from(kind - 4) && model.id != 0 {
                u8::try_from(number(p, "ParameterIndex")?).map_err(error)?
            } else {
                model.expression_pedal_index
            }
        } else {
            model.expression_pedal_index
        };
        let active = match number(m, "BypassState")? {
            0 => false,
            1 => true,
            _ => return Err(error("Unknown bypass state")),
        };
        if kind == 5 {
            if m.attribute("BypassMode")
                .map(str::parse::<u8>)
                .transpose()
                .map_err(error)?
                .is_some_and(|v| v != model.bypass_mode)
            {
                return Err(error("Amplifier bypass mode cannot be represented"));
            }
            if number(m, "POS")? != 0 {
                return Err(error("Amplifier position must be zero"));
            }
            if active {
                b[52] &= !64;
            } else {
                b[52] |= 64;
            }
            b[53] = expression;
        } else {
            b[18] = u8::try_from(number(m, "POS")?).map_err(error)?;
            b[19] = expression;
            b[20] = model.tap_parameter_index;
            b[21] = m
                .attribute("BypassMode")
                .map(str::parse::<u8>)
                .transpose()
                .map_err(error)?
                .unwrap_or(model.bypass_mode);
            b[22] = u8::from(!active);
        }
        out.push(Report64::from_bytes(b));
    }
    if required.contains(&13) {
        let node = child(root, "UsbGain")?.ok_or_else(|| error("Missing UsbGain"))?;
        check(node, &[], &[])?;
        let gain = scalar(node).trim().parse::<u8>().map_err(error)?;
        let mut b = frame(13, control, slot);
        b[6] = 1;
        b[7] = 1;
        b[16] = if out
            .iter()
            .any(|r| r.as_bytes()[2] == 5 && r.as_bytes()[16..18] == [0, 0])
        {
            0
        } else {
            gain
        };
        out.push(Report64::from_bytes(b));
    }
    if required.contains(&10) {
        let mut b = frame(10, control, slot);
        b[18] = 255;
        let valid = pedal
            .and_then(|p| number(p, "ExpressionModeBehavior").ok())
            .is_some_and(|mode| {
                mode == 6
                    || (3..=5).contains(&mode)
                        && out.iter().any(|r| {
                            (7..=9).contains(&r.as_bytes()[2])
                                && u16::from(r.as_bytes()[2]) == mode + 4
                                && r.as_bytes()[16..18] != [0, 0]
                        })
            });
        if let Some(p) = pedal.filter(|_| valid) {
            for (attr, index) in [
                ("VolumeModeBehavior", 16),
                ("ExpressionModeBehavior", 20),
                ("PedalMode", 24),
                ("VolumeSwitchRevert", 26),
                ("DefaultPedalState", 27),
                ("PedalOverrideState", 28),
            ] {
                b[index] = u8::try_from(number(p, attr)?).map_err(error)?;
            }
            b[19] = u8::from(number(p, "BypassEffectWhenVolumeMode")? != 0);
            b[22] = (number(p, "HeelSetting")? >> 8) as u8;
            b[23] = (number(p, "ToeSetting")? >> 8) as u8;
        } else {
            b[20] = 6;
            b[23] = 255;
            b[24] = 1;
        }
        out.push(Report64::from_bytes(b));
    }
    out.push(Report64::from_bytes(frame(0, control, 0)));
    Ok(out)
}
