//! Decoding plans: how `json.decode` makes a value of a type from a JSON
//! document (see [JSON decoding](crate::check::json#json-decoding)).
//!
//! # Decoding plans
//!
//! The checker plans a decode from the type expected of the call, once, as
//! it plans a type test for `is` and `as`, and the evaluator follows the plan
//! (see [`crate::eval::json`]). A plan is no deeper than the type it was made
//! from, and types are never recursive, so neither is a plan.
//!
//! A JSON value of each kind decodes as one kind of GCSL value:
//!
//! - a string as a `String`;
//! - a number as an `Int` if it is an integer in range, written with no
//!   fraction or exponent, and as a `Float` whatever it is;
//! - `true` and `false` as a `Bool`;
//! - an array as a list;
//! - an object as a record: each of the type's named fields from the key of
//!   its name, a key that is missing or `null` giving the field's default,
//!   and every other key as a field of its own name, by the rest-field's
//!   plan, if the type has one, a `null` left out, or left out if it has
//!   none. A rest-field's fields are named as the document names them, as
//!   `record` names its fields (see [Records of any
//!   names](crate::check::entries#records-of-any-names)).
//!
//! A union decodes as whichever of its members takes the JSON value's kind,
//! so no two members may take the same kind, except that an `Int` and a
//! `Float` share numbers, an integer going to the `Int`. Nothing else
//! decodes: not a `Duration` or `Size`, whose JSON form is no standard, a
//! file or directory, which a document can only name, an enum, a secret, a
//! function or `Never`.

use crate::check::Constant;
use crate::names::{Name, Names};

use super::{TyId, TyKind, TypeTable};

/// How a JSON value is decoded as a value of a type (see [Decoding
/// plans](#decoding-plans)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decode {
    /// A JSON string, as a `String`.
    Str,
    /// A JSON integer, as an `Int`.
    Int,
    /// A JSON number, as a `Float`.
    Float,
    /// `true` or `false`, as a `Bool`.
    Bool,
    /// A JSON array, as a list each of whose elements decodes by the plan.
    List(Box<Decode>),
    /// A JSON object, as a record.
    Record(Box<DecodeRecord>),
    /// A JSON value of any of the kinds the plans take, each plan taking
    /// kinds no other does, but an `Int` and a `Float`, which share
    /// numbers; at least two.
    Any(Box<[Decode]>),
}

/// How a JSON object is decoded as a record: see [`Decode::Record`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeRecord {
    /// Each named field of the record type, sorted by name.
    pub fields: Box<[DecodeField]>,
    /// How each key the fields do not name is decoded, as a field of its
    /// name; `None` to leave them out.
    pub rest: Option<Decode>,
}

/// A named field of a record decoded from a JSON object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeField {
    /// The field's name.
    pub name: Name,
    /// Its text: the key it is decoded from.
    pub text: Box<str>,
    /// How the key's value is decoded.
    pub plan: Decode,
    /// What the field is if the key is missing or `null`; `None` if it
    /// must be there.
    pub default: Option<Constant>,
}

/// Why a type has no decoding plan: the part of it, `ty`, that JSON cannot
/// be decoded as, and what is wrong with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Undecodable {
    /// The part of the type with no plan: the type itself, or a field's,
    /// an element's or a member's.
    pub ty: TyId,
    /// Why, as a clause: "a `Duration` has no JSON form".
    pub why: &'static str,
}

/// The kinds of JSON value a plan takes, for telling a union's members
/// apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonKind {
    String,
    Number,
    Bool,
    Array,
    Object,
}

impl Decode {
    /// The kinds of JSON value it takes.
    fn kinds(&self) -> Vec<JsonKind> {
        match self {
            Decode::Str => vec![JsonKind::String],
            Decode::Int | Decode::Float => vec![JsonKind::Number],
            Decode::Bool => vec![JsonKind::Bool],
            Decode::List(_) => vec![JsonKind::Array],
            Decode::Record(_) => vec![JsonKind::Object],
            Decode::Any(plans) => plans.iter().flat_map(Decode::kinds).collect(),
        }
    }
}

