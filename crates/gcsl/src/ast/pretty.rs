//! Printing a tree: the annotations a printer may add, the pretty printer the
//! dumps use, and how a type expression is written.

use std::fmt;

use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::{FieldName, Quoted};
use crate::types::write_constant;

use super::{
    ArgKind, ArgRange, Ast, DefTypeId, Effect, ElementRange, Expr, ExprId, ExprKind, FieldDefault,
    FieldTyKind, FnId, ForHead, ForId, ImportDecl, ImportForm, ImportMember, ImportName, ItemId,
    ItemKind, ItemRange, Jump, Label, MetaTy, NeedTyRange, ParamTyRange, PathDisplay, PathRange,
    PathSegment, StrPart, TypeArgRange, TypeExprId, TypeExprKind, UnaryOp, WithRange,
};

/// Per-node information written by [`Ast::pretty_with`].
pub trait Annotate {
    /// Writes the annotation for node `id`, including any leading space.
    ///
    /// # Errors
    ///
    /// Propagates errors from the formatter.
    fn annotate(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result;

    /// Writes the annotation for the `type` item `item`, after its type as
    /// written, including any leading space: by default, nothing.
    ///
    /// # Errors
    ///
    /// Propagates errors from the formatter.
    fn annotate_type(&self, item: ItemId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = (item, f);
        Ok(())
    }

    /// If the def-type reference `id`, `DT PATH`, was found to have a path
    /// that goes on past the definition it names, as `package
    /// dev.peios.testpkg.versions` reads the field `versions` of the meta of
    /// `package dev.peios.testpkg` (see the checker's "Metas"): the prefix
    /// of its read ([`DefTypeUse::read`](super::DefTypeUse::read)) that reads the definition. The
    /// reference is then printed as the reference to the definition with a
    /// field access around it for each segment past it. By default, `None`:
    /// every reference as it is parsed.
    fn reference_past(&self, id: ExprId) -> Option<ExprId> {
        let _ = id;
        None
    }

    /// Writes the annotation, including any leading space, of the reference
    /// to the definition that the path of the def-type reference `id` goes
    /// on past ([`Annotate::reference_past`]): by default, nothing.
    ///
    /// # Errors
    ///
    /// Propagates errors from the formatter.
    fn annotate_reference(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = (id, f);
        Ok(())
    }
}

/// The empty annotation used by [`Ast::pretty`].
#[derive(Clone, Copy, Debug)]
pub struct NoAnnotation;

impl Annotate for NoAnnotation {
    fn annotate(&self, _: ExprId, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

/// An [`Ast`] rendered as indented S-expressions, one node per line:
///
/// ```text
/// (file
///   (let x : Float @0..16
///     (+ @10..16
///       (int 1 @10..11)
///       (int 2 @15..16)))
///   (expr @17..18
///     (name x @17..18)))
/// ```
///
/// Items print as `(let x ...)`, `(let x : Type ...)`, `(let mut x ...)`, `(let
/// mut x : Type ...)`, `(field x ...)`, `(field x : Type ...)`, `(rebind x
/// ...)`, `(spread ...)`, `(result ...)` and `(expr ...)`, followed by their
/// span and their expression, a dotted path in full, as in `(let a.b.c ...)`,
/// and an index segment as it is written, as in `(rebind xs[i + 1].up ...)`,
/// its position then shown before the value as `(at EXPR)`, one for each index
/// segment, in order; blocks as `(block ...)` followed by their items;
/// ascriptions as `(: Type ...)`; field accesses as `(. x ...)` followed by
/// their record expression. A field name, or a path segment, that is not a
/// plain identifier is quoted, as in `(field "a b" ...)`. Types print as
/// written, whatever the expansion of the tree's types made of them
/// ([`Ast::written_type`]), a record type's fields and spreads in source order:
/// `{x: Int, "a b": {}}`, `{...PackageDef, name: String}`, `fn(x: Int) ->
/// String`, and a qualified name as its path, `t.PackageDef`. A `type` item
/// prints as `(type NAME TYPE @0..20)`, with the annotation's
/// [`Annotate::annotate_type`] after its type, which `--dump-types` makes its
/// expansion. A function expression prints as `(fn ...)` followed by `(params
/// (x Int) (y String))`, `(ret pure Int)`, `(ret memo Int)` or `(ret impure
/// Int)`, and its body; a call as `(call ...)` followed by its callee and its
/// arguments in source order, a named one as `(named x EXPR)` and the trailing
/// block as `(trailing BLOCK)`. A named function item is the `let` it is parsed
/// as, printed as written, `(fn f (fn ...))`, and a `def` the field, `(def f
/// (fn ...))`; `let f = fn ...` and `f = fn ...` print as `(let f (fn ...))`
/// and `(field f (fn ...))`. A `lazy` prints as `(lazy ...)` followed by its
/// body, and an `await` as `(await ...)` followed by its operand. A list
/// literal prints as `(list ...)` followed by its elements in source order, a
/// spread one as `(spread EXPR)`, and an index as `(index ...)` followed by its
/// list and its position. A `for` prints as `(for x ...)`, `x` being its
/// variable, followed by its iterable, its guard as `(guard EXPR)` if it has
/// one, and its body; a condition loop as `(for ...)` followed by the items
/// before its condition, its condition as `(cond EXPR)` if it has one, the
/// items after it and its body; a `do` loop as `(do ...)` followed by its body
/// and its condition as `(cond EXPR)`. A loop's label follows its keyword as it
/// is written, as in `(for'rows r ...)`, and a block's follows `block`, as in
/// `(block'pick @0..30 ...)`. A `break`, `continue` or `return` item prints as
/// `(break @0..5)`, its label after its keyword, as in `(continue'rows @0..14)`
/// or `(return'pick @2..16 ...)`, and its value, if it has one, after its span.
/// A range prints as its operator, `..` or `..=`, like any binary operator.
/// Applied and union types print as written, `Future<Int>`, `Present<Int>` and
/// `Int | Future<Int>`, a function type that is a member of a union in
/// parentheses, `(fn(): Int) | Future<fn(): Int>`, since its return type would
/// otherwise take the rest of the union; a list type written `T[]` prints so,
/// its element type in parentheses if it is a function type or a union, `(fn():
/// Int)[]`; a function type's meta prints after its return type as written,
/// `fn(): Int META-> {a: Int}`, the return type in parentheses if it is a
/// function type; other parentheses in a type leave no trace in the tree and
/// are not printed. Booleans print as `(bool true)`, string literals as `(str
/// "text")` with the text quoted and escaped, and interpolated strings as
/// `(interp ...)` followed by their parts, literal text as `(lit "text")`
/// (which has no span) and expressions as their nodes. `(if ...)` is followed
/// by its condition, its block and its else branch, which for an `if` without
/// `else` is the parser's empty block. Operators print as their symbol, except
/// negation, `(neg ...)`. A def-type definition prints as `(def-type-def
/// package dev.peios.testpkg ...)`, its def-type and its path as written,
/// followed by its function, whose parameters are their names alone, as in
/// `(params a b)`, with no `(ret ...)`; a reference to one as `(def-type-ref
/// package dev.peios.testpkg @0..30)`, with nothing under it, its whole path as
/// written, unless the annotation says the path goes on past the definition
/// ([`Annotate::reference_past`]): then as a field access for each segment past
/// it, `(. versions ...)`, around the reference to the definition,
/// `(def-type-ref package dev.peios.testpkg ...)`; a definition's meta as
/// `(defmeta package dev.peios.testpkg ...)`, followed by its function, of no
/// parameters, with no `(ret ...)`, whose body is the block written, or for
/// `defmeta DT PATH = EXPR` a block whose result is the expression. The file
/// has no span. An item marked `#[shadowable]`, `#[def_type]` or `#[attribute]`
/// is preceded by the attribute, as in `#[shadowable] (field arch ...)` or
/// `#[def_type] (def package ...)`, several in that order, then each use of an
/// attribute on it, in source order, its arguments, if it has any, shown as
/// `(…)`, as in `#[retry(…)] (fn f ...)`; and a file marked `#![no_prelude]`
/// has it after `file`, as in `(file #![no_prelude] ...)`; the implicit import
/// a workspace gives a file is not printed (see
/// [Attributes](crate::ast#attributes)). There is no trailing newline.
pub struct Pretty<'a, A> {
    pub(super) ast: &'a Ast,
    pub(super) annotation: &'a A,
}

impl<A: Annotate> Pretty<'_, A> {
    pub(super) fn items(
        &self,
        f: &mut fmt::Formatter<'_>,
        range: ItemRange,
        depth: usize,
    ) -> fmt::Result {
        for &item in self.ast.items(range) {
            f.write_str("\n")?;
            self.item(f, item, depth)?;
        }
        Ok(())
    }

