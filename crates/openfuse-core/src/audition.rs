//! Temporary working-preset operations, separate from offline response encoding.
//! Builds temporary upload plans and verifies readback without persisting presets.
//! Persistent plans are exposed through `bank`; no firmware or factory reset.
use crate::{
    model_catalog::{self, ModuleCategory},
    preset::PresetDocument,
    protocol::Report64,
    wire_preset,
};
use std::fmt;
mod packets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditionError(String);
impl fmt::Display for AuditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for AuditionError {}
type Result<T> = std::result::Result<T, AuditionError>;
fn error(value: impl Into<String>) -> AuditionError {
    AuditionError(value.into())
}
fn text_error(value: impl fmt::Display) -> AuditionError {
    error(value.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Preset,
    Module(String),
    Bypass(String),
    Routing,
    SelectSlot(u16),
    SaveSlot(u16),
    RestoreSlot(u16),
    Settings,
}
impl Operation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Preset => "Audition preset",
            Self::Module(_) => "Change module",
            Self::Bypass(_) => "Change bypass",
            Self::Routing => "Change routing",
            Self::SelectSlot(_) => "Select stored slot",
            Self::SaveSlot(_) => "Save working preset to slot",
            Self::RestoreSlot(_) => "Restore preset to slot",
            Self::Settings => "Change device settings",
        }
    }
}

/// Immutable reviewed operation. The worker must send steps once, in order,
/// correlate each ACK-required report, then read current state and verify it.
#[derive(Debug, Clone)]
pub struct Plan {
    pid: u16,
    operation: Operation,
    steps: Vec<Report64>,
    expected: Option<PresetDocument>,
    expected_slot: u16,
    expected_globals: Vec<(u8, u8, u8)>,
    expected_auxiliary: Vec<Report64>,
    warnings: Vec<String>,
}

/// Controls supplies a narrowly scoped original command sequence and a copy
/// of the captured state with only those settings changed.
pub(crate) fn settings_plan(
    pid: u16,
    current: &[Report64],
    steps: Vec<Report64>,
    expected_reports: &[Report64],
    warnings: Vec<String>,
) -> Result<Plan> {
    wire_preset::decode(pid, current).map_err(text_error)?;
    let expected = wire_preset::decode(pid, expected_reports)
        .map_err(text_error)?
        .document;
    Ok(Plan {
        pid,
        operation: Operation::Settings,
        steps,
        expected: Some(expected),
        expected_slot: current_slot(current)?,
        expected_globals: global_fields(expected_reports),
        expected_auxiliary: expected_reports
            .iter()
            .copied()
            .filter(|r| {
                let b = r.as_bytes();
                b[0] == 0x1c && b[3] == 0 && matches!(b[2], 10 | 13 | 14)
            })
            .collect(),
        warnings,
    })
}

