use super::*;
use std::fmt::Write as _;

use crate::MAX_DEPTH;
use crate::ast::{
    ArgKind, Effect, FieldDefault, FieldTyKind, ForHead, ImportForm, ImportId, ImportMember,
    ImportName, ItemKind, Jump, PathRange, SegmentKind, TypeArgRange, TypeExprKind, UnaryOp,
    path_span,
};
use crate::lexer::tokenize;
use crate::names::Name;
use crate::strings::FieldName;
use crate::value::Quantity;

use basics::{on_big_stack, too_deep};
use functions::{labels, only_value};

fn parse_text(text: &str) -> Result<Ast, Diagnostic> {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (tokens, names, strings) = tokenize(&source)?;
    parse(&tokens, names, strings, &source)
}

/// Renders a file compactly, making grouping explicit: items are joined
/// by `; `, binary operations and prefix operations are parenthesised,
/// parens show as `[ ]`, blocks as `{ }`, ascriptions as `(e : T)`,
/// type tests and narrowings as `(e is T)` and `(e as T)`, field
/// accesses as `(e . x)`, fields as `x = e`, strings as `"text"`
/// with interpolations as `<e>`, an `if` as `(if c {..} else ..)`, or
/// `(if c {..})` without `else`, and a `for` as it is written, `for x in
/// xs if g {..}`. Record types show as
/// `{x: T, "a b": T}`.
fn shape(text: &str) -> String {
    let ast = parse_text(text).unwrap();
    let mut out = String::new();
    body(&ast, ast.root(), &mut out);
    out
}

fn body(ast: &Ast, range: ItemRange, out: &mut String) {
    for (index, &id) in ast.items(range).iter().enumerate() {
        if index > 0 {
            out.push_str("; ");
        }
        body_item(ast, id, out);
    }
}

/// One item of a body, as [`shape`] writes it: a `break`, `continue` or
/// `return` as it is written, its label after its keyword and its
/// value after a space.
/// Writes the `break`, `continue` or `return` item `id`, of `jump`.
fn jump_item(ast: &Ast, id: ItemId, jump: Jump, out: &mut String) {
    out.push_str(match ast.item(id).kind {
        ItemKind::Break(_) => "break",
        ItemKind::Return(_) => "return",
        _ => "continue",
    });
    if let Some(label) = jump.label {
        out.push('\'');
        out.push_str(ast.names().get(label.name));
    }
    if let Some(value) = jump.value {
        out.push(' ');
        expr(ast, value, out);
    }
}

fn body_item(ast: &Ast, id: ItemId, out: &mut String) {
    if let Some(context) = ast.contexts().iter().find(|context| context.item == id) {
        return context_item(ast, context, out);
    }
    match ast.item(id).kind {
        ItemKind::Error => out.push_str("<error>"),
        // Shown from its declaration, above.
        ItemKind::Context { .. } => {}
        ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => {
            jump_item(ast, id, jump, out);
        }
        ItemKind::Let {
            path,
            value,
            function: true,
            ..
        }
        | ItemKind::Field {
            path,
            value,
            function: true,
            ..
        } => {
            // As written: the keyword, the name, then the function
            // expression from its `(` on.
            let keyword = match ast.item(id).kind {
                ItemKind::Field { .. } => "def ",
                _ => "fn ",
            };
            out.push_str(keyword);
            path_shape(ast, path, out);
            let mut function = String::new();
            expr(ast, value, &mut function);
            out.push_str(function.strip_prefix("fn").unwrap());
        }
        ItemKind::Let {
            path, ty, value, ..
        } => {
            out.push_str("let ");
            path_shape(ast, path, out);
            if let Some(ty) = ty {
                out.push_str(": ");
                type_name(ast, ty, out);
            }
            out.push_str(" = ");
            expr(ast, value, out);
        }
        ItemKind::Rebind { path, value } => {
            path_shape(ast, path, out);
            out.push_str(" := ");
            expr(ast, value, out);
        }
        ItemKind::Field {
            path, ty, value, ..
        } => {
            path_shape(ast, path, out);
            if let Some(ty) = ty {
                out.push_str(": ");
                type_name(ast, ty, out);
            }
            out.push_str(" = ");
            expr(ast, value, out);
        }
        ItemKind::Spread(value) => {
            out.push_str("...");
            expr(ast, value, out);
        }
        ItemKind::Result(value) => {
            out.push_str("= ");
            expr(ast, value, out);
        }
        ItemKind::Expr(value) => expr(ast, value, out),
        ItemKind::Import(import) => import_shape(ast, import, out),
        ItemKind::Type { name, ty, .. } => {
            out.push_str("type ");
            out.push_str(ast.names().get(name));
            out.push_str(" = ");
            type_name(ast, ty, out);
        }
        ItemKind::Enum { .. } | ItemKind::Variant { .. } => enum_item(ast, id, out),
        ItemKind::DefTypeDef { def, value } => {
            use std::fmt::Write as _;
            // As written: `def`, the def-type, the path, the parameters'
            // names and the body.
            let (def_type, path) = ast.show_def_type(def);
            let ExprKind::Fn(function) = ast.get(value).kind else {
                panic!("a definition's value is not a function");
            };
            let decl = ast.fn_decl(function);
            let params: Vec<&str> = ast
                .params(decl.params)
                .iter()
                .map(|param| ast.names().get(param.name))
                .collect();
            write!(out, "def {def_type} {path}({}) ", params.join(", ")).unwrap();
            expr(ast, decl.body, out);
        }
        ItemKind::DefMeta { def, value } => defmeta_shape(ast, (def, value), out),
    }
}

