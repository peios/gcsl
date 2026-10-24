//! Type expressions as the tree holds them: their arena, the problems of
//! resolving them, and the store a type's fields are copied out of a tree into.
//!
//! # Types
//!
//! A `type` item, `type NAME = TYPE`
//! ([`ItemKind::Type`](super::ItemKind::Type)), declares a type, an alias of
//! `TYPE`, in the file's namespace of types, apart from its variables and
//! fields. Its name is read only in a type position, written bare,
//! `PackageDef`, or qualified through an import, `t.PackageDef`, a
//! [`TypeExprKind::Path`] in the path pool. A record type may spread another,
//! `{...PackageDef, name: String}`, and may have a rest-field, the type of
//! every field it does not name, `{to: String, *: Int}`: each an entry of its
//! fields with no name ([`FieldTy`], [`FieldTyKind`]). A spread's rest-field
//! merges as its fields do. A field item may be annotated, `x: Int = 1`, as a
//! `let` may: the annotation is in its
//! [`ItemKind::Field`](super::ItemKind::Field), and an [`Item`](super::Item) is
//! still 36 bytes.
//!
//! The type expressions of a tree are a [`TypeArena`]. Before a file is
//! checked, a workspace *expands* its types ([`Ast::expand_types`]): each
//! name of a declared type becomes what the type's definition is, as if the
//! definition were written there, its nodes copied into the tree's arena,
//! after the parsed ones, out of a [`TypeStore`], which holds every
//! declaration's definition apart from its file's tree, each *settled*: its
//! names of other declarations made what they define, its spreads merged.
//! And each record type with spreads becomes the record type of its merged
//! fields. A spread's fields are SOFT, as value spread's are: a later field
//! of one's name replaces it, and so does a later spread's; a spread's field
//! of the name of a field written before the spread is a problem, and so is
//! a spread of a type that is not a record type. What the expansion finds
//! wrong it keeps as the tree's problems ([`TypeProblem`],
//! [`Ast::type_problems`]), by node, for the checker to report where it
//! resolves the node; and what each node it changed was written as, for the
//! printer ([`Ast::written_type`]). So the checker sees the expanded types,
//! and the printer the written ones. The checker knows the copies apart
//! ([`Ast::is_definition_copy`]), since what is wrong in one is its own
//! file's to report. A copy is made once per tree and declaration, and a
//! definition used twice shares its nodes, so a tree's type expressions may
//! share children; the parser's are still allocated children first.
//!
//! A `type` item may be generic, `type Pair<A, B> = {first: A, second: B}`,
//! its type parameters recorded as a function item's are ([`Generics`](super::Generics),
//! [`GenericOwner::Type`]), and is then used applied to its type arguments,
//! `Pair<Int, String>` ([`TypeExprKind::Applied`], or
//! [`TypeExprKind::PathApplied`] qualified). In the store, its definition
//! holds each of its type parameters as a [`TypeExprKind::Param`], and a
//! use of it in another definition is an [`TypeExprKind::Instance`] of it,
//! its arguments; settling makes an instance the definition with each
//! parameter the argument of its number (`TypeArena::substitute`), and
//! expanding a tree does the same at each use. Substituting makes new nodes
//! only above a parameter, sharing the rest and each argument, so an
//! instance costs no more nodes than its definition has, however deeply
//! its arguments nest. A generic type is its definition, as any declared
//! type is: types are structural, and `Pair<Int, String>` is
//! `{first: Int, second: String}`. A spread of a type parameter is the
//! problem a spread of anything but a record type is, the definition being
//! merged before any argument is known.
//!
//! A field of a record type may have a default, `{exclude: List<String> =
//! [], from: String}` (see the checker's "Field defaults"): its
//! [`FieldTy::default`], a [`FieldDefault`]. As the parser writes it, it is
//! an expression of the tree, any expression, parsed after the field's type
//! and so before whatever follows the type (a `let`'s value, a function's
//! body), and its nodes are part of the subtree of the expression or item
//! the type is written in (`Ast::subtree`). A copy of a definition cannot
//! hold an expression of another tree, so a default copied out of a tree
//! into a [`TypeStore`] is made the [`Constant`] it is written as, if it is
//! one, its integers left as integers and nothing filled
//! (`check::written_constant`), held in the arena's pool of constants,
//! and copied on, pool and all, into each tree using its declaration; the
//! checker fits it to its field's type there.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::MAX_DEPTH;
use crate::check::{Constant, written_constant};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;

use super::{
    Ast, ExprId, FieldDefault, FieldTy, FieldTyKind, FieldTyRange, GenericOwner, MetaTy, ParamTy,
    ParamTyRange, Replace, ToConstant, TypeArgRange, TypeExpr, TypeExprId, TypeExprKind,
};

