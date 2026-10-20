//! Offline, bounded FUSE XML editing with byte preservation outside edited fields.
//!
//! Preserves the preset XML representation, including module parameters and
//! unknown fields outside the edited region.
//! Raw parameters are not calibrated controls and do not authorize device writes.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    ops::Range,
};

use roxmltree::{Document, Node, ParsingOptions};

pub mod annotations;
pub mod band;
pub mod expression;
pub mod module_import;
pub use annotations::PresetAnnotations;

pub const MAX_PRESET_BYTES: usize = 2 * 1024 * 1024;
const MAX_NODES: u32 = 65_536;
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetError(String);

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for PresetError {}

type Result<T> = std::result::Result<T, PresetError>;
type Edit = (Range<usize>, String);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetMetadata {
    pub name: String,
    pub author: String,
    pub description: String,
    /// Original tags string; separators and vocabulary are not normalized.
    pub tags: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetParameter {
    /// XML ControlIndex is the parameter index, not a USB hardware control index.
    pub index: u8,
    pub value: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetModule {
    pub key: String,
    /// Category only; unknown model IDs are never given invented names.
    pub label: String,
    pub model_id: Option<u16>,
    pub position: Option<u16>,
    /// FUSE stores IsActivated as BypassState: 1 is active, 0 is bypassed.
    /// None means the file does not declare a state, not an inferred default.
    pub active: Option<bool>,
    pub bypass_mode: Option<u8>,
    pub parameters: Vec<PresetParameter>,
}

#[derive(Debug, Clone)]
pub struct PresetDocument {
    xml: String,
    metadata: PresetMetadata,
    modules: Vec<PresetModule>,
    product_id: Option<u32>,
}

impl PresetDocument {
    pub fn parse(xml: &str) -> Result<Self> {
        let document = parse_xml(xml)?;
        let root = document.root_element();
        if !named(root, "Preset") {
            return Err(error("Expected an unnamespaced Preset root"));
        }
        let product_id = attribute(root, "ProductId")
            .map(|value| number(value, "preset product ID"))
            .transpose()?;
        let info = child(root, "FUSE")?
            .map(|fuse| child(fuse, "Info"))
            .transpose()?
            .flatten();
        let metadata = info
            .map(|node| PresetMetadata {
                name: attribute(node, "name").unwrap_or_default().into(),
                author: attribute(node, "author").unwrap_or_default().into(),
                description: direct_text(node),
                tags: attribute(node, "tags").unwrap_or_default().into(),
            })
            .unwrap_or_default();
        let mut modules = Vec::new();
        for (key, module) in module_nodes(root)? {
            let mut parameters = Vec::new();
            let mut indices = BTreeSet::new();
            for param in module.children().filter(|node| named(*node, "Param")) {
                if param.children().any(|node| node.is_element()) {
                    return Err(error("A Param cannot contain nested elements"));
                }
                let index = number::<u8>(
                    attribute(param, "ControlIndex")
                        .ok_or_else(|| error("Param is missing ControlIndex"))?,
                    "ControlIndex",
                )?;
                if !indices.insert(index) {
                    return Err(error("Duplicate parameter index in a module"));
                }
                parameters.push(PresetParameter {
                    index,
                    value: number(&direct_text(param), "parameter value")?,
                });
            }
            modules.push(PresetModule {
                key: key.into(),
                label: key.into(),
                model_id: attribute(module, "ID")
                    .map(|value| number(value, "model ID"))
                    .transpose()?,
                position: attribute(module, "POS")
                    .map(|value| number(value, "module position"))
                    .transpose()?,
                active: attribute(module, "BypassState")
                    .map(|value| number::<u8>(value, "activation state").map(|value| value != 0))
                    .transpose()?,
                bypass_mode: attribute(module, "BypassMode")
                    .map(|value| number(value, "bypass mode"))
                    .transpose()?,
                parameters,
            });
        }
        Ok(Self {
            xml: xml.into(),
            metadata,
            modules,
            product_id,
        })
    }

    /// Untouched documents round-trip byte-for-byte, including XML declarations,
    /// comments, whitespace, unknown extensions, and namespace prefixes.
    pub fn to_xml(&self) -> Result<String> {
        Ok(self.xml.clone())
    }

    pub fn metadata(&self) -> PresetMetadata {
        self.metadata.clone()
    }

    pub fn modules(&self) -> Vec<PresetModule> {
        self.modules.clone()
    }

    /// FUSE's preset family identifier, which is distinct from a USB product ID.
    pub fn product_id(&self) -> Option<u32> {
        self.product_id
    }

    pub fn set_module_active(&mut self, module_key: &str, active: bool) -> Result<()> {
        self.set_module_attribute(module_key, "BypassState", &u8::from(active).to_string())
    }

    pub fn set_module_position(&mut self, module_key: &str, position: u16) -> Result<()> {
        self.set_module_positions(&[(module_key, position)])
    }

    /// Move a populated effect using FUSE's bank-local insertion behavior.
    /// Colliding effects shift toward a free slot, preferring the right side.
    /// SuperChamp mode-dependent routing is unavailable without device state.
    pub fn move_module(&mut self, module_key: &str, position: u16) -> Result<()> {
        let product = self
            .product_id
            .and_then(crate::model_catalog::product)
            .ok_or_else(|| error("Unknown preset family; chain rules unavailable"))?;
        if product.id == 2259 {
            return Err(error(
                "SuperChamp routing requires device mode; offline moves are unavailable",
            ));
        }
        if module_key == "Amplifier" {
            return Err(error("The amplifier is the fixed chain boundary"));
        }
        let modules = self.modules();
        let moving = modules
            .iter()
            .find(|module| module.key == module_key)
            .ok_or_else(|| error("Unknown module key"))?;
        if moving.model_id.is_none_or(|id| id == 0) {
            return Err(error("Select an effect model before moving it"));
        }
        let maximum = if product.orderable { 7 } else { 1 };
        if position > maximum
            || (module_key == "Stompbox" && !product.post_amp_stomp && position != 0)
        {
            return Err(error(
                "Position is unavailable for this effect and preset family",
            ));
        }
        if !product.orderable {
            return self.set_module_position(module_key, position);
        }
        let mut occupied = BTreeMap::new();
        for module in modules
            .iter()
            .filter(|module| module.key != "Amplifier" && module.model_id != Some(0))
        {
            let slot = module
                .position
                .filter(|slot| *slot <= 7)
                .ok_or_else(|| error("Existing effect has an unknown or invalid chain position"))?;
            if occupied.insert(slot, module.key.as_str()).is_some() {
                return Err(error("Existing effects share a chain position"));
            }
        }
        if moving.position == Some(position) {
            return Ok(());
        }
        occupied.retain(|_, key| *key != module_key);
        let first = position / 4 * 4;
        let last = first + 3;
        let mut updates = vec![(module_key, position)];
        if let Some(free) = (position..=last).find(|slot| !occupied.contains_key(slot)) {
            for slot in position..free {
                updates.push((occupied[&slot], slot + 1));
            }
        } else if let Some(free) = (first..position)
            .rev()
            .find(|slot| !occupied.contains_key(slot))
        {
            for slot in free + 1..=position {
                updates.push((occupied[&slot], slot - 1));
            }
        } else {
            return Err(error("No available slot in the destination bank"));
        }
        self.set_module_positions(&updates)
    }

    /// Change several positions atomically. Family-specific order constraints
    /// belong to the catalog layer; unknown stored positions remain readable.
    pub fn set_module_positions(&mut self, positions: &[(&str, u16)]) -> Result<()> {
        let document = parse_xml(&self.xml)?;
        let mut keys = BTreeSet::new();
        let mut edits = Vec::new();
        for &(key, position) in positions {
            if !keys.insert(key) {
                return Err(error("Duplicate module key in position update"));
            }
            let module = find_module(document.root_element(), key)?;
            edit_attributes(
                &self.xml,
                module,
                &[("POS", position.to_string())],
                &mut edits,
            )?;
        }
        self.apply(edits)
    }

    fn set_module_attribute(&mut self, module_key: &str, key: &str, value: &str) -> Result<()> {
        let document = parse_xml(&self.xml)?;
        let module = find_module(document.root_element(), module_key)?;
        let mut edits = Vec::new();
        edit_attributes(&self.xml, module, &[(key, value.into())], &mut edits)?;
        self.apply(edits)
    }

    /// Replace the known model/parameter state with caller-supplied catalog
    /// defaults. Values must already be encoded for FUSE XML (not display units).
    /// Unknown module extensions and retained parameter attributes survive.
    /// Refuses to remove a parameter carrying unknown content instead of losing it.
    pub fn replace_module_model(
        &mut self,
        module_key: &str,
        model_id: u16,
        parameters: &[PresetParameter],
        bypass_mode: Option<u8>,
    ) -> Result<()> {
        let mut indices = BTreeSet::new();
        for parameter in parameters {
            if !indices.insert(parameter.index) {
                return Err(error("Duplicate replacement parameter index"));
            }
        }
        let document = parse_xml(&self.xml)?;
        let module = find_module(document.root_element(), module_key)?;
        let mut edits = Vec::new();
        let mut attrs = vec![("ID", model_id.to_string())];
        if let Some(mode) = bypass_mode {
            attrs.push(("BypassMode", mode.to_string()));
        }
        edit_attributes(&self.xml, module, &attrs, &mut edits)?;
        let mut existing = BTreeSet::new();
        for node in module.children().filter(|node| named(*node, "Param")) {
            let index = number::<u8>(attribute(node, "ControlIndex").unwrap(), "ControlIndex")?;
            existing.insert(index);
            if let Some(parameter) = parameters.iter().find(|parameter| parameter.index == index) {
                if number::<u16>(&direct_text(node), "parameter value")? != parameter.value {
                    edit_text(&self.xml, node, &parameter.value.to_string(), &mut edits)?;
                }
            } else {
                if node
                    .attributes()
                    .any(|attr| attr.namespace().is_some() || attr.name() != "ControlIndex")
                    || node.children().any(|child| !child.is_text())
                {
                    return Err(error(format!(
                        "Cannot remove parameter {index} containing unknown attributes or nodes"
                    )));
                }
                edits.push((node.range(), String::new()));
            }
        }
        let mut additions = String::new();
        for parameter in parameters {
            if !existing.contains(&parameter.index) {
                additions.push_str(&format!(
                    "<Param ControlIndex=\"{}\">{}</Param>",
                    parameter.index, parameter.value
                ));
            }
        }
        if !additions.is_empty() {
            append_child(&self.xml, module, &additions, &mut edits)?;
        }
        self.update_expression_bindings(
            document.root_element(),
            module_key,
            model_id,
            parameters,
            &mut edits,
        )?;
        self.apply(edits)
    }

    fn update_expression_bindings(
        &self,
        root: Node<'_, '_>,
        module_key: &str,
        model_id: u16,
        parameters: &[PresetParameter],
        edits: &mut Vec<Edit>,
    ) -> Result<()> {
        use crate::model_catalog::{self, ModuleCategory};
        for (tag, first) in [
            ("FirstExpressionPedal", true),
            ("SecondExpressionPedal", false),
        ] {
            let Some(pedal) = child(root, tag)? else {
                continue;
            };
            let behavior = attribute(pedal, "ExpressionModeBehavior")
                .ok_or_else(|| error(format!("{tag} is missing ExpressionModeBehavior; model change cannot preserve its binding")))?;
            let behavior = number::<u16>(behavior, "expression mode behavior")?;
            let associated = match behavior {
                0 | 6 => continue,
                1 => "Amplifier",
                2 => "Stompbox",
                3 => "Modulation",
                4 => "Delay",
                5 => "Reverb",
                _ => {
                    return Err(error(format!(
                        "{tag} has an unknown expression mode; model change cannot preserve its binding"
                    )));
                }
            };
            if associated != module_key {
                continue;
            }
            if model_id == 0 && module_key != "Amplifier" {
                // Removing an effect turns off associated expression behavior.
                edit_attributes(
                    &self.xml,
                    pedal,
                    &[
                        ("ExpressionModeBehavior", "0".into()),
                        ("ParameterIndex", "0".into()),
                    ],
                    edits,
                )?;
            } else if first {
                let category = ModuleCategory::from_key(module_key)
                    .ok_or_else(|| error("Unknown module category"))?;
                let model = self.product_id.and_then(|id| model_catalog::model(id, category, model_id))
                    .ok_or_else(|| error("First expression pedal requires a known model's default parameter binding"))?;
                let index = model.expression_pedal_index;
                if !parameters.iter().any(|parameter| parameter.index == index) {
                    return Err(error(format!(
                        "First expression pedal default parameter {index} is absent from the replacement model"
                    )));
                }
                edit_attributes(
                    &self.xml,
                    pedal,
                    &[("ParameterIndex", index.to_string())],
                    edits,
                )?;
            } else {
                // The second pedal stores its own GenericIndex, independently
                // of the module's default. Keep it only when still meaningful.
                let index = attribute(pedal, "ParameterIndex")
                    .ok_or_else(|| error("Second expression pedal is missing ParameterIndex"))?;
                let index = number::<u8>(index, "second expression parameter index")?;
                if !parameters.iter().any(|parameter| parameter.index == index) {
                    return Err(error(format!(
                        "Second expression pedal parameter {index} is absent from the replacement model; reassign it before changing models"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Changes only the supplied metadata fields; ratings, colors, unknown
    /// attributes/elements, and all device settings remain verbatim.
    pub fn set_metadata(&mut self, metadata: &PresetMetadata) -> Result<()> {
        if metadata == &self.metadata {
            return Ok(());
        }
        let document = parse_xml(&self.xml)?;
        let root = document.root_element();
        let fuse = child(root, "FUSE")?;
        let info = fuse.map(|node| child(node, "Info")).transpose()?.flatten();
        let mut edits = Vec::new();
        if let Some(info) = info {
            edit_attributes(
                &self.xml,
                info,
                &[
                    ("name", metadata.name.clone()),
                    ("author", metadata.author.clone()),
                    ("tags", metadata.tags.clone()),
                ],
                &mut edits,
            )?;
            if metadata.description != self.metadata.description {
                edit_text(&self.xml, info, &metadata.description, &mut edits)?;
            }
        } else {
            let info = format!(
                "<Info name=\"{}\" author=\"{}\" tags=\"{}\">{}</Info>",
                escape(&metadata.name, true),
                escape(&metadata.author, true),
                escape(&metadata.tags, true),
                escape(&metadata.description, false)
            );
            if let Some(fuse) = fuse {
                append_child(&self.xml, fuse, &info, &mut edits)?;
            } else {
                append_child(&self.xml, root, &format!("<FUSE>{info}</FUSE>"), &mut edits)?;
            }
        }
        self.apply(edits)
    }

    pub fn set_parameter(&mut self, module_key: &str, index: u8, value: u16) -> Result<()> {
        self.set_parameters(module_key, &[PresetParameter { index, value }])
    }

    /// Apply a primary parameter plus dependent parameter updates as one edit.
    /// This is atomic and does not interpret or execute catalog rules itself.
    pub fn set_parameters(&mut self, module_key: &str, values: &[PresetParameter]) -> Result<()> {
        let document = parse_xml(&self.xml)?;
        let module = find_module(document.root_element(), module_key)?;
        let mut seen = BTreeSet::new();
        let mut edits = Vec::new();
        for value in values {
            if !seen.insert(value.index) {
                return Err(error("Duplicate parameter index in update"));
            }
            let parameter = module
                .children()
                .find(|node| {
                    named(*node, "Param")
                        && attribute(*node, "ControlIndex")
                            .and_then(|text| text.trim().parse::<u8>().ok())
                            == Some(value.index)
                })
                .ok_or_else(|| error("Unknown parameter index"))?;
            if number::<u16>(&direct_text(parameter), "parameter value")? != value.value {
                edit_text(&self.xml, parameter, &value.value.to_string(), &mut edits)?;
            }
        }
        self.apply(edits)
    }

    fn apply(&mut self, mut edits: Vec<Edit>) -> Result<()> {
        // Edits are transactional. Invalid XML characters, limits, or any
        // revalidation failure leave the current document unchanged.
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        if edits.windows(2).any(|pair| pair[0].0.end > pair[1].0.start) {
            return Err(error("Overlapping XML edits"));
        }
        let mut xml = self.xml.clone();
        for (range, value) in edits.into_iter().rev() {
            xml.replace_range(range, &value);
        }
        *self = Self::parse(&xml)?;
        Ok(())
    }
}

fn error(message: impl Into<String>) -> PresetError {
    PresetError(message.into())
}

pub(crate) fn parse_xml(xml: &str) -> Result<Document<'_>> {
    if xml.len() > MAX_PRESET_BYTES {
        return Err(error("Preset exceeds 2 MiB limit"));
    }
    // Defensive explicit prohibition also covers empty/external-only doctypes.
    // These tokens in comments or CDATA are conservatively rejected as well.
    if xml.contains("<!DOCTYPE") || xml.contains("<!ENTITY") {
        return Err(error("DTD and entity declarations are forbidden"));
    }
    let document = Document::parse_with_options(
        xml,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            entity_resolver: None,
        },
    )
    .map_err(|err| error(err.to_string()))?;
    if document
        .descendants()
        .any(|node| node.ancestors().take(MAX_DEPTH + 1).count() > MAX_DEPTH)
    {
        return Err(error("Preset exceeds XML nesting limit"));
    }
    Ok(document)
}

fn named(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().namespace().is_none() && node.tag_name().name() == name
}

fn attribute_node<'a, 'input>(
    node: Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Attribute<'a, 'input>> {
    node.attributes()
        .find(|attribute| attribute.namespace().is_none() && attribute.name() == name)
}

fn attribute<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<&'a str> {
    attribute_node(node, name).map(|attribute| attribute.value())
}

pub(crate) fn child<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>> {
    let mut children = parent.children().filter(|node| named(*node, name));
    let first = children.next();
    if children.next().is_some() {
        return Err(error(format!("Ambiguous duplicate {name} element")));
    }
    Ok(first)
}

fn module_nodes<'a, 'input>(
    root: Node<'a, 'input>,
) -> Result<Vec<(&'static str, Node<'a, 'input>)>> {
    let mut modules = Vec::new();
    if let Some(module) = child(root, "Amplifier")?
        .map(|node| child(node, "Module"))
        .transpose()?
        .flatten()
    {
        modules.push(("Amplifier", module));
    }
    if let Some(fx) = child(root, "FX")? {
        for key in ["Stompbox", "Modulation", "Delay", "Reverb"] {
            if let Some(module) = child(fx, key)?
                .map(|node| child(node, "Module"))
                .transpose()?
                .flatten()
            {
                modules.push((key, module));
            }
        }
    }
    Ok(modules)
}

fn find_module<'a, 'input>(root: Node<'a, 'input>, key: &str) -> Result<Node<'a, 'input>> {
    module_nodes(root)?
        .into_iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, node)| node)
        .ok_or_else(|| error("Unknown module key"))
}

pub(crate) fn edit_attributes(
    xml: &str,
    node: Node<'_, '_>,
    values: &[(&str, String)],
    edits: &mut Vec<Edit>,
) -> Result<()> {
    let mut additions = String::new();
    for (key, value) in values {
        if let Some(attr) = attribute_node(node, key) {
            if attr.value() != value {
                let range = attr.range();
                let raw = &xml[range.clone()];
                let equals = raw
                    .find('=')
                    .ok_or_else(|| error("Invalid attribute span"))?;
                let quote_at = raw[equals + 1..]
                    .find(['\'', '"'])
                    .ok_or_else(|| error("Invalid attribute quote"))?
                    + equals
                    + 1;
                edits.push((
                    range.start + quote_at + 1..range.end - 1,
                    escape(value, true),
                ));
            }
        } else {
            additions.push_str(&format!(" {key}=\"{}\"", escape(value, true)));
        }
    }
    if !additions.is_empty() {
        let end = opening_end(xml, node)?;
        let at = end
            - if xml.as_bytes()[end - 2] == b'/' {
                2
            } else {
                1
            };
        edits.push((at..at, additions));
    }
    Ok(())
}

fn number<T: std::str::FromStr>(value: &str, field: &str) -> Result<T> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(format!("Invalid unsigned decimal {field}")));
    }
    value
        .parse()
        .map_err(|_| error(format!("Out-of-range {field}")))
}

fn direct_text(node: Node<'_, '_>) -> String {
    node.children()
        .filter(|node| node.is_text())
        .filter_map(|node| node.text())
        .collect()
}

fn escape(value: &str, attribute: bool) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' if attribute => escaped.push_str("&quot;"),
            '\'' if attribute => escaped.push_str("&apos;"),
            '\n' if attribute => escaped.push_str("&#10;"),
            '\t' if attribute => escaped.push_str("&#9;"),
            '\r' => escaped.push_str("&#13;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn opening_end(xml: &str, node: Node<'_, '_>) -> Result<usize> {
    let mut quote = None;
    for (offset, byte) in xml.as_bytes()[node.range()].iter().copied().enumerate() {
        match (byte, quote) {
            (b'\'' | b'"', None) => quote = Some(byte),
            (_, Some(current)) if current == byte => quote = None,
            (b'>', None) => return Ok(node.range().start + offset + 1),
            _ => {}
        }
    }
    Err(error("Cannot locate opening tag"))
}

pub(crate) fn append_child(
    xml: &str,
    node: Node<'_, '_>,
    content: &str,
    edits: &mut Vec<Edit>,
) -> Result<()> {
    let end = opening_end(xml, node)?;
    if xml.as_bytes()[end - 2] == b'/' {
        edits.push((
            end - 2..end,
            format!(">{content}</{}>", node.tag_name().name()),
        ));
    } else {
        let closing = xml[node.range()]
            .rfind("</")
            .ok_or_else(|| error("Cannot locate closing tag"))?
            + node.range().start;
        edits.push((closing..closing, content.into()));
    }
    Ok(())
}

pub(crate) fn edit_text(
    xml: &str,
    node: Node<'_, '_>,
    value: &str,
    edits: &mut Vec<Edit>,
) -> Result<()> {
    let mut text = node.children().filter(|node| node.is_text());
    if let Some(first) = text.next() {
        edits.push((first.range(), escape(value, false)));
        edits.extend(text.map(|node| (node.range(), String::new())));
    } else {
        append_child(xml, node, &escape(value, false), edits)?;
    }
    Ok(())
}
