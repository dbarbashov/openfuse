use super::*;
use std::collections::BTreeSet;
#[derive(Debug, Clone)]
pub struct Selection {
    pid: u16,
    destination_fingerprint: String,
    mappings: Vec<Mapping>,
    steps: Vec<Report64>,
    expected: Snapshot,
}
impl Selection {
    pub fn from_control_preset(
        document: &crate::control_preset::ControlPreset,
        destination: &Snapshot,
        slot: u16,
    ) -> Result<Self> {
        let key = Key::Bank {
            control: document.control(),
            slot,
        };
        if document.is_clean_voice() && slot != 16 {
            return Err(error(
                "A clean-channel voice can only be written to slot 16",
            ));
        }
        let xml = document.to_xml().map_err(error)?;
        crate::auxiliary::plan_xml(document.pid(), key, &xml)?;
        let source = Snapshot {
            pid: document.pid(),
            entries: vec![Entry {
                key,
                label: document.name(),
                xml: Some(xml),
                raw: vec![],
                restore_error: None,
            }],
            presets: vec![],
            raw: vec![],
            warnings: vec![],
            legacy: true,
        };
        Self::new(
            &source,
            destination,
            &[Mapping {
                source: key,
                target: key,
            }],
        )
    }

    pub fn new(source: &Snapshot, destination: &Snapshot, mappings: &[Mapping]) -> Result<Self> {
        if source.pid != destination.pid {
            return Err(error(
                "Auxiliary archive PID must exactly match destination",
            ));
        }
        if destination.legacy {
            return Err(error(
                "Destination must be a complete fresh hardware/simulator capture",
            ));
        }
        if mappings.is_empty() || mappings.len() > 128 {
            return Err(error("Select 1..128 auxiliary entries"));
        }
        let mut sources = BTreeSet::new();
        let mut targets = BTreeSet::new();
        let mut steps = Vec::new();
        let mut expected = destination.clone();
        for mapping in mappings {
            if !mapping.source.same_kind(mapping.target)
                || !sources.insert(mapping.source)
                || !targets.insert(mapping.target)
            {
                return Err(error(
                    "Auxiliary mappings require unique source/destination entries in the same bank",
                ));
            }
            let input = source
                .entries
                .iter()
                .find(|e| e.key == mapping.source)
                .ok_or_else(|| error("Missing source auxiliary entry"))?;
            if let Some(reason) = &input.restore_error {
                return Err(error(reason));
            }
            let index = expected
                .entries
                .iter()
                .position(|e| e.key == mapping.target)
                .ok_or_else(|| error("Missing destination auxiliary entry"))?;
            if let Some(reason) = &expected.entries[index].restore_error {
                return Err(error(reason));
            }
            let (writes, entry) = codec::restore(source.pid, input, &expected.entries[index])?;
            steps.extend(writes);
            expected.entries[index] = entry;
        }
        Ok(Self {
            pid: source.pid,
            destination_fingerprint: destination.fingerprint(),
            mappings: mappings.to_vec(),
            steps,
            expected,
        })
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }
    pub fn len(&self) -> usize {
        self.mappings.len()
    }
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }
    pub fn mappings(&self) -> &[Mapping] {
        &self.mappings
    }
    pub fn steps(&self) -> Vec<Report64> {
        self.steps.clone()
    }
    pub fn validate_destination(&self, snapshot: &Snapshot) -> Result<()> {
        if snapshot.legacy
            || snapshot.pid != self.pid
            || snapshot.fingerprint() != self.destination_fingerprint
        {
            return Err(error(
                "Auxiliary destination changed since the preview; read and review again",
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
                "Auxiliary readback differs from selected writes or unchanged sections",
            ));
        }
        Ok(())
    }
}