/// The type expressions of a tree, or of a [`TypeStore`]: the nodes, in an
/// arena indexed by [`TypeExprId`], with the fields of record types and the
/// parameters of function types each in a pool of their own, and the
/// defaults of fields that copies hold ([`FieldDefault::Copied`]) in a
/// third, each the constant it is, or `None` for one that is not a
/// constant.
#[derive(Clone, Debug, Default)]
pub struct TypeArena {
    pub(super) exprs: Vec<TypeExpr>,
    pub(super) fields: Vec<FieldTy>,
    pub(super) params: Vec<ParamTy>,
    pub(super) args: Vec<TypeExprId>,
    pub(super) constants: Vec<Option<Constant>>,
}

impl TypeArena {
    /// Adds `ty` to the arena and returns its id.
    pub(super) fn alloc(&mut self, ty: TypeExpr) -> TypeExprId {
        let id = TypeExprId(next_id(self.exprs.len()));
        self.exprs.push(ty);
        id
    }

    /// Appends `fields` to the field pool as one record type's and returns
    /// their range.
    pub(super) fn push_fields(&mut self, fields: &[FieldTy]) -> FieldTyRange {
        let start = next_id(self.fields.len());
        self.fields.extend_from_slice(fields);
        FieldTyRange {
            start,
            len: next_id(fields.len()),
        }
    }

    /// Appends `params` to the parameter pool as one function type's and
    /// returns their range.
    pub(super) fn push_params(&mut self, params: &[ParamTy]) -> ParamTyRange {
        let start = next_id(self.params.len());
        self.params.extend_from_slice(params);
        ParamTyRange {
            start,
            len: next_id(params.len()),
        }
    }

    /// Appends `args` to the type-argument pool as one applied type's and
    /// returns their range.
    pub(super) fn push_args(&mut self, args: &[TypeExprId]) -> TypeArgRange {
        let start = next_id(self.args.len());
        self.args.extend_from_slice(args);
        TypeArgRange {
            start,
            len: next_id(args.len()),
        }
    }

    /// The type arguments of the applied type whose arguments are `range`.
    ///
    /// # Panics
    ///
    /// Panics if `range` is not of the arena.
    pub(super) fn args(&self, range: TypeArgRange) -> &[TypeExprId] {
        let start = range.start as usize;
        &self.args[start..start + range.len as usize]
    }

    /// The node `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not a node of the arena.
    pub(super) fn get(&self, id: TypeExprId) -> &TypeExpr {
        &self.exprs[id.index()]
    }

    /// The kind of the node `id`, [`TypeExprKind::Invalid`] if the arena has
    /// no such node.
    pub(super) fn kind(&self, id: TypeExprId) -> TypeExprKind {
        self.exprs
            .get(id.index())
            .map_or(TypeExprKind::Invalid, |ty| ty.kind)
    }

    /// Makes the node `id` one of kind `kind`, keeping its span.
    pub(super) fn set_kind(&mut self, id: TypeExprId, kind: TypeExprKind) {
        if let Some(entry) = self.exprs.get_mut(id.index()) {
            entry.kind = kind;
        }
    }

    /// The fields of the record type whose fields are `range`.
    ///
    /// # Panics
    ///
    /// Panics if `range` is not of the arena.
    pub(super) fn fields(&self, range: FieldTyRange) -> &[FieldTy] {
        let start = range.start as usize;
        &self.fields[start..start + range.len as usize]
    }

    /// The parameters of the function type whose parameters are `range`.
    ///
    /// # Panics
    ///
    /// Panics if `range` is not of the arena.
    pub(super) fn params(&self, range: ParamTyRange) -> &[ParamTy] {
        let start = range.start as usize;
        &self.params[start..start + range.len as usize]
    }

    /// The number of nodes.
    pub(super) fn len(&self) -> usize {
        self.exprs.len()
    }

    /// The ids of the nodes from number `start` on, in order.
    pub(super) fn ids_from(&self, start: usize) -> impl Iterator<Item = TypeExprId> + use<> {
        (next_id(start)..next_id(self.exprs.len())).map(TypeExprId)
    }