pub(crate) fn expression_packet(
    document: &PresetDocument,
    slot: crate::preset::expression::PedalSlot,
    globals: (u8, u8),
) -> Result<Report64> {
    let pedal = document
        .expression_pedal(slot)
        .map_err(text_error)?
        .ok_or_else(|| error("Missing current expression settings"))?;
    packets::expression(&pedal, slot, globals)
}
impl Plan {
    pub fn steps(&self) -> &[Report64] {
        &self.steps
    }
    pub fn expected(&self) -> Option<&PresetDocument> {
        self.expected.as_ref()
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn operation(&self) -> &Operation {
        &self.operation
    }
    pub fn label(&self) -> &'static str {
        self.operation.label()
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }
    pub fn expected_slot(&self) -> u16 {
        self.expected_slot
    }
    /// ACKs establish delivery only. This compares a subsequent complete read
    /// against the expected quantized/cascaded sound state and selected slot.
    pub fn matches_stored_readback(&self, reports: &[Report64]) -> Result<()> {
        let normalized: Vec<_> = reports
            .iter()
            .map(|report| {
                let mut b = *report.as_bytes();
                if b[0] == 0x1c && b[2] == 4 && b[3] == 0 {
                    b[6] = 1;
                }
                Report64::from_bytes(b)
            })
            .collect();
        self.matches_readback(&normalized)
    }
    pub fn matches_readback(&self, reports: &[Report64]) -> Result<()> {
        let actual = wire_preset::decode(self.pid, reports)
            .map_err(text_error)?
            .document;
        if current_slot(reports)? != self.expected_slot {
            return Err(error("Readback selected slot does not match the operation"));
        }
        for expected in &self.expected_auxiliary {
            let b = expected.as_bytes();
            let actual = reports.iter().find(|r| {
                let a = r.as_bytes();
                a[0] == 0x1c && a[3] == 0 && a[2] == b[2] && (b[2] != 10 || a[25] == b[25])
            });
            if actual.is_none_or(|r| r.as_bytes()[16..] != b[16..]) {
                return Err(error(
                    "Readback device settings payload differs from the expected retained state",
                ));
            }
        }
        for &(slot, heel, toe) in &self.expected_globals {
            let observed = reports.iter().find(|r| {
                let b = r.as_bytes();
                b[0] == 0x1c && b[2] == 10 && b[3] == 0 && b[25] == slot
            });
            if observed
                .is_none_or(|report| report.as_bytes()[17] != heel || report.as_bytes()[18] != toe)
            {
                return Err(error(
                    "Readback global volume endpoints differ from the retained device settings",
                ));
            }
        }
        if let Some(expected) = &self.expected {
            if expected.product_id() != actual.product_id()
                || expected.metadata().name != actual.metadata().name
            {
                return Err(error(
                    "Readback preset family/name differs from the preview",
                ));
            }
            if expected.modules() != actual.modules() {
                return Err(error(
                    "Readback models, parameters, routing, or bypass differ from the preview",
                ));
            }
            for slot in [
                crate::preset::expression::PedalSlot::First,
                crate::preset::expression::PedalSlot::Second,
            ] {
                if expected.expression_pedal(slot).map_err(text_error)?
                    != actual.expression_pedal(slot).map_err(text_error)?
                {
                    return Err(error(
                        "Readback expression settings differ from the preview",
                    ));
                }
            }
            for key in ["UsbGain", "FxLoopBypass"] {
                if scalar(expected, key)? != scalar(&actual, key)? {
                    return Err(error(format!("Readback {key} differs from the preview")));
                }
            }
        }
        Ok(())
    }
}

/// Stored slots, using the protocol's zero-based index. Super Champ has no
/// full-preset slot bank in this interface; unknown PIDs are errors.
pub fn slot_count(pid: u16) -> Result<u16> {
    match pid {
        4 | 0xa | 0x10 | 0x14 | 0x26 => Ok(24),
        5 | 0x12 | 0x16 => Ok(100),
        0xe => Ok(0),
        _ => Err(error("No evidenced stored-slot operation for this PID")),
    }
}
pub fn slot_select(pid: u16, slot: u16) -> Result<Plan> {
    if slot >= slot_count(pid)? {
        return Err(error(
            "Stored slot is outside the selected device's preset bank",
        ));
    }
    let mut b = [0; 64];
    b[0] = 0x1c;
    b[1] = 0xc3;
    b[2] = 1;
    b[4..6].copy_from_slice(&slot.to_le_bytes());
    b[6] = 1;
    Ok(Plan{pid,operation:Operation::SelectSlot(slot),steps:vec![Report64::from_bytes(b)],expected:None,expected_slot:slot,expected_globals:vec![],expected_auxiliary:vec![],warnings:vec!["Selecting a stored slot replaces the temporary working preset; it does not save current edits".into()]})
}
fn global_fields(reports: &[Report64]) -> Vec<(u8, u8, u8)> {
    reports
        .iter()
        .filter(|r| {
            let b = r.as_bytes();
            b[0] == 0x1c && b[2] == 10 && b[3] == 0
        })
        .map(|r| {
            let b = r.as_bytes();
            (b[25], b[17], b[18])
        })
        .collect()
}
fn current_slot(reports: &[Report64]) -> Result<u16> {
    let names: Vec<_> = reports
        .iter()
        .filter(|r| {
            let b = r.as_bytes();
            b[0] == 0x1c && b[2] == 4 && b[3] == 0 && b[6] != 0
        })
        .collect();
    if names.len() != 1 {
        return Err(error("Exactly one current-preset Name report is required"));
    }
    let b = names[0].as_bytes();
    Ok(u16::from_le_bytes([b[4], b[5]]))
}
fn scalar(document: &PresetDocument, key: &str) -> Result<Option<String>> {
    let source = document.to_xml().map_err(text_error)?;
    let xml = roxmltree::Document::parse(&source).map_err(text_error)?;
    let nodes: Vec<_> = xml
        .root_element()
        .children()
        .filter(|n| n.has_tag_name(key))
        .collect();
    if nodes.len() > 1 {
        return Err(error(format!("Duplicate {key}")));
    }
    Ok(nodes.first().map(|n| n.text().unwrap_or("").trim().into()))
}
fn family(pid: u16, document: &PresetDocument) -> Result<u32> {
    let product = wire_preset::product_for_pid(pid)
        .ok_or_else(|| error("No evidenced audition protocol for this PID"))?;
    if document.product_id() != Some(product) {
        return Err(error("Preset family does not match selected amplifier"));
    }
    Ok(product)
}

