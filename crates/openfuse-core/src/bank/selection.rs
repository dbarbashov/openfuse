use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotMapping {
    pub source: u16,
    pub target: u16,
}
#[derive(Debug, Clone)]
pub struct RestoreEntry {
    source: u16,
    target: u16,
    document: PresetDocument,
}
impl RestoreEntry {
    pub fn source(&self) -> u16 {
        self.source
    }
    pub fn target(&self) -> u16 {
        self.target
    }
    pub fn document(&self) -> &PresetDocument {
        &self.document
    }
}
/// Frozen, validated mapping. Construction never sends data; `plans` preflights
/// every selected preset against one current snapshot before any write begins.
#[derive(Debug, Clone)]
pub struct RestoreSelection {
    pid: u16,
    entries: Vec<RestoreEntry>,
}
impl RestoreSelection {
    pub fn new(backup: &Backup, pid: u16, mappings: &[SlotMapping]) -> Result<Self> {
        Self::from_documents(
            backup.pid(),
            backup
                .slots()
                .iter()
                .map(|s| (s.slot(), s.document().clone()))
                .collect(),
            pid,
            mappings,
        )
    }
    /// `source_pid` must be established by the source, or explicitly chosen by
    /// the user when a legacy folder records only an ambiguous preset family.
    pub fn from_documents(
        source_pid: u16,
        documents: Vec<(u16, PresetDocument)>,
        target_pid: u16,
        mappings: &[SlotMapping],
    ) -> Result<Self> {
        let capacity = count(target_pid)?;
        if source_pid != target_pid {
            return Err(error("Archive PID must exactly match the selected device"));
        }
        if mappings.is_empty() || mappings.len() > usize::from(capacity) {
            return Err(error("Restore requires 1..bank-capacity mappings"));
        }
        let family = wire_preset::product_for_pid(source_pid)
            .ok_or_else(|| error("Unknown archive device family"))?;
        let mut sources = BTreeMap::new();
        if documents.len() > 100 {
            return Err(error("Archive exceeds the supported source slot limit"));
        }
        for (slot, document) in documents {
            if sources.insert(slot, document).is_some() {
                return Err(error("Archive contains a duplicate source preset"));
            }
        }
        let mut destinations = BTreeSet::new();
        let mut selected = BTreeSet::new();
        let mut entries = Vec::new();
        for mapping in mappings {
            if mapping.target >= capacity
                || !destinations.insert(mapping.target)
                || !selected.insert(mapping.source)
            {
                return Err(error(
                    "Restore mappings contain a duplicate source/destination or out-of-range destination",
                ));
            }
            let document = sources
                .get(&mapping.source)
                .ok_or_else(|| error("Mapped source slot is absent from the archive"))?
                .clone();
            if mapping.source >= capacity || document.product_id() != Some(family) {
                return Err(error(
                    "Selected source preset is out of range or incompatible with the declared device",
                ));
            }
            entries.push(RestoreEntry {
                source: mapping.source,
                target: mapping.target,
                document,
            });
        }
        Ok(Self {
            pid: target_pid,
            entries,
        })
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }
    pub fn entries(&self) -> &[RestoreEntry] {
        &self.entries
    }
    pub fn validate_for_pid(&self, pid: u16) -> Result<()> {
        if pid != self.pid {
            return Err(error("Restore selection belongs to another device PID"));
        }
        Ok(())
    }
    pub fn plans(&self, current: &[Report64]) -> Result<Vec<audition::Plan>> {
        self.entries
            .iter()
            .map(|entry| restore_slot(self.pid, &entry.document, current, entry.target))
            .collect()
    }
}