    /// Copies the node `root` of `from`, and every node under it, into this
    /// arena, children first, and returns the copy of `root`: a node `memo`
    /// has a copy of already is that copy, shared, and every node copied is
    /// added to it; a node `replace` gives a kind is a node of that kind, of
    /// its span, with nothing under it, but an applied type it makes a
    /// [`TypeExprKind::Ref`], which is an [`TypeExprKind::Instance`] of its
    /// arguments, copied; every other node is a node of its
    /// kind and span, its children copied. A qualified name `replace` gives
    /// no kind is [`TypeExprKind::Invalid`], its path being of `from`'s tree.
    /// A field's default is copied into this arena's pool of constants: one
    /// `from` holds as an expression, of `from`'s tree, as what `constant`
    /// makes of it, and one `from` holds in its pool as it is there.
    /// Recurses once per level of nesting under `root`.
    pub(super) fn copy(
        &mut self,
        from: &TypeArena,
        root: TypeExprId,
        memo: &mut HashMap<TypeExprId, TypeExprId>,
        (replace, constant): (Replace<'_>, ToConstant<'_>),
    ) -> TypeExprId {
        if let Some(&copied) = memo.get(&root) {
            return copied;
        }
        let TypeExpr { kind, span } = from.exprs.get(root.index()).copied().unwrap_or(TypeExpr {
            kind: TypeExprKind::Invalid,
            span: Span::new(0, 0),
        });
        let kind = match (replace(root), kind) {
            // A declared type applied to arguments, whose arguments are
            // copied as its instance's.
            (
                Some(TypeExprKind::Ref(decl)),
                TypeExprKind::Applied { args, .. } | TypeExprKind::PathApplied { args, .. },
            ) => TypeExprKind::Instance {
                decl,
                args: self.copy_args(from, args, memo, (replace, constant)),
            },
            (Some(kind), _) => kind,
            (None, kind) => match kind {
                TypeExprKind::Record(range) => {
                    let fields: Vec<FieldTy> = from
                        .fields(range)
                        .iter()
                        .map(|field| {
                            let default = field.default.map(|default| {
                                let copied = match default {
                                    FieldDefault::Expr(expr) => constant(expr),
                                    FieldDefault::Copied(at) => {
                                        from.constants.get(at as usize).cloned().flatten()
                                    }
                                };
                                let at = next_id(self.constants.len());
                                self.constants.push(copied);
                                FieldDefault::Copied(at)
                            });
                            FieldTy {
                                ty: self.copy(from, field.ty, memo, (replace, constant)),
                                default,
                                ..*field
                            }
                        })
                        .collect();
                    TypeExprKind::Record(self.push_fields(&fields))
                }
                TypeExprKind::Fn {
                    params,
                    ret,
                    effect,
                    meta,
                } => {
                    let params: Vec<ParamTy> = from
                        .params(params)
                        .iter()
                        .map(|param| ParamTy {
                            ty: self.copy(from, param.ty, memo, (replace, constant)),
                            ..*param
                        })
                        .collect();
                    let ret = self.copy(from, ret, memo, (replace, constant));
                    let meta = meta.map(|meta| MetaTy {
                        ty: self.copy(from, meta.ty, memo, (replace, constant)),
                        ..meta
                    });
                    TypeExprKind::Fn {
                        params: self.push_params(&params),
                        ret,
                        effect,
                        meta,
                    }
                }
                TypeExprKind::Applied { name, span, args } => TypeExprKind::Applied {
                    name,
                    span,
                    args: self.copy_args(from, args, memo, (replace, constant)),
                },
                TypeExprKind::List(element) => {
                    TypeExprKind::List(self.copy(from, element, memo, (replace, constant)))
                }
                TypeExprKind::Union { lhs, rhs, bar } => {
                    let lhs = self.copy(from, lhs, memo, (replace, constant));
                    let rhs = self.copy(from, rhs, memo, (replace, constant));
                    TypeExprKind::Union { lhs, rhs, bar }
                }
                TypeExprKind::Instance { decl, args } => TypeExprKind::Instance {
                    decl,
                    args: self.copy_args(from, args, memo, (replace, constant)),
                },
                TypeExprKind::Path(_) | TypeExprKind::PathApplied { .. } => TypeExprKind::Invalid,
                kind @ (TypeExprKind::Named(_)
                | TypeExprKind::Ref(_)
                | TypeExprKind::Param(_)
                | TypeExprKind::Enum { .. }
                | TypeExprKind::Context(_)
                | TypeExprKind::Invalid) => kind,
            },
        };
        let id = self.alloc(TypeExpr { kind, span });
        memo.insert(root, id);
        id
    }