    /// Writes an import item as it is written, without its span.
    pub(super) fn import(&self, f: &mut fmt::Formatter<'_>, decl: &ImportDecl) -> fmt::Result {
        let ast = self.ast;
        let name = |name: ImportName| ast.names().get(name.name);
        f.write_str("import ")?;
        let path = ast.show_import_path(decl);
        match decl.form {
            ImportForm::Whole(alias) => {
                write!(f, "{path}")?;
                if let Some(alias) = alias {
                    write!(f, " as {}", name(alias))?;
                }
            }
            ImportForm::Members(range) => {
                for (index, member) in ast.import_members(range).iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    match *member {
                        ImportMember::Named {
                            name: imported,
                            alias,
                        } => {
                            f.write_str(name(imported))?;
                            if let Some(alias) = alias {
                                write!(f, " as {}", name(alias))?;
                            }
                        }
                        ImportMember::All(_) => f.write_str("*")?,
                        ImportMember::DefType { def, .. } => {
                            write!(f, "{} *", ast.show_def_type(def).0)?;
                        }
                    }
                }
                write!(f, " from {path}")?;
            }
        }
        Ok(())
    }

    /// Writes the attributes of the item `id`, each followed by a space: the
    /// parser's, then the uses of others, in source order.
    pub(super) fn attributes(&self, f: &mut fmt::Formatter<'_>, id: ItemId) -> fmt::Result {
        let ast = self.ast;
        let marks = [
            (ast.is_shadowable(id), "#[shadowable] "),
            (ast.is_def_type(id), "#[def_type] "),
            (ast.is_attribute(id), "#[attribute] "),
            (ast.is_defers(id), "#[defers] "),
        ];
        for (marked, text) in marks {
            if marked {
                f.write_str(text)?;
            }
        }
        for used in ast.attribute_uses(id) {
            let name = ast.names().get(used.name);
            let args = if used.args.is_some() { "(…)" } else { "" };
            write!(f, "#[{name}{args}] ")?;
        }
        Ok(())
    }