/// Plan the original complete temporary SendPreset transaction. Missing file
/// auxiliary settings inherit the last read; global expression volume endpoints
/// always come from that read because they do not exist in .FUSE XML.
pub fn preset(pid: u16, desired: &PresetDocument, current: &[Report64]) -> Result<Plan> {
    let product = family(pid, desired)?;
    let original = wire_preset::decode(pid, current)
        .map_err(text_error)?
        .document;
    let slot = current_slot(current)?;
    let (effective, mut warnings) = inherit_auxiliary(product, desired, &original)?;
    let globals = if product != 11 {
        let pairs: Vec<_> = current
            .iter()
            .filter(|r| r.as_bytes()[0] == 0x1c && r.as_bytes()[2] == 10 && r.as_bytes()[3] == 0)
            .map(|r| (r.as_bytes()[17], r.as_bytes()[18]))
            .collect();
        let value=pairs.last().copied().ok_or_else(||error("Read expression data first: global volume heel/toe cannot be invented from a preset file"))?;
        if pairs.iter().any(|pair| *pair != value) {
            warnings.push("Current expression packets disagree about global volume endpoints; the last received pair is retained, matching the original global-state loader".into());
        }
        Some(value)
    } else {
        None
    };
    // Validation/precision preview only. These synthetic response reports are
    // deliberately NOT reused as host upload packets.
    let preview = wire_preset::encode(pid, &effective).map_err(text_error)?;
    warnings.extend(
        preview
            .warnings
            .into_iter()
            .filter(|w| !w.contains("global volume endpoints"))
            .map(audition_warning),
    );
    let steps = packets::preset(&effective, slot, globals)?;
    let expected = wire_preset::decode(pid, &steps)
        .map_err(text_error)?
        .document;
    validate_chain(product, &expected, &original)?;
    validate_bindings(&expected)?;
    warnings.push("This operation replaces only the temporary working preset. ACKs are followed by a fresh read and comparison; no persistent-save command is emitted; hardware behavior remains unverified".into());
    Ok(Plan {
        pid,
        operation: Operation::Preset,
        expected_globals: global_fields(&steps),
        expected_auxiliary: vec![],
        steps,
        expected: Some(expected),
        expected_slot: slot,
        warnings,
    })
}
fn inherit_auxiliary(
    product: u32,
    desired: &PresetDocument,
    current: &PresetDocument,
) -> Result<(PresetDocument, Vec<String>)> {
    let source = desired.to_xml().map_err(text_error)?;
    let xml = roxmltree::Document::parse(&source).map_err(text_error)?;
    let captured = current.to_xml().map_err(text_error)?;
    let captured_xml = roxmltree::Document::parse(&captured).map_err(text_error)?;
    let mut required = vec!["UsbGain"];
    if product != 11 {
        required.push("FirstExpressionPedal");
    }
    if product == 7 {
        required.extend(["SecondExpressionPedal", "FxLoopBypass"]);
    }
    let mut additions = String::new();
    let mut warnings = Vec::new();
    for key in required {
        if xml.root_element().children().any(|n| n.has_tag_name(key)) {
            continue;
        }
        let node=captured_xml.root_element().children().find(|n|n.has_tag_name(key)).ok_or_else(||error(format!("{key} is missing from both preset and current capture; read complete device settings first")))?;
        additions.push_str(&captured[node.range()]);
        warnings.push(format!(
            "{key} is absent from the file; current device settings are retained"
        ));
    }
    let root_end = xml.root_element().range().end;
    let insertion = source[..root_end]
        .rfind("</")
        .ok_or_else(|| error("Preset root has no closing tag"))?;
    let effective = format!(
        "{}{}{}",
        &source[..insertion],
        additions,
        &source[insertion..]
    );
    Ok((
        PresetDocument::parse(&effective).map_err(text_error)?,
        warnings,
    ))
}