    /// The node `root` with the type argument `args[i]` in place of each
    /// [`TypeExprKind::Param`] `i` under it: an argument the arena has no
    /// node for, `args` lacking it, a node `Invalid`. A node with no
    /// parameter under it is itself, shared, and so is a parameter's
    /// argument; every other is a new node of its kind and span, its
    /// children substituted, made children first, with the defaults of a
    /// record type's fields as they are. `memo` holds what each node
    /// substituted is, so that a node shared under `root` is substituted
    /// once.
    ///
    /// Recurses once per level of nesting under `root`.
    pub(super) fn substitute(
        &mut self,
        root: TypeExprId,
        args: &[TypeExprId],
        memo: &mut HashMap<TypeExprId, TypeExprId>,
    ) -> TypeExprId {
        if let Some(&done) = memo.get(&root) {
            return done;
        }
        let Some(&TypeExpr { kind, span }) = self.exprs.get(root.index()) else {
            return root;
        };
        let kind = match kind {
            TypeExprKind::Param(index) => {
                let invalid = TypeExpr {
                    kind: TypeExprKind::Invalid,
                    span,
                };
                let done =
                    (args.get(usize::from(index)).copied()).unwrap_or_else(|| self.alloc(invalid));
                memo.insert(root, done);
                return done;
            }
            TypeExprKind::Record(range) => {
                let fields = self.fields(range).to_vec();
                let substituted: Vec<FieldTy> = fields
                    .iter()
                    .map(|field| FieldTy {
                        ty: self.substitute(field.ty, args, memo),
                        ..*field
                    })
                    .collect();
                (substituted != fields)
                    .then(|| TypeExprKind::Record(self.push_fields(&substituted)))
            }
            TypeExprKind::Fn {
                params,
                ret,
                effect,
                meta,
            } => {
                let written = self.params(params).to_vec();
                let substituted: Vec<ParamTy> = written
                    .iter()
                    .map(|param| ParamTy {
                        ty: self.substitute(param.ty, args, memo),
                        ..*param
                    })
                    .collect();
                let new_ret = self.substitute(ret, args, memo);
                let new_meta = meta.map(|meta| MetaTy {
                    ty: self.substitute(meta.ty, args, memo),
                    ..meta
                });
                (substituted != written || new_ret != ret || new_meta != meta).then(|| {
                    TypeExprKind::Fn {
                        params: self.push_params(&substituted),
                        ret: new_ret,
                        effect,
                        meta: new_meta,
                    }
                })
            }
            TypeExprKind::Applied {
                args: given,
                name,
                span,
            } => self
                .substitute_args(given, args, memo)
                .map(|given| TypeExprKind::Applied {
                    name,
                    span,
                    args: given,
                }),
            TypeExprKind::Instance { decl, args: given } => self
                .substitute_args(given, args, memo)
                .map(|given| TypeExprKind::Instance { decl, args: given }),
            TypeExprKind::List(element) => {
                let new = self.substitute(element, args, memo);
                (new != element).then_some(TypeExprKind::List(new))
            }
            TypeExprKind::Union { lhs, rhs, bar } => {
                let new = (
                    self.substitute(lhs, args, memo),
                    self.substitute(rhs, args, memo),
                );
                (new != (lhs, rhs)).then_some(TypeExprKind::Union {
                    lhs: new.0,
                    rhs: new.1,
                    bar,
                })
            }
            TypeExprKind::Named(_)
            | TypeExprKind::Path(_)
            | TypeExprKind::PathApplied { .. }
            | TypeExprKind::Ref(_)
            | TypeExprKind::Enum { .. }
            | TypeExprKind::Context(_)
            | TypeExprKind::Invalid => None,
        };
        let done = match kind {
            Some(kind) => self.alloc(TypeExpr { kind, span }),
            None => root,
        };
        memo.insert(root, done);
        done
    }

    /// The type arguments `given` with [`TypeArena::substitute`]'s
    /// substitution of `args` made in each, if that changes any.
    pub(super) fn substitute_args(
        &mut self,
        given: TypeArgRange,
        args: &[TypeExprId],
        memo: &mut HashMap<TypeExprId, TypeExprId>,
    ) -> Option<TypeArgRange> {
        let written = self.args(given).to_vec();
        let substituted: Vec<TypeExprId> = written
            .iter()
            .map(|&arg| self.substitute(arg, args, memo))
            .collect();
        (substituted != written).then(|| self.push_args(&substituted))
    }

    /// Copies the type arguments `args` of `from` into this arena, as
    /// [`TypeArena::copy`] copies each, and returns their range here.
    pub(super) fn copy_args(
        &mut self,
        from: &TypeArena,
        args: TypeArgRange,
        memo: &mut HashMap<TypeExprId, TypeExprId>,
        (replace, constant): (Replace<'_>, ToConstant<'_>),
    ) -> TypeArgRange {
        let copied: Vec<TypeExprId> = from
            .args(args)
            .iter()
            .map(|&arg| self.copy(from, arg, memo, (replace, constant)))
            .collect();
        self.push_args(&copied)
    }

    /// The height of the node `id` as it is now, its children's being in
    /// `heights` by id: 1 for one with no children (1 too for a child
    /// `heights` lacks), one more than its tallest child otherwise.
    pub(super) fn height(&self, id: TypeExprId, heights: &[u32]) -> u32 {
        self.children(id)
            .iter()
            .map(|child| heights.get(child.index()).copied().unwrap_or(1))
            .max()
            .map_or(1, |tallest| tallest.saturating_add(1))
    }

    /// The children of the node `id`, as they are now: a record type's
    /// fields' types, a function type's parameters', return type and meta's,
    /// an applied type's arguments, a list type's element, a union's sides.
    pub(super) fn children(&self, id: TypeExprId) -> Vec<TypeExprId> {
        match self.kind(id) {
            TypeExprKind::Record(range) => self.fields(range).iter().map(|f| f.ty).collect(),
            TypeExprKind::Fn {
                params, ret, meta, ..
            } => self
                .params(params)
                .iter()
                .map(|p| p.ty)
                .chain([ret])
                .chain(meta.map(|meta| meta.ty))
                .collect(),
            TypeExprKind::Applied { args, .. }
            | TypeExprKind::PathApplied { args, .. }
            | TypeExprKind::Instance { args, .. } => self.args(args).to_vec(),
            TypeExprKind::List(element) => vec![element],
            TypeExprKind::Union { lhs, rhs, .. } => vec![lhs, rhs],
            TypeExprKind::Named(_)
            | TypeExprKind::Path(_)
            | TypeExprKind::Ref(_)
            | TypeExprKind::Param(_)
            | TypeExprKind::Enum { .. }
            | TypeExprKind::Context(_)
            | TypeExprKind::Invalid => Vec::new(),
        }
    }