    /// Writes the head of the item `id`, a `type`, an `enum` or a variant.
    pub(super) fn type_item(&self, f: &mut fmt::Formatter<'_>, id: ItemId) -> fmt::Result {
        let ast = self.ast;
        match ast.item(id).kind {
            ItemKind::Type { name, ty, .. } => {
                write!(f, "type {} ", ast.names().get(name))?;
                self.type_expr(f, ty)?;
                self.annotation.annotate_type(id, f)
            }
            ItemKind::Enum { name, bound, .. } => {
                write!(f, "enum {} extensible", ast.names().get(name))?;
                match bound {
                    Some(bound) => {
                        f.write_str(" ")?;
                        self.type_expr(f, bound)
                    }
                    None => Ok(()),
                }
            }
            ItemKind::Variant {
                of, name, payload, ..
            } => {
                f.write_str("enum variant ")?;
                self.type_expr(f, of)?;
                write!(f, "::{}", ast.names().get(name))?;
                match payload {
                    Some(payload) => {
                        f.write_str(" ")?;
                        self.type_expr(f, payload)
                    }
                    None => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }

    /// Writes the head of the `let` item `id` of `path`, a `let mut` if
    /// `mutable`, a `fn` if `function`: a method and a context's default
    /// shown by their names, not their bindings'.
    pub(super) fn let_head(
        &self,
        f: &mut fmt::Formatter<'_>,
        id: ItemId,
        (path, mutable, function): (PathRange, bool, bool),
    ) -> fmt::Result {
        let ast = self.ast;
        let keyword = match (mutable, function) {
            (_, true) => "fn",
            (true, false) => "let mut",
            (false, false) => "let",
        };
        let context = ast.contexts().iter().find(|context| context.item == id);
        match (ast.method(id), context) {
            (Some(method), _) => {
                let keyword = if method.public { "def" } else { "fn" };
                write!(f, "{keyword} method {}", ast.names().get(method.name))
            }
            (None, Some(context)) => {
                self.context_head(f, context.name, context.ty)?;
                f.write_str(" =")
            }
            (None, None) => write!(f, "{keyword} {}", ast.show_path(ast.path(path))),
        }
    }

    /// Writes a context's declaration, of `name` and the type `ty`, but its
    /// default: `context arch : String`.
    pub(super) fn context_head(
        &self,
        f: &mut fmt::Formatter<'_>,
        name: Name,
        ty: TypeExprId,
    ) -> fmt::Result {
        write!(f, "context {} : ", self.ast.names().get(name))?;
        self.type_expr(f, ty)
    }

    pub(super) fn item(&self, f: &mut fmt::Formatter<'_>, id: ItemId, depth: usize) -> fmt::Result {
        let item = self.ast.item(id);
        let ast = self.ast;
        write!(f, "{:indent$}", "", indent = depth * 2)?;
        self.attributes(f, id)?;
        f.write_str("(")?;
        let value = match item.kind {
            ItemKind::Let {
                path,
                ty,
                value,
                mutable,
                function,
            } => {
                self.let_head(f, id, (path, mutable, function))?;
                if let Some(ty) = ty {
                    f.write_str(" : ")?;
                    self.type_expr(f, ty)?;
                }
                value
            }
            ItemKind::Field {
                path,
                ty,
                value,
                function,
            } => {
                let keyword = if function { "def" } else { "field" };
                write!(f, "{keyword} {}", ast.show_path(ast.path(path)))?;
                if let Some(ty) = ty {
                    f.write_str(" : ")?;
                    self.type_expr(f, ty)?;
                }
                value
            }
            ItemKind::Rebind { path, value } => {
                write!(f, "rebind {}", ast.show_path(ast.path(path)))?;
                value
            }
            ItemKind::Spread(value) => {
                f.write_str("spread")?;
                value
            }
            ItemKind::Result(value) => {
                f.write_str("result")?;
                value
            }
            ItemKind::Expr(value) => {
                f.write_str("expr")?;
                value
            }
            ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => {
                let keyword = match item.kind {
                    ItemKind::Break(_) => "break",
                    ItemKind::Return(_) => "return",
                    _ => "continue",
                };
                return self.jump(f, (keyword, jump), item.span, depth);
            }
            ItemKind::Import(id) => {
                self.import(f, ast.import_decl(id))?;
                return write!(f, " @{})", item.span);
            }
            ItemKind::Type { .. } | ItemKind::Enum { .. } | ItemKind::Variant { .. } => {
                self.type_item(f, id)?;
                return write!(f, " @{})", item.span);
            }
            ItemKind::Context { name, ty, .. } => {
                self.context_head(f, name, ty)?;
                return write!(f, " @{})", item.span);
            }
            ItemKind::DefTypeDef { def, value } | ItemKind::DefMeta { def, value } => {
                self.def_type_head(f, item.kind, def)?;
                value
            }
            ItemKind::Error => return write!(f, "error @{})", item.span),
        };
        writeln!(f, " @{}", item.span)?;
        // The positions of a `:=` path's index segments, in order, before
        // its value, as they are parsed.
        if let ItemKind::Rebind { path, .. } = item.kind {
            for position in ast.path(path).iter().filter_map(PathSegment::position) {
                writeln!(f, "{:indent$}(at", "", indent = (depth + 1) * 2)?;
                self.node(f, position, depth + 2)?;
                f.write_str(")\n")?;
            }
        }
        self.node(f, value, depth + 1)?;
        f.write_str(")")
    }

    /// Writes what an item of `kind`, a definition made with a def-type or a
    /// definition's meta, whose use of its def-type is `def`, shows before
    /// its span: `def-type-def` or `defmeta`, then its def-type and its path,
    /// each as written.
    pub(super) fn def_type_head(
        &self,
        f: &mut fmt::Formatter<'_>,
        kind: ItemKind,
        def: DefTypeId,
    ) -> fmt::Result {
        let (def_type, path) = self.ast.show_def_type(def);
        let keyword = match kind {
            ItemKind::DefMeta { .. } => "defmeta",
            _ => "def-type-def",
        };
        write!(f, "{keyword} {def_type} {path}")
    }

    /// Writes the rest of a `break` or `continue` item, `keyword`, whose
    /// label and value are `jump`, spanning `span`, at `depth`, after its
    /// `(`: the keyword and the label, as in `break'outer`, its span, and
    /// its value, if it has one, on a line of its own.
    pub(super) fn jump(
        &self,
        f: &mut fmt::Formatter<'_>,
        (keyword, jump): (&str, Jump),
        span: Span,
        depth: usize,
    ) -> fmt::Result {
        f.write_str(keyword)?;
        self.label(f, jump.label)?;
        write!(f, " @{span}")?;
        if let Some(value) = jump.value {
            f.write_str("\n")?;
            self.node(f, value, depth + 1)?;
        }
        f.write_str(")")
    }

    /// Writes `label`, if there is one, as it is written: `'` and its name.
    pub(super) fn label(&self, f: &mut fmt::Formatter<'_>, label: Option<Label>) -> fmt::Result {
        match label {
            Some(label) => write!(f, "'{}", self.ast.names().get(label.name)),
            None => Ok(()),
        }
    }

    /// Writes a type expression as it is written in source: as the parser
    /// made it, whatever the expansion of the tree's types made of it
    /// ([`Ast::written_type`]).
    pub(super) fn type_expr(&self, f: &mut fmt::Formatter<'_>, id: TypeExprId) -> fmt::Result {
        WrittenType {
            ast: self.ast,
            names: self.ast.names(),
        }
        .type_expr(f, id)
    }
}

/// The type expressions of a tree as they are written in source, their
/// names shown with a table that need not be the tree's own: the printer's
/// ([`Pretty`]), and [`Ast::show_type_with`]'s.
pub(super) struct WrittenType<'a> {
    pub(super) ast: &'a Ast,
    pub(super) names: &'a Names,
}

impl WrittenType<'_> {
    /// Writes the qualified type name `path` as it is written.
    pub(super) fn path(&self, f: &mut fmt::Formatter<'_>, path: PathRange) -> fmt::Result {
        let shown = PathDisplay {
            names: self.names,
            strings: &self.ast.strings,
            segments: self.ast.path(path),
            definition: false,
        };
        write!(f, "{shown}")
    }

    /// Writes the type arguments `args` as they are written, `<A, B>`.
    pub(super) fn type_args(&self, f: &mut fmt::Formatter<'_>, args: TypeArgRange) -> fmt::Result {
        f.write_str("<")?;
        for (index, &arg) in self.ast.type_args(args).iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            self.type_expr(f, arg)?;
        }
        f.write_str(">")
    }