/// Replace one module in the current working preset, followed by Complete.
/// Only that module's desired fields are used; unrelated current state is kept.
pub fn module(pid: u16, desired: &PresetDocument, current: &[Report64], key: &str) -> Result<Plan> {
    let product = family(pid, desired)?;
    let original = wire_preset::decode(pid, current)
        .map_err(text_error)?
        .document;
    let preview = wire_preset::encode(pid, desired).map_err(text_error)?;
    let (kind, _, _) = packets::categories()
        .iter()
        .find(|(_, candidate, _)| *candidate == key)
        .copied()
        .ok_or_else(|| error("Unknown module category"))?;
    let replacement = packets::module(desired, kind, key)?;
    let mut expected_reports = current.to_vec();
    let target = expected_reports
        .iter_mut()
        .find(|r| r.as_bytes()[0] == 0x1c && r.as_bytes()[2] == kind && r.as_bytes()[3] == 0)
        .ok_or_else(|| error("Current module is absent"))?;
    *target = replacement;
    let expected = wire_preset::decode(pid, &expected_reports)
        .map_err(text_error)?
        .document;
    validate_chain(product, &expected, &original)?;
    validate_bindings(&expected)?;
    Ok(Plan {
        pid,
        operation: Operation::Module(key.into()),
        steps: vec![replacement, packets::complete()],
        expected: Some(expected),
        expected_slot: current_slot(current)?,
        expected_globals: global_fields(current),
        expected_auxiliary: vec![],
        warnings: preview
            .warnings
            .into_iter()
            .filter(|w| !w.contains("global volume endpoints"))
            .map(audition_warning)
            .collect(),
    })
}

pub fn bypass(pid: u16, current: &[Report64], key: &str, active: bool) -> Result<Plan> {
    let decoded = wire_preset::decode(pid, current).map_err(text_error)?;
    let (kind, _, _) = packets::categories()
        .iter()
        .find(|(_, candidate, _)| *candidate == key)
        .copied()
        .ok_or_else(|| error("Unknown module category"))?;
    let modules = decoded.document.modules();
    let module = modules
        .iter()
        .find(|m| m.key == key)
        .ok_or_else(|| error("Current module is absent"))?;
    let position = u8::try_from(module.position.unwrap_or(0))
        .map_err(|_| error("Module position exceeds one byte"))?;
    let step = crate::messages::effect_state(kind - 3, active, position);
    let mut expected = current.to_vec();
    let report = expected
        .iter_mut()
        .find(|r| r.as_bytes()[0] == 0x1c && r.as_bytes()[2] == kind && r.as_bytes()[3] == 0)
        .ok_or_else(|| error("Missing module report"))?;
    let mut b = *report.as_bytes();
    if kind == 5 {
        if active {
            b[52] &= !0x40
        } else {
            b[52] |= 0x40
        }
    } else {
        b[22] = u8::from(!active)
    };
    *report = Report64::from_bytes(b);
    let expected = wire_preset::decode(pid, &expected)
        .map_err(text_error)?
        .document;
    Ok(Plan {
        pid,
        operation: Operation::Bypass(key.into()),
        steps: vec![step],
        expected: Some(expected),
        expected_slot: current_slot(current)?,
        expected_globals: global_fields(current),
        expected_auxiliary: vec![],
        warnings: vec![],
    })
}