    /// Merges the spreads of the record type `id` into its fields, as value
    /// spread merges a record block's (see [Types](#types)): a spread's
    /// fields join the record SOFT; a later field of a soft field's name
    /// replaces it, and so does a later spread's; a spread's field of the
    /// name of a field written before it is the problem
    /// [`TypeProblem::Replaces`], once for the spread, the field written
    /// before standing. A spread of anything but a record type is
    /// [`TypeProblem::NotRecord`], or, of an erroneous type,
    /// [`TypeProblem::Erroneous`], and spreads nothing. Two fields written of
    /// one name are both kept, for the checker to report; a spread of a
    /// record type with two of one name spreads the first, the later being
    /// [`TypeProblem::Repeated`]. A rest-field merges as a field of its own
    /// name would: a spread's is soft, and replaces or is replaced as a
    /// field is. The node becomes
    /// the record type of the merged fields; one without a spread is left
    /// as it is. Returns the problems, in source order. A spread's operand
    /// must have its own spreads merged already.
    pub(super) fn merge_record(&mut self, id: TypeExprId) -> Vec<TypeProblem> {
        let TypeExprKind::Record(range) = self.kind(id) else {
            return Vec::new();
        };
        let written = self.fields(range);
        if written
            .iter()
            .all(|field| field.kind != FieldTyKind::Spread)
        {
            return Vec::new();
        }
        let written = written.to_vec();
        let mut merged: Vec<(FieldTy, bool)> = Vec::with_capacity(written.len());
        // Where each field is merged, by its name, `None` being the
        // rest-field's.
        let mut at: HashMap<Option<Name>, usize> = HashMap::new();
        let mut problems = Vec::new();
        // Whether the field merged at an index is soft.
        let soft = |merged: &[(FieldTy, bool)], index: usize| {
            merged.get(index).is_some_and(|&(_, soft)| soft)
        };
        for entry in written {
            if entry.kind != FieldTyKind::Spread {
                let name = entry.name();
                match at.get(&name).copied() {
                    Some(index) if soft(&merged, index) => {
                        if let Some(slot) = merged.get_mut(index) {
                            *slot = (entry, false);
                        }
                    }
                    _ => {
                        at.entry(name).or_insert(merged.len());
                        merged.push((entry, false));
                    }
                }
                continue;
            }
            let operand = entry.ty;
            let spread = match self.kind(operand) {
                TypeExprKind::Record(spread) => spread,
                TypeExprKind::Invalid => {
                    problems.push(TypeProblem::Erroneous);
                    continue;
                }
                _ => {
                    problems.push(TypeProblem::NotRecord(operand));
                    continue;
                }
            };
            // The first field written before the spread, and the number of
            // others.
            let mut replaced: Option<(Option<Name>, TypeExprId, Span)> = None;
            let mut others = 0_u32;
            // The names the spread has given so far: one it gives twice, a
            // record type written in place with a name twice, is the
            // repeated-name error it is anywhere else, the later dropped.
            let mut given: HashSet<Option<Name>> = HashSet::new();
            for field in self.fields(spread) {
                if field.kind == FieldTyKind::Spread {
                    continue;
                }
                let name = field.name();
                if !given.insert(name) {
                    problems.push(TypeProblem::Repeated {
                        name,
                        at: field.span,
                    });
                    continue;
                }
                match at.get(&name).copied() {
                    Some(index) if soft(&merged, index) => {
                        if let Some(slot) = merged.get_mut(index) {
                            *slot = (*field, true);
                        }
                    }
                    Some(index) => match replaced {
                        None => {
                            let first = merged.get(index).map_or(entry.span, |(hard, _)| hard.span);
                            replaced = Some((name, field.ty, first));
                        }
                        Some(_) => others = others.saturating_add(1),
                    },
                    None => {
                        at.insert(name, merged.len());
                        merged.push((*field, true));
                    }
                }
            }
            if let Some((name, ty, first)) = replaced {
                problems.push(TypeProblem::Replaces {
                    operand,
                    name,
                    ty,
                    first,
                    others,
                });
            }
        }
        let fields: Vec<FieldTy> = merged.into_iter().map(|(field, _)| field).collect();
        let range = self.push_fields(&fields);
        self.set_kind(id, TypeExprKind::Record(range));
        problems
    }
}

