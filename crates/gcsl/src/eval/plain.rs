//! Plain values: the data a host keeps of a step, apart from GCSL.
//!
//! # Plain values
//!
//! A [`Plain`] is a value as plain data, which a host can hash, store, send
//! to another process and give back: no `Rc`, no interned name, nothing
//! of the evaluator's. It holds what a step can take and give: integers,
//! floats, booleans, strings, durations and sizes, the empty record,
//! records by field name, lists, variants and variants as values by their
//! keys' text, secrets with their identity and their value, and files and
//! directories by path and identity. A future that has run is its value.
//! What is no data, a function, a built-in, a sandbox, a future that has not
//! run or a step's result not given yet, has no plain form: "a step's result
//! must be plain data: it holds a function", naming what it holds. A plain
//! value made a value again ([`from_plain`]) interns its names in the
//! workspace's table, so that it is the value it was made of: equal to it,
//! by `===`.

use std::rc::Rc;

use crate::names::Names;
use crate::value::{File, Layout, List, Quantity, Record, Secret, Tag, Value, Variant};

/// A value as plain data (see [Plain values](#plain-values)).
#[derive(Clone, Debug, PartialEq)]
pub enum Plain {
    /// An `Int`.
    Int(i64),
    /// A `Float`.
    Float(f64),
    /// A `Bool`.
    Bool(bool),
    /// A `String`.
    Str(String),
    /// A `Duration` or a `Size`, its amount of its smallest unit.
    Quantity(Quantity, i64),
    /// The empty record, `{}`.
    Empty,
    /// A record: each field's name and value, in its layout's order.
    Record(Vec<(String, Plain)>),
    /// A list.
    List(Vec<Plain>),
    /// A variant as a value, or with a payload.
    Variant {
        /// Its key's text, its identity.
        key: String,
        /// Its enum's key's text.
        of: String,
        /// How it is shown, `Toolchain::C`.
        shown: String,
        /// Its payload; `None` for the variant as a value.
        payload: Option<Box<Plain>>,
    },
    /// A secret: its identity, whether it is revealed, and its value.
    Secret {
        /// Its public identity.
        identity: String,
        /// Whether it is `Revealed<T>`.
        revealed: bool,
        /// Its value, which a host must keep as it keeps any secret.
        value: Box<Plain>,
    },
    /// A file, by its path and identity.
    File {
        /// Its path.
        path: String,
        /// Its identity, a derived secret's, if it has one.
        identity: Option<String>,
    },
    /// A directory, by its path and identity.
    Dir {
        /// Its path.
        path: String,
        /// Its identity, if it has one.
        identity: Option<String>,
    },
}

impl Plain {
    /// Every file and directory the value holds, at any depth, each with
    /// whether it is a directory, in the order they are found: what a host
    /// stages for a step that is given the value.
    #[must_use]
    pub fn files(&self) -> Vec<(&str, bool)> {
        let mut found = Vec::new();
        let mut pending = vec![self];
        while let Some(plain) = pending.pop() {
            match plain {
                Plain::File { path, .. } => found.push((path.as_str(), false)),
                Plain::Dir { path, .. } => found.push((path.as_str(), true)),
                Plain::Record(fields) => {
                    pending.extend(fields.iter().rev().map(|(_, value)| value));
                }
                Plain::List(items) => pending.extend(items.iter().rev()),
                Plain::Variant {
                    payload: Some(payload),
                    ..
                } => pending.push(payload),
                Plain::Secret { value, .. } => pending.push(value),
                _ => {}
            }
        }
        found
    }
}