/// Positions in Stomp, Modulation, Delay, Reverb order. The stricter OpenFUSE
/// validation refuses colliding populated effects and mode-dependent SC moves.
pub fn routing(pid: u16, current: &[Report64], positions: [u8; 4]) -> Result<Plan> {
    let decoded = wire_preset::decode(pid, current).map_err(text_error)?;
    if decoded.document.product_id() == Some(2259) {
        return Err(error(
            "Super Champ routing requires device mode; direct routing is unavailable",
        ));
    }
    if positions.iter().any(|position| *position > 7) {
        return Err(error("Effect position is outside 0..7"));
    }
    let modules = decoded.document.modules();
    let mut occupied = Vec::new();
    for (offset, (_, key, _)) in packets::categories()[1..].iter().enumerate() {
        let module = modules
            .iter()
            .find(|m| m.key == *key)
            .ok_or_else(|| error("Missing effect"))?;
        if module.model_id != Some(0) {
            if occupied.contains(&positions[offset]) {
                return Err(error("Populated effects cannot occupy the same position"));
            }
            occupied.push(positions[offset]);
        }
    }
    let mut expected = current.to_vec();
    for report in &mut expected {
        let mut b = *report.as_bytes();
        if b[0] == 0x1c && b[3] == 0 && (6..=9).contains(&b[2]) {
            b[18] = positions[usize::from(b[2] - 6)];
            *report = Report64::from_bytes(b);
        }
    }
    let expected = wire_preset::decode(pid, &expected)
        .map_err(text_error)?
        .document;
    Ok(Plan {
        pid,
        operation: Operation::Routing,
        steps: vec![crate::messages::effect_positions(
            positions[0],
            positions[1],
            positions[2],
            positions[3],
        )],
        expected: Some(expected),
        expected_slot: current_slot(current)?,
        expected_globals: global_fields(current),
        expected_auxiliary: vec![],
        warnings: vec![],
    })
}

fn validate_chain(product: u32, expected: &PresetDocument, current: &PresetDocument) -> Result<()> {
    let capabilities =
        model_catalog::product(product).ok_or_else(|| error("Unknown preset family"))?;
    let current_modules = current.modules();
    let mut occupied = Vec::new();
    for module in expected
        .modules()
        .into_iter()
        .filter(|module| module.key != "Amplifier")
    {
        let position = module
            .position
            .ok_or_else(|| error("Missing effect position"))?;
        if position > 7 {
            return Err(error("Effect position is outside the supported 0..7 chain"));
        }
        if product == 2259 {
            let previous = current_modules
                .iter()
                .find(|m| m.key == module.key)
                .ok_or_else(|| error("Current effect is missing"))?;
            if previous.position != module.position {
                return Err(error(
                    "Super Champ routing changes require device mode; retain current positions",
                ));
            }
        } else if module.key == "Stompbox" && !capabilities.post_amp_stomp && position > 3 {
            return Err(error(
                "This family does not support post-amplifier stomp routing",
            ));
        }
        if module.model_id != Some(0) {
            if occupied.contains(&position) {
                return Err(error(
                    "Populated effects cannot occupy the same chain position",
                ));
            }
            occupied.push(position);
        }
    }
    Ok(())
}

