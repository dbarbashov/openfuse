//! Ordered cascading parameter changes. All values are XML encoded.
use super::ModuleCategory;
use std::sync::OnceLock;

#[derive(Debug)]
struct Setter {
    index: u8,
    value: u16,
    suppress: bool,
}
#[derive(Debug, PartialEq, Eq)]
enum Trigger {
    All,
    Default,
    Value(u16),
}
#[derive(Debug)]
struct Rule {
    product: u32,
    index: u8,
    setters: Vec<(Trigger, Setter)>,
}
fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let mut result: Vec<Rule> = Vec::new();
        for line in include_str!("../data/rules.tsv")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let f: Vec<_> = line.split('\t').collect();
            let id: usize = f[0].parse().expect("embedded rule index");
            if id == result.len() {
                result.push(Rule {
                    product: f[1].parse().expect("embedded rule product"),
                    index: f[3].parse().expect("embedded rule parameter"),
                    setters: Vec::new(),
                });
            }
            assert_eq!(f[2], "0", "all recovered rules are amplifier rules");
            let trigger = match f[4] {
                "all" => Trigger::All,
                "default" => Trigger::Default,
                s => Trigger::Value(s.parse().expect("embedded rule trigger")),
            };
            result[id].setters.push((
                trigger,
                Setter {
                    index: f[5].parse().expect("embedded rule target"),
                    value: f[6].parse().expect("embedded rule value"),
                    suppress: f[7] == "1",
                },
            ));
        }
        result
    })
}

struct Operation {
    product: u32,
    values: Vec<(u8, u16)>,
    suppressed: Vec<u8>,
}
impl Operation {
    fn apply(&mut self, index: u8, value: u16) {
        self.apply_with_force(index, value, false);
    }
    fn apply_with_force(&mut self, index: u8, value: u16, force: bool) {
        let Some((_, current)) = self.values.iter_mut().find(|(i, _)| *i == index) else {
            return;
        };
        if *current == value && !force {
            return;
        }
        *current = value;
        if self.suppressed.contains(&index) {
            return;
        }
        let product = self.product;
        for rule in rules()
            .iter()
            .filter(|rule| rule.product == product && rule.index == index)
        {
            self.run(rule, &Trigger::All);
            let trigger = if rule
                .setters
                .iter()
                .any(|(t, _)| t == &Trigger::Value(value))
            {
                Trigger::Value(value)
            } else {
                Trigger::Default
            };
            self.run(rule, &trigger);
        }
    }
    fn run(&mut self, rule: &Rule, trigger: &Trigger) {
        for (_, setter) in rule.setters.iter().filter(|(t, _)| t == trigger) {
            if setter.suppress && !self.suppressed.contains(&setter.index) {
                self.suppressed.push(setter.index);
            }
            self.apply(setter.index, setter.value);
        }
    }
}

/// Apply the recovered preset rules atomically to a copy of the supplied values.
/// Unknown parameter indices are preserved; absent parameters are not invented.
/// Returning the full list lets callers record one undo transaction. The model
/// identifier is reserved for future rules: all 31 shipped rules are family-wide.
pub fn cascade_parameters(
    product_id: u32,
    category: ModuleCategory,
    _model_id: u16,
    parameters: &[(u8, u16)],
    index: u8,
    value: u16,
) -> Vec<(u8, u16)> {
    let mut op = Operation {
        product: product_id,
        values: parameters.to_vec(),
        suppressed: Vec::new(),
    };
    if category == ModuleCategory::Amplifier {
        op.apply(index, value);
    } else if let Some((_, v)) = op.values.iter_mut().find(|(i, _)| *i == index) {
        *v = value;
    }
    op.values
}

/// Original ToAmpModule/ToFxModule load values in order with force=true and
/// one shared suppression list. This differs from a single editor operation.
pub(crate) fn load_wire_parameters(
    product: u32,
    category: ModuleCategory,
    defaults: &[(u8, u16)],
    incoming: &[(u8, u16)],
) -> Vec<(u8, u16)> {
    if category != ModuleCategory::Amplifier {
        return incoming.to_vec();
    }
    let mut op = Operation {
        product,
        values: defaults.to_vec(),
        suppressed: Vec::new(),
    };
    for &(index, value) in incoming {
        if !op.suppressed.contains(&index) {
            op.apply_with_force(index, value, true);
        }
    }
    op.values
}
