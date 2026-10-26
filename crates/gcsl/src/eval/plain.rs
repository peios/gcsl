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
//! directories by path and identity, and sandboxes, which a step's body
//! passes between its effects. A future that has run is its value. What is
//! no data, a function, a built-in, a future that has not run or a step's
//! result not given yet, has no plain form: "a step's result must be plain
//! data: it holds a function", naming what it holds. A plain
//! value made a value again ([`from_plain`]) interns its names in the
//! workspace's table, so that it is the value it was made of: equal to it,
//! by `===`.
//!
//! # Keys of functions
//!
//! A host keys a step by what it reads, and what a step's function captures
//! and the context it runs in can hold functions: the toolchains a workspace
//! registers are its definitions. So for a key alone ([`to_key_plain`]), a
//! function is [`Plain::Closure`]: the input of its code's file, its number
//! among that code's functions, and what it captures, keyed so too, which a
//! host keys by that file's name and contents. A built-in, or a future that
//! has not run, is still no data. A closure's key is never a step's argument
//! or result, and is never made a value again.

use std::rc::Rc;

use crate::names::Names;
use crate::value::{File, Layout, List, Quantity, Record, Sandbox, Secret, Tag, Value, Variant};

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
    /// A sandbox, by its copy's path, its commands' working directory and
    /// their environment: what a step's body passes between its effects.
    Sandbox {
        /// Its copy, its `/`.
        root: String,
        /// Its commands' working directory, inside it.
        cwd: String,
        /// Its commands' environment.
        env: Vec<(String, String)>,
    },
    /// A function, by its code, for a host's key alone (see [Keys of
    /// functions](#keys-of-functions)): never a step's argument or result.
    Closure {
        /// The input of the file whose code it is.
        input: u32,
        /// Its number among that code's functions.
        function: u32,
        /// What it captures, as keys.
        captures: Vec<Plain>,
    },
    /// A step's result, by the number of its node in its graph, for a host's
    /// identity of a step that is given it alone (see
    /// [`to_identity_plain`]): never a step's argument or result, nor a key.
    Step(u32),
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
                Plain::Closure { captures, .. } => pending.extend(captures.iter().rev()),
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
    plain_of(value, names, Mode::Data)
}

/// `value` as plain data for a host's key (see [Keys of
/// functions](#keys-of-functions)): as [`to_plain`] makes it, but each
/// function by its code and what it captures.
///
/// # Errors
///
/// What it holds that is no data and no function, a future that has not run
/// say, as [`to_plain`] says.
pub fn to_key_plain(value: &Value, names: &Names) -> Result<Plain, String> {
    plain_of(value, names, Mode::Key)
}

/// `value` as plain data for a host's identity of a step: as
/// [`to_key_plain`] makes it, but each step's result it holds as
/// [`Plain::Step`], by its node, whether it has been given or not, so that a
/// host can name a step before the steps it waits for have run, and names
/// it the same however far they have got.
///
/// # Errors
///
/// What it holds that is no data, no function and no step's result, as
/// [`to_key_plain`] says.
pub fn to_identity_plain(value: &Value, names: &Names) -> Result<Plain, String> {
    plain_of(value, names, Mode::Identity)
}

/// What a value is made plain for: see [`to_plain`], [`to_key_plain`] and
/// [`to_identity_plain`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A step's argument or result: data alone.
    Data,
    /// A key: data, and functions by their code.
    Key,
    /// A step's identity: a key's, and steps' results not given yet by their
    /// nodes.
    Identity,
}

/// `value` as plain data, as `mode` says (see [`to_plain`], [`to_key_plain`]
/// and [`to_identity_plain`]).
fn plain_of(value: &Value, names: &Names, mode: Mode) -> Result<Plain, String> {
    let closures = mode != Mode::Data;
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
                fields.push((name.to_string(), plain_of(value, names, mode)?));
            }
            Plain::Record(fields)
        }
        Value::List(list) => Plain::List(
            list.values()
                .iter()
                .map(|value| plain_of(value, names, mode))
                .collect::<Result<_, _>>()?,
        ),
        Value::Tag(tag) => plain_variant(tag, None, names),
        Value::Variant(variant) => {
            let payload = plain_of(&variant.payload, names, mode)?;
            plain_variant(&variant.tag, Some(payload), names)
        }
        Value::Secret(secret) => Plain::Secret {
            identity: secret.identity.to_string(),
            revealed: secret.revealed,
            value: Box::new(plain_of(&secret.value, names, mode)?),
        },
        Value::File(file) => Plain::File {
            path: file.path.to_string(),
            identity: file.identity.as_deref().map(str::to_owned),
        },
        Value::Dir(dir) => Plain::Dir {
            path: dir.path.to_string(),
            identity: dir.identity.as_deref().map(str::to_owned),
        },
        // A step's result is its step's for an identity, given or not, so
        // that what names a step never depends on how far the build got.
        Value::Future(future) => match (future.step_node(), future.value()) {
            (Some(node), _) if mode == Mode::Identity => Plain::Step(node),
            (_, Some(done)) => plain_of(&done, names, mode)?,
            (Some(_), None) => return refused("a step's result not given yet"),
            (None, None) => return refused("a future that has not run"),
        },
        Value::Fn(closure) if closures => Plain::Closure {
            input: closure.code.input(),
            function: u32::try_from(closure.function.index()).unwrap_or(u32::MAX),
            captures: closure
                .captures
                .iter()
                .map(|value| plain_of(value, names, mode))
                .collect::<Result<_, _>>()?,
        },
        Value::Fn(_) | Value::Builtin(_) => return refused("a function"),
        Value::Sandbox(sandbox) => Plain::Sandbox {
            root: sandbox.root.to_string(),
            cwd: sandbox.cwd.to_string(),
            env: sandbox
                .env
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        },
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
/// (see [Plain values](#plain-values)): `{}` for a closure's key, which is
/// no value's plain form.
pub fn from_plain(plain: &Plain, names: &mut Names) -> Value {
    match plain {
        Plain::Int(n) => Value::Int(*n),
        Plain::Float(x) => Value::Float(*x),
        Plain::Bool(b) => Value::Bool(*b),
        Plain::Str(s) => Value::Str(Rc::from(s.as_str())),
        Plain::Quantity(quantity, amount) => Value::Quantity(*quantity, *amount),
        // A closure's is a key's alone, and a step's result not given yet
        // an identity's, never a step's argument or result, so never made a
        // value again.
        Plain::Empty | Plain::Closure { .. } | Plain::Step(_) => Value::Empty,
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
        Plain::Sandbox { root, cwd, env } => Value::Sandbox(Rc::new(Sandbox {
            root: Rc::from(root.as_str()),
            cwd: Rc::from(cwd.as_str()),
            env: env
                .iter()
                .map(|(name, value)| (Rc::from(name.as_str()), Rc::from(value.as_str())))
                .collect(),
        })),
    }
}

/// The file or directory at `path`, of the identity `identity`.
fn plain_file(path: &str, identity: Option<&str>) -> Rc<File> {
    Rc::new(File {
        path: Rc::from(path),
        identity: identity.map(Rc::from),
    })
}