/// A definition's meta, of the use of its def-type `def` and the function
/// `value`, as written: `defmeta`, the def-type, the path and the body.
fn defmeta_shape(ast: &Ast, (def, value): (crate::ast::DefTypeId, ExprId), out: &mut String) {
    use std::fmt::Write as _;
    let (def_type, path) = ast.show_def_type(def);
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("a meta's value is not a function");
    };
    write!(out, "defmeta {def_type} {path} ").unwrap();
    expr(ast, ast.fn_decl(function).body, out);
}

/// An import item as it is written.
fn import_shape(ast: &Ast, import: ImportId, out: &mut String) {
    let name = |name: ImportName| ast.names().get(name.name).to_owned();
    let as_alias = |alias: Option<ImportName>| alias.map(|a| format!(" as {}", name(a)));
    let decl = ast.import_decl(import);
    let path = ast.show_import_path(decl);
    out.push_str("import ");
    match decl.form {
        ImportForm::Whole(alias) => {
            out.push_str(&path);
            out.push_str(&as_alias(alias).unwrap_or_default());
        }
        ImportForm::Members(range) => {
            let members: Vec<String> = ast
                .import_members(range)
                .iter()
                .map(|member| match *member {
                    ImportMember::Named { name: n, alias } => {
                        name(n) + &as_alias(alias).unwrap_or_default()
                    }
                    ImportMember::All(_) => "*".to_owned(),
                    ImportMember::DefType { def, .. } => format!("{} *", ast.show_def_type(def).0),
                })
                .collect();
            out.push_str(&members.join(", "));
            out.push_str(" from ");
            out.push_str(&path);
        }
    }
}

/// A path as written, each name as [`Ast::show_path`] shows it, and each
/// index segment's position in its shape: `xs[(i + 1)].name`.
fn path_shape(ast: &Ast, path: PathRange, out: &mut String) {
    for (index, segment) in ast.path(path).iter().enumerate() {
        match segment.kind {
            SegmentKind::Name(_) => {
                if index > 0 {
                    out.push('.');
                }
                out.push_str(&ast.show_path(std::slice::from_ref(segment)).to_string());
            }
            SegmentKind::Index { position, .. } => {
                out.push('[');
                expr(ast, position, out);
                out.push(']');
            }
        }
    }
}

/// Writes the item `id`, an enum or a variant, as it is written.
fn enum_item(ast: &Ast, id: ItemId, out: &mut String) {
    let (written, ty) = match ast.item(id).kind {
        ItemKind::Enum { name, bound, .. } => {
            (format!("enum {} extensible", ast.names().get(name)), bound)
        }
        ItemKind::Variant {
            of, name, payload, ..
        } => {
            let mut of_text = String::new();
            type_name(ast, of, &mut of_text);
            (
                format!("enum {of_text}::{}", ast.names().get(name)),
                payload,
            )
        }
        _ => return,
    };
    out.push_str(&written);
    if let Some(ty) = ty {
        out.push('<');
        type_name(ast, ty, out);
        out.push('>');
    }
}

