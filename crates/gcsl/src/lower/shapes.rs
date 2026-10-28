//! The shapes of values that typed comparisons and lazy values walk.

use std::collections::HashMap;

use crate::diagnostic::InternalError;
use crate::ir::{LazyField, LazyRest, LazyShape, LazyShapeId, ListShape, RestShape, Shape, ShapeId};
use crate::names::Name;
use crate::source::Span;
use crate::types::{Laziness, TOWARD_DEPTH, TyId};

use super::Lowerer;

impl Lowerer<'_> {
    /// The shape of the record or list type `ty`, made, with the shapes of
    /// the record and list types nested in it, if this is the first
    /// comparison of it. Each shape is given its id when it is first met
    /// and its fields, or its element's shape, when it is taken off the work
    /// list, so that nesting does not recurse. A field or an element that
    /// may be a future is compared as its value once awaited, which the
    /// deep await of the operands has done, so the shape follows its type
    /// stripped.
    pub(super) fn shape(&mut self, ty: TyId, span: Span) -> Result<ShapeId, InternalError> {
        if let Some(&id) = self.shapes.get(&ty) {
            return Ok(id);
        }
        let table = self.table;
        let id = self.new_shape(ty);
        while let Some(ty) = self.shape_pending.pop() {
            if let Some(element) = table.list_element(ty) {
                let element = self.nested_shape(table.strip(element));
                if let Some(&shape) = self.shapes.get(&ty) {
                    self.builder.set_list_shape(shape, ListShape { element });
                }
                continue;
            }
            let Some(fields) = table.record_fields(ty) else {
                self.shape_pending.clear();
                return Err(InternalError {
                    message: "a record or list comparison of a type that is neither",
                    span,
                });
            };
            let mut shaped = Vec::with_capacity(fields.len());
            for &(name, field_ty) in fields {
                let nested = self.nested_shape(table.strip(field_ty));
                shaped.push((name, nested));
            }
            let rest = table.record_rest(ty).map(|rest| RestShape {
                shape: self.nested_shape(table.strip(rest)),
            });
            let Some(&shape) = self.shapes.get(&ty) else {
                continue;
            };
            self.builder.set_shape(
                shape,
                Shape {
                    fields: shaped.into_boxed_slice(),
                    rest,
                },
            );
        }
        Ok(id)
    }

    /// The shape a record's field or a list's element of type `ty`, stripped,
    /// is compared through: its own if it is a record or a list type, made
    /// and queued if it has none yet, and none otherwise.
    pub(super) fn nested_shape(&mut self, ty: TyId) -> Option<ShapeId> {
        if !self.is_container(ty) {
            return None;
        }
        Some(match self.shapes.get(&ty) {
            Some(&nested) => nested,
            None => self.new_shape(ty),
        })
    }

    /// Whether `ty` is a record or a list type, which can hold values of
    /// other types.
    pub(super) fn is_container(&self, ty: TyId) -> bool {
        self.table.is_record(ty) || self.table.list_element(ty).is_some()
    }

    /// Gives the record or list type `ty` a shape with no fields yet, and
    /// queues it to be given its fields or its element's shape.
    pub(super) fn new_shape(&mut self, ty: TyId) -> ShapeId {
        let id = self.builder.push_shape(Shape {
            fields: Box::default(),
            rest: None,
        });
        self.shapes.insert(ty, id);
        self.shape_pending.push(ty);
        id
    }

    /// The lazy shape of `ty`, a type present at its top: for a record type,
    /// each field whose type is a future or an either, with the lazy shape
    /// of its type stripped, and each present record or list field with
    /// lazy values somewhere inside, with its own, and its rest-field, if it
    /// has one that is either, for every field it does not name; for a list type, what is
    /// done with each element, in the same way, if its type is lazy or has
    /// lazy values inside; for a union, the lazy shapes of its members that
    /// have one (see [`Ir::lazy_union`](super::Ir::lazy_union)); [`LazyShapeId::NONE`] if it has no
    /// such field, element or member, or is none of those. Made once per
    /// type, and the shapes of the record, list and union types inside it
    /// first, through a work list rather than recursion, since record and
    /// list types can nest as deeply as a program is long.
    pub(super) fn lazy_shape(&mut self, ty: TyId) -> LazyShapeId {
        let table = self.table;
        if let Some(&id) = self.lazy_shapes.get(&ty) {
            return id;
        }
        self.lazy_pending.push((ty, false));
        while let Some((container, queued)) = self.lazy_pending.pop() {
            if self.lazy_shapes.contains_key(&container) {
                continue;
            }
            let element = table.list_element(container);
            let fields = table.record_fields(container).unwrap_or_default();
            let rest = table.record_rest(container);
            let members = table.union_members(container).unwrap_or_default();
            if !queued {
                // The record, list and union types inside it first, then it
                // again.
                self.lazy_pending.push((container, true));
                let inside = fields
                    .iter()
                    .map(|&(_, field_ty)| field_ty)
                    .chain(rest)
                    .chain(element)
                    .chain(members.iter().copied());
                for inner in inside {
                    let inner = table.strip(inner);
                    if (self.is_container(inner) || table.union_members(inner).is_some())
                        && !self.lazy_shapes.contains_key(&inner)
                    {
                        self.lazy_pending.push((inner, false));
                    }
                }
                continue;
            }
            if !members.is_empty() {
                let shapes: Box<[LazyShapeId]> = members
                    .iter()
                    .filter_map(|member| self.lazy_shapes.get(member).copied())
                    .filter(|&shape| shape != LazyShapeId::NONE)
                    .collect();
                let id = if shapes.is_empty() {
                    LazyShapeId::NONE
                } else {
                    self.builder.push_lazy_union(shapes)
                };
                self.lazy_shapes.insert(container, id);
                continue;
            }
            // What a deep await does with a value of type `ty` inside it.
            let action = |lazy_shapes: &HashMap<TyId, LazyShapeId>, ty: TyId| {
                let inner = lazy_shapes
                    .get(&table.strip(ty))
                    .copied()
                    .unwrap_or(LazyShapeId::NONE);
                if table.laziness(ty) != Laziness::Present {
                    Some(LazyField::Await(inner))
                } else if inner != LazyShapeId::NONE {
                    Some(LazyField::Within(inner))
                } else {
                    None
                }
            };
            let id = if let Some(element) = element {
                match action(&self.lazy_shapes, element) {
                    Some(each) => self.builder.push_lazy_list(each),
                    None => LazyShapeId::NONE,
                }
            } else {
                let lazy: Vec<(Name, LazyField)> = fields
                    .iter()
                    .filter_map(|&(name, field_ty)| {
                        action(&self.lazy_shapes, field_ty).map(|field| (name, field))
                    })
                    .collect();
                // The fields a rest-field covers, which a type does not name.
                let rest = rest
                    .and_then(|rest| action(&self.lazy_shapes, rest))
                    .map(|field| LazyRest {
                        named: fields.iter().map(|&(name, _)| name).collect(),
                        field,
                    });
                if lazy.is_empty() && rest.is_none() {
                    LazyShapeId::NONE
                } else {
                    self.builder.push_lazy_shape(LazyShape {
                        fields: lazy.into_boxed_slice(),
                        rest,
                    })
                }
            };
            self.lazy_shapes.insert(container, id);
        }
        self.lazy_shapes
            .get(&ty)
            .copied()
            .unwrap_or(LazyShapeId::NONE)
    }

    /// The lazy shape of an await of a value of type `found`, present at
    /// its top, toward an expected type, which leaves it of type `awaited`
    /// (see [`TypeTable::present_toward`](crate::types::TypeTable::present_toward)):
    /// as [`Lowerer::lazy_shape`]'s, but that a field or an element whose
    /// type `awaited` leaves as `found` has it is left as it is, a future
    /// among them. Where the two do not line up, as through a union or a
    /// rest-field, it is [`Lowerer::lazy_shape`]'s; and so it is below
    /// [`TOWARD_DEPTH`] levels of nesting, which `awaited` never leaves lazy
    /// below.
    pub(super) fn lazy_shape_toward(&mut self, found: TyId, awaited: TyId) -> LazyShapeId {
        self.shape_toward(found, self.table.strip(awaited), TOWARD_DEPTH)
    }

    /// [`Lowerer::lazy_shape_toward`] at `depth` more levels of nesting.
    fn shape_toward(&mut self, found: TyId, awaited: TyId, depth: u32) -> LazyShapeId {
        let table = self.table;
        if found == awaited {
            return LazyShapeId::NONE;
        }
        if let Some(&id) = self.lazy_toward.get(&(found, awaited)) {
            return id;
        }
        let Some(depth) = depth.checked_sub(1) else {
            return self.lazy_shape(found);
        };
        // What the await does with a field or an element of type `ty`,
        // which it leaves of type `left`.
        let action = |lowerer: &mut Self, ty: TyId, left: TyId| {
            if ty == left {
                return None;
            }
            let inner = lowerer.shape_toward(table.strip(ty), table.strip(left), depth);
            if table.laziness(ty) != Laziness::Present {
                Some(LazyField::Await(inner))
            } else if inner != LazyShapeId::NONE {
                Some(LazyField::Within(inner))
            } else {
                None
            }
        };
        let id = if let (Some(element), Some(left)) =
            (table.list_element(found), table.list_element(awaited))
        {
            match action(self, element, left) {
                Some(each) => self.builder.push_lazy_list(each),
                None => LazyShapeId::NONE,
            }
        } else if let (Some(fields), Some(left), None) = (
            table.record_fields(found),
            table.record_fields(awaited),
            table.record_rest(found),
        ) {
            let mut lazy: Vec<(Name, LazyField)> = Vec::new();
            for &(name, ty) in fields {
                let Some(&(_, left)) = left.iter().find(|&&(other, _)| other == name) else {
                    return self.lazy_shape(found);
                };
                if let Some(field) = action(self, ty, left) {
                    lazy.push((name, field));
                }
            }
            if lazy.is_empty() {
                LazyShapeId::NONE
            } else {
                self.builder.push_lazy_shape(LazyShape {
                    fields: lazy.into_boxed_slice(),
                    rest: None,
                })
            }
        } else {
            self.lazy_shape(found)
        };
        self.lazy_toward.insert((found, awaited), id);
        id
    }
}