/// `value` as plain data, its interned names' text from `names` (see [Plain
/// values](#plain-values)).
///
/// # Errors
///
/// What it holds that is no data: "a step's result must be plain data: it
/// holds a function".
pub fn to_plain(value: &Value, names: &Names) -> Result<Plain, String> {
    let refused = |what: &str| {
        Err(format!(
            "a step's result must be plain data: it holds {what}"
        ))
    };
    Ok(match value {
        Value::Int(n) => Plain::Int(*n),
        Value::Float(x) => Plain::Float(*x),
        Value::Bool(b) => Plain::Bool(*b),
        Value::Str(s) => Plain::Str(s.to_string()),
        Value::Quantity(quantity, amount) => Plain::Quantity(*quantity, *amount),
        Value::Empty => Plain::Empty,
        Value::Record(record) => {
            let mut fields = Vec::with_capacity(record.values.len());
            for (name, value) in record.layout.texts.iter().zip(record.values.iter()) {
                fields.push((name.to_string(), to_plain(value, names)?));
            }
            Plain::Record(fields)
        }
        Value::List(list) => Plain::List(
            list.values()
                .iter()
                .map(|value| to_plain(value, names))
                .collect::<Result<_, _>>()?,
        ),
        Value::Tag(tag) => plain_variant(tag, None, names),
        Value::Variant(variant) => {
            let payload = to_plain(&variant.payload, names)?;
            plain_variant(&variant.tag, Some(payload), names)
        }
        Value::Secret(secret) => Plain::Secret {
            identity: secret.identity.to_string(),
            revealed: secret.revealed,
            value: Box::new(to_plain(&secret.value, names)?),
        },
        Value::File(file) => Plain::File {
            path: file.path.to_string(),
            identity: file.identity.as_deref().map(str::to_owned),
        },
        Value::Dir(dir) => Plain::Dir {
            path: dir.path.to_string(),
            identity: dir.identity.as_deref().map(str::to_owned),
        },
        Value::Future(future) => match future.value() {
            Some(done) => to_plain(&done, names)?,
            None if future.host_node().is_some() => {
                return refused("a step's result not given yet");
            }
            None => return refused("a future that has not run"),
        },
        Value::Fn(_) | Value::Builtin(_) => return refused("a function"),
        Value::Sandbox(_) => return refused("a sandbox"),
    })
}

/// The plain form of the variant `tag`, with the payload `payload`, if it
/// has one.
fn plain_variant(tag: &Tag, payload: Option<Plain>, names: &Names) -> Plain {
    Plain::Variant {
        key: names.get(tag.key).to_owned(),
        of: names.get(tag.of).to_owned(),
        shown: tag.shown.to_string(),
        payload: payload.map(Box::new),
    }
}

/// The value `plain` is the plain form of, its names interned in `names`
/// (see [Plain values](#plain-values)).
pub fn from_plain(plain: &Plain, names: &mut Names) -> Value {
    match plain {
        Plain::Int(n) => Value::Int(*n),
        Plain::Float(x) => Value::Float(*x),
        Plain::Bool(b) => Value::Bool(*b),
        Plain::Str(s) => Value::Str(Rc::from(s.as_str())),
        Plain::Quantity(quantity, amount) => Value::Quantity(*quantity, *amount),
        Plain::Empty => Value::Empty,
        Plain::Record(fields) if fields.is_empty() => Value::Empty,
        Plain::Record(fields) => {
            let layout = Layout {
                names: fields.iter().map(|(name, _)| names.intern(name)).collect(),
                texts: fields
                    .iter()
                    .map(|(name, _)| Box::from(name.as_str()))
                    .collect(),
            };
            let values = fields
                .iter()
                .map(|(_, value)| from_plain(value, names))
                .collect();
            Value::Record(Rc::new(Record {
                layout: Rc::new(layout),
                values,
            }))
        }
        Plain::List(items) => Value::List(List::from_vec(
            items.iter().map(|item| from_plain(item, names)).collect(),
        )),
        Plain::Variant {
            key,
            of,
            shown,
            payload,
        } => {
            let tag = Rc::new(Tag {
                key: names.intern(key),
                of: names.intern(of),
                shown: Rc::from(shown.as_str()),
            });
            match payload {
                None => Value::Tag(tag),
                Some(payload) => Value::Variant(Rc::new(Variant {
                    tag,
                    payload: from_plain(payload, names),
                })),
            }
        }
        Plain::Secret {
            identity,
            revealed,
            value,
        } => Value::Secret(Rc::new(Secret {
            value: from_plain(value, names),
            identity: Rc::from(identity.as_str()),
            revealed: *revealed,
        })),
        Plain::File { path, identity } => Value::File(plain_file(path, identity.as_deref())),
        Plain::Dir { path, identity } => Value::Dir(plain_file(path, identity.as_deref())),
    }
}

/// The file or directory at `path`, of the identity `identity`.
fn plain_file(path: &str, identity: Option<&str>) -> Rc<File> {
    Rc::new(File {
        path: Rc::from(path),
        identity: identity.map(Rc::from),
    })
}