/// What the expansion of a tree's types ([`Ast::expand_types`]) found
/// wrong with one of its type expressions, for the checker to report where
/// it resolves it (see [Types](#types)).
#[derive(Clone, Debug, PartialEq)]
pub enum TypeProblem {
    /// The type names nothing it can: the error, as it is reported. The
    /// node is [`TypeExprKind::Invalid`].
    Unresolved(Diagnostic),
    /// The record type spreads the type `operand`, which is not a record
    /// type: "cannot spread type T: it is not a record type", at the
    /// operand.
    NotRecord(TypeExprId),
    /// The record type spreads the type `operand`, which has a field
    /// `name`, of type `ty`, that the record type writes before the spread,
    /// its name at `first`, and `others` more such fields: "field `x` is
    /// defined twice", at the operand; for the rest-field, "the rest-field
    /// is defined twice".
    Replaces {
        /// The type spread.
        operand: TypeExprId,
        /// The first field it has that is written before it: its name, or
        /// `None` for the rest-field.
        name: Option<Name>,
        /// The type it has that field of.
        ty: TypeExprId,
        /// Where the field written before it is.
        first: Span,
        /// How many more such fields it has.
        others: u32,
    },
    /// The record type spreads a type that is erroneous, whose error is
    /// reported where it is written: the record type is erroneous too.
    Erroneous,
    /// The record type spreads a record type written in place that has the
    /// field `name` twice, the later at `at`: "field `x` appears twice in
    /// this type", at the later, as anywhere else; for the rest-field, "the
    /// rest-field appears twice in this type".
    Repeated {
        /// The name written twice, or `None` for the rest-field.
        name: Option<Name>,
        /// Where it is written the second time.
        at: Span,
    },
}

/// What a type expression of a tree that reads a declared type by its name
/// names, as the workspace resolves it in the tree's file (see
/// [`crate::workspace`], "Types"), for [`Ast::expand_types`] and
/// [`TypeStore::declare`]. A name of no declared type has none: a built-in
/// type, or an unknown one, which the checker reports.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeLink {
    /// The declaration numbered so in the [`TypeStore`].
    Decl(u32),
    /// Nothing it can: the error.
    Error(Diagnostic),
}

/// Where a [`TypeStore`] was, to put it back there ([`TypeStore::mark`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeStoreMark {
    pub(super) exprs: usize,
    pub(super) fields: usize,
    pub(super) params: usize,
    pub(super) args: usize,
    pub(super) constants: usize,
    pub(super) decls: usize,
}

/// What a host knows an attribute by (see the parser's
/// [Attributes](crate::parser::attributes#attributes)): its declaration, by the module
/// that declares it, the path of its file without `.gcsl`, and its name;
/// or one of GCSL's own, `retry` and `timeout`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AttributeId {
    /// A `type` marked `#[attribute]`.
    Declared {
        /// The module that declares it.
        module: Rc<str>,
        /// The name it declares.
        name: Rc<str>,
    },
    /// One of GCSL's own, by its name.
    Builtin(&'static str),
}

impl AttributeId {
    /// Its name, as an attribute is written.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            AttributeId::Declared { name, .. } => name,
            AttributeId::Builtin(name) => name,
        }
    }
}

/// The definitions of declared types, apart from any tree, so that a tree
/// can be given the definition of a type another file declares, as if it
/// were written there (see [Types](#types)): each declaration's definition
/// copied out of its file's tree, each name of another declared type in it
/// a [`TypeExprKind::Ref`], until it is settled ([`TypeStore::settle`]),
/// after which it refers to nothing and holds no spread. A workspace's scan
/// has one; a session keeps one, its inputs' declarations after each
/// other's.
#[derive(Clone, Debug, Default)]
pub struct TypeStore {
    pub(super) arena: TypeArena,
    /// The height of each node, once its declaration is settled: 1 for a
    /// name, one more than its tallest child otherwise.
    pub(super) heights: Vec<u32>,
    /// Each declaration's definition: the number of its first node, and its
    /// root, every node from the first to the root being its own.
    pub(super) decls: Vec<(u32, TypeExprId)>,
    /// The declarations marked `#[attribute]`, by their numbers, each with
    /// what a host knows it by.
    pub(super) attributes: HashMap<u32, AttributeId>,
    /// The declarations of attributes marked `#[defers]` too, by their
    /// numbers.
    pub(super) deferring: HashSet<u32>,
    /// How many type parameters each declaration has, by number: none but
    /// for a generic one.
    pub(super) arities: Vec<u32>,
}