impl TypeTable {
    /// The plan for decoding a JSON value as a value of type `ty`, made
    /// present first (see [`TypeTable::present`]) (see [Decoding
    /// plans](#decoding-plans)).
    ///
    /// # Errors
    ///
    /// The part of the type that has no plan, and why, if one has none.
    ///
    /// Recurses once per level of record, list or union nesting.
    pub fn decode_plan(&mut self, ty: TyId, names: &Names) -> Result<Decode, Undecodable> {
        let ty = self.present(ty);
        let undecodable = |why| Err(Undecodable { ty, why });
        match self.kind(ty).clone() {
            TyKind::String => Ok(Decode::Str),
            TyKind::Int => Ok(Decode::Int),
            TyKind::Float => Ok(Decode::Float),
            TyKind::Bool => Ok(Decode::Bool),
            TyKind::List(element) => Ok(Decode::List(Box::new(self.decode_plan(element, names)?))),
            TyKind::Record(record) => {
                let mut fields = Vec::with_capacity(record.fields.len());
                for &(name, field) in &record.fields {
                    fields.push(DecodeField {
                        name,
                        text: Box::from(names.get(name)),
                        plan: self.decode_plan(field, names)?,
                        default: record
                            .defaults()
                            .iter()
                            .find(|&&(defaulted, _)| defaulted == name)
                            .map(|(_, constant)| constant.clone()),
                    });
                }
                let rest = match record.rest() {
                    Some(rest) => Some(self.decode_plan(rest, names)?),
                    None => None,
                };
                Ok(Decode::Record(Box::new(DecodeRecord {
                    fields: fields.into_boxed_slice(),
                    rest,
                })))
            }
            TyKind::Union(members) => self.decode_union(ty, &members, names),
            TyKind::Quantity(_) => undecodable("a `Duration` or `Size` has no JSON form"),
            TyKind::File | TyKind::Dir | TyKind::Sandbox => {
                undecodable("a document can name a file, but not hold one")
            }
            TyKind::Enum { .. } | TyKind::Variant { .. } => {
                undecodable("an enum's values have no JSON form")
            }
            TyKind::Secret(..) => undecodable("a secret cannot come from a document"),
            TyKind::Fn(_) | TyKind::Builtin(_) => undecodable("a function has no JSON form"),
            TyKind::Never => undecodable("no value is a `Never`"),
            TyKind::Partial(_) => undecodable("a `Partial` record has no JSON form"),
            TyKind::Future(_)
            | TyKind::Either(_)
            | TyKind::Provisional { .. }
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Param { .. } => undecodable("its type is not known"),
        }
    }

    /// The plan for decoding the union `ty` of `members` (see
    /// [`TypeTable::decode_plan`]): each member's, none taking a kind of JSON
    /// value another does, but an `Int` and a `Float`, the `Int` first, so
    /// that an integer goes to it.
    fn decode_union(
        &mut self,
        ty: TyId,
        members: &[TyId],
        names: &Names,
    ) -> Result<Decode, Undecodable> {
        let mut plans = Vec::with_capacity(members.len());
        for &member in members {
            let plan = self.decode_plan(member, names)?;
            let shares = |other: &Decode| {
                let numbers = matches!(
                    (other, &plan),
                    (Decode::Int, Decode::Float) | (Decode::Float, Decode::Int)
                );
                !numbers && other.kinds().iter().any(|kind| plan.kinds().contains(kind))
            };
            if plans.iter().any(shares) {
                return Err(Undecodable {
                    ty,
                    why: "two of its members are the same kind of JSON value, so a document \
                          cannot say which it is",
                });
            }
            plans.push(plan);
        }
        plans.sort_by_key(|plan| !matches!(plan, Decode::Int));
        Ok(Decode::Any(plans.into_boxed_slice()))
    }
}