/// Writes an interpolated string, each expression in it in angle brackets.
fn interp_shape(ast: &Ast, parts: crate::ast::PartRange, out: &mut String) {
    out.push('"');
    for &part in ast.parts(parts) {
        match part {
            StrPart::Lit(text) => out.push_str(ast.strings().get(text)),
            StrPart::Expr(inner) => {
                out.push('<');
                expr(ast, inner, out);
                out.push('>');
            }
        }
    }
    out.push('"');
}

/// Writes a variant as a value, `of::name`.
fn variant_expr(ast: &Ast, (of, name): (TypeExprId, Name), out: &mut String) {
    type_name(ast, of, out);
    out.push_str("::");
    out.push_str(ast.names().get(name));
}

fn type_args(ast: &Ast, args: TypeArgRange, out: &mut String) {
    out.push('<');
    for (index, &arg) in ast.type_args(args).iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        type_name(ast, arg, out);
    }
    out.push('>');
}

/// Writes a `recover`, `recover BODY else (NAME) FALLBACK`.
fn recover_shape(ast: &Ast, id: crate::ast::RecoverId, out: &mut String) {
    let decl = ast.recover_decl(id);
    out.push_str("recover ");
    expr(ast, decl.body, out);
    out.push_str(" else ");
    if let Some((name, _)) = decl.error {
        write!(out, "({}) ", ast.names().get(name)).unwrap();
    }
    expr(ast, decl.fallback, out);
}

/// Writes a context's declaration, `context NAME: TYPE = DEFAULT`.
fn context_item(ast: &Ast, context: &crate::ast::ContextDecl, out: &mut String) {
    out.push_str("context ");
    out.push_str(ast.names().get(context.name));
    out.push_str(": ");
    type_name(ast, context.ty, out);
    if let ItemKind::Let { value, .. } = ast.item(context.item).kind
        && let ExprKind::Fn(function) = ast.get(value).kind
    {
        out.push_str(" = ");
        expr(ast, ast.fn_decl(function).body, out);
    }
}

/// Writes a `with`, `(BODY with {NAME = VALUE; NAME.PATH = VALUE})`.
fn with_shape(ast: &Ast, (bindings, body): (crate::ast::WithRange, ExprId), out: &mut String) {
    out.push('(');
    expr(ast, body, out);
    out.push_str(" with {");
    for (index, binding) in ast.with_bindings(bindings).iter().enumerate() {
        if index > 0 {
            out.push_str("; ");
        }
        out.push_str(ast.names().get(binding.name));
        for segment in ast.path(binding.path) {
            if let Some(name) = segment.name() {
                out.push('.');
                out.push_str(ast.names().get(name));
            }
        }
        out.push_str(" = ");
        expr(ast, binding.value, out);
    }
    out.push_str("})");
}

