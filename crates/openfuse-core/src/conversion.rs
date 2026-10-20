//! Explicit, source-bound FUSE compatibility conversion. Normal XML parsing
//! stays strict. The candidate is a new document; the input is never rewritten.
use crate::{
    model_catalog::{self, ModuleCategory},
    preset::{self, PresetDocument},
};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, ops::Range};
pub const MAX_CHANGES: usize = 4096;
mod auxiliary;
mod legacy;
mod modules;
pub use legacy::{LegacyDelay, legacy_delay_value, repair_double_encoded_xml};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionError(String);
impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ConversionError {}
pub type Result<T> = std::result::Result<T, ConversionError>;
fn error(value: impl fmt::Display) -> ConversionError {
    ConversionError(value.to_string())
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Options {
    pub target_product: u32,
    pub repair_double_entities: bool,
    pub normalize_legacy_delays: bool,
    #[serde(default)]
    pub assume_missing_gdec: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Repair,
    Replacement,
    Loss,
    Normalization,
    Metadata,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub message: String,
}
#[derive(Debug, Clone)]
pub struct Plan {
    fingerprint: String,
    source_product: Option<u32>,
    target_product: u32,
    changes: Vec<Change>,
    result: PresetDocument,
}
pub fn supported_targets(source_product: u32) -> Vec<u32> {
    model_catalog::products()
        .iter()
        .map(|p| p.id)
        .filter(|target| compatible(source_product, *target))
        .collect()
}
fn compatible(source: u32, target: u32) -> bool {
    match target {
        1 | 6 | 7 | 13 => matches!(source, 1 | 6 | 7 | 13),
        2259 => matches!(source, 2259 | 1 | 13),
        10 => source == 10,
        11 => source == 11,
        _ => false,
    }
}
fn fingerprint(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
impl Plan {
    pub fn preview(source: &str, options: Options) -> Result<Self> {
        if source.len() > preset::MAX_PRESET_BYTES {
            return Err(error("Preset XML exceeds 2 MiB"));
        }
        let mut changes = vec![];
        let mut xml = if options.repair_double_entities {
            legacy::repair(source, &mut changes)?
        } else {
            source.to_string()
        };
        let initial = preset::parse_xml(&xml).map_err(error)?;
        let source_product = initial
            .root_element()
            .attribute("ProductId")
            .and_then(|v| v.trim().parse::<u16>().ok())
            .map(u32::from);
        if source_product.is_none() {
            if !options.assume_missing_gdec || options.target_product != 10 {
                return Err(error(
                    "Missing or invalid ProductId requires explicit assume_missing_gdec and target family 10",
                ));
            }
            let mut repairs = vec![];
            attr(
                &xml,
                initial.root_element(),
                "ProductId",
                Some("10".into()),
                "Preset/@ProductId",
                ChangeKind::Repair,
                "Original missing/invalid ProductId fallback, explicitly selected",
                &mut repairs,
                &mut changes,
            )?;
            xml = apply_edits(&xml, repairs)?;
        }
        let input = PresetDocument::parse(&xml).map_err(error)?;
        let from = input
            .product_id()
            .expect("validated or explicitly repaired family");
        if model_catalog::product(from).is_none() {
            return Err(error(
                "Unknown source family; legacy ProductId 2 is not silently relabelled",
            ));
        }
        if !compatible(from, options.target_product) {
            return Err(error(
                "Target family is outside the original FUSE import compatibility list",
            ));
        }
        let document = preset::parse_xml(&xml).map_err(error)?;
        let root = document.root_element();
        let mut edits = vec![];
        attr(
            &xml,
            root,
            "ProductId",
            Some(options.target_product.to_string()),
            "Preset/@ProductId",
            ChangeKind::Metadata,
            "Target preset family",
            &mut edits,
            &mut changes,
        )?;
        let device = match options.target_product {
            1 => "Mustang I/II",
            13 => "Mustang V2 I/II",
            7 => "Mustang Floor",
            6 => "Passport Mini",
            2259 => "SuperChampX2",
            11 => "Bronco",
            10 => "G-DEC3",
            _ => unreachable!(),
        };
        attr(
            &xml,
            root,
            "amplifier",
            Some(device.into()),
            "Preset/@amplifier",
            ChangeKind::Metadata,
            "Original target capability label",
            &mut edits,
            &mut changes,
        )?;
        modules::convert(&xml, root, &input, options, &mut edits, &mut changes)?;
        auxiliary::convert(&xml, root, &input, options, &mut edits, &mut changes)?;
        if changes.len() > MAX_CHANGES {
            return Err(error("Conversion preview exceeds 4096 reported changes"));
        }
        let result = apply_edits(&xml, edits)?;
        let result = PresetDocument::parse(&result).map_err(error)?;
        Ok(Self {
            fingerprint: fingerprint(source),
            source_product,
            target_product: options.target_product,
            changes,
            result,
        })
    }
    pub fn source_fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn source_product(&self) -> Option<u32> {
        self.source_product
    }
    pub fn target_product(&self) -> u32 {
        self.target_product
    }
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }
    pub fn result(&self) -> &PresetDocument {
        &self.result
    }
    pub fn apply(&self, source: &str) -> Result<PresetDocument> {
        if fingerprint(source) != self.fingerprint {
            return Err(error(
                "Conversion source changed since preview; review again",
            ));
        }
        Ok(self.result.clone())
    }
}
type Edit = (Range<usize>, String);
fn change(
    changes: &mut Vec<Change>,
    kind: ChangeKind,
    path: impl Into<String>,
    before: Option<String>,
    after: Option<String>,
    message: impl Into<String>,
) {
    changes.push(Change {
        kind,
        path: path.into(),
        before,
        after,
        message: message.into(),
    });
}
#[allow(clippy::too_many_arguments)]
fn attr(
    xml: &str,
    node: Node<'_, '_>,
    key: &str,
    value: Option<String>,
    path: &str,
    kind: ChangeKind,
    message: &str,
    edits: &mut Vec<Edit>,
    changes: &mut Vec<Change>,
) -> Result<()> {
    let previous = node.attribute(key).map(str::to_string);
    if previous == value {
        return Ok(());
    }
    if let Some(value) = &value {
        preset::edit_attributes(xml, node, &[(key, value.clone())], edits).map_err(error)?;
    } else if let Some(attribute) = node
        .attributes()
        .find(|a| a.namespace().is_none() && a.name() == key)
    {
        edits.push((attribute.range(), String::new()));
    }
    change(changes, kind, path, previous, value, message);
    Ok(())
}
fn apply_edits(xml: &str, mut edits: Vec<Edit>) -> Result<String> {
    edits.sort_by_key(|e| e.0.start);
    if edits.windows(2).any(|w| w[0].0.end > w[1].0.start) {
        return Err(error("Overlapping conversion edits"));
    }
    let mut result = xml.to_string();
    for (range, value) in edits.into_iter().rev() {
        result.replace_range(range, &value);
    }
    Ok(result)
}
fn child<'a, 'i>(root: Node<'a, 'i>, key: &str) -> Result<Option<Node<'a, 'i>>> {
    preset::child(root, key).map_err(error)
}
fn text(node: Node<'_, '_>) -> String {
    node.children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect()
}
