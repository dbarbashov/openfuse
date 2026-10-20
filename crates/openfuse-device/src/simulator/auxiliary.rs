use super::*;
pub(super) type Banks = BTreeMap<(u8, u16), Vec<Report64>>;

pub(super) fn fixtures(pid: u16, document: &PresetDocument) -> Banks {
    let mut banks = Banks::new();
    if pid != 0x0e {
        let mut b = [0; 64];
        b[0] = 0x1c;
        b[1] = 1;
        b[2] = 11;
        for (i, value) in b[16..32].iter_mut().enumerate() {
            *value = i as u8;
        }
        b[32] = 5;
        b[36] = 255;
        banks.insert((0, 0), vec![Report64::from_bytes(b)]);
    }
    let specs: &[(u8, u16, &[u8])] = match pid {
        0x14 => &[(1, 12, &[7]), (2, 12, &[8, 9])],
        0x0e => &[(1, 17, &[5, 6, 13]), (2, 15, &[7, 8, 9, 10])],
        _ => &[],
    };
    if specs.is_empty() {
        return banks;
    }
    let encoded = wire_preset::encode(pid, document)
        .expect("synthetic auxiliary fixture")
        .reports;
    for &(control, count, types) in specs {
        for slot in 0..count {
            let name = if pid == 0x0e && control == 1 && slot == 16 {
                "Clean channel".into()
            } else {
                format!("Simulated bank {control} slot {slot}")
            };
            let mut b = [0; 64];
            b[0] = 0x1c;
            b[1] = 1;
            b[2] = 4;
            b[3] = control;
            b[4..6].copy_from_slice(&slot.to_le_bytes());
            b[16..16 + name.len()].copy_from_slice(name.as_bytes());
            let mut reports = vec![Report64::from_bytes(b)];
            for &kind in types {
                let mut raw = *encoded
                    .iter()
                    .find(|r| r.message_id() == 0x1c && r.as_bytes()[2] == kind)
                    .expect("fixture module")
                    .as_bytes();
                raw[1] = 1;
                raw[3] = control;
                raw[4..6].copy_from_slice(&slot.to_le_bytes());
                raw[6] = 0;
                raw[7] = 0;
                if kind == 10 {
                    raw[17] = 0;
                    raw[18] = 255;
                }
                reports.push(Report64::from_bytes(raw));
            }
            reports.push(frame(&[(0, 0x1c), (1, 1), (3, control)]));
            banks.insert((control, slot), reports);
        }
    }
    banks
}
pub(super) fn superchamp_document() -> PresetDocument {
    use openfuse_core::model_catalog::{self, ModuleCategory};
    let family = 2259;
    let mut xml = format!("<Preset ProductId=\"{family}\">");
    for (key, category, pos) in [
        ("Amplifier", ModuleCategory::Amplifier, 0),
        ("Stompbox", ModuleCategory::Stompbox, 0),
        ("Modulation", ModuleCategory::Modulation, 1),
        ("Delay", ModuleCategory::Delay, 4),
        ("Reverb", ModuleCategory::Reverb, 5),
    ] {
        if key == "Stompbox" {
            xml.push_str("<FX>");
        }
        let model = model_catalog::models(family, category)
            .find(|m| m.id != 0)
            .unwrap();
        let category_id = match key {
            "Amplifier" => 0,
            "Stompbox" => 1,
            "Modulation" => 2,
            "Delay" => 3,
            _ => 4,
        };
        xml.push_str(&format!("<{key} ID=\"{category_id}\"><Module ID=\"{}\" POS=\"{pos}\" BypassState=\"1\" BypassMode=\"{}\">",model.id,model.bypass_mode));
        for p in &model.parameters {
            xml.push_str(&format!(
                "<Param ControlIndex=\"{}\">{}</Param>",
                p.index, p.default_raw
            ));
        }
        xml.push_str(&format!("</Module></{key}>"));
    }
    xml.push_str(
        "</FX><UsbGain>9</UsbGain><FUSE><Info name=\"Simulated Super Champ\"/></FUSE></Preset>",
    );
    let mut document = PresetDocument::parse(&xml).unwrap();
    use openfuse_core::preset::expression::{ExpressionPedal, PedalSlot};
    document
        .set_expression_pedal(
            PedalSlot::First,
            &ExpressionPedal::defaults(PedalSlot::First),
        )
        .unwrap();
    let reports = wire_preset::encode(0x0e, &document).unwrap().reports;
    wire_preset::decode(0x0e, &reports).unwrap().document
}
impl Peer {
    pub(super) fn auxiliary_write(&mut self, report: Report64) -> Result<(), String> {
        let b = report.as_bytes();
        if self.staged.is_some() || self.save.is_some() {
            return Err("Working transaction already active".into());
        }
        match b[2] {
            4 => {
                let key = (b[3], u16::from_le_bytes([b[4], b[5]]));
                if b[1] != 1
                    || b[6] != 1
                    || b[7] != 1
                    || self.staged_auxiliary.is_some()
                    || !self.auxiliary.lock().unwrap().contains_key(&key)
                {
                    return Err("Invalid auxiliary Name".into());
                }
                self.staged_auxiliary = Some((key, vec![report]));
            }
            0 => {
                let (key, mut reports) = self
                    .staged_auxiliary
                    .take()
                    .ok_or("Auxiliary Complete without Name")?;
                if b[1] != 3 || b[3] != key.0 || b[4..].iter().any(|v| *v != 0) {
                    return Err("Invalid auxiliary Complete".into());
                }
                reports.push(report);
                for report in &mut reports {
                    let mut raw = *report.as_bytes();
                    raw[1] = 1;
                    raw[6] = 0;
                    raw[7] = 0;
                    *report = Report64::from_bytes(raw);
                }
                self.auxiliary.lock().unwrap().insert(key, reports);
                self.ack(0x1c);
            }
            5..=10 | 13 => {
                let (key, reports) = self
                    .staged_auxiliary
                    .as_mut()
                    .ok_or("Auxiliary module without Name")?;
                if b[1] != 3
                    || b[3] != key.0
                    || u16::from_le_bytes([b[4], b[5]]) != key.1
                    || reports.len() >= 16
                {
                    return Err("Invalid auxiliary module identity".into());
                }
                reports.push(report);
                self.ack(0x1c);
            }
            _ => return Err("Simulator refuses unsupported auxiliary/system write".into()),
        }
        Ok(())
    }
}