fn type_name(ast: &Ast, id: TypeExprId, out: &mut String) {
    match ast.type_expr(id).kind {
        TypeExprKind::Named(name) => out.push_str(ast.names().get(name)),
        TypeExprKind::Context(name) => write!(out, "^{}", ast.names().get(name)).unwrap(),
        TypeExprKind::Path(path) => out.push_str(&ast.show_path(ast.path(path)).to_string()),
        TypeExprKind::Record(fields) => {
            out.push('{');
            for (index, field) in ast.field_tys(fields).iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                match field.kind {
                    FieldTyKind::Field(name) => {
                        out.push_str(&FieldName(ast.names().get(name)).to_string());
                        out.push_str(": ");
                    }
                    FieldTyKind::Spread => out.push_str("..."),
                    FieldTyKind::Rest => out.push_str("*: "),
                }
                type_name(ast, field.ty, out);
                if let Some(FieldDefault::Expr(default)) = field.default {
                    out.push_str(" = ");
                    expr(ast, default, out);
                }
            }
            out.push('}');
        }
        TypeExprKind::Ref(_)
        | TypeExprKind::Instance { .. }
        | TypeExprKind::Param(_)
        | TypeExprKind::Enum { .. }
        | TypeExprKind::Invalid => out.push('?'),
        TypeExprKind::Fn {
            params,
            needs,
            ret,
            effect,
            meta,
        } => {
            out.push_str("fn(");
            for (index, param) in ast.param_tys(params).iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                if let Some(name) = param.name {
                    out.push_str(ast.names().get(name));
                    out.push_str(": ");
                }
                type_name(ast, param.ty, out);
            }
            for need in ast.need_tys(needs) {
                if out.ends_with('(') {
                    out.push('^');
                } else {
                    out.push_str(", ^");
                }
                out.push_str(ast.names().get(need.name));
            }
            out.push_str(effect.params_end());
            type_name(ast, ret, out);
            if let Some(meta) = meta {
                out.push(' ');
                out.push_str(meta.effect.meta_marker());
                out.push(' ');
                type_name(ast, meta.ty, out);
            }
        }
        TypeExprKind::Applied { name, args, .. } => {
            out.push_str(ast.names().get(name));
            type_args(ast, args, out);
        }
        TypeExprKind::PathApplied { path, args } => {
            out.push_str(&ast.show_path(ast.path(path)).to_string());
            type_args(ast, args, out);
        }
        TypeExprKind::Union { lhs, rhs, .. } => {
            out.push('(');
            type_name(ast, lhs, out);
            out.push_str(" | ");
            type_name(ast, rhs, out);
            out.push(')');
        }
        TypeExprKind::List(element) => {
            out.push('(');
            type_name(ast, element, out);
            out.push_str(")[]");
        }
    }
}

/// The operator between an expression and the type after it, with the
/// spaces around it, as `expr` writes it: ` : `, ` is ` or ` as `.
fn type_operator(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Is { .. } => " is ",
        ExprKind::As { .. } => " as ",
        _ => " : ",
    }
}

/// The duration or size literal `kind`, as GCSL shows its value.
fn quantity(kind: ExprKind) -> String {
    let value = match kind {
        ExprKind::Duration(amount) => crate::value::Value::Quantity(Quantity::Duration, amount),
        ExprKind::Size(amount) => crate::value::Value::Quantity(Quantity::Size, amount),
        _ => panic!("{kind:?} is no quantity"),
    };
    value.to_string()
}

