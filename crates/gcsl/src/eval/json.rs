//! Evaluating `json.decode`: parsing the document and making the value its
//! plan says (see [JSON decoding](crate::check::json#json-decoding) and
//! [Decoding plans](crate::types::decode#decoding-plans)).
//!
//! The text is parsed whole first ([`parse`]), then the tree is walked by
//! the plan, recursing once per level of the plan, which is no deeper than
//! the type. A failure of either is a runtime error at the call: "the text
//! is not JSON", with the reason and where, or "the document does not fit
//! the type", with the path to the value that does not, `units[3].kind`,
//! and what it is.
//!
//! A record decoded by a plan has its fields in the plan's order, sorted
//! by name, and every record one plan makes shares one layout.

mod parse;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use crate::check::Constant;
use crate::diagnostic::Diagnostic;
use crate::check::Builtin;
use crate::ir::{DecodeId, Ir, IrKind, IrNode};
use crate::names::Name;
use crate::types::{Decode, DecodeField, DecodeRecord};
use crate::value::{Layout, List, Record, Value};

use super::kinds::{mixed, of_kind};
use super::{Evaluator, Flow, MIXED, mistyped};
use parse::{Json, Number};

impl Evaluator<'_> {
    /// Evaluates the `JsonDecode` `node` as a value, which must be of the
    /// kind the node states, or, for
    /// [`ValueKind::Mixed`](crate::value::ValueKind::Mixed), of any kind.
    pub(super) fn decode_node(&mut self, node: &IrNode) -> Flow<Value> {
        let value = self.decoded(node)?;
        match node.kind {
            IrKind::JsonDecode { kind, .. } if of_kind(&value, kind) => Ok(value),
            _ => mistyped(
                node,
                "a decode produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `JsonDecode` `node` as a value that `extract` accepts:
    /// one of the kind the node states.
    pub(super) fn decode_as<T>(
        &mut self,
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.decoded(node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "a decode produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `JsonDecode` `node`: its text, parsed and decoded by
    /// its plan.
    fn decoded(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::JsonDecode { text, decode, .. } = node.kind else {
            return mistyped(node, "a node that is not a decode decoded");
        };
        let text = self.string(text)?;
        self.profiled_builtin(Builtin::JsonDecode, |this| {
            this.decode_text(&text, decode, node)
        })
    }

    /// Parses `text` and decodes it by the plan `decode`, for the
    /// `JsonDecode` `node`.
    pub(super) fn decode_text(&self, text: &str, decode: DecodeId, node: &IrNode) -> Flow<Value> {
        let ir = self.ir;
        let Some(plan) = ir.decode(decode) else {
            return mistyped(node, "a decode of a plan its IR lacks");
        };
        let json = parse::parse(text).map_err(|reason| {
            Diagnostic::error(format!("the text is not JSON: {reason}"), node.span)
        })?;
        let mut decoder = Decoder {
            ir,
            names: self.shared.names,
            layouts: HashMap::new(),
            path: String::new(),
        };
        decoder.decode(plan, &json).map_err(|reason| {
            let at = if decoder.path.is_empty() {
                "the document".to_owned()
            } else {
                format!("`{}`", decoder.path)
            };
            Diagnostic::error("the document does not fit the type", node.span)
                .with_label(format!("{at} {reason}"))
                .into()
        })
    }
}

/// A walk of a parsed document by a plan.
struct Decoder<'a> {
    /// The IR, for the texts of the field names of defaults.
    ir: &'a Ir,
    /// The names of the run's owner, which a rest-field's fields are named
    /// in; `None` in a run with none.
    names: Option<&'a std::cell::RefCell<crate::names::Names>>,
    /// The layout of the records each record plan makes, by the address of
    /// its fields.
    layouts: HashMap<*const DecodeField, Rc<Layout>>,
    /// The path to the value being decoded, `units[3].kind`; at a failure,
    /// the path to the value that failed.
    path: String,
}

impl Decoder<'_> {
    /// `json` decoded by `plan`, or what is wrong with it, as a clause:
    /// "is a string, not an array".
    ///
    /// Recurses once per level of the plan.
    fn decode(&mut self, plan: &Decode, json: &Json) -> Result<Value, String> {
        match (plan, json) {
            (Decode::Str, Json::Str(text)) => Ok(Value::Str(Rc::from(text.as_str()))),
            (Decode::Bool, Json::Bool(value)) => Ok(Value::Bool(*value)),
            (Decode::Int, Json::Number(number)) => number
                .int
                .map(Value::Int)
                .ok_or_else(|| format!("is the number {}, not an integer", number.text)),
            (Decode::Float, Json::Number(number)) => number
                .float
                .map(Value::Float)
                .ok_or_else(|| format!("is the number {}, too large for a Float", number.text)),
            (Decode::List(element), Json::Array(elements)) => {
                let length = self.path.len();
                let mut values = Vec::with_capacity(elements.len());
                for (index, each) in elements.iter().enumerate() {
                    let _ = write!(self.path, "[{index}]");
                    values.push(self.decode(element, each)?);
                    self.path.truncate(length);
                }
                Ok(Value::List(List::from_vec(values)))
            }
            (Decode::Record(record), Json::Object(members)) => self.record(record, members),
            (Decode::Any(plans), _) => {
                let chosen = plans.iter().find(|each| takes(each, json));
                match chosen {
                    Some(each) => self.decode(each, json),
                    None => Err(format!("is {}, not {}", described(json), expected(plan))),
                }
            }
            _ => Err(format!("is {}, not {}", described(json), expected(plan))),
        }
    }

    /// The record `record` says, decoded from the object of `members`.
    fn record(
        &mut self,
        record: &DecodeRecord,
        members: &[(String, Json)],
    ) -> Result<Value, String> {
        let fields = &record.fields;
        let length = self.path.len();
        let mut values = Vec::with_capacity(fields.len());
        for field in fields {
            let given = members
                .iter()
                .find(|(key, _)| **key == *field.text)
                .map(|(_, value)| value)
                .filter(|value| **value != Json::Null);
            let value = match (given, &field.default) {
                (Some(value), _) => {
                    if !self.path.is_empty() {
                        self.path.push('.');
                    }
                    self.path.push_str(&field.text);
                    let value = self.decode(&field.plan, value)?;
                    self.path.truncate(length);
                    value
                }
                (None, Some(default)) => constant(default, self.ir)
                    .ok_or_else(|| "has a default whose field names are unknown".to_owned())?,
                (None, None) => {
                    return Err(format!("has no key {:?}, nor a default for it", field.text));
                }
            };
            values.push(value);
        }
        let mut names: Vec<Name> = Vec::new();
        let mut texts: Vec<Box<str>> = Vec::new();
        if let Some(rest) = &record.rest {
            for (key, value) in members {
                if *value == Json::Null || fields.iter().any(|field| *field.text == **key) {
                    continue;
                }
                if !self.path.is_empty() {
                    self.path.push('.');
                }
                self.path.push_str(key);
                values.push(self.decode(rest, value)?);
                self.path.truncate(length);
                names.push(self.name_of(key)?);
                texts.push(Box::from(key.as_str()));
            }
        }
        if values.is_empty() {
            return Ok(Value::Empty);
        }
        if !names.is_empty() {
            let layout = Layout {
                names: fields.iter().map(|field| field.name).chain(names).collect(),
                texts: fields
                    .iter()
                    .map(|field| field.text.clone())
                    .chain(texts)
                    .collect(),
            };
            return Ok(Value::Record(Rc::new(Record {
                layout: Rc::new(layout),
                values: values.into_boxed_slice(),
            })));
        }
        let layout = self
            .layouts
            .entry(fields.as_ptr())
            .or_insert_with(|| {
                Rc::new(Layout {
                    names: fields.iter().map(|field| field.name).collect(),
                    texts: fields.iter().map(|field| field.text.clone()).collect(),
                })
            })
            .clone();
        Ok(Value::Record(Rc::new(Record {
            layout,
            values: values.into_boxed_slice(),
        })))
    }
}

impl Decoder<'_> {
    /// The name of the key `key` in the names of the run's owner, interned
    /// there if it is new.
    fn name_of(&self, key: &str) -> Result<Name, String> {
        let names = self
            .names
            .ok_or("is a map, which a run with no names to name its keys cannot decode")?;
        let mut names = names
            .try_borrow_mut()
            .map_err(|_| "is a map, whose keys' names are already being made".to_owned())?;
        Ok(names.intern(key))
    }
}

/// Whether `plan`, a member of a union's, takes `json`: is of its kind,
/// a number of an `Int`'s only if it is an integer it holds.
fn takes(plan: &Decode, json: &Json) -> bool {
    match (plan, json) {
        (Decode::Int, Json::Number(Number { int, .. })) => int.is_some(),
        (Decode::Float, Json::Number(_))
        | (Decode::Str, Json::Str(_))
        | (Decode::Bool, Json::Bool(_))
        | (Decode::List(_), Json::Array(_))
        | (Decode::Record(_), Json::Object(_)) => true,
        (Decode::Any(plans), _) => plans.iter().any(|each| takes(each, json)),
        _ => false,
    }
}

/// What a plan takes, with its article: "an array".
fn expected(plan: &Decode) -> String {
    match plan {
        Decode::Str => "a string".into(),
        Decode::Int => "an integer".into(),
        Decode::Float => "a number".into(),
        Decode::Bool => "`true` or `false`".into(),
        Decode::List(_) => "an array".into(),
        Decode::Record(_) => "an object".into(),
        Decode::Any(plans) => {
            let each: Vec<String> = plans.iter().map(expected).collect();
            each.join(" or ")
        }
    }
}

/// What a JSON value is, with its article: "a string", "`null`".
fn described(json: &Json) -> String {
    match json {
        Json::Null => "`null`".into(),
        Json::Bool(value) => format!("`{value}`"),
        Json::Number(number) => format!("the number {}", number.text),
        Json::Str(_) => "a string".into(),
        Json::Array(_) => "an array".into(),
        Json::Object(_) => "an object".into(),
    }
}

/// The value of the constant `constant`, a field's default; `None` if it
/// is a record with a field whose name has no text in `ir`, which lowering
/// records for every default of a plan.
///
/// Recurses once per level of list and record nesting.
fn constant(constant: &Constant, ir: &Ir) -> Option<Value> {
    Some(match constant {
        Constant::Int(value) => Value::Int(*value),
        Constant::Float(bits) => Value::Float(f64::from_bits(*bits)),
        &Constant::Quantity(quantity, amount) => Value::Quantity(quantity, amount),
        Constant::Bool(value) => Value::Bool(*value),
        Constant::Str(text) => Value::Str(Rc::from(&**text)),
        Constant::Empty => Value::Empty,
        Constant::List(elements) => Value::List(List::from_vec(
            elements
                .iter()
                .map(|each| self::constant(each, ir))
                .collect::<Option<Vec<_>>>()?,
        )),
        Constant::Record(fields) => {
            let mut names = Vec::with_capacity(fields.len());
            let mut texts = Vec::with_capacity(fields.len());
            let mut values = Vec::with_capacity(fields.len());
            for (name, value) in fields {
                names.push(*name);
                texts.push(Box::from(ir.name_text(*name)?));
                values.push(self::constant(value, ir)?);
            }
            Value::Record(Rc::new(Record {
                layout: Rc::new(Layout {
                    names: names.into_boxed_slice(),
                    texts: texts.into_boxed_slice(),
                }),
                values: values.into_boxed_slice(),
            }))
        }
    })
}

#[cfg(test)]
mod tests;
