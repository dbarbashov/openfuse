//! Compatibility facts recovered from FUSE 2.7.1, not a hardware support claim.
//!
//! Parameter values here are the on-disk XML representation. In particular,
//! G-DEC delay times are milliseconds, not FUSE's internal normalized values.
//! Catalog availability is not a claim of validated hardware support.

use std::{ops::RangeInclusive, sync::OnceLock};

#[path = "model_rules.rs"]
mod rules;
#[path = "model_values.rs"]
mod values;
pub use rules::cascade_parameters;
pub(crate) use rules::load_wire_parameters;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleCategory {
    Amplifier,
    Stompbox,
    Modulation,
    Delay,
    Reverb,
}
impl ModuleCategory {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "Amplifier" | "amplifier" => Some(Self::Amplifier),
            "Stompbox" | "Distortion" | "stompbox" => Some(Self::Stompbox),
            "Modulation" | "modulation" => Some(Self::Modulation),
            "Delay" | "delay" => Some(Self::Delay),
            "Reverb" | "reverb" => Some(Self::Reverb),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Product {
    pub id: u32,
    pub name: &'static str,
    pub fxdb_id: u16,
    pub default_amp_id: u16,
    pub orderable: bool,
    pub post_amp_stomp: bool,
    pub store_bypass: bool,
}
const PRODUCTS: [Product; 7] = [
    Product {
        id: 10,
        name: "G-DEC 3",
        fxdb_id: 10,
        default_amp_id: 3,
        orderable: false,
        post_amp_stomp: false,
        store_bypass: false,
    },
    Product {
        id: 1,
        name: "Mustang",
        fxdb_id: 1,
        default_amp_id: 117,
        orderable: true,
        post_amp_stomp: true,
        store_bypass: true,
    },
    Product {
        id: 13,
        name: "Mustang V2",
        fxdb_id: 13,
        default_amp_id: 241,
        orderable: true,
        post_amp_stomp: true,
        store_bypass: true,
    },
    Product {
        id: 7,
        name: "Mustang Floor",
        fxdb_id: 7,
        default_amp_id: 241,
        orderable: true,
        post_amp_stomp: true,
        store_bypass: true,
    },
    Product {
        id: 11,
        name: "Bronco",
        fxdb_id: 11,
        default_amp_id: 150,
        orderable: true,
        post_amp_stomp: true,
        store_bypass: true,
    },
    Product {
        id: 2259,
        name: "Super Champ X2",
        fxdb_id: 2259,
        default_amp_id: 117,
        orderable: true,
        post_amp_stomp: false,
        store_bypass: true,
    },
    Product {
        id: 6,
        name: "Passport Mini",
        fxdb_id: 6,
        default_amp_id: 117,
        orderable: true,
        post_amp_stomp: true,
        store_bypass: true,
    },
];
pub fn products() -> &'static [Product] {
    &PRODUCTS
}
pub fn product(id: u32) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.id == id)
}

#[derive(Debug, Clone)]
pub struct Model {
    pub id: u16,
    pub name: String,
    pub category: ModuleCategory,
    pub parameters: Vec<Parameter>,
    pub bypass_mode: u8,
    pub expression_pedal_index: u8,
    pub tap_parameter_index: u8,
    product_ids: Vec<u32>,
}
#[derive(Debug, Clone)]
pub struct Parameter {
    pub index: u8,
    pub control_index: u8,
    pub name: String,
    pub type_id: u16,
    pub group: u8,
    /// Exact default in XML units; not necessarily within the documented domain.
    pub default_raw: u16,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: u16,
    pub label: String,
}
impl Parameter {
    /// Empty for continuous, integer, reserved, or incompletely recovered types.
    pub fn choices(&self) -> Vec<Choice> {
        values::choices(self.type_id)
    }
    /// Suggested editing domain. Never use this to clamp a loaded preset.
    pub fn raw_range(&self) -> RangeInclusive<u16> {
        values::range(self.type_id)
    }
    pub fn format_value(&self, value: u16) -> String {
        values::format(self.type_id, value)
    }
}

pub fn models(product_id: u32, category: ModuleCategory) -> impl Iterator<Item = &'static Model> {
    catalog()
        .iter()
        .filter(move |m| m.category == category && m.product_ids.contains(&product_id))
}
pub fn model(product_id: u32, category: ModuleCategory, id: u16) -> Option<&'static Model> {
    models(product_id, category).find(|m| m.id == id)
}

fn catalog() -> &'static [Model] {
    static CATALOG: OnceLock<Vec<Model>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut result: Vec<Model> = Vec::new();
        // This table is embedded at compilation and exhaustively validated in tests.
        for line in include_str!("../data/models.tsv")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let fields: Vec<_> = line.split('\t').collect();
            if fields[0] == "M" {
                result.push(Model {
                    product_ids: fields[1]
                        .split(',')
                        .map(|v| v.parse().expect("embedded product ID"))
                        .collect(),
                    category: ModuleCategory::from_key(fields[2]).expect("embedded category"),
                    id: fields[3].parse().expect("embedded model ID"),
                    bypass_mode: fields[4].parse().expect("embedded bypass mode"),
                    name: fields[5].to_owned(),
                    expression_pedal_index: fields[6].parse().expect("embedded expression index"),
                    tap_parameter_index: fields[7].parse().expect("embedded tap index"),
                    parameters: Vec::new(),
                });
            } else {
                result
                    .last_mut()
                    .expect("parameter follows model")
                    .parameters
                    .push(Parameter {
                        index: fields[1].parse().expect("embedded parameter ID"),
                        control_index: fields[2].parse().expect("embedded control ID"),
                        type_id: fields[3].parse().expect("embedded parameter type"),
                        group: fields[4].parse().expect("embedded group"),
                        default_raw: fields[5].parse().expect("embedded default"),
                        name: fields[6].to_owned(),
                    });
            }
        }
        result
    })
}
