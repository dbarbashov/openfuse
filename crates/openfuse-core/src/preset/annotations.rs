//! Source-preserving local metadata and color edits. These do not contact Fender.
//!
//! Rating uses decimal text, avoiding binary floating point and preserving an
//! imported value's precision. Unknown signed enum values remain representable.

use super::{
    Edit, Node, PresetDocument, Result, append_child, attribute, attribute_node, child,
    direct_text, edit_attributes, edit_text, error, escape, named, parse_xml,
};

#[path = "vocabulary.rs"]
mod vocabulary;
pub use vocabulary::{DEFAULT_PEDAL_COLORS, GENRES, Genre, PEDAL_COLORS, PedalColor};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetAnnotations {
    /// A decimal using one dot or comma separator, with optional sign, no
    /// exponent or grouping. The original separator remains unchanged.
    /// None represents an absent attribute, distinct from zero.
    pub rating: Option<String>,
    /// Main genre, subgenre 1, subgenre 2. Unknown i32 enum values are retained.
    pub genres: [Option<i32>; 3],
    pub fender_id: Option<i32>,
}

impl PresetDocument {
    /// Missing fields stay absent; malformed present fields produce an error.
    /// Unrelated raw XML remains available even when these fields cannot project.
    pub fn annotations(&self) -> Result<PresetAnnotations> {
        let document = parse_xml(&self.xml)?;
        let Some(info) = info_node(document.root_element())? else {
            return Ok(PresetAnnotations::default());
        };
        let rating = attribute(info, "rating").map(str::to_owned);
        if let Some(rating) = &rating {
            validate_decimal(rating)?;
        }
        let mut genres = [None; 3];
        for (i, key) in ["genre1", "genre2", "genre3"].iter().enumerate() {
            genres[i] = attribute(info, key)
                .map(|value| signed_number(value, key))
                .transpose()?;
        }
        let fender_id = attribute(info, "fenderid")
            .map(|value| signed_number(value, "fenderid"))
            .transpose()?;
        Ok(PresetAnnotations {
            rating,
            genres,
            fender_id,
        })
    }

    /// Apply a complete annotation snapshot. None removes only that known
    /// attribute. Unchanged values preserve their original XML encoding.
    pub fn set_annotations(&mut self, values: &PresetAnnotations) -> Result<()> {
        if let Some(rating) = &values.rating {
            validate_decimal(rating)?;
        }
        let current = self.annotations()?;
        if current == *values {
            return Ok(());
        }
        let fields = [
            ("rating", values.rating.clone(), current.rating),
            (
                "genre1",
                values.genres[0].map(|v| v.to_string()),
                current.genres[0].map(|v| v.to_string()),
            ),
            (
                "genre2",
                values.genres[1].map(|v| v.to_string()),
                current.genres[1].map(|v| v.to_string()),
            ),
            (
                "genre3",
                values.genres[2].map(|v| v.to_string()),
                current.genres[2].map(|v| v.to_string()),
            ),
            (
                "fenderid",
                values.fender_id.map(|v| v.to_string()),
                current.fender_id.map(|v| v.to_string()),
            ),
        ];
        let document = parse_xml(&self.xml)?;
        let root = document.root_element();
        let mut edits = Vec::new();
        if let Some(info) = info_node(root)? {
            let mut additions = Vec::new();
            for (key, value, previous) in fields {
                if value == previous {
                    continue;
                }
                if let Some(value) = value {
                    additions.push((key, value));
                } else if let Some(attr) = attribute_node(info, key) {
                    edits.push((attr.range(), String::new()));
                }
            }
            edit_attributes(&self.xml, info, &additions, &mut edits)?;
        } else {
            let mut content = String::from("<Info");
            for (key, value, _) in fields {
                if let Some(value) = value {
                    content.push_str(&format!(" {key}=\"{}\"", escape(&value, true)));
                }
            }
            content.push_str("/>");
            append_to_fuse(&self.xml, root, &content, &mut edits)?;
        }
        self.apply(edits)
    }

