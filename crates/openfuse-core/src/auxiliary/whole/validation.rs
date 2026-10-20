use super::*;

pub(super) fn validate_normalizations(
    pid: u16,
    original: &[Report64],
    expected: &[Report64],
    section: &str,
    changes: &mut Vec<Normalization>,
) -> Result<()> {
    let identity = |r: &Report64| {
        (
            r.as_bytes()[2],
            if r.as_bytes()[2] == 10 {
                r.as_bytes()[25]
            } else {
                0
            },
        )
    };
    if original.len() != expected.len() {
        return Err(error(format!("{section}: unknown report count")));
    }
    let mut seen = BTreeSet::new();
    for source in original {
        if !seen.insert(identity(source)) {
            return Err(error(format!("{section}: duplicate report identity")));
        }
        let target = expected
            .iter()
            .find(|r| identity(r) == identity(source))
            .ok_or_else(|| error(format!("{section}: unsupported report kind")))?;
        let a = source.as_bytes();
        let b = target.as_bytes();
        if a[0] != 0x1c || a[1] & !0xc7 != 0 || a[6] & !1 != 0 || a[7] & !1 != 0 {
            return Err(error(format!("{section}: unknown header bits")));
        }
        let mut known = [0u8; 64];
        match a[2] {
            4 => known[16..].fill(255),
            5..=9 => {
                if a[16..18] != b[16..18] {
                    return Err(error(format!("{section}: model substitution refused")));
                }
                let category = wire_preset::CATEGORIES
                    .iter()
                    .find(|(k, _, _)| *k == a[2])
                    .unwrap()
                    .2;
                let model = crate::model_catalog::model(
                    wire_preset::product_for_pid(pid).unwrap(),
                    category,
                    u16::from_le_bytes([a[16], a[17]]),
                )
                .ok_or_else(|| error("Unknown model"))?;
                for p in &model.parameters {
                    known[32 + usize::from(p.index)] = 255;
                }
                if a[2] == 5 {
                    known[52] |= 64;
                    known[53] = 255;
                } else {
                    known[18] = 255;
                    known[19] = 255;
                    known[20] = 255;
                    known[21] = if a[21] <= 3 { 255 } else { 0 };
                    known[22] = 1;
                }
            }
            10 => {
                for i in [16, 17, 18, 20, 22, 23, 24, 26, 27, 28] {
                    known[i] = 255;
                }
                known[19] = 1;
                if a[25] == 1 {
                    known[21] = 255;
                }
            }
            13 => known[16] = 255,
            14 => known[16] = 1,
            0 => (),
            _ => return Err(error(format!("{section}: unknown report"))),
        }
        for offset in 8..64 {
            if a[offset] == b[offset] {
                continue;
            }
            if (a[offset] ^ b[offset]) & !known[offset] != 0 {
                return Err(error(format!(
                    "{section}: unknown/reserved report {} byte {offset} would be lost",
                    a[2]
                )));
            }
            changes.push(Normalization {
                section: section.into(),
                kind: a[2],
                offset,
                before: u16::from(a[offset]),
                after: u16::from(b[offset]),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_xml(document: &PresetDocument) -> Result<()> {
    let source = document.to_xml().map_err(error)?;
    let xml = roxmltree::Document::parse(&source).map_err(error)?;
    let root = xml.root_element();
    let roots = [
        "Amplifier",
        "FX",
        "FUSE",
        "Band",
        "FirstExpressionPedal",
        "SecondExpressionPedal",
        "UsbGain",
        "FxLoopBypass",
    ];
    codec::check(root, &["ProductId", "amplifier"], &roots)?;
    for key in roots {
        codec::child(root, key)?;
    }
    let fx = codec::child(root, "FX")?.ok_or_else(|| error("Missing FX"))?;
    codec::check(fx, &[], &["Stompbox", "Modulation", "Delay", "Reverb"])?;
    for (_, key, _) in wire_preset::CATEGORIES {
        let category = codec::child(if key == "Amplifier" { root } else { fx }, key)?
            .ok_or_else(|| error("Missing category"))?;
        codec::check(category, &["ID"], &["Module"])?;
        let module = codec::child(category, "Module")?.ok_or_else(|| error("Missing Module"))?;
        codec::check(
            module,
            &["ID", "POS", "BypassState", "BypassMode"],
            &["Param"],
        )?;
        for p in module.children().filter(|n| n.has_tag_name("Param")) {
            codec::check(p, &["ControlIndex"], &[])?;
        }
    }
    for key in ["UsbGain", "FxLoopBypass"] {
        if let Some(node) = codec::child(root, key)? {
            codec::check(node, &[], &[])?;
            if node.children().filter(|n| n.is_text()).count() > 1 {
                return Err(error(
                    "Split scalar text requires explicit XML normalization before whole restore",
                ));
            }
        }
    }
    for key in ["FirstExpressionPedal", "SecondExpressionPedal"] {
        if let Some(node) = codec::child(root, key)? {
            codec::check(
                node,
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
                    "PedalIndex",
                ],
                &[],
            )?;
            if node
                .children()
                .any(|n| n.is_text() && !n.text().unwrap_or("").trim().is_empty())
            {
                return Err(error("Expression attributes contain unknown text"));
            }
        }
    }
    Ok(())
}