impl TypeStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of declarations.
    #[must_use]
    pub fn len(&self) -> u32 {
        next_id(self.decls.len())
    }

    /// Whether the store has no declaration.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }

    /// Where the store is now.
    #[must_use]
    pub fn mark(&self) -> TypeStoreMark {
        TypeStoreMark {
            exprs: self.arena.exprs.len(),
            fields: self.arena.fields.len(),
            params: self.arena.params.len(),
            args: self.arena.args.len(),
            constants: self.arena.constants.len(),
            decls: self.decls.len(),
        }
    }

    /// Forgets everything added since `mark`.
    pub fn truncate(&mut self, mark: TypeStoreMark) {
        self.arena.exprs.truncate(mark.exprs);
        self.arena.fields.truncate(mark.fields);
        self.arena.params.truncate(mark.params);
        self.arena.args.truncate(mark.args);
        self.arena.constants.truncate(mark.constants);
        self.heights.truncate(mark.exprs);
        self.decls.truncate(mark.decls);
        self.arities.truncate(mark.decls);
        let decls = u32::try_from(mark.decls).unwrap_or(u32::MAX);
        self.attributes.retain(|&decl, _| decl < decls);
    }

    /// Records that the declaration numbered `decl` is marked
    /// `#[attribute]`, known to a host as `id`.
    pub fn mark_attribute(&mut self, decl: u32, id: AttributeId) {
        self.attributes.insert(decl, id);
    }

    /// Records that the declaration numbered `decl`, an attribute's, is
    /// marked `#[defers]`.
    pub fn mark_deferring(&mut self, decl: u32) {
        self.deferring.insert(decl);
    }

    /// Whether the declaration numbered `decl` is an attribute's marked
    /// `#[defers]`.
    #[must_use]
    pub fn defers(&self, decl: u32) -> bool {
        self.deferring.contains(&decl)
    }

    /// What a host knows the declaration numbered `decl` by, if it is
    /// marked `#[attribute]`.
    #[must_use]
    pub fn attribute(&self, decl: u32) -> Option<&AttributeId> {
        self.attributes.get(&decl)
    }

    /// Adds the declarations numbered from the store's length on, `count`
    /// of them, each defined as nothing yet, to be defined by
    /// [`TypeStore::declare`] in any order. Returns the first's number.
    pub fn reserve(&mut self, count: usize) -> u32 {
        let first = self.len();
        let nothing = TypeExprId(u32::MAX);
        self.decls
            .extend(std::iter::repeat_n((u32::MAX, nothing), count));
        self.arities.resize(self.decls.len(), 0);
        first
    }

    /// The enum the declaration numbered `decl` is, by its key and its
    /// name, if it is an enum's (see [Types](#types)).
    #[must_use]
    pub fn enum_of(&self, decl: u32) -> Option<(Name, Name)> {
        match self.arena.kind(self.root(decl)?) {
            TypeExprKind::Enum { key, name } => Some((key, name)),
            _ => None,
        }
    }

    /// Defines the declaration `decl` as the enum of the key `key`, named
    /// `name`, declared at `span`, an [`TypeExprKind::Enum`]: a type of its
    /// own, which nothing it refers to defines (see [Types](#types)).
    pub fn declare_enum(&mut self, decl: u32, (key, name): (Name, Name), span: Span) {
        let first = next_id(self.arena.len());
        let root = self.arena.alloc(TypeExpr {
            kind: TypeExprKind::Enum { key, name },
            span,
        });
        self.heights.resize(self.arena.len(), 1);
        if let Some(entry) = self.decls.get_mut(decl as usize) {
            *entry = (first, root);
        }
    }

    /// Records that the declaration numbered `decl` has `arity` type
    /// parameters, as [`Ast::type_arity`] says of its item.
    pub fn set_arity(&mut self, decl: u32, arity: u32) {
        if let Some(entry) = self.arities.get_mut(decl as usize) {
            *entry = arity;
        }
    }

    /// How many type parameters the declaration numbered `decl` has: none
    /// unless it is generic.
    #[must_use]
    pub fn arity(&self, decl: u32) -> u32 {
        self.arities.get(decl as usize).copied().unwrap_or(0)
    }

    /// Defines the declaration `decl` as the type expression `root` of
    /// `ast`, copied out of it, with each node `links` has an entry for (by
    /// id, in order) a [`TypeExprKind::Ref`] to the declaration it names, an
    /// [`TypeExprKind::Instance`] of it if it applies it to arguments, or
    /// [`TypeExprKind::Invalid`] if it names none it can; and each name that
    /// is a type parameter of the declaration's own, a generic one's, the
    /// [`TypeExprKind::Param`] of its number. Spreads are
    /// copied as they are written, until it is settled. A field's default
    /// is copied as the constant it is written as, if it is one
    /// (`check::written_constant`), which the checker of each tree using
    /// the declaration fits to the field's type; what is wrong in it, its
    /// own file's check reports.
    pub fn declare(
        &mut self,
        decl: u32,
        (ast, root): (&Ast, TypeExprId),
        links: &[(TypeExprId, TypeLink)],
    ) {
        let first = next_id(self.arena.len());
        let replace = |id: TypeExprId| {
            if let TypeExprKind::Named(name) = ast.types.kind(id)
                && let Some((GenericOwner::Type(_), index)) = ast.type_param(id, name)
            {
                return Some(TypeExprKind::Param(index));
            }
            let at = links.binary_search_by_key(&id.0, |&(at, _)| at.0).ok()?;
            Some(match links.get(at).map(|(_, link)| link) {
                Some(TypeLink::Decl(decl)) => TypeExprKind::Ref(*decl),
                _ => TypeExprKind::Invalid,
            })
        };
        let constant = |expr: ExprId| written_constant(ast, expr);
        let copied = self
            .arena
            .copy(&ast.types, root, &mut HashMap::new(), (&replace, &constant));
        self.heights.resize(self.arena.len(), 1);
        if let Some(entry) = self.decls.get_mut(decl as usize) {
            *entry = (first, copied);
        }
    }

    /// The declarations the definition of `decl` refers to by name, each
    /// with where, in its own nodes' order: the edges of a cycle of types.
    #[must_use]
    pub fn refs(&self, decl: u32) -> Vec<(u32, Span)> {
        let Some(&(first, root)) = self.decls.get(decl as usize) else {
            return Vec::new();
        };
        (first..=root.0)
            .filter_map(|at| {
                let ty = self.arena.exprs.get(at as usize)?;
                match ty.kind {
                    TypeExprKind::Ref(to) | TypeExprKind::Instance { decl: to, .. } => {
                        Some((to, ty.span))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    /// Settles the declaration `decl`, every declaration it refers to being
    /// settled already, or it being on a cycle of types (`cyclic`): each
    /// reference to another becomes what that one's definition is, shared,
    /// and each record type its spreads merged, its own nodes taken in
    /// order, children first (see [Types](#types)). It is erroneous, its
    /// definition then [`TypeExprKind::Invalid`] alone, if it is on a cycle,
    /// if a spread of it has a problem, or if it would be nested more than
    /// [`MAX_DEPTH`] levels deep; whatever is wrong is reported where it is
    /// written, in its file. A reference to an erroneous one is erroneous.
    pub fn settle(&mut self, decl: u32, cyclic: bool) {
        let Some(&(first, root)) = self.decls.get(decl as usize) else {
            return;
        };
        let mut erroneous = cyclic;
        for id in self
            .arena
            .ids_from(first as usize)
            .take_while(|&id| id.0 <= root.0)
        {
            if erroneous {
                break;
            }
            match self.arena.kind(id) {
                TypeExprKind::Ref(to) => {
                    let target = self.root(to);
                    let kind = target.map_or(TypeExprKind::Invalid, |at| self.arena.kind(at));
                    self.arena.set_kind(id, kind);
                    let height = target
                        .and_then(|at| self.heights.get(at.index()))
                        .copied()
                        .unwrap_or(1);
                    if let Some(entry) = self.heights.get_mut(id.index()) {
                        *entry = height;
                    }
                    continue;
                }
                TypeExprKind::Instance { decl: to, args } => {
                    let height = self.instantiate(id, to, args);
                    erroneous |= height as usize > MAX_DEPTH;
                    continue;
                }
                TypeExprKind::Record(_) => {
                    erroneous |= !self.arena.merge_record(id).is_empty();
                }
                _ => {}
            }
            let height = self.arena.height(id, &self.heights);
            erroneous |= height as usize > MAX_DEPTH;
            if let Some(entry) = self.heights.get_mut(id.index()) {
                *entry = height;
            }
        }
        if erroneous {
            self.arena.set_kind(root, TypeExprKind::Invalid);
            if let Some(entry) = self.heights.get_mut(root.index()) {
                *entry = 1;
            }
        }
    }

    /// Makes the node `id`, an instance of the declaration `decl` applied to
    /// `args`, what `decl`'s settled definition is with `args` substituted
    /// for its type parameters (`TypeArena::substitute`), the nodes made
    /// shared as a reference's are, or `Invalid` if `decl` is erroneous.
    /// Returns its height.
    pub(super) fn instantiate(&mut self, id: TypeExprId, decl: u32, args: TypeArgRange) -> u32 {
        let args = self.arena.args(args).to_vec();
        let target = self
            .root(decl)
            .filter(|&root| self.arena.kind(root) != TypeExprKind::Invalid);
        let (kind, height) = match target {
            Some(root) => {
                let start = self.arena.len();
                let made = self.arena.substitute(root, &args, &mut HashMap::new());
                self.heights.resize(self.arena.len(), 1);
                for new in self.arena.ids_from(start) {
                    let height = self.arena.height(new, &self.heights);
                    if let Some(entry) = self.heights.get_mut(new.index()) {
                        *entry = height;
                    }
                }
                let height = self.heights.get(made.index()).copied().unwrap_or(1);
                (self.arena.kind(made), height)
            }
            None => (TypeExprKind::Invalid, 1),
        };
        self.arena.set_kind(id, kind);
        if let Some(entry) = self.heights.get_mut(id.index()) {
            *entry = height;
        }
        height
    }

    /// The root of the definition of `decl`, if it is defined.
    pub(super) fn root(&self, decl: u32) -> Option<TypeExprId> {
        self.decls
            .get(decl as usize)
            .map(|&(_, root)| root)
            .filter(|root| root.index() < self.arena.len())
    }
}

/// Converts an arena length to the next id, saturating past `u32::MAX`.
///
/// An arena holds at most `u32::MAX` entries. Every entry consumes at least
/// one byte of source and sources are limited to `u32::MAX` bytes, so a
/// parser can never reach the limit; past it, ids would saturate.
pub(super) fn next_id(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}