fn expr(ast: &Ast, id: ExprId, out: &mut String) {
    match ast.get(id).kind {
        ExprKind::Int(value) => write!(out, "{value}").unwrap(),
        ExprKind::Float(value) => write!(out, "{value:?}").unwrap(),
        kind @ (ExprKind::Duration(_) | ExprKind::Size(_)) => out.push_str(&quantity(kind)),
        ExprKind::Bool(value) => write!(out, "{value}").unwrap(),
        ExprKind::Str(text) => write!(out, "\"{}\"", ast.strings().get(text)).unwrap(),
        ExprKind::Interp(parts) => interp_shape(ast, parts, out),
        ExprKind::If {
            cond,
            then,
            else_,
            has_else,
        } => {
            out.push_str("(if ");
            expr(ast, cond, out);
            out.push(' ');
            expr(ast, then, out);
            if has_else {
                out.push_str(" else ");
                expr(ast, else_, out);
            } else {
                // The implicit else branch is an empty block with an
                // empty span.
                let empty = ast.get(else_);
                assert_eq!(empty.span.start, empty.span.end);
                assert!(matches!(
                    empty.kind,
                    ExprKind::Block { items, label: None } if ast.items(items).is_empty()
                ));
            }
            out.push(')');
        }
        ExprKind::Name(name) => out.push_str(ast.names().get(name)),
        ExprKind::Unary { op, operand } => {
            out.push('(');
            out.push_str(op.symbol());
            if op != UnaryOp::Neg {
                out.push(' ');
            }
            expr(ast, operand, out);
            out.push(')');
        }
        ExprKind::Lazy(function) => {
            out.push_str("(lazy ");
            expr(ast, ast.fn_decl(function).body, out);
            out.push(')');
        }
        ExprKind::Binary { op, lhs, rhs, .. } => {
            out.push('(');
            expr(ast, lhs, out);
            out.push(' ');
            out.push_str(op.symbol());
            out.push(' ');
            expr(ast, rhs, out);
            out.push(')');
        }
        ExprKind::Paren(inner) => {
            out.push('[');
            expr(ast, inner, out);
            out.push(']');
        }
        ExprKind::Block { items, label } => block_shape(ast, (items, label), out),
        ExprKind::Ascribe { expr: inner, ty }
        | ExprKind::Is { expr: inner, ty }
        | ExprKind::As { expr: inner, ty } => {
            out.push('(');
            expr(ast, inner, out);
            out.push_str(type_operator(ast.get(id).kind));
            type_name(ast, ty, out);
            out.push(')');
        }
        ExprKind::Field { expr: inner, name } => {
            out.push('(');
            expr(ast, inner, out);
            out.push_str(" . ");
            out.push_str(&FieldName(ast.names().get(name)).to_string());
            out.push(')');
        }
        ExprKind::Fn(function) => fn_shape(ast, function, out),
        ExprKind::Call { callee, args } => call_shape(ast, callee, args, out),
        ExprKind::List(elements) => list_shape(ast, elements, out),
        ExprKind::Index { expr: inner, index } => index_shape(ast, (inner, index), out),
        ExprKind::For(id) => for_shape(ast, id, out),
        ExprKind::Variant { of, name } => variant_expr(ast, (of, name), out),
        ExprKind::ContextRead(name) => write!(out, "^{}", ast.names().get(name)).unwrap(),
        ExprKind::Assert { cond, message } => {
            out.push_str("assert ");
            expr(ast, cond, out);
            out.push_str(", ");
            expr(ast, message, out);
        }
        ExprKind::Recover(id) => recover_shape(ast, id, out),
        ExprKind::With { bindings, body } => with_shape(ast, (bindings, body), out),
        ExprKind::DefTypeRef(def) => {
            let (def_type, path) = ast.show_def_type(def);
            write!(out, "({def_type} {path})").unwrap();
        }
        _ => unparsed_shape(ast, id, out),
    }
}

/// What the parser could not parse, `<error>`, or kept of a half-typed
/// field access, `(a . <missing>)`, or call, `<unclosed f(1)>`.
fn unparsed_shape(ast: &Ast, id: ExprId, out: &mut String) {
    match ast.get(id).kind {
        ExprKind::NoField(inner) => {
            out.push('(');
            expr(ast, inner, out);
            out.push_str(" . <missing>)");
        }
        ExprKind::Unclosed { callee, args } => {
            out.push_str("<unclosed ");
            call_shape(ast, callee, args, out);
            out.push('>');
        }
        _ => out.push_str("<error>"),
    }
}

/// A block as `{..}`, its label after its `{` and a space, as in `{'a
/// ..}`.
fn block_shape(
    ast: &Ast,
    (items, label): (ItemRange, Option<crate::ast::LabelId>),
    out: &mut String,
) {
    out.push('{');
    if let Some(label) = label {
        out.push('\'');
        out.push_str(ast.names().get(ast.label(label).name));
        out.push(' ');
    }
    body(ast, items, out);
    out.push('}');
}

/// A loop as it is written, so that it parses back to itself: `for x in
/// xs if g {..}`, without ` if g` if it has no guard; `for BEFORE; c;
/// AFTER {..}`, each item as `body` writes it, `for c {..}` without
/// items and `for {..}` without a condition; and `do {..} for c`; each
/// with its label after its keyword, as in `for'rows`.
fn for_shape(ast: &Ast, id: crate::ast::ForId, out: &mut String) {
    let decl = ast.for_decl(id);
    out.push_str(match decl.head {
        ForHead::Do { .. } => "do",
        ForHead::Each { .. } | ForHead::While { .. } => "for",
    });
    if let Some(label) = decl.label {
        out.push('\'');
        out.push_str(ast.names().get(label.name));
    }
    match decl.head {
        ForHead::Each {
            name,
            iterable,
            guard,
            ..
        } => {
            out.push(' ');
            out.push_str(ast.names().get(name));
            out.push_str(" in ");
            expr(ast, iterable, out);
            if let Some(guard) = guard {
                out.push_str(" if ");
                expr(ast, guard, out);
            }
        }
        ForHead::While {
            before,
            cond,
            after,
        } => {
            for &item in ast.items(before) {
                out.push(' ');
                body_item(ast, item, out);
                out.push(';');
            }
            if let Some(cond) = cond {
                out.push(' ');
                expr(ast, cond, out);
            }
            for &item in ast.items(after) {
                out.push_str("; ");
                body_item(ast, item, out);
            }
        }
        ForHead::Do { cond } => {
            out.push(' ');
            expr(ast, decl.body, out);
            out.push_str(" for ");
            expr(ast, cond, out);
            return;
        }
    }
    out.push(' ');
    expr(ast, decl.body, out);
}