    /// Original FUSE reads colors in child order, irrespective of Color/@ID.
    /// Exactly four recognized Color elements are required if PedalColors exists.
    /// Namespaced extensions are opaque; duplicate/missing values are errors.
    pub fn pedal_colors(&self) -> Result<Option<[i32; 4]>> {
        let document = parse_xml(&self.xml)?;
        let Some(palette) = palette_node(document.root_element())? else {
            return Ok(None);
        };
        let nodes: Vec<_> = palette
            .children()
            .filter(|node| named(*node, "Color"))
            .collect();
        if nodes.len() != 4 {
            return Err(error("PedalColors requires exactly four Color elements"));
        }
        let mut colors = [0; 4];
        for (i, node) in nodes.into_iter().enumerate() {
            if node.children().any(|child| child.is_element()) {
                return Err(error("A Color cannot contain nested elements"));
            }
            colors[i] = signed_number(&direct_text(node), "pedal color")?;
        }
        Ok(Some(colors))
    }

    /// Unknown numeric colors remain usable. The UI should offer the known
    /// palette while displaying unfamiliar IDs without substituting a color.
    pub fn set_pedal_colors(&mut self, colors: [i32; 4]) -> Result<()> {
        let current = self.pedal_colors()?;
        if current == Some(colors) {
            return Ok(());
        }
        let document = parse_xml(&self.xml)?;
        let root = document.root_element();
        let mut edits = Vec::new();
        if let Some(palette) = palette_node(root)? {
            let current = current.expect("a present palette was validated above");
            for (i, node) in palette
                .children()
                .filter(|node| named(*node, "Color"))
                .enumerate()
            {
                if current[i] != colors[i] {
                    edit_text(&self.xml, node, &colors[i].to_string(), &mut edits)?;
                }
            }
        } else {
            let mut content = String::from("<PedalColors>");
            for (i, value) in colors.iter().enumerate() {
                content.push_str(&format!("<Color ID=\"{}\">{value}</Color>", i + 1));
            }
            content.push_str("</PedalColors>");
            append_to_fuse(&self.xml, root, &content, &mut edits)?;
        }
        self.apply(edits)
    }
}

fn info_node<'a, 'b>(root: Node<'a, 'b>) -> Result<Option<Node<'a, 'b>>> {
    child(root, "FUSE")?
        .map(|fuse| child(fuse, "Info"))
        .transpose()
        .map(Option::flatten)
}

fn palette_node<'a, 'b>(root: Node<'a, 'b>) -> Result<Option<Node<'a, 'b>>> {
    child(root, "FUSE")?
        .map(|fuse| child(fuse, "PedalColors"))
        .transpose()
        .map(Option::flatten)
}

fn append_to_fuse(
    xml: &str,
    root: Node<'_, '_>,
    content: &str,
    edits: &mut Vec<Edit>,
) -> Result<()> {
    if let Some(fuse) = child(root, "FUSE")? {
        append_child(xml, fuse, content, edits)
    } else {
        append_child(xml, root, &format!("<FUSE>{content}</FUSE>"), edits)
    }
}

fn signed_number(value: &str, field: &str) -> Result<i32> {
    let value = value.trim();
    let digits = value.strip_prefix(['-', '+']).unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(format!("Invalid signed decimal {field}")));
    }
    value
        .parse()
        .map_err(|_| error(format!("Out-of-range {field}")))
}

fn validate_decimal(value: &str) -> Result<()> {
    let value = value.trim();
    let magnitude = value.strip_prefix(['-', '+']).unwrap_or(value);
    let mut parts = magnitude.split(['.', ',']);
    let whole = parts.next().unwrap_or_default();
    let fractional = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || (whole.is_empty() && fractional.is_empty())
        || !whole
            .bytes()
            .chain(fractional.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(error(
            "Rating must use a single dot or comma decimal separator without grouping or exponent",
        ));
    }
    // Decimal's exact .NET representable subset: 96-bit coefficient, scale 0..28.
    // Trailing zeros may reduce scale without changing the exact numeric value.
    let fractional = fractional.trim_end_matches('0');
    let coefficient = format!("{whole}{fractional}");
    let coefficient = coefficient.trim_start_matches('0');
    let coefficient = if coefficient.is_empty() {
        0
    } else {
        coefficient
            .parse::<u128>()
            .map_err(|_| error("Rating exceeds decimal precision"))?
    };
    if fractional.len() > 28 || coefficient > ((1_u128 << 96) - 1) {
        return Err(error("Rating exceeds decimal precision"));
    }
    Ok(())
}
