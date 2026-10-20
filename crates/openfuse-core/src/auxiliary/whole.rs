//! Exact-profile whole-device restore. Source reports are validated, never replayed.
use super::*;
use crate::{
    archive::Archive, audition, conversion, legacy_backup::EntryKind, preset::PresetDocument,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
mod validation;
use validation::{validate_normalizations, validate_xml};
#[derive(Debug, Clone)]
pub struct RestoreStep {
    pub report: Report64,
    pub ack_timeout_ms: u64,
    pub label: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Normalization {
    pub section: String,
    pub kind: u8,
    pub offset: usize,
    pub before: u16,
    pub after: u16,
}
#[derive(Debug, Clone)]
pub struct WholeRestore {
    pid: u16,
    source_fingerprint: String,
    destination_fingerprint: String,
    steps: Vec<RestoreStep>,
    expected: Snapshot,
    normalizations: Vec<Normalization>,
    warnings: Vec<String>,
}
impl WholeRestore {
    pub fn new(source: &Snapshot, destination: &Snapshot) -> Result<Self> {
        if source.legacy {
            return Err(error(
                "Use the complete historical archive, not its auxiliary-only projection",
            ));
        }
        if source.pid != destination.pid {
            return Err(error("Whole restore requires an exact USB PID match"));
        }
        let (steps, expected, normalizations) = prepare(
            source,
            &source
                .presets
                .iter()
                .map(|p| (p.slot(), p.document().clone()))
                .collect::<Vec<_>>(),
        )?;
        Self::finish(
            source.pid,
            digest(source.to_json()?.as_bytes()),
            destination,
            steps,
            expected,
            normalizations,
        )
    }
    pub fn validate_source(source: &Snapshot) -> Result<()> {
        Self::new(source, source).map(|_| ())
    }
    pub fn from_archive(archive: &Archive, pid: u16, destination: &Snapshot) -> Result<Self> {
        if destination.pid != pid {
            return Err(error("Selected source PID differs from destination"));
        }
        if let Some(source) = archive.auxiliary() {
            return Self::new(source, destination);
        }
        let legacy = archive.legacy().ok_or_else(|| {
            error("Whole restore requires a full capture or historical full-device archive")
        })?;
        let source = archive.auxiliary_for_pid(pid).map_err(error)?;
        let mut sound_files = 0;
        for entry in legacy.entries() {
            match entry.kind() {
                EntryKind::Unknown | EntryKind::BandTrack => {
                    return Err(error(format!(
                        "Unrepresented historical content: {}",
                        entry.path()
                    )));
                }
                EntryKind::EffectBank | EntryKind::VoiceBank | EntryKind::SystemSettings => {
                    sound_files += 1
                }
                _ => (),
            }
        }
        if sound_files != source.entries.len() {
            return Err(error(
                "Historical archive contains omitted or incompatible device sections",
            ));
        }
        let presets = archive
            .presets()
            .iter()
            .map(|p| (p.slot, p.document.clone()))
            .collect::<Vec<_>>();
        let (steps, expected, normalizations) = prepare(&source, &presets)?;
        let identity = legacy
            .entries()
            .iter()
            .map(|e| (e.path(), e.sha256()))
            .collect::<Vec<_>>();
        Self::finish(
            pid,
            digest(&serde_json::to_vec(&(pid, identity)).map_err(error)?),
            destination,
            steps,
            expected,
            normalizations,
        )
    }
    fn finish(
        pid: u16,
        source_fingerprint: String,
        destination: &Snapshot,
        steps: Vec<RestoreStep>,
        expected: Snapshot,
        normalizations: Vec<Normalization>,
    ) -> Result<Self> {
        if destination.legacy {
            return Err(error(
                "Destination requires a fresh complete device capture",
            ));
        }
        coverage(destination, destination.presets.len())?;
        // Destination unknown state is also a refusal: do not overwrite fields
        // whose role or firmware compatibility has not been established.
        let _ = prepare(
            destination,
            &destination
                .presets
                .iter()
                .map(|p| (p.slot(), p.document().clone()))
                .collect::<Vec<_>>(),
        )?;
        Ok(Self {
            pid,
            source_fingerprint,
            destination_fingerprint: destination.fingerprint(),
            steps,
            expected,
            normalizations,
            warnings: vec!["Whole restore replaces every stored preset, system setting and physical bank. Local FUSE metadata and media are not uploaded.".into(), "Original preset uploads transmit global expression endpoints 0/0 before System. Firmware may expose later System endpoints in stored readback; that unverified behavior causes a strict verification failure, never a hidden match.".into(), "Known original loader cascades and packed parameter quantization are listed explicitly; unknown header and reserved bytes remain a refusal.".into()],
        })
    }
    pub fn normalizations(&self) -> &[Normalization] {
        &self.normalizations
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }
    pub fn source_fingerprint(&self) -> &str {
        &self.source_fingerprint
    }
    pub fn destination_fingerprint(&self) -> &str {
        &self.destination_fingerprint
    }
    pub fn preset_count(&self) -> usize {
        self.expected.presets.len()
    }
    pub fn bank_count(&self) -> usize {
        self.expected
            .entries
            .iter()
            .filter(|e| e.key != Key::System)
            .count()
    }
    pub fn has_system(&self) -> bool {
        self.expected.entries.iter().any(|e| e.key == Key::System)
    }
    pub fn steps(&self) -> &[RestoreStep] {
        &self.steps
    }
    pub fn validate_destination(&self, snapshot: &Snapshot) -> Result<()> {
        if snapshot.legacy
            || snapshot.pid != self.pid
            || snapshot.fingerprint() != self.destination_fingerprint
        {
            return Err(error(
                "Whole-device destination changed since review; capture and review again",
            ));
        }
        Ok(())
    }
    pub fn matches_readback(&self, snapshot: &Snapshot) -> Result<()> {
        if snapshot.legacy
            || snapshot.pid != self.pid
            || snapshot.fingerprint() != self.expected.fingerprint()
        {
            return Err(error(
                "Whole-device readback differs in presets, system or physical banks",
            ));
        }
        Ok(())
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn coverage(snapshot: &Snapshot, preset_count: usize) -> Result<()> {
    let caps = capabilities(snapshot.pid)?;
    if preset_count != usize::from(audition::slot_count(snapshot.pid).map_err(error)?) {
        return Err(error(
            "Whole restore requires every stored preset exactly once",
        ));
    }
    let mut expected = BTreeSet::new();
    if caps.system {
        expected.insert(Key::System);
    }
    for (control, count) in caps.banks {
        for slot in 0..count {
            expected.insert(Key::Bank { control, slot });
        }
    }
    let actual = snapshot
        .entries
        .iter()
        .map(|e| e.key)
        .collect::<BTreeSet<_>>();
    if actual != expected || actual.len() != snapshot.entries.len() {
        return Err(error(
            "Whole restore requires exactly all system and physical-bank sections, with no unknown extras",
        ));
    }
    Ok(())
}
fn boundary(operation: u8) -> Report64 {
    let mut b = [0; 64];
    b[0] = 0x14;
    b[1] = if operation == 1 { 0xc3 } else { 3 };
    b[3] = operation;
    b[4] = 1;
    Report64::from_bytes(b)
}
fn step(report: Report64, label: String) -> RestoreStep {
    RestoreStep {
        ack_timeout_ms: if report.as_bytes()[1] & 2 == 0 {
            0
        } else if report.message_id() == 0x14 {
            2000
        } else {
            5000
        },
        report,
        label,
    }
}
fn prepare(
    source: &Snapshot,
    presets: &[(u16, PresetDocument)],
) -> Result<(Vec<RestoreStep>, Snapshot, Vec<Normalization>)> {
    coverage(source, presets.len())?;
    let mut steps = vec![step(boundary(1), "Begin whole-device restore".into())];
    let mut expected_reports = Vec::new();
    let mut normalizations = Vec::new();
    let system = source
        .entries
        .iter()
        .find(|e| e.key == Key::System)
        .map(|entry| {
            let report = decode_system_xml(
                entry
                    .xml
                    .as_deref()
                    .ok_or_else(|| error("Missing System XML"))?,
            )?;
            if !entry.raw.is_empty()
                && storage::canonical(&entry.raw) != storage::canonical(&[report])
            {
                return Err(error(
                    "System contains unknown header/reserved fields; original retained",
                ));
            }
            Ok(report)
        })
        .transpose()?;
    let mut seen = BTreeSet::new();
    let mut ordered = presets.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|p| p.0);
    for (slot, document) in ordered {
        if !seen.insert(*slot) || *slot >= audition::slot_count(source.pid).map_err(error)? {
            return Err(error("Duplicate or out-of-range full restore preset"));
        }
        let product = wire_preset::product_for_pid(source.pid).unwrap();
        if document.product_id() != Some(product) {
            return Err(error(
                "Historical preset family does not match explicit USB PID",
            ));
        }
        validate_xml(document)?;
        let loaded = conversion::Plan::preview(
            &document.to_xml().map_err(error)?,
            conversion::Options {
                target_product: product,
                repair_double_entities: false,
                normalize_legacy_delays: false,
                assume_missing_gdec: false,
            },
        )
        .map_err(error)?;
        let packets =
            audition::whole_restore_packets(source.pid, loaded.result(), *slot).map_err(error)?;
        let expected = packets[..packets.len() - 2].to_vec();
        let synthetic;
        let original = if source.legacy {
            for module in document.modules() {
                let category = crate::model_catalog::ModuleCategory::from_key(&module.key)
                    .ok_or_else(|| error("Unknown category"))?;
                let model =
                    crate::model_catalog::model(product, category, module.model_id.unwrap_or(0))
                        .ok_or_else(|| error("Unknown model"))?;
                for param in module.parameters {
                    if model
                        .parameters
                        .iter()
                        .any(|p| p.index == param.index && p.type_id < 128)
                        && param.value & 255 != 0
                    {
                        normalizations.push(Normalization {
                            section: format!(
                                "Preset {slot} / {} parameter {} XML u16",
                                module.key, param.index
                            ),
                            kind: wire_preset::CATEGORIES
                                .iter()
                                .find(|(_, k, _)| *k == module.key)
                                .unwrap()
                                .0,
                            offset: 32 + usize::from(param.index),
                            before: param.value,
                            after: param.value & 0xff00,
                        });
                    }
                }
            }
            synthetic = wire_preset::encode(source.pid, document)
                .map_err(error)?
                .reports
                .into_iter()
                .filter(|r| r.message_id() == 0x1c)
                .collect::<Vec<_>>();
            synthetic.as_slice()
        } else {
            source
                .presets
                .iter()
                .find(|p| p.slot() == *slot)
                .ok_or_else(|| error("Missing source preset"))?
                .raw_reports()
        };
        validate_normalizations(
            source.pid,
            original,
            &expected,
            &format!("Preset {slot}"),
            &mut normalizations,
        )?;
        expected_reports.extend(expected);
        steps.extend(
            packets
                .into_iter()
                .map(|r| step(r, format!("Preset {slot}"))),
        );
    }
    if let Some(system) = system {
        steps.push(step(system, "System settings".into()));
        expected_reports.push(system);
    }
    for entry in source.entries.iter().filter(|e| e.key != Key::System) {
        if source.pid == 14 && matches!(entry.key, Key::Bank { control: 1, .. }) {
            let xml = roxmltree::Document::parse(
                entry
                    .xml
                    .as_deref()
                    .ok_or_else(|| error("Missing voice XML"))?,
            )
            .map_err(error)?;
            if codec::child(xml.root_element(), "Stompbox")?.is_none() {
                return Err(error(
                    "Original whole-device restore requires Stompbox in every voice, including the clean slot",
                ));
            }
        }
        let reports = plan_xml(
            source.pid,
            entry.key,
            entry
                .xml
                .as_deref()
                .ok_or_else(|| error("Missing physical-bank XML"))?,
        )?;
        if !entry.raw.is_empty() {
            validate_normalizations(
                source.pid,
                &entry.raw,
                &reports,
                &entry.label(),
                &mut normalizations,
            )?;
        }
        steps.extend(reports.iter().map(|r| step(*r, entry.label())));
        expected_reports.extend(reports);
    }
    steps.push(step(boundary(2), "Finish whole-device restore".into()));
    if steps.len() > 4096 {
        return Err(error("Whole restore exceeds transaction limit"));
    }
    let mut terminal = [0; 64];
    terminal[0] = 0x14;
    terminal[1] = 1;
    terminal[3] = 2;
    terminal[4] = 1;
    expected_reports.push(Report64::from_bytes(terminal));
    let expected = Snapshot::from_reports(source.pid, &expected_reports)?;
    Ok((steps, expected, normalizations))
}
