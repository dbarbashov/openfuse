//! Explicit synthetic Mustang peer, never instantiated by the real USB backend.
use super::{
    DeviceInfo,
    backend::{Backend, Connection, TransportError},
};
use openfuse_core::{preset::PresetDocument, protocol::Report64, wire_preset};
use std::collections::{BTreeMap, VecDeque};
mod auxiliary;
mod whole_restore;
use std::sync::{Arc, Mutex};

pub(crate) struct Simulator {
    bank: Arc<Mutex<Vec<PresetDocument>>>,
    pid: u16,
    auxiliary: Arc<Mutex<auxiliary::Banks>>,
    stored_globals: Arc<Mutex<BTreeMap<u16, (u8, u8)>>>,
}
impl Default for Simulator {
    fn default() -> Self {
        Self {
            bank: Arc::new(Mutex::new((0..24).map(stored_document).collect())),
            pid: 20,
            stored_globals: Default::default(),
            auxiliary: Arc::new(Mutex::new(auxiliary::fixtures(20, &document()))),
        }
    }
}
impl Simulator {
    pub(crate) fn floor() -> Self {
        let docs = (0..100)
            .map(|slot| {
                let mut d = PresetDocument::parse(
                    &stored_document(slot)
                        .to_xml()
                        .unwrap()
                        .replace("ProductId=\"13\"", "ProductId=\"7\""),
                )
                .unwrap();
                use openfuse_core::preset::expression::{ExpressionPedal, PedalSlot};
                d.set_expression_pedal(
                    PedalSlot::Second,
                    &ExpressionPedal::defaults(PedalSlot::Second),
                )
                .unwrap();
                PresetDocument::parse(
                    &d.to_xml()
                        .unwrap()
                        .replace("</Preset>", "<FxLoopBypass>0</FxLoopBypass></Preset>"),
                )
                .unwrap()
            })
            .collect();
        Self {
            bank: Arc::new(Mutex::new(docs)),
            pid: 0x12,
            stored_globals: Default::default(),
            auxiliary: Arc::new(Mutex::new(auxiliary::fixtures(0x12, &document()))),
        }
    }
    pub(crate) fn superchamp() -> Self {
        let doc = auxiliary::superchamp_document();
        Self {
            bank: Arc::new(Mutex::new(vec![doc.clone(); 8])),
            pid: 0x0e,
            stored_globals: Default::default(),
            auxiliary: Arc::new(Mutex::new(auxiliary::fixtures(0x0e, &doc))),
        }
    }
    fn info(&self) -> DeviceInfo {
        let mut d = device();
        if self.pid == 0x12 {
            d.product_id = self.pid;
            d.id = "simulated-floor".into();
            d.name = "Simulated Mustang Floor".into();
        }
        if self.pid == 0x0e {
            d.product_id = self.pid;
            d.id = "simulated-superchamp".into();
            d.name = "Simulated Super Champ X2".into();
        }
        d
    }
}
pub(crate) fn device() -> DeviceInfo {
    DeviceInfo {
        id: "simulated-mustang".into(),
        name: "Simulated Mustang V2".into(),
        product_id: 20,
        serial: Some("OPENFUSE-SIMULATION".into()),
        interface_number: 0,
        unavailable_reason: None,
    }
}
impl Backend for Simulator {
    fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, String> {
        Ok(vec![self.info()])
    }
    fn open(&mut self, d: &DeviceInfo) -> Result<Box<dyn Connection>, String> {
        if *d != self.info() {
            return Err("Unknown simulated device".into());
        }
        Ok(Box::new(Peer {
            reports: VecDeque::new(),
            document: self.bank.lock().unwrap()[7].clone(),
            slot: 7,
            globals: (4, 250),
            staged: None,
            bank: self.bank.clone(),
            save: None,
            pid: self.pid,
            tuner: false,
            last_tone: std::time::Instant::now(),
            auxiliary: self.auxiliary.clone(),
            staged_auxiliary: None,
            whole_restore: false,
            stored_globals: self.stored_globals.clone(),
        }))
    }
}
struct Peer {
    reports: VecDeque<Report64>,
    document: PresetDocument,
    slot: u16,
    globals: (u8, u8),
    staged: Option<Vec<Report64>>,
    bank: Arc<Mutex<Vec<PresetDocument>>>,
    save: Option<(u16, String)>,
    pid: u16,
    tuner: bool,
    last_tone: std::time::Instant,
    auxiliary: Arc<Mutex<auxiliary::Banks>>,
    staged_auxiliary: Option<((u8, u16), Vec<Report64>)>,
    whole_restore: bool,
    stored_globals: Arc<Mutex<BTreeMap<u16, (u8, u8)>>>,
}
impl Peer {
    fn ack(&mut self, command: u8) {
        self.reports
            .push_back(frame(&[(0, 0), (1, 0xc1), (2, command), (3, 0)]));
    }
    fn snapshot(&self) -> Vec<Report64> {
        let mut reports = wire_preset::encode(self.pid, &self.document)
            .expect("simulator sound state encodes")
            .reports;
        for report in &mut reports {
            let mut b = *report.as_bytes();
            if b[0] == 0x1c && b[2] == 4 {
                b[4..6].copy_from_slice(&self.slot.to_le_bytes());
            }
            if b[0] == 0x1c && b[2] == 10 {
                b[17] = self.globals.0;
                b[18] = self.globals.1;
            }
            *report = Report64::from_bytes(b);
        }
        reports
    }
    fn stored_reports(&self, slot: u16) -> Vec<Report64> {
        let doc = self.bank.lock().unwrap()[usize::from(slot)].clone();
        let mut reports = wire_preset::encode(self.pid, &doc)
            .expect("stored fixture encodes")
            .reports;
        reports.retain(|r| r.message_id() == 0x1c);
        for report in &mut reports {
            let mut b = *report.as_bytes();
            if b[2] == 4 {
                b[4..6].copy_from_slice(&slot.to_le_bytes());
                b[6] = 0;
                b[7] = 0;
            }
            if b[2] == 10 {
                let globals = self
                    .stored_globals
                    .lock()
                    .unwrap()
                    .get(&slot)
                    .copied()
                    .unwrap_or((4, 250));
                b[17] = globals.0;
                b[18] = globals.1;
            }
            *report = Report64::from_bytes(b);
        }
        reports
    }
    fn working_reports(&self) -> Vec<Report64> {
        self.snapshot()
            .into_iter()
            .filter(|r| r.message_id() == 0x1c && r.as_bytes()[2] != 0)
            .collect()
    }
    fn preset_report(&mut self, report: Report64) -> Result<(), String> {
        let b = report.as_bytes();
        if b[3] != 0 {
            return self.auxiliary_write(report);
        }
        match b[2] {
            11 => return self.restore_system(report),
            1 => {
                let slot = u16::from_le_bytes([b[4], b[5]]);
                if usize::from(slot) >= self.bank.lock().unwrap().len()
                    || b[1] != 0xc3
                    || b[6] != 1
                    || b[7] != 0
                    || self.staged.is_some()
                {
                    return Err("Invalid simulated slot selection".into());
                }
                self.slot = slot;
                self.document = self.bank.lock().unwrap()[usize::from(slot)].clone();
                self.ack(0x1c);
            }
            3 => {
                let slot = u16::from_le_bytes([b[4], b[5]]);
                if b[1] != 1
                    || b[6] != 1
                    || b[7] != 0
                    || usize::from(slot) >= self.bank.lock().unwrap().len()
                    || self.staged.is_some()
                    || self.save.is_some()
                {
                    return Err("Invalid persistent save request".into());
                }
                let name = b[16..]
                    .iter()
                    .take_while(|&&v| v != 0)
                    .map(|&v| char::from(v))
                    .collect();
                self.save = Some((slot, name));
            }
            4 => {
                if b[1] != 1
                    || b[6] != 1
                    || b[7] != 1
                    || self.staged.is_some()
                    || usize::from(u16::from_le_bytes([b[4], b[5]]))
                        >= self.bank.lock().unwrap().len()
                    || self.save.is_some()
                {
                    return Err("Invalid simulated temporary-preset Name".into());
                }
                self.staged = Some(vec![report]);
            }
            5..=10 | 13 | 14 => {
                if b[1] != 3 {
                    return Err("Packed temporary report must request ACK".into());
                }
                if self.staged.is_none() {
                    self.staged = Some(self.working_reports());
                }
                let staged = self.staged.as_mut().expect("created staging");
                let key = (b[2], if b[2] == 10 { b[25] } else { 0 });
                if let Some(existing) = staged.iter_mut().find(|r| {
                    let bytes = r.as_bytes();
                    (bytes[2], if bytes[2] == 10 { bytes[25] } else { 0 }) == key
                }) {
                    *existing = report;
                } else {
                    staged.push(report);
                }
                if staged.len() > 16 {
                    self.staged = None;
                    return Err("Temporary transaction too large".into());
                }
                self.ack(0x1c);
            }
            0 => {
                if b[1] != 3 || b[2..].iter().any(|byte| *byte != 0) {
                    return Err("Invalid temporary Complete".into());
                }
                if let Some((slot, name)) = self.save.take() {
                    let mut info = self.document.metadata();
                    info.name = name;
                    self.document
                        .set_metadata(&info)
                        .map_err(|e| e.to_string())?;
                    self.bank.lock().unwrap()[usize::from(slot)] = self.document.clone();
                    self.stored_globals
                        .lock()
                        .unwrap()
                        .insert(slot, self.globals);
                    self.slot = slot;
                    self.ack(0x1c);
                    return Ok(());
                }
                let mut staged = self
                    .staged
                    .take()
                    .ok_or("Complete without pending temporary transaction")?;
                staged.push(report);
                let decoded = wire_preset::decode(self.pid, &staged).map_err(|e| e.to_string())?;
                if let Some(last) = staged.iter().rev().find(|r| r.as_bytes()[2] == 10) {
                    self.globals = (last.as_bytes()[17], last.as_bytes()[18]);
                }
                if let Some(name) = staged.iter().find(|r| r.as_bytes()[2] == 4) {
                    self.slot = u16::from_le_bytes([name.as_bytes()[4], name.as_bytes()[5]]);
                }
                self.document = decoded.document;
                self.ack(0x1c);
            }
            _ => {
                return Err(
                    "Simulator refuses persistent save, unknown, and system preset operations"
                        .into(),
                );
            }
        }
        Ok(())
    }
    fn direct_change(&mut self, report: Report64) -> Result<(), String> {
        if self.staged.is_some() {
            return Err("A temporary preset transaction is pending".into());
        }
        let b = report.as_bytes();
        let mut reports = self.snapshot();
        match b[0] {
            0x19 if b[2] == 15 && self.pid == 0x12 => {
                let aux = reports
                    .iter_mut()
                    .find(|r| r.message_id() == 0x1c && r.as_bytes()[2] == 14)
                    .ok_or("Missing loop report")?;
                let mut raw = *aux.as_bytes();
                raw[16] = u8::from(b[3] == 0);
                *aux = Report64::from_bytes(raw);
            }
            0x19 => {
                let kind = b[2]
                    .checked_add(3)
                    .filter(|kind| (5..=9).contains(kind))
                    .ok_or("Unknown bypass module")?;
                let report = reports
                    .iter_mut()
                    .find(|r| r.as_bytes()[0] == 0x1c && r.as_bytes()[2] == kind)
                    .ok_or("Missing bypass module")?;
                let mut raw = *report.as_bytes();
                if kind == 5 {
                    if b[3] == 0 {
                        raw[52] &= !0x40
                    } else {
                        raw[52] |= 0x40
                    }
                } else {
                    raw[22] = u8::from(b[3] != 0);
                    raw[18] = b[4];
                }
                *report = Report64::from_bytes(raw);
            }
            0x1b => {
                for report in &mut reports {
                    let mut raw = *report.as_bytes();
                    if raw[0] == 0x1c && (6..=9).contains(&raw[2]) {
                        raw[18] = b[usize::from(raw[2] - 4)];
                        *report = Report64::from_bytes(raw);
                    }
                }
            }
            _ => return Err("Unsupported direct operation".into()),
        }
        self.document = wire_preset::decode(self.pid, &reports)
            .map_err(|e| e.to_string())?
            .document;
        self.ack(b[0]);
        Ok(())
    }
}
impl Connection for Peer {
    fn write(&mut self, report: Report64) -> Result<(), String> {
        if self.staged_auxiliary.is_some()
            && (report.message_id() != 0x1c || report.as_bytes()[3] == 0)
        {
            return Err("Auxiliary transaction already active".into());
        }
        match report.message_id() {
            0 => self.ack(0),
            0x0a => {
                let b = report.as_bytes();
                if b[1] != 1 || b[2] > 1 || b[3] > 1 || b[4] > 1 {
                    return Err("Invalid tuner control".into());
                }
                self.tuner = b[2] != 0;
                self.reports.push_back(report);
                if self.tuner {
                    self.reports
                        .push_back(frame(&[(0, 0x0b), (1, 1), (2, 0), (3, 0)]));
                }
            }
            0x1a => self
                .reports
                .push_back(frame(&[(0, 1), (1, 0xc1), (2, 2), (3, 0), (20, 42)])),
            0xff => {
                if self.staged.is_some() {
                    return Err("Cannot sync during temporary transaction".into());
                }
                if report.as_bytes()[1] == 0xc1 && self.pid != 0x0e {
                    let slots = self.bank.lock().unwrap().len() as u16;
                    for slot in 0..slots {
                        self.reports.push_back(self.stored_reports(slot)[0]);
                    }
                }
                self.reports.extend(self.snapshot());
            }
            0x14 => {
                if self.restore_boundary(report)? {
                    return Ok(());
                }
                let b = report.as_bytes();
                if b[1] != 1
                    || b[3] != 0
                    || b[4] > 1
                    || usize::from(b[2]) >= self.bank.lock().unwrap().len()
                    || self.staged.is_some()
                    || self.save.is_some()
                {
                    return Err("Invalid bank read request".into());
                }
                let slots: Vec<u16> = if b[4] == 1 {
                    (0..self.bank.lock().unwrap().len() as u16).collect()
                } else {
                    vec![u16::from(b[2])]
                };
                if self.pid != 0x0e {
                    for slot in slots {
                        self.reports.extend(self.stored_reports(slot));
                    }
                }
                if b[4] == 1 {
                    for reports in self.auxiliary.lock().unwrap().values() {
                        self.reports.extend(reports.iter().copied());
                    }
                }
                self.reports
                    .push_back(frame(&[(0, 0x14), (1, 1), (3, 2), (4, b[4])]));
            }
            0x1c => self.preset_report(report)?,
            0x19 | 0x1b => self.direct_change(report)?,
            5 => {
                if self.staged.is_some() {
                    return Err("Cannot edit a parameter during temporary transaction".into());
                }
                let openfuse_core::messages::MessageFields::Parameter(change) =
                    openfuse_core::messages::decode_inbound(&report)
                else {
                    unreachable!()
                };
                let key = crate::live::module_key(change.module_type)
                    .ok_or("Unsupported simulator module")?;
                let mut expected = wire_preset::parameter_change(
                    self.pid,
                    &self.document,
                    key,
                    change.parameter_index,
                )
                .map_err(|e| e.to_string())?;
                expected.wire_value = change.wire_value;
                if expected != change {
                    return Err("Simulator parameter identity mismatch".into());
                }
                self.document
                    .set_parameter(key, change.parameter_index, change.wire_value)
                    .map_err(|e| e.to_string())?;
                self.ack(5);
            }
            id => return Err(format!("Simulator refuses unsupported command {id:#x}")),
        }
        Ok(())
    }
    fn read(&mut self) -> Result<Option<Report64>, TransportError> {
        if self.reports.is_empty()
            && self.tuner
            && self.last_tone.elapsed() >= std::time::Duration::from_millis(150)
        {
            self.last_tone = std::time::Instant::now();
            self.reports
                .push_back(frame(&[(0, 0x0b), (1, 1), (2, 0), (3, 0)]));
        }
        Ok(self.reports.pop_front())
    }
}
fn frame(values: &[(usize, u8)]) -> Report64 {
    let mut b = [0; 64];
    for &(index, value) in values {
        b[index] = value;
    }
    Report64::from_bytes(b)
}
fn stored_document(slot: u16) -> PresetDocument {
    let mut document = document();
    if slot != 7 {
        let mut info = document.metadata();
        info.name = format!("Simulated slot {slot}");
        document.set_metadata(&info).expect("synthetic metadata");
        document
            .set_parameter("Amplifier", 1, (u32::from(slot) * 1024).min(65535) as u16)
            .expect("synthetic gain");
    }
    document
}
/// Own fixture from factual catalog defaults, not a copied factory preset.
pub(crate) fn document() -> PresetDocument {
    use openfuse_core::model_catalog::{self, ModuleCategory};
    let mut xml = String::from("<Preset ProductId=\"13\">");
    for (key, id, position, active) in [
        ("Amplifier", 103, 0, true),
        ("Stompbox", 60, 0, false),
        ("Modulation", 18, 1, false),
        ("Delay", 43, 4, true),
        ("Reverb", 33, 5, true),
    ] {
        if key == "Stompbox" {
            xml.push_str("<FX>");
        }
        let category = ModuleCategory::from_key(key).unwrap();
        let model = model_catalog::model(13, category, id).unwrap();
        let category_id = match key {
            "Amplifier" => 0,
            "Stompbox" => 1,
            "Modulation" => 2,
            "Delay" => 3,
            _ => 4,
        };
        xml.push_str(&format!("<{key} ID=\"{category_id}\"><Module ID=\"{id}\" POS=\"{position}\" BypassState=\"{}\" BypassMode=\"{}\">",u8::from(active),model.bypass_mode));
        for p in &model.parameters {
            xml.push_str(&format!(
                "<Param ControlIndex=\"{}\">{}</Param>",
                p.index, p.default_raw
            ));
        }
        xml.push_str(&format!("</Module></{key}>"));
    }
    xml.push_str(
        "</FX><FUSE><Info name=\"Simulated clean preset\" author=\"\" tags=\"\"/></FUSE></Preset>",
    );
    let mut document = PresetDocument::parse(&xml).unwrap();
    document
        .set_expression_pedal(
            openfuse_core::preset::expression::PedalSlot::First,
            &openfuse_core::preset::expression::ExpressionPedal::defaults(
                openfuse_core::preset::expression::PedalSlot::First,
            ),
        )
        .unwrap();
    let xml = document
        .to_xml()
        .unwrap()
        .replace("</Preset>", "<UsbGain>9</UsbGain></Preset>");
    PresetDocument::parse(&xml).unwrap()
}

#[cfg(test)]
mod tests;