fn audition_warning(message: String) -> String {
    if message.starts_with("Rehearsal encodes sound state only;") {
        "Only sound state is sent; author, tags, rating, artwork colors, band references and other file metadata remain in the local preset".into()
    } else {
        message
    }
}
fn validate_bindings(document: &PresetDocument) -> Result<()> {
    use crate::preset::expression::{self, PedalSlot};
    let product = document
        .product_id()
        .ok_or_else(|| error("Missing family"))?;
    let modules = document.modules();
    for slot in [PedalSlot::First, PedalSlot::Second] {
        let Some(pedal) = document.expression_pedal(slot).map_err(text_error)? else {
            continue;
        };
        let Some(key) = pedal
            .expression_mode_behavior
            .and_then(expression::target_key)
        else {
            continue;
        };
        let module = modules
            .iter()
            .find(|m| m.key == key)
            .ok_or_else(|| error("Expression target module is missing"))?;
        let model_id=module.model_id.filter(|id|*id!=0).ok_or_else(||error("Expression pedal points to an empty module; choose a valid assignment before audition"))?;
        let category = ModuleCategory::from_key(key)
            .ok_or_else(|| error("Unknown expression target category"))?;
        let index = pedal
            .parameter_index
            .ok_or_else(|| error("Expression assignment is missing ParameterIndex"))?;
        if model_catalog::model(product, category, model_id).is_none_or(|model| {
            !model
                .parameters
                .iter()
                .any(|parameter| parameter.index == index)
        }) {
            return Err(error(
                "Expression assignment points to a parameter absent from the desired model; choose a valid assignment before audition",
            ));
        }
    }
    Ok(())
}

/// Current-save clears IsModified before original SavePresetToAmp. The two
/// packets are NameSave (no ACK) then Complete (ACK).
pub(crate) fn save_current(pid: u16, current: &[Report64], target: u16) -> Result<Plan> {
    slot_select(pid, target)?;
    current_slot(current)?;
    let expected = wire_preset::decode(pid, current)
        .map_err(text_error)?
        .document;
    let steps = save_packets(&expected, target)?;
    Ok(Plan { pid, operation: Operation::SaveSlot(target), steps, expected: Some(expected), expected_slot: target, expected_globals: global_fields(current),
        expected_auxiliary: vec![], warnings: vec!["This explicitly overwrites one stored preset slot. ACK delivery must be followed by a separate stored-slot read and comparison; physical flash behavior remains unverified".into()] })
}

pub(crate) fn restore_slot(
    pid: u16,
    desired: &PresetDocument,
    current: &[Report64],
    target: u16,
) -> Result<Plan> {
    slot_select(pid, target)?;
    let mut plan = preset(pid, desired, current)?;
    let mut name = *plan.steps[0].as_bytes();
    name[4..6].copy_from_slice(&target.to_le_bytes());
    plan.steps[0] = Report64::from_bytes(name);
    plan.steps.extend(save_packets(
        plan.expected.as_ref().expect("preset expected"),
        target,
    )?);
    plan.operation = Operation::RestoreSlot(target);
    plan.expected_slot = target;
    plan.warnings
        .retain(|warning| !warning.starts_with("This operation replaces only"));
    plan.warnings.push("Preset-only restore overwrites this slot, changes the working preset and retains current global expression endpoints. Captured raw unknown bytes, global/system settings and physical knobs are not replayed; hardware behavior remains unverified".into());
    Ok(plan)
}
fn save_packets(document: &PresetDocument, target: u16) -> Result<Vec<Report64>> {
    let mut b = [0; 64];
    b[0] = 0x1c;
    b[1] = 1;
    b[2] = 3;
    b[4..6].copy_from_slice(&target.to_le_bytes());
    b[6] = 1;
    for (index, character) in document.metadata().name.chars().enumerate() {
        if index >= 48 || u32::from(character) > 255 || character == '\0' {
            return Err(error("Preset name is not representable in the device"));
        }
        b[16 + index] = character as u8;
    }
    Ok(vec![Report64::from_bytes(b), packets::complete()])
}

/// Original whole-backup loader constructs a fresh Preset; its global volume
/// endpoints are zero until the later System message. Never inherit live state.
pub(crate) fn whole_restore_packets(
    pid: u16,
    document: &PresetDocument,
    target: u16,
) -> Result<Vec<Report64>> {
    slot_select(pid, target)?;
    family(pid, document)?;
    wire_preset::encode(pid, document).map_err(text_error)?;
    let mut reports = packets::preset(document, target, Some((0, 0)))?;
    reports.extend(save_packets(document, target)?);
    Ok(reports)
}