    /// Writes the function type of the parameters `params`, the needs
    /// `needs`, the return type `ret` after the marker of `effect`, and the
    /// meta `meta`, as [`WrittenType::type_expr`] writes a type.
    fn fn_type(
        &self,
        f: &mut fmt::Formatter<'_>,
        (params, needs): (ParamTyRange, NeedTyRange),
        (ret, effect, meta): (TypeExprId, Effect, Option<MetaTy>),
    ) -> fmt::Result {
        let names = self.names;
        f.write_str("fn(")?;
        let params = self.ast.param_tys(params);
        for (index, param) in params.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            if param.rest {
                f.write_str("...")?;
            }
            if let Some(name) = param.name {
                write!(f, "{}: ", FieldName(names.get(name)))?;
            }
            self.type_expr(f, param.ty)?;
        }
        for (index, need) in self.ast.need_tys(needs).iter().enumerate() {
            if index > 0 || !params.is_empty() {
                f.write_str(", ")?;
            }
            write!(f, "^{}", names.get(need.name))?;
        }
        f.write_str(effect.params_end())?;
        let Some(meta) = meta else {
            return self.type_expr(f, ret);
        };
        // A return type that is a function type would take the meta.
        self.union_member(f, ret)?;
        write!(f, " {} ", meta.effect.meta_marker())?;
        self.type_expr(f, meta.ty)
    }

    /// Writes a type expression as it is written in source: as the parser
    /// made it, whatever the expansion of the tree's types made of it
    /// ([`Ast::written_type`]).
    pub(super) fn type_expr(&self, f: &mut fmt::Formatter<'_>, id: TypeExprId) -> fmt::Result {
        let names = self.names;
        match self.ast.written_type(id) {
            // An enum, a declaration's definition, is written by its name.
            TypeExprKind::Named(name) | TypeExprKind::Enum { name, .. } => {
                f.write_str(names.get(name))
            }
            TypeExprKind::Context(name) => write!(f, "^{}", self.names.get(name)),
            TypeExprKind::Path(path) => self.path(f, path),
            TypeExprKind::Record(fields) => {
                f.write_str("{")?;
                for (index, field) in self.ast.field_tys(fields).iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    match field.kind {
                        FieldTyKind::Field(name) => write!(f, "{}: ", FieldName(names.get(name)))?,
                        FieldTyKind::Spread => f.write_str("...")?,
                        FieldTyKind::Rest => f.write_str("*: ")?,
                    }
                    self.type_expr(f, field.ty)?;
                    match field.default {
                        Some(FieldDefault::Expr(default)) => {
                            f.write_str(" = ")?;
                            self.default_expr(f, default)?;
                        }
                        Some(FieldDefault::Copied(at)) => {
                            f.write_str(" = ")?;
                            match self.ast.copied_default(at) {
                                Some(constant) => write_constant(f, constant, names)?,
                                None => f.write_str("...")?,
                            }
                        }
                        None => {}
                    }
                }
                f.write_str("}")
            }
            TypeExprKind::Fn {
                params,
                needs,
                ret,
                effect,
                meta,
            } => self.fn_type(f, (params, needs), (ret, effect, meta)),
            TypeExprKind::Applied { name, args, .. } => {
                f.write_str(names.get(name))?;
                self.type_args(f, args)
            }
            TypeExprKind::PathApplied { path, args } => {
                self.path(f, path)?;
                self.type_args(f, args)
            }
            TypeExprKind::Union { lhs, rhs, .. } => {
                self.union_member(f, lhs)?;
                f.write_str(" | ")?;
                self.union_member(f, rhs)
            }
            TypeExprKind::List(element) => {
                // The `[]` would belong to a function type's return type,
                // and bind tighter than a union's `|`.
                if matches!(
                    self.ast.written_type(element),
                    TypeExprKind::Fn { .. } | TypeExprKind::Union { .. }
                ) {
                    f.write_str("(")?;
                    self.type_expr(f, element)?;
                    f.write_str(")[]")
                } else {
                    self.type_expr(f, element)?;
                    f.write_str("[]")
                }
            }
            // Never written: the store's references and the expansion's
            // errors are kept apart from what a tree prints.
            TypeExprKind::Ref(_)
            | TypeExprKind::Instance { .. }
            | TypeExprKind::Param(_)
            | TypeExprKind::Invalid => f.write_str("<invalid>"),
        }
    }

    /// Writes the type expression `id`, a member of a union, in parentheses
    /// if it is a function type, whose return type would otherwise take
    /// what follows.
    pub(super) fn union_member(&self, f: &mut fmt::Formatter<'_>, id: TypeExprId) -> fmt::Result {
        if matches!(self.ast.written_type(id), TypeExprKind::Fn { .. }) {
            f.write_str("(")?;
            self.type_expr(f, id)?;
            f.write_str(")")
        } else {
            self.type_expr(f, id)
        }
    }

    /// Writes the expression `id`, a field's default as the parser made it,
    /// on one line, as a type is written: a literal as a value's `repr`
    /// shows it (an integer or a float as written, `true`, `false`, a
    /// string quoted), `-` before a number, a list literal of such in
    /// brackets, `[1, 2]`, and a record block of fields of one name each,
    /// `{a = 1; b = 2}`, `{}` included; and anything else, which is no
    /// constant (see the checker's "Field defaults"), as `...`.
    ///
    /// Recurses once per level of list and record nesting.
    pub(super) fn default_expr(&self, f: &mut fmt::Formatter<'_>, id: ExprId) -> fmt::Result {
        let ast = self.ast;
        match ast.get(id).kind {
            ExprKind::Int(value) => write!(f, "{value}"),
            ExprKind::Float(value) => write!(f, "{value:?}"),
            ExprKind::Bool(value) => write!(f, "{value}"),
            ExprKind::Str(text) => write!(f, "{}", Quoted(ast.strings.get(text))),
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => {
                f.write_str("-")?;
                self.default_expr(f, operand)
            }
            ExprKind::List(elements) => {
                f.write_str("[")?;
                for (index, element) in ast.elements(elements).iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    if element.spread.is_some() {
                        f.write_str("...")?;
                    }
                    self.default_expr(f, element.value)?;
                }
                f.write_str("]")
            }
            ExprKind::Block { items, label: None } => {
                f.write_str("{")?;
                for (index, &item) in ast.items(items).iter().enumerate() {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    let ItemKind::Field { path, value, .. } = ast.item(item).kind else {
                        f.write_str("...")?;
                        continue;
                    };
                    let name = match ast.path(path) {
                        [field] => field.name(),
                        _ => None,
                    };
                    match name {
                        Some(name) => {
                            write!(f, "{} = ", FieldName(self.names.get(name)))?;
                            self.default_expr(f, value)?;
                        }
                        None => f.write_str("...")?,
                    }
                }
                f.write_str("}")
            }
            _ => f.write_str("..."),
        }
    }
}