/// An index as `(xs at 0)`.
fn index_shape(ast: &Ast, (list, index): (ExprId, ExprId), out: &mut String) {
    out.push('(');
    expr(ast, list, out);
    out.push_str(" at ");
    expr(ast, index, out);
    out.push(')');
}

/// A list literal as `list(1, ...xs)`.
fn list_shape(ast: &Ast, elements: crate::ast::ElementRange, out: &mut String) {
    out.push_str("list(");
    for (index, element) in ast.elements(elements).iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        if element.spread.is_some() {
            out.push_str("...");
        }
        expr(ast, element.value, out);
    }
    out.push(')');
}

/// A function expression as `fn(x: Int): Int {..}`, `->> Int` if
/// memoised and `-> Int` if impure, a parameter with a default as `x:
/// Int = 1`.
fn fn_shape(ast: &Ast, function: crate::ast::FnId, out: &mut String) {
    let decl = ast.fn_decl(function);
    out.push_str("fn(");
    for (index, param) in ast.params(decl.params).iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(ast.names().get(param.name));
        out.push_str(": ");
        type_name(ast, param.ty, out);
        if let Some(default) = param.default {
            out.push_str(" = ");
            expr(ast, default, out);
        }
    }
    out.push_str(decl.effect.params_end());
    type_name(ast, decl.ret.unwrap(), out);
    out.push(' ');
    expr(ast, decl.body, out);
}

/// A call as `callee(1, y = 2)`, followed by its trailing block if it
/// has one: `callee(1){..}`. A call with only a trailing block, however
/// written, is `callee(){..}`.
fn call_shape(ast: &Ast, callee: ExprId, args: crate::ast::ArgRange, out: &mut String) {
    expr(ast, callee, out);
    out.push('(');
    let mut trailing = None;
    for (index, arg) in ast.args(args).iter().enumerate() {
        match arg.kind {
            ArgKind::Trailing => {
                trailing = Some(arg.value);
                continue;
            }
            ArgKind::Positional | ArgKind::Named { .. } | ArgKind::Spread { .. } if index > 0 => {
                out.push_str(", ");
            }
            ArgKind::Positional | ArgKind::Named { .. } | ArgKind::Spread { .. } => {}
        }
        if let ArgKind::Spread { .. } = arg.kind {
            out.push_str("...");
        }
        if let ArgKind::Named {
            shorthand: true, ..
        } = arg.kind
        {
            // Its value is the read of its name: written once.
            expr(ast, arg.value, out);
            continue;
        }
        if let ArgKind::Named { name, .. } = arg.kind {
            out.push_str(ast.names().get(name));
            out.push_str(" = ");
        }
        expr(ast, arg.value, out);
    }
    out.push(')');
    if let Some(block) = trailing {
        expr(ast, block, out);
    }
}

fn error(text: &str) -> (String, Span) {
    let diagnostic = parse_text(text).unwrap_err();
    (diagnostic.message, diagnostic.span)
}

fn message(text: &str) -> String {
    parse_text(text).unwrap_err().message
}

// ----- Functions and calls -----

// ----- Futures -----

// ----- Lists -----

// ----- Ranges and `for` -----

// ----- Errors and recovery -----

mod basics;
mod conditions;
mod extensions;
mod functions;
mod imports;
mod items;
mod lists;
mod loops;
mod mutation;
mod records;
mod recovery;