impl<A: Annotate> Pretty<'_, A> {
    /// Writes the parameters and return type of the function expression
    /// `id`, each on a line of its own at `depth`, with a leading newline: a
    /// parameter's default, if it has one, is its node, one level deeper,
    /// after its type, `(greeting String` then the node and `)`.
    pub(super) fn fn_head(
        &self,
        f: &mut fmt::Formatter<'_>,
        id: FnId,
        depth: usize,
    ) -> fmt::Result {
        let decl = self.ast.fn_decl(id);
        write!(f, "\n{:indent$}(params", "", indent = depth * 2)?;
        for param in self.ast.params(decl.params) {
            let name = FieldName(self.ast.names().get(param.name));
            // A def-type definition's parameter is written by name alone,
            // its type the def-type's, and so is its result.
            if self.ast.written_type(param.ty) == TypeExprKind::Invalid {
                write!(f, " {name}")?;
                continue;
            }
            let rest = if param.rest { "..." } else { "" };
            write!(f, " ({rest}{name} ")?;
            self.type_expr(f, param.ty)?;
            // A default, on lines of its own after its type.
            if let Some(default) = param.default {
                f.write_str("\n")?;
                self.node(f, default, depth + 1)?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")?;
        if decl.ret.is_none() {
            return Ok(());
        }
        let effect = decl.effect.name();
        write!(f, "\n{:indent$}(ret {effect}", "", indent = depth * 2)?;
        // Written: a definition, which writes none, has returned above.
        if let Some(ret) = decl.ret {
            f.write_str(" ")?;
            self.type_expr(f, ret)?;
        }
        f.write_str(")")
    }

    /// Writes the arguments `range` of a call, each on a line of its own at
    /// `depth`, with a leading newline.
    pub(super) fn args(
        &self,
        f: &mut fmt::Formatter<'_>,
        range: ArgRange,
        depth: usize,
    ) -> fmt::Result {
        for arg in self.ast.args(range) {
            f.write_str("\n")?;
            match arg.kind {
                ArgKind::Positional => self.node(f, arg.value, depth)?,
                ArgKind::Named {
                    shorthand: true, ..
                } => {
                    // Its value is the read of its name, which says it all.
                    write!(f, "{:indent$}(shorthand ", "", indent = depth * 2)?;
                    self.node(f, arg.value, 0)?;
                    f.write_str(")")?;
                }
                ArgKind::Named { name, .. } => {
                    let name = FieldName(self.ast.names().get(name));
                    writeln!(f, "{:indent$}(named {name}", "", indent = depth * 2)?;
                    self.node(f, arg.value, depth + 1)?;
                    f.write_str(")")?;
                }
                ArgKind::Spread { .. } => {
                    writeln!(f, "{:indent$}(spread", "", indent = depth * 2)?;
                    self.node(f, arg.value, depth + 1)?;
                    f.write_str(")")?;
                }
                ArgKind::Trailing => {
                    writeln!(f, "{:indent$}(trailing", "", indent = depth * 2)?;
                    self.node(f, arg.value, depth + 1)?;
                    f.write_str(")")?;
                }
            }
        }
        Ok(())
    }

    /// Writes the elements `range` of a list literal, each on a line of its
    /// own at `depth`, with a leading newline.
    pub(super) fn elements(
        &self,
        f: &mut fmt::Formatter<'_>,
        range: ElementRange,
        depth: usize,
    ) -> fmt::Result {
        for element in self.ast.elements(range) {
            f.write_str("\n")?;
            if element.spread.is_some() {
                writeln!(f, "{:indent$}(spread", "", indent = depth * 2)?;
                self.node(f, element.value, depth + 1)?;
                f.write_str(")")?;
            } else {
                self.node(f, element.value, depth)?;
            }
        }
        Ok(())
    }

    /// Writes the head of the loop `id`: its keyword, `for` or `do`, its
    /// label, if it has one, and, for an iteration, its variable's name, as
    /// in `for x`, `for'rows r`, `for` and `do'again`.
    pub(super) fn for_head(&self, f: &mut fmt::Formatter<'_>, id: ForId) -> fmt::Result {
        let decl = self.ast.for_decl(id);
        f.write_str(match decl.head {
            ForHead::Do { .. } => "do",
            ForHead::Each { .. } | ForHead::While { .. } => "for",
        })?;
        self.label(f, decl.label)?;
        match decl.head {
            ForHead::Each { name, .. } => write!(f, " {}", self.ast.names().get(name)),
            ForHead::While { .. } | ForHead::Do { .. } => Ok(()),
        }
    }

    /// Writes the parts of the loop `id`, each on a line of its own at
    /// `depth`, with a leading newline: for an iteration, its iterable, its
    /// guard, as `(guard EXPR)`, if there is one, and its body; for a
    /// condition loop, the items before its condition, its condition, as
    /// `(cond EXPR)`, if it has one, the items after it and its body; and
    /// for a `do` loop, its body and its condition.
    pub(super) fn for_parts(
        &self,
        f: &mut fmt::Formatter<'_>,
        id: ForId,
        depth: usize,
    ) -> fmt::Result {
        let decl = self.ast.for_decl(id);
        let (first, guard, cond, last) = match decl.head {
            ForHead::Each {
                iterable, guard, ..
            } => (Some(iterable), guard, None, Some(decl.body)),
            ForHead::While {
                before,
                cond,
                after,
            } => {
                for &item in self.ast.items(before) {
                    f.write_str("\n")?;
                    self.item(f, item, depth)?;
                }
                if let Some(cond) = cond {
                    self.wrapped(f, ("cond", cond), depth)?;
                }
                for &item in self.ast.items(after) {
                    f.write_str("\n")?;
                    self.item(f, item, depth)?;
                }
                (Some(decl.body), None, None, None)
            }
            ForHead::Do { cond } => (Some(decl.body), None, Some(cond), None),
        };
        if let Some(first) = first {
            f.write_str("\n")?;
            self.node(f, first, depth)?;
        }
        if let Some(guard) = guard {
            self.wrapped(f, ("guard", guard), depth)?;
        }
        if let Some(cond) = cond {
            self.wrapped(f, ("cond", cond), depth)?;
        }
        if let Some(last) = last {
            f.write_str("\n")?;
            self.node(f, last, depth)?;
        }
        Ok(())
    }

    /// Writes `expr` wrapped as `(WORD EXPR)`, at `depth`, with a leading
    /// newline: a loop's guard or condition.
    pub(super) fn wrapped(
        &self,
        f: &mut fmt::Formatter<'_>,
        (word, expr): (&str, ExprId),
        depth: usize,
    ) -> fmt::Result {
        writeln!(f, "\n{:indent$}({word}", "", indent = depth * 2)?;
        self.node(f, expr, depth + 1)?;
        f.write_str(")")
    }

    /// Writes the head of the node `expr`: what it is, and for a literal,
    /// a name, a field access, an operator, an ascription, an `is` and an
    /// `as`, what it holds.
    pub(super) fn head(&self, f: &mut fmt::Formatter<'_>, expr: &Expr) -> fmt::Result {
        match expr.kind {
            ExprKind::Int(value) => write!(f, "int {value}"),
            ExprKind::Float(value) => write!(f, "float {value:?}"),
            ExprKind::Duration(value) => write!(f, "duration {value}"),
            ExprKind::Size(value) => write!(f, "size {value}"),
            ExprKind::Variant { of, name, .. } => {
                f.write_str("variant ")?;
                WrittenType {
                    ast: self.ast,
                    names: self.ast.names(),
                }
                .type_expr(f, of)?;
                write!(f, "::{}", self.ast.names().get(name))
            }
            ExprKind::Bool(value) => write!(f, "bool {value}"),
            ExprKind::Str(id) => write!(f, "str {}", Quoted(self.ast.strings().get(id))),
            ExprKind::Interp(_) => f.write_str("interp"),
            ExprKind::If { .. } => f.write_str("if"),
            ExprKind::Unary { op, .. } => match op {
                UnaryOp::Neg => f.write_str("neg"),
                UnaryOp::Not => f.write_str("not"),
                UnaryOp::Await => f.write_str("await"),
            },
            ExprKind::Binary { op, .. } => f.write_str(op.symbol()),
            ExprKind::Paren(_) => f.write_str("paren"),
            ExprKind::Name(name) => write!(f, "name {}", self.ast.names().get(name)),
            ExprKind::Block { label, .. } => {
                f.write_str("block")?;
                self.label(f, label.map(|label| self.ast.label(label)))
            }
            ExprKind::Field { name, .. } => {
                write!(f, ". {}", FieldName(self.ast.names().get(name)))
            }
            ExprKind::Ascribe { ty, .. } => {
                f.write_str(": ")?;
                self.type_expr(f, ty)
            }
            ExprKind::Is { ty, .. } => {
                f.write_str("is ")?;
                self.type_expr(f, ty)
            }
            ExprKind::As { ty, .. } => {
                f.write_str("as ")?;
                self.type_expr(f, ty)
            }
            ExprKind::Fn(_) => f.write_str("fn"),
            ExprKind::Call { .. } => f.write_str("call"),
            ExprKind::Lazy(_) => f.write_str("lazy"),
            ExprKind::List(_) => f.write_str("list"),
            ExprKind::Index { .. } => f.write_str("index"),
            ExprKind::For(id) => self.for_head(f, id),
            ExprKind::DefTypeRef(def) => {
                let (def_type, path) = self.ast.show_def_type(def);
                write!(f, "def-type-ref {def_type} {path}")
            }
            ExprKind::ContextRead(name) => write!(f, "context ^{}", self.ast.names().get(name)),
            ExprKind::With { .. } => f.write_str("with"),
            ExprKind::Assert { .. } => f.write_str("assert"),
            ExprKind::Recover(id) => match self.ast.recover_decl(id).error {
                Some((name, _)) => write!(f, "recover ({})", self.ast.names().get(name)),
                None => f.write_str("recover"),
            },
            ExprKind::Error => f.write_str("error"),
            ExprKind::NoField(_) => f.write_str(". <missing>"),
            ExprKind::Unclosed { .. } => f.write_str("unclosed call"),
        }
    }

    /// Writes the def-type reference `id`, of `def`, whose path goes on past
    /// the definition its read's prefix `read` names, at `depth`: as a field
    /// access for each segment past the definition, the outermost spanning
    /// the reference and annotated as it is, around the reference to the
    /// definition, its def-type and the path's prefix, annotated with
    /// [`Annotate::annotate_reference`]; each spanning from the reference's
    /// def-type through its last segment.
    pub(super) fn reference_past(
        &self,
        f: &mut fmt::Formatter<'_>,
        (id, def, read): (ExprId, DefTypeId, ExprId),
        depth: usize,
    ) -> fmt::Result {
        let ast = self.ast;
        let use_ = ast.def_type(def);
        let start = ast.get(use_.dt).span;
        // The accesses past the definition, outermost first.
        let mut accesses = Vec::new();
        let mut at = use_.read.unwrap_or(read);
        while at != read {
            let ExprKind::Field { expr, name } = ast.get(at).kind else {
                break;
            };
            accesses.push((at, name));
            at = expr;
        }
        for (index, &(access, name)) in accesses.iter().enumerate() {
            if index > 0 {
                f.write_str("\n")?;
            }
            let indent = (depth + index) * 2;
            write!(f, "{:indent$}(. {}", "", FieldName(ast.names().get(name)))?;
            if index == 0 {
                self.annotation.annotate(id, f)?;
                write!(f, " @{}", ast.get(id).span)?;
            } else {
                self.annotation.annotate(access, f)?;
                write!(f, " @{}", Span::join(start, ast.get(access).span))?;
            }
        }
        let (def_type, _) = ast.show_def_type(def);
        let segments = ast.path(use_.path);
        let defined = segments.len().saturating_sub(accesses.len());
        let path = ast.show_definition_path(segments.get(..defined).unwrap_or_default());
        if !accesses.is_empty() {
            f.write_str("\n")?;
        }
        write!(
            f,
            "{:indent$}(def-type-ref {def_type} {path}",
            "",
            indent = (depth + accesses.len()) * 2
        )?;
        self.annotation.annotate_reference(id, f)?;
        write!(f, " @{})", Span::join(start, ast.get(read).span))?;
        for _ in &accesses {
            f.write_str(")")?;
        }
        Ok(())
    }

    /// Writes the bindings, `bindings`, and the body, `body`, of a `with`,
    /// each on a line of its own at `depth`: a binding as `(bind NAME`, its
    /// dotted path, then its read of what it updates, if it is dotted, and
    /// its value.
    pub(super) fn with_parts(
        &self,
        f: &mut fmt::Formatter<'_>,
        (bindings, body): (WithRange, ExprId),
        depth: usize,
    ) -> fmt::Result {
        for binding in self.ast.with_bindings(bindings) {
            let name = self.ast.names().get(binding.name);
            write!(f, "\n{:indent$}(bind {name}", "", indent = depth * 2)?;
            for segment in self.ast.path(binding.path) {
                if let Some(field) = segment.name() {
                    write!(f, ".{}", self.ast.names().get(field))?;
                }
            }
            for part in binding.current.into_iter().chain([binding.value]) {
                f.write_str("\n")?;
                self.node(f, part, depth + 1)?;
            }
            f.write_str(")")?;
        }
        f.write_str("\n")?;
        self.node(f, body, depth)
    }

    pub(super) fn node(&self, f: &mut fmt::Formatter<'_>, id: ExprId, depth: usize) -> fmt::Result {
        let expr = self.ast.get(id);
        if let ExprKind::DefTypeRef(def) = expr.kind
            && let Some(read) = self.annotation.reference_past(id)
        {
            return self.reference_past(f, (id, def, read), depth);
        }
        write!(f, "{:indent$}(", "", indent = depth * 2)?;
        self.head(f, expr)?;
        self.annotation.annotate(id, f)?;
        write!(f, " @{}", expr.span)?;
        match expr.kind {
            ExprKind::Fn(function) => {
                self.fn_head(f, function, depth + 1)?;
                f.write_str("\n")?;
                self.node(f, self.ast.fn_decl(function).body, depth + 1)?;
            }
            ExprKind::Lazy(function) => {
                f.write_str("\n")?;
                self.node(f, self.ast.fn_decl(function).body, depth + 1)?;
            }
            ExprKind::Call { callee, args } | ExprKind::Unclosed { callee, args } => {
                f.write_str("\n")?;
                self.node(f, callee, depth + 1)?;
                self.args(f, args, depth + 1)?;
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Duration(_)
            | ExprKind::Size(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Name(_)
            | ExprKind::Variant { .. }
            | ExprKind::DefTypeRef(_)
            | ExprKind::ContextRead(_)
            | ExprKind::Error => {}
            ExprKind::Unary { operand, .. }
            | ExprKind::Paren(operand)
            | ExprKind::Field { expr: operand, .. }
            | ExprKind::NoField(operand)
            | ExprKind::Ascribe { expr: operand, .. }
            | ExprKind::Is { expr: operand, .. }
            | ExprKind::As { expr: operand, .. } => {
                f.write_str("\n")?;
                self.node(f, operand, depth + 1)?;
            }
            ExprKind::With { bindings, body } => self.with_parts(f, (bindings, body), depth + 1)?,
            ExprKind::Assert { cond, message } => {
                for part in [cond, message] {
                    f.write_str("\n")?;
                    self.node(f, part, depth + 1)?;
                }
            }
            ExprKind::Recover(id) => {
                let decl = self.ast.recover_decl(id);
                for part in [decl.body, decl.fallback] {
                    f.write_str("\n")?;
                    self.node(f, part, depth + 1)?;
                }
            }
            ExprKind::Binary { lhs, rhs, .. }
            | ExprKind::Index {
                expr: lhs,
                index: rhs,
            } => {
                f.write_str("\n")?;
                self.node(f, lhs, depth + 1)?;
                f.write_str("\n")?;
                self.node(f, rhs, depth + 1)?;
            }
            ExprKind::List(elements) => self.elements(f, elements, depth + 1)?,
            ExprKind::For(id) => self.for_parts(f, id, depth + 1)?,
            ExprKind::If {
                cond, then, else_, ..
            } => {
                for child in [cond, then, else_] {
                    f.write_str("\n")?;
                    self.node(f, child, depth + 1)?;
                }
            }
            ExprKind::Interp(parts) => {
                for &part in self.ast.parts(parts) {
                    f.write_str("\n")?;
                    match part {
                        StrPart::Lit(text) => write!(
                            f,
                            "{:indent$}(lit {})",
                            "",
                            Quoted(self.ast.strings().get(text)),
                            indent = (depth + 1) * 2
                        )?,
                        StrPart::Expr(expr) => self.node(f, expr, depth + 1)?,
                    }
                }
            }
            ExprKind::Block { items, .. } => self.items(f, items, depth + 1)?,
        }
        f.write_str(")")
    }
}

impl<A: Annotate> fmt::Display for Pretty<'_, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(file")?;
        if self.ast.no_prelude.is_some() {
            f.write_str(" #![no_prelude]")?;
        }
        self.items(f, self.ast.root, 1)?;
        f.write_str(")")
    }
}
