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
    let path = ast.show_module_path(ast.path(decl.path));
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
            ArgKind::Positional | ArgKind::Named { .. } if index > 0 => {
                out.push_str(", ");
            }
            ArgKind::Positional | ArgKind::Named { .. } => {}
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

#[test]
fn literals() {
    assert_eq!(shape("42"), "42");
    assert_eq!(shape("2.5"), "2.5");
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    assert_eq!(shape("1 + 2 * 3"), "(1 + (2 * 3))");
    assert_eq!(shape("1 * 2 + 3"), "((1 * 2) + 3)");
    assert_eq!(shape("1 - 6 / 3 // 2 % 5"), "(1 - (((6 / 3) // 2) % 5))");
}

#[test]
fn binary_operators_are_left_associative() {
    assert_eq!(shape("8 - 3 - 2"), "((8 - 3) - 2)");
    assert_eq!(shape("1 + 2 - 3 + 4"), "(((1 + 2) - 3) + 4)");
    assert_eq!(shape("2 * 3 // 4"), "((2 * 3) // 4)");
    assert_eq!(shape("8 / 4 / 2"), "((8 / 4) / 2)");
    assert_eq!(shape("9 % 5 * 2"), "((9 % 5) * 2)");
}

#[test]
fn unary_minus_binds_tighter_than_multiplication() {
    assert_eq!(shape("-2 * 3"), "((-2) * 3)");
    assert_eq!(shape("-2 + 3"), "((-2) + 3)");
    assert_eq!(shape("-x * y"), "((-x) * y)");
}

#[test]
fn nested_unary_minus() {
    // Negating a negation needs parentheses: `--2` and `--x` are lexical
    // errors (reported through `parse_text`, which lexes first), and
    // `- - -2` starts with a subtraction `-`.
    assert_eq!(shape("-(-2)"), "(-[(-2)])");
    assert_eq!(shape("-(-x)"), "(-[(-x)])");
    assert_eq!(
        error("- - -2"),
        ("expected expression, found `-`".to_owned(), Span::new(0, 1))
    );
    for text in ["--2", "--x", "1 + --x"] {
        let diagnostic = parse_text(text).unwrap_err();
        assert_eq!(diagnostic.message, "unexpected character `-`", "{text}");
    }
    assert_eq!(error("--x").1, Span::new(0, 1));
}

#[test]
fn spaced_minus_cannot_start_an_expression() {
    assert_eq!(
        error("- 2 * 3"),
        ("expected expression, found `-`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("1 * - 2"),
        ("expected expression, found `-`".to_owned(), Span::new(4, 5))
    );
}

#[test]
fn unary_minus_inside_binary() {
    assert_eq!(shape("2 * -3"), "(2 * (-3))");
    assert_eq!(shape("2 - -3"), "(2 - (-3))");
}

#[test]
fn negation_directly_after_an_item_is_reported_as_a_likely_subtraction() {
    let diagnostic = parse_text("a -b").unwrap_err();
    assert_eq!(diagnostic.message, "expected end of item, found `-b`");
    assert_eq!(diagnostic.span, Span::new(2, 4));
    assert_eq!(
        diagnostic.notes,
        vec!["write `a - b` with spaces for subtraction"]
    );
    let diagnostic = parse_text("let x = 1 -2.5").unwrap_err();
    assert_eq!(diagnostic.message, "expected end of item, found `-2.5`");
    assert_eq!(diagnostic.span, Span::new(10, 14));
    let diagnostic = parse_text("{ x -hello-world }").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected end of item, found `-hello-world`"
    );
}

#[test]
fn parentheses_group() {
    assert_eq!(shape("(1 + 2) * 3"), "([(1 + 2)] * 3)");
    assert_eq!(shape("((1))"), "[[1]]");
    assert_eq!(shape("2 * (3 - (4 + 5))"), "(2 * [(3 - [(4 + 5)])])");
    assert_eq!(shape("-(1 + 2)"), "(-[(1 + 2)])");
}

#[test]
fn names() {
    assert_eq!(shape("x"), "x");
    assert_eq!(shape("hello-world + x-1"), "(hello-world + x-1)");
    assert_eq!(shape("x - 1"), "(x - 1)");
}

#[test]
fn items_are_separated_by_newlines_and_semicolons() {
    assert_eq!(shape("let x = 1\nx"), "let x = 1; x");
    assert_eq!(shape("let x = 1; x"), "let x = 1; x");
    assert_eq!(shape("let x = 1\nx := 2; = x"), "let x = 1; x := 2; = x");
    // Blank lines, comment-only lines and repeated separators.
    assert_eq!(
        shape("\n\n# comment\nlet x = 1;;\n\n  ;\n# another\n x ;\n"),
        "let x = 1; x"
    );
    assert_eq!(shape(";"), "");
    assert_eq!(shape(""), "");
    assert_eq!(shape("# only a comment"), "");
}

#[test]
fn items_continue_after_a_trailing_operator() {
    for op in ["+", "-", "*", "/", "//", "%"] {
        let text = format!("1 {op}\n\n  2");
        assert_eq!(shape(&text), format!("(1 {op} 2)"), "{text}");
    }
    assert_eq!(shape("1 :\nFloat"), "(1 : Float)");
    // A comment after the operator does not end the item either.
    assert_eq!(shape("1 + # more\n2"), "(1 + 2)");
}

#[test]
fn items_continue_after_eq_colon_eq_and_let() {
    assert_eq!(shape("let x =\n1"), "let x = 1");
    assert_eq!(shape("let\nx = 1"), "let x = 1");
    assert_eq!(shape("let x:\nInt = 1"), "let x: Int = 1");
    assert_eq!(shape("x :=\n\n1"), "x := 1");
    assert_eq!(shape("=\n1"), "= 1");
}

#[test]
fn newlines_elsewhere_end_the_item() {
    // A leading operator on the next line does not continue the item.
    assert_eq!(
        error("1\n+ 2"),
        ("expected expression, found `+`".to_owned(), Span::new(2, 3))
    );
    assert_eq!(
        error("let x\n= 1"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(5, 6)
        )
    );
}

#[test]
fn items_continue_inside_parentheses() {
    assert_eq!(shape("(1\n+\n2)\n3"), "[(1 + 2)]; 3");
    assert_eq!(shape("let x = (\n  1\n)"), "let x = [1]");
}

#[test]
fn blocks() {
    assert_eq!(shape("{}"), "{}");
    assert_eq!(shape("{ 1 }"), "{1}");
    assert_eq!(
        shape("let x = {\n  let y = 1\n  y + 1\n}"),
        "let x = {let y = 1; (y + 1)}"
    );
    assert_eq!(shape("{ {} ; { = 1 } }"), "{{}; {= 1}}");
    assert_eq!(shape("{{{ 1 }}}"), "{{{1}}}");
    assert_eq!(shape("1 + { 2 } * 3"), "(1 + ({2} * 3))");
    let ast = parse_text("x + {\n1 }").unwrap();
    let root = ast.items(ast.root())[0];
    let ItemKind::Expr(sum) = ast.item(root).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Binary { rhs, .. } = ast.get(sum).kind else {
        panic!("expected a binary node");
    };
    // A block's span covers its braces.
    assert_eq!(ast.get(rhs).span, Span::new(4, 9));
}

#[test]
fn result_items_parse_anywhere_in_a_body() {
    // Whether `= e` is the last item is checked by the checker, which
    // can point at the item after it.
    assert_eq!(shape("= 1; 2"), "= 1; 2");
    assert_eq!(shape("= 1; = 2"), "= 1; = 2");
}

#[test]
fn eq_is_only_accepted_at_the_start_of_an_item() {
    // Or after a field's name.
    assert_eq!(
        error("(x) = 1"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(4, 5)
        )
    );
    assert_eq!(
        error("x + 1 = 1"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("1 + = 2"),
        ("expected expression, found `=`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("(= 1)"),
        ("expected expression, found `=`".to_owned(), Span::new(1, 2))
    );
    assert_eq!(
        error("let x = = 1"),
        ("expected expression, found `=`".to_owned(), Span::new(8, 9))
    );
}

#[test]
fn rebind_needs_a_bare_name() {
    assert_eq!(
        error("(x) := 1"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(4, 6)
        )
    );
    assert_eq!(
        error("x + y := 1"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(6, 8)
        )
    );
    // `: =` is an ascription missing its type, not a rebind.
    assert_eq!(
        error("x : = 1"),
        ("expected a type".to_owned(), Span::new(4, 5))
    );
}

#[test]
fn item_spans() {
    let ast = parse_text("let x: Int = 1 + 2\nx := 3\n= x\nx").unwrap();
    let spans: Vec<Span> = ast
        .items(ast.root())
        .iter()
        .map(|&id| ast.item(id).span)
        .collect();
    assert_eq!(
        spans,
        vec![
            Span::new(0, 18),
            Span::new(19, 25),
            Span::new(26, 29),
            Span::new(30, 31)
        ]
    );
    let ItemKind::Let { path, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a let");
    };
    assert_eq!(ast.path(path)[0].span, Span::new(4, 5));
}

#[test]
fn ascription_binds_between_multiplication_and_negation() {
    assert_eq!(shape("1 + 2 : Float"), "(1 + (2 : Float))");
    assert_eq!(shape("-5 : Float"), "((-5) : Float)");
    assert_eq!(shape("2 * 3 : Float"), "(2 * (3 : Float))");
    assert_eq!(shape("(1 + 2) : Float"), "([(1 + 2)] : Float)");
    assert_eq!(shape("x : Int : Int"), "((x : Int) : Int)");
    assert_eq!(shape("x : Int + 1"), "((x : Int) + 1)");
}

#[test]
fn ascription_shapes_are_exact() {
    // `1 + 2 : Float` is Add(1, Ascribe(2)).
    let ast = parse_text("1 + 2 : Float").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Binary {
        op: BinaryOp::Add,
        lhs,
        rhs,
        ..
    } = ast.get(root).kind
    else {
        panic!("expected an addition, got {:?}", ast.get(root));
    };
    assert_eq!(ast.get(lhs).kind, ExprKind::Int(1));
    let ExprKind::Ascribe { expr, ty } = ast.get(rhs).kind else {
        panic!("expected an ascription, got {:?}", ast.get(rhs));
    };
    assert_eq!(ast.get(expr).kind, ExprKind::Int(2));
    assert_eq!(ast.get(rhs).span, Span::new(4, 13));
    assert_eq!(ast.type_expr(ty).span, Span::new(8, 13));

    // `-5 : Float` is Ascribe(Neg(5)).
    let ast = parse_text("-5 : Float").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Ascribe { expr, .. } = ast.get(root).kind else {
        panic!("expected an ascription, got {:?}", ast.get(root));
    };
    let ExprKind::Unary {
        op: UnaryOp::Neg,
        operand,
    } = ast.get(expr).kind
    else {
        panic!("expected a negation, got {:?}", ast.get(expr));
    };
    assert_eq!(ast.get(operand).kind, ExprKind::Int(5));
}

#[test]
fn spans() {
    let ast = parse_text("-(1 + 2) * 30").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let root = ast.get(root);
    assert_eq!(root.span, Span::new(0, 13));
    let ExprKind::Binary { lhs, rhs, .. } = root.kind else {
        panic!("expected binary, got {root:?}");
    };
    let neg = ast.get(lhs);
    assert_eq!(neg.span, Span::new(0, 8));
    let ExprKind::Unary { operand, .. } = neg.kind else {
        panic!("expected unary, got {neg:?}");
    };
    let paren = ast.get(operand);
    assert_eq!(paren.span, Span::new(1, 8));
    let ExprKind::Paren(sum) = paren.kind else {
        panic!("expected paren, got {paren:?}");
    };
    assert_eq!(ast.get(sum).span, Span::new(2, 7));
    assert_eq!(ast.get(rhs).span, Span::new(11, 13));
}

#[test]
fn expected_expression() {
    assert_eq!(
        error("1 +"),
        (
            "expected expression, found end of input".to_owned(),
            Span::new(3, 3)
        )
    );
    assert_eq!(
        error("* 2"),
        ("expected expression, found `*`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("()"),
        ("expected expression, found `)`".to_owned(), Span::new(1, 2))
    );
    assert_eq!(
        error("1 + // 2"),
        (
            "expected expression, found `//`".to_owned(),
            Span::new(4, 6)
        )
    );
    assert_eq!(
        error("1 + let"),
        (
            "expected expression, found `let`".to_owned(),
            Span::new(4, 7)
        )
    );
    assert_eq!(
        error("{ 1 + }"),
        ("expected expression, found `}`".to_owned(), Span::new(6, 7))
    );
    assert_eq!(
        error("x := ;"),
        ("expected expression, found `;`".to_owned(), Span::new(5, 6))
    );
}

#[test]
fn expected_close_paren() {
    assert_eq!(
        error("(1 + 2"),
        (
            "expected `)`, found end of input".to_owned(),
            Span::new(6, 6)
        )
    );
    let diagnostic = parse_text("(1 2)").unwrap_err();
    assert_eq!(diagnostic.message, "expected `)`, found `2`");
    assert_eq!(diagnostic.span, Span::new(3, 4));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Span::new(0, 1));
    assert_eq!(diagnostic.labels[0].text, "unclosed `(`");
    assert_eq!(message("(1; 2)"), "expected `)`, found `;`");
}

#[test]
fn unterminated_block() {
    let diagnostic = parse_text("let x = {\n  1\n").unwrap_err();
    assert_eq!(diagnostic.message, "expected `}`, found end of input");
    assert_eq!(diagnostic.span, Span::new(13, 13));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Span::new(8, 9));
    assert_eq!(diagnostic.labels[0].text, "unclosed `{`");
    // The innermost unclosed brace is the one reported.
    let diagnostic = parse_text("{ { }").unwrap_err();
    assert_eq!(diagnostic.labels[0].span, Span::new(0, 1));
    let diagnostic = parse_text("{ {").unwrap_err();
    assert_eq!(diagnostic.labels[0].span, Span::new(2, 3));
}

#[test]
fn stray_close_brace_at_top_level() {
    assert_eq!(error("}"), ("unexpected `}`".to_owned(), Span::new(0, 1)));
    assert_eq!(error("1 }"), ("unexpected `}`".to_owned(), Span::new(2, 3)));
    assert_eq!(
        error("{ 1 }\n}"),
        ("unexpected `}`".to_owned(), Span::new(6, 7))
    );
}

#[test]
fn let_errors() {
    assert_eq!(
        error("let x 5"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("let x: Int 5"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(11, 12)
        )
    );
    assert_eq!(
        error("let x"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(5, 5)
        )
    );
    assert_eq!(
        error("let 5 = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("let let = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 7))
    );
    assert_eq!(
        error("let = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("let"),
        ("expected a name after `let`".to_owned(), Span::new(3, 3))
    );
}

#[test]
fn expected_a_type() {
    assert_eq!(
        error("let x: = 1"),
        ("expected a type".to_owned(), Span::new(7, 8))
    );
    assert_eq!(
        error("1 : 2"),
        ("expected a type".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("1 :"),
        ("expected a type".to_owned(), Span::new(3, 3))
    );
    // A type may be parenthesised, and what is inside must be a type.
    assert_eq!(shape("1 : (Int)"), "(1 : Int)");
    assert_eq!(
        error("1 : (2)"),
        ("expected a type".to_owned(), Span::new(5, 6))
    );
}

#[test]
fn binary_nodes_record_their_operator() {
    let ast = parse_text("1 + 2 // 3").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let root = ast.get(root);
    assert_eq!(root.operator_span(), Some(Span::new(2, 3)));
    let ExprKind::Binary { rhs, .. } = root.kind else {
        panic!("expected binary, got {root:?}");
    };
    assert_eq!(ast.get(rhs).operator_span(), Some(Span::new(6, 8)));
}

#[test]
fn expected_end_of_item() {
    assert_eq!(
        error("1 2"),
        (
            "expected end of item, found `2`".to_owned(),
            Span::new(2, 3)
        )
    );
    assert_eq!(
        error("1 1_000.5e3"),
        (
            "expected end of item, found `1_000.5e3`".to_owned(),
            Span::new(2, 11)
        )
    );
    assert_eq!(
        error("(1))"),
        (
            "expected end of item, found `)`".to_owned(),
            Span::new(3, 4)
        )
    );
    // (`2 (3)` is a call of `2`, which the checker rejects.)
    assert_eq!(
        error("f(3) x"),
        (
            "expected end of item, found `x`".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("{ x 1 }"),
        (
            "expected end of item, found `1`".to_owned(),
            Span::new(4, 5)
        )
    );
    assert_eq!(
        error("let x = 1 let y = 2"),
        (
            "expected end of item, found `let`".to_owned(),
            Span::new(10, 13)
        )
    );
}

#[test]
fn pretty_output_for_a_small_program() {
    let ast = parse_text("let x: Float = 1\ny := { x : Float }\n= -x").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let x : Float @0..16\n\
             \x20   (int 1 @15..16))\n\
             \x20 (rebind y @17..35\n\
             \x20   (block @22..35\n\
             \x20     (expr @24..33\n\
             \x20       (: Float @24..33\n\
             \x20         (name x @24..25)))))\n\
             \x20 (result @36..40\n\
             \x20   (neg @38..40\n\
             \x20     (name x @39..40))))"
    );
}

/// Runs `f` on a thread with [`crate::STACK_SIZE`] of stack, as the
/// pipeline is meant to be run. Test threads are far smaller by default.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn too_deep(text: String) -> Diagnostic {
    on_big_stack(move || parse_text(&text).unwrap_err())
}

#[test]
fn nesting_by_recursion_is_limited() {
    // MAX_DEPTH parentheses around a literal is MAX_DEPTH + 1 levels.
    let just_over = format!("{}1{}", "(".repeat(MAX_DEPTH), ")".repeat(MAX_DEPTH));
    let diagnostic = too_deep(just_over);
    assert_eq!(diagnostic.message, "expression nested too deeply");
    assert_eq!(
        diagnostic.notes,
        vec![format!("the limit is {MAX_DEPTH} levels")]
    );
    let deepest = format!(
        "{}1{}",
        "(".repeat(MAX_DEPTH - 1),
        ")".repeat(MAX_DEPTH - 1)
    );
    on_big_stack(move || parse_text(&deepest).unwrap());
    // Negations as `-(-(...))`: each pair is two levels.
    let pairs = MAX_DEPTH / 2;
    let negations = format!("{}1{}", "-(".repeat(pairs), ")".repeat(pairs));
    too_deep(negations);
}

#[test]
fn nesting_blocks_is_limited() {
    // Each block is two levels of recursion: the block's body and the
    // expression inside it. With the outermost expression, N blocks are
    // 2N + 1 levels.
    let deepest_blocks = (MAX_DEPTH - 1) / 2;
    let deepest = format!(
        "{}1{}",
        "{".repeat(deepest_blocks),
        "}".repeat(deepest_blocks)
    );
    on_big_stack(move || parse_text(&deepest).unwrap());
    let just_over = format!(
        "{}1{}",
        "{".repeat(deepest_blocks + 1),
        "}".repeat(deepest_blocks + 1)
    );
    let diagnostic = too_deep(just_over);
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // Items inside the blocks do not change the count.
    let with_items = format!(
        "{}1{}",
        "{ let x = 1\n".repeat(deepest_blocks + 1),
        "}".repeat(deepest_blocks + 1)
    );
    too_deep(with_items);
}

#[test]
fn blocks_count_towards_height() {
    // A chain one short of the limit, inside one block, is at the limit;
    // inside two, it is over.
    let chain = format!("1{}", " + 1".repeat(MAX_DEPTH - 2));
    let one = format!("{{ {chain} }}");
    on_big_stack(move || parse_text(&one).unwrap());
    let two = format!("{{ {{ {chain} }} }}");
    let diagnostic = too_deep(two);
    assert_eq!(diagnostic.message, "expression nested too deeply");
}

#[test]
fn nesting_by_left_chaining_is_limited() {
    // Each `+` is one level above its left operand, so N terms are N
    // levels deep. This nests without recursion in the parser.
    let just_over = format!("1{}", " + 1".repeat(MAX_DEPTH));
    let diagnostic = too_deep(just_over);
    assert_eq!(diagnostic.message, "expression nested too deeply");
    let deepest = format!("1{}", " + 1".repeat(MAX_DEPTH - 1));
    on_big_stack(move || parse_text(&deepest).unwrap());
    // Ascriptions chain the same way.
    let ascriptions = format!("1{}", " : Int".repeat(MAX_DEPTH));
    too_deep(ascriptions);
}

#[test]
fn precedence_of_logic_and_comparison() {
    assert_eq!(shape("not a == b"), "(not (a == b))");
    assert_eq!(shape("a + 1 < b"), "((a + 1) < b)");
    assert_eq!(shape("a or b and c"), "(a or (b and c))");
    assert_eq!(shape("not not a"), "(not (not a))");
    assert_eq!(shape("-x < y : Int"), "((-x) < (y : Int))");
    assert_eq!(shape("not a and b"), "((not a) and b)");
    assert_eq!(shape("a and b or c and d"), "((a and b) or (c and d))");
    assert_eq!(shape("a or b or c"), "((a or b) or c)");
    assert_eq!(shape("a and b and c"), "((a and b) and c)");
    assert_eq!(shape("a == b and c != d"), "((a == b) and (c != d))");
    assert_eq!(shape("1 + 2 * 3 >= 7 // 2"), "((1 + (2 * 3)) >= (7 // 2))");
    assert_eq!(shape("not -x <= y"), "(not ((-x) <= y))");
    assert_eq!(shape("true or false"), "(true or false)");
    assert_eq!(
        shape("\"a\" + \"b\" == \"ab\""),
        "((\"a\" + \"b\") == \"ab\")"
    );
    // Every comparison operator, total equality included, at the same
    // level.
    for op in ["==", "!=", "===", "!==", "<", "<=", ">", ">="] {
        assert_eq!(shape(&format!("a {op} b")), format!("(a {op} b)"));
        assert_eq!(
            shape(&format!("a + 1 {op} b and c")),
            format!("(((a + 1) {op} b) and c)")
        );
    }
    assert_eq!(shape("not a === b"), "(not (a === b))");
}

#[test]
fn is_binds_as_comparisons_and_as_as_ascription() {
    // `is` takes in arithmetic, and is taken in by `not` and `and`.
    assert_eq!(shape("x + 1 is Int"), "((x + 1) is Int)");
    assert_eq!(shape("not x is Int"), "(not (x is Int))");
    assert_eq!(
        shape("x is Int and y is String"),
        "((x is Int) and (y is String))"
    );
    assert_eq!(shape("x is Int | String"), "(x is (Int | String))");
    assert_eq!(shape("x.a is {a: Int}"), "((x . a) is {a: Int})");
    // `as` binds as `:` does, and chains with it.
    assert_eq!(shape("a + b as Int"), "(a + (b as Int))");
    assert_eq!(shape("-x as Int"), "((-x) as Int)");
    assert_eq!(shape("x as Int : Int"), "((x as Int) : Int)");
    assert_eq!(shape("x : Int as Int"), "((x : Int) as Int)");
    assert_eq!(shape("x as Int is Int"), "((x as Int) is Int)");
    assert_eq!(shape("x as Int == 1"), "((x as Int) == 1)");
    // In a condition, the block after the type is the `if`'s.
    assert_eq!(shape("if x is Int { x }"), "(if (x is Int) {x})");
    assert_eq!(
        shape("if x is {a: Int} { x } else { 0 }"),
        "(if (x is {a: Int}) {x} else {0})"
    );
    // A newline after the keyword continues the item.
    assert_eq!(shape("x is\n  Int"), "(x is Int)");
    assert_eq!(shape("x as\n  Int"), "(x as Int)");
    let ast = parse_text("x is Int").unwrap();
    let printed = ast.pretty().to_string();
    assert!(printed.contains("(is Int @0..8"), "{printed}");
    let ast = parse_text("x as Int").unwrap();
    assert!(ast.pretty().to_string().contains("(as Int @0..8"));
}

#[test]
fn is_does_not_chain_with_comparisons() {
    for (text, at) in [
        ("a == b is Int", Span::new(7, 9)),
        ("x is Int == y", Span::new(9, 11)),
        ("x is Int is Bool", Span::new(9, 11)),
        ("a < b is Bool", Span::new(6, 8)),
    ] {
        assert_eq!(
            error(text),
            ("comparison operators cannot be chained".to_owned(), at),
            "{text}"
        );
    }
    assert_eq!(shape("(a == b) is Bool"), "([(a == b)] is Bool)");
    // Neither keyword can be a name.
    assert_eq!(error("let is = 1").0, "expected a name after `let`");
    assert_eq!(error("as").0, "expected expression, found `as`");
}

#[test]
fn written_unions_of_any_members() {
    for (text, shown) in [
        ("let x: Int | String = y", "let x: (Int | String) = y"),
        (
            "let x: {a: Int} | List<Int> | Bool = y",
            "let x: (({a: Int} | List<Int>) | Bool) = y",
        ),
        (
            "let x: (fn(): Int) | Int = y",
            "let x: (fn(): Int | Int) = y",
        ),
        (
            "let x: Int | String | Future<Int | String> = y",
            "let x: ((Int | String) | Future<(Int | String)>) = y",
        ),
    ] {
        assert_eq!(shape(text), shown, "{text}");
    }
}

#[test]
fn comparisons_cannot_be_chained() {
    let diagnostic = parse_text("a < b < c").unwrap_err();
    assert_eq!(diagnostic.message, "comparison operators cannot be chained");
    assert_eq!(diagnostic.span, Span::new(6, 7));
    assert_eq!(
        diagnostic.notes,
        vec!["write `a < b and b < c`, or add parentheses"]
    );
    assert_eq!(error("a == b != c").1, Span::new(7, 9));
    assert_eq!(error("1 + 2 < 3 == true").1, Span::new(10, 12));
    assert_eq!(error("x and a >= b > c").1, Span::new(13, 14));
    assert_eq!(error("not a < b <= c").1, Span::new(10, 12));
    // Total equality is non-associative too, with every comparison.
    assert_eq!(error("a === b === c").1, Span::new(8, 11));
    assert_eq!(error("a !== b == c").1, Span::new(8, 10));
    assert_eq!(error("a == b !== c").1, Span::new(7, 10));
    assert_eq!(shape("(a === b) !== c"), "([(a === b)] !== c)");
    // Parentheses and logic operators separate comparisons.
    assert_eq!(shape("(a < b) == c"), "([(a < b)] == c)");
    assert_eq!(shape("a == (b < c)"), "(a == [(b < c)])");
    assert_eq!(shape("a < b and b < c"), "((a < b) and (b < c))");
}

#[test]
fn items_continue_after_logic_and_comparison_operators() {
    for op in ["==", "!=", "===", "!==", "<", "<=", ">", ">=", "and", "or"] {
        let text = format!("a {op}\n\n  b");
        assert_eq!(shape(&text), format!("(a {op} b)"), "{text}");
    }
}

#[test]
fn if_shapes() {
    assert_eq!(shape("if a { 1 } else { 2 }"), "(if a {1} else {2})");
    assert_eq!(
        shape("if a { 1 } else if b { 2 } else { 3 }"),
        "(if a {1} else (if b {2} else {3}))"
    );
    assert_eq!(
        shape("if a { 1 } else if b { 2 }"),
        "(if a {1} else (if b {2}))"
    );
    assert_eq!(shape("if a { 1 }"), "(if a {1})");
    assert_eq!(shape("if a {}"), "(if a {})");
    assert_eq!(shape("if a == b and c { x }"), "(if ((a == b) and c) {x})");
    assert_eq!(shape("if { a } { 1 }"), "(if {a} {1})");
    assert_eq!(
        shape("let x = if a { 1 } else { 2 }"),
        "let x = (if a {1} else {2})"
    );
    // An `if` is an operand like any other prefix expression.
    assert_eq!(
        shape("1 + if a { 2 } else { 3 }"),
        "(1 + (if a {2} else {3}))"
    );
    assert_eq!(
        shape("if a { 1 } else { 2 } * 3"),
        "((if a {1} else {2}) * 3)"
    );
    assert_eq!(
        shape("if a { 1 } else { 2 } : Float"),
        "((if a {1} else {2}) : Float)"
    );
    assert_eq!(
        shape("if if a { b } else { c } { 1 }"),
        "(if (if a {b} else {c}) {1})"
    );
    // Blocks keep their items.
    assert_eq!(
        shape("if a {\n  let y = 1\n  y\n} else {\n  2\n}"),
        "(if a {let y = 1; y} else {2})"
    );
}

#[test]
fn else_may_start_a_later_line() {
    assert_eq!(shape("if a { 1 }\nelse { 2 }"), "(if a {1} else {2})");
    assert_eq!(
        shape("if a { 1 }\n\n# comment\nelse if b { 2 }\nelse { 3 }"),
        "(if a {1} else (if b {2} else {3}))"
    );
}

#[test]
fn without_else_the_newlines_still_separate_items() {
    assert_eq!(shape("if a { 1 }\n2"), "(if a {1}); 2");
    assert_eq!(shape("if a { 1 }\n\n\nx := 2"), "(if a {1}); x := 2");
    assert_eq!(shape("{ if a { 1 }\n}"), "{(if a {1})}");
    assert_eq!(shape("if a { 1 }\n"), "(if a {1})");
    // The empty else block sits at the end of the `if`'s block, and the
    // `if` spans only its keyword through that block.
    let ast = parse_text("if a { 1 }\n").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::If {
        else_, has_else, ..
    } = ast.get(root).kind
    else {
        panic!("expected an if");
    };
    assert!(!has_else);
    assert_eq!(ast.get(root).span, Span::new(0, 10));
    assert_eq!(ast.get(else_).span, Span::new(10, 10));
}

#[test]
fn if_errors() {
    assert_eq!(
        error("if a 1"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("if a\n{ 1 }"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(4, 5)
        )
    );
    assert_eq!(
        error("if (a) + 1"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(10, 10)
        )
    );
    assert_eq!(
        error("if a { 1 } else 2"),
        (
            "expected `{` or `if` after `else`".to_owned(),
            Span::new(16, 17)
        )
    );
    assert_eq!(
        error("if a { 1 } else\n{ 2 }"),
        (
            "expected `{` or `if` after `else`".to_owned(),
            Span::new(15, 16)
        )
    );
    assert_eq!(
        error("if { 1 }"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(8, 8)
        )
    );
    assert_eq!(
        error("else { 1 }"),
        (
            "expected expression, found `else`".to_owned(),
            Span::new(0, 4)
        )
    );
    assert_eq!(
        error("if a { 1 } 2"),
        (
            "expected end of item, found `2`".to_owned(),
            Span::new(11, 12)
        )
    );
}

#[test]
fn literals_of_the_new_types() {
    assert_eq!(shape("true; false"), "true; false");
    assert_eq!(shape("\"hi\""), "\"hi\"");
    assert_eq!(shape("r\"\\n\""), "\"\\n\"");
    let ast = parse_text("\"a\" + \"\"").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Binary { lhs, rhs, .. } = ast.get(root).kind else {
        panic!("expected a sum");
    };
    assert_eq!(ast.get(lhs).span, Span::new(0, 3));
    assert_eq!(ast.get(rhs).span, Span::new(6, 8));
}

#[test]
fn interpolation_parts() {
    assert_eq!(shape("\"a{x}b{y + 1}\""), "\"a<x>b<(y + 1)>\"");
    assert_eq!(shape("\"{ \"{x}\" }\""), "\"<\"<x>\">\"");
    assert_eq!(shape("\"{ {} }\""), "\"<{}>\"");
    assert_eq!(
        shape("\"status: {if ok { \"ok\" } else { \"failed\" }}\""),
        "\"status: <(if ok {\"ok\"} else {\"failed\"})>\""
    );
    let ast = parse_text("\"a{x}b{y}\"").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let interp = ast.get(root);
    assert_eq!(interp.span, Span::new(0, 10));
    let ExprKind::Interp(parts) = interp.kind else {
        panic!("expected an interpolation, got {interp:?}");
    };
    let parts = ast.parts(parts);
    let text = |part: &StrPart| match *part {
        StrPart::Lit(id) => ast.strings().get(id).to_owned(),
        StrPart::Expr(id) => format!("<{:?}>", ast.get(id).kind),
    };
    let shown: Vec<String> = parts.iter().map(text).collect();
    assert_eq!(shown.len(), 5);
    assert_eq!(shown[0], "a");
    assert!(shown[1].starts_with("<Name"));
    assert_eq!(shown[2], "b");
    assert!(shown[3].starts_with("<Name"));
    assert_eq!(shown[4], "");
}

#[test]
fn interpolation_errors() {
    let diagnostic = parse_text("\"{1 2}\"").unwrap_err();
    assert_eq!(diagnostic.message, "expected `}`, found `2`");
    assert_eq!(diagnostic.span, Span::new(4, 5));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Span::new(1, 2));
    assert_eq!(diagnostic.labels[0].text, "unclosed `{`");
    // In a later interpolation, the label is on that one's `{`.
    let diagnostic = parse_text("\"{a}{b 1}\"").unwrap_err();
    assert_eq!(diagnostic.labels[0].span, Span::new(4, 5));
    // An empty interpolation, and one missing its closing parenthesis,
    // are reported at the `}`.
    assert_eq!(
        error("\"{}\""),
        ("expected expression, found `}`".to_owned(), Span::new(2, 3))
    );
    assert_eq!(
        error("\"{a}{}b\""),
        ("expected expression, found `}`".to_owned(), Span::new(5, 6))
    );
    assert_eq!(
        error("\"{(1}\""),
        ("expected `)`, found `}`".to_owned(), Span::new(4, 5))
    );
    // Only an expression may be interpolated.
    assert_eq!(
        message("\"{let x = 1}\""),
        "expected expression, found `let`"
    );
}

#[test]
fn pretty_output_for_a_program_with_strings_and_ifs() {
    let text = "let s = \"n: {n}\"\n\
                    if not a == b and c { s } else if d { \"x\" }";
    let ast = parse_text(text).unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let s @0..16\n\
             \x20   (interp @8..16\n\
             \x20     (lit \"n: \")\n\
             \x20     (name n @13..14)\n\
             \x20     (lit \"\")))\n\
             \x20 (expr @17..60\n\
             \x20   (if @17..60\n\
             \x20     (and @20..36\n\
             \x20       (not @20..30\n\
             \x20         (== @24..30\n\
             \x20           (name a @24..25)\n\
             \x20           (name b @29..30)))\n\
             \x20       (name c @35..36))\n\
             \x20     (block @37..42\n\
             \x20       (expr @39..40\n\
             \x20         (name s @39..40)))\n\
             \x20     (if @48..60\n\
             \x20       (name d @51..52)\n\
             \x20       (block @53..60\n\
             \x20         (expr @55..58\n\
             \x20           (str \"x\" @55..58)))\n\
             \x20       (block @60..60)))))"
    );
}

#[test]
fn else_if_chains_are_limited() {
    // A chain of N `if`s is N + 1 levels deep: the innermost `if` is one
    // above its blocks.
    let chain = |ifs: usize| format!("{}{{}}", "if a {} else ".repeat(ifs));
    let deepest = chain(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(chain(MAX_DEPTH));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // Far beyond the limit, where uncounted recursion would overflow.
    let diagnostic = too_deep(chain(200_000));
    assert_eq!(diagnostic.message, "expression nested too deeply");
}

#[test]
fn interpolations_count_towards_depth() {
    // An interpolation is one level above its expression: around N
    // parentheses and a name, N + 2 levels. (Literals nested in each
    // other's interpolations are limited the same way; the lexer test
    // `nesting_is_limited` and the end-to-end depth test cover them.)
    let wrapped = |parens: usize| format!("\"{{{}x{}}}\"", "(".repeat(parens), ")".repeat(parens));
    let deepest = wrapped(MAX_DEPTH - 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(wrapped(MAX_DEPTH - 1));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // Two levels of literal around one name.
    let ast = parse_text("\"{\"{x}\"}\"").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Interp(parts) = ast.get(root).kind else {
        panic!("expected an interpolation");
    };
    let StrPart::Expr(inner) = ast.parts(parts)[1] else {
        panic!("expected an expression part");
    };
    assert!(matches!(ast.get(inner).kind, ExprKind::Interp(_)));
}

#[test]
fn field_items_with_plain_and_quoted_names() {
    assert_eq!(shape("x = 1"), "x = 1");
    assert_eq!(shape("{ x = 1; y = x + 1 }"), "{x = 1; y = (x + 1)}");
    assert_eq!(
        shape("\"a b\" = 1\n\"\"\"c\"\"\" = 2"),
        "\"a b\" = 1; c = 2"
    );
    assert_eq!(shape("x =\n  1"), "x = 1");
    assert_eq!(shape("hello-world = { n = 1 }"), "hello-world = {n = 1}");
    // A quoted name is interned like an identifier: `"x"` is `x`.
    let ast = parse_text("\"x\" = 1; x = 2").unwrap();
    let names: Vec<Name> = ast
        .items(ast.root())
        .iter()
        .map(|&id| match ast.item(id).kind {
            ItemKind::Field { path, .. } => ast.path(path)[0].name().unwrap(),
            other => panic!("expected a field, got {other:?}"),
        })
        .collect();
    assert_eq!(names[0], names[1]);
    assert_eq!(ast.names().get(names[0]), "x");
    // Spans: the item from the name, which includes its quotes.
    let ast = parse_text("\"a b\" = 1 + 2").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(0, 13));
    let ItemKind::Field { path, .. } = item.kind else {
        panic!("expected a field");
    };
    assert_eq!(ast.path(path)[0].span, Span::new(0, 5));
    // `==` is a comparison, not a field.
    assert_eq!(shape("x == 1"), "(x == 1)");
    assert_eq!(shape("\"a\" == \"b\""), "(\"a\" == \"b\")");
}

#[test]
fn field_names_must_be_plain() {
    assert_eq!(
        error("r\"a\" = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(0, 4))
    );
    assert_eq!(
        error("\"a{x}\" = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(0, 6))
    );
    assert_eq!(
        error("a.r\"b\""),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(2, 6))
    );
    assert_eq!(
        error("a.\"{x}\""),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(2, 7))
    );
    assert_eq!(
        error("1 : {r\"a\": Int}"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(5, 9))
    );
    assert_eq!(
        error("1 : {\"{x}\": Int}"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(5, 10))
    );
    // An interpolated literal is fine where an expression is.
    assert_eq!(shape("\"a{x}\" == 1"), "(\"a<x>\" == 1)");
}

#[test]
fn variables_need_plain_names() {
    let message = "variables need plain names".to_owned();
    assert_eq!(error("let \"x\" = 1"), (message.clone(), Span::new(4, 7)));
    assert_eq!(error("let r\"x\" = 1"), (message.clone(), Span::new(4, 8)));
    assert_eq!(error("let \"{y}\" = 1"), (message.clone(), Span::new(4, 9)));
    // A quoted name is never a variable, so the base of a `:=` path
    // cannot be one either, whatever follows it.
    assert_eq!(error("\"x\" := 1"), (message.clone(), Span::new(0, 3)));
    assert_eq!(error("r\"x\" := 1"), (message.clone(), Span::new(0, 4)));
    assert_eq!(error("\"{y}\" := 1"), (message.clone(), Span::new(0, 5)));
    assert_eq!(error("\"x\".y := 1"), (message.clone(), Span::new(0, 3)));
    assert_eq!(error("let \"x\".y = 1"), (message, Span::new(4, 7)));
}

#[test]
fn paths_in_fields_lets_and_rebinds() {
    assert_eq!(shape("a.b = 1"), "a.b = 1");
    assert_eq!(shape("a.b.c = 1"), "a.b.c = 1");
    assert_eq!(shape("a.b := 1"), "a.b := 1");
    assert_eq!(shape("let a.b.c = 1"), "let a.b.c = 1");
    assert_eq!(shape("let a.b: Int = 1"), "let a.b: Int = 1");
    assert_eq!(shape("let a . b = 1"), "let a.b = 1");
    // Quoted segments, anywhere but the start of a variable's path.
    assert_eq!(
        shape("\"log level\".format = \"json\""),
        "\"log level\".format = \"json\""
    );
    assert_eq!(shape("a.\"b c\".d = 1"), "a.\"b c\".d = 1");
    assert_eq!(shape("let a.\"b c\" = 1"), "let a.\"b c\" = 1");
    assert_eq!(shape("a.\"b c\" := 1"), "a.\"b c\" := 1");
    // A quoted segment is the same name as a plain one.
    assert_eq!(shape("a.\"b\" = 1"), "a.b = 1");
    // In blocks, and as values.
    assert_eq!(
        shape("{ server.host = \"h\"; server.port = 80 }"),
        "{server.host = \"h\"; server.port = 80}"
    );
    assert_eq!(shape("a.b =\n  1"), "a.b = 1");
    // `==` after a path is a comparison, and a path alone an access.
    assert_eq!(shape("a.b == 1"), "((a . b) == 1)");
    assert_eq!(shape("a.b"), "(a . b)");
    // Spans: each segment, quotes included, and the item from its first.
    let ast = parse_text("  a . \"b c\".d = 1").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(2, 17));
    let ItemKind::Field { path, .. } = item.kind else {
        panic!("expected a field");
    };
    let spans: Vec<Span> = ast.path(path).iter().map(|s| s.span).collect();
    assert_eq!(
        spans,
        vec![Span::new(2, 3), Span::new(6, 11), Span::new(12, 13)]
    );
    let ast = parse_text("\"x\".y := 1").unwrap_err();
    assert_eq!(ast.span, Span::new(0, 3));
    let ast = parse_text("let a.\"b\".c: Int = 1").unwrap();
    let ItemKind::Let { path, ty, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a let");
    };
    assert!(ty.is_some());
    let spans: Vec<Span> = ast.path(path).iter().map(|s| s.span).collect();
    assert_eq!(
        spans,
        vec![Span::new(4, 5), Span::new(6, 9), Span::new(10, 11)]
    );
    let ast = parse_text("\"q\".r = 1").unwrap();
    let ItemKind::Field { path, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a field");
    };
    let spans: Vec<Span> = ast.path(path).iter().map(|s| s.span).collect();
    assert_eq!(spans, vec![Span::new(0, 3), Span::new(4, 5)]);
    // The expression a path was read from leaves no nodes behind: only
    // the values remain.
    assert_eq!(parse_text("a.b.c = 1; d := 2; \"e\" = 3").unwrap().len(), 3);
    assert_eq!(parse_text("{ a.b = { c.d = 1 } }").unwrap().len(), 3);
}

#[test]
fn anything_else_before_eq_is_not_a_path() {
    let found_eq = |at: u32| {
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(at, at + 1),
        )
    };
    assert_eq!(error("a.b + 1 = 2"), found_eq(8));
    assert_eq!(error("(a).b = 1"), found_eq(6));
    assert_eq!(error("(a.b) = 1"), found_eq(6));
    assert_eq!(error("-a.b = 1"), found_eq(5));
    assert_eq!(error("(a).b : Int = 1"), found_eq(12));
    assert_eq!(error("{ x = 1 }.x = 1"), found_eq(12));
    assert_eq!(error("1.x = 1"), found_eq(4));
    assert_eq!(
        error("a.b + 1 := 2"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(8, 10)
        )
    );
    assert_eq!(
        error("(a).b := 2"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(6, 8)
        )
    );
    // A raw or interpolated literal is not a field name, at the start of
    // a path as alone.
    assert_eq!(
        error("r\"a\".b = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(0, 4))
    );
    assert_eq!(
        error("\"a{x}\".b = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(0, 6))
    );
    // After `let`, a segment must be a name.
    assert_eq!(
        error("let a.1 = 1"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("let a. = 1"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(7, 8)
        )
    );
    assert_eq!(
        error("let a.r\"b\" = 1"),
        (PLAIN_FIELD_NAME.to_owned(), Span::new(6, 10))
    );
    assert_eq!(
        error("let a.b 1"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(8, 9)
        )
    );
}

#[test]
fn pretty_output_for_paths() {
    let ast = parse_text("let a.b.c = 1\nx.\"y z\" = 2\na.b := 3").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let a.b.c @0..13\n\
             \x20   (int 1 @12..13))\n\
             \x20 (field x.\"y z\" @14..25\n\
             \x20   (int 2 @24..25))\n\
             \x20 (rebind a.b @26..34\n\
             \x20   (int 3 @33..34)))"
    );
}

#[test]
fn paths_count_towards_height() {
    // A path of N segments is 2(N - 1) levels above its value: with a
    // literal, one short of the limit when 2(N - 1) + 1 is.
    let segments = (MAX_DEPTH - 1) / 2 + 1;
    let path = |segments: usize| format!("a{}", ".b".repeat(segments - 1));
    let deepest = format!("let {} = 1", path(segments));
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(format!("let {} = 1", path(segments + 1)));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    assert_eq!(diagnostic.span.start, 0);
    // A field's path too, and in blocks, whose height it adds to: the
    // deepest item is at the limit in one block and over it in two.
    let field = format!("{} = 1", path(segments));
    on_big_stack(move || parse_text(&field).unwrap());
    too_deep(format!("{} = 1", path(segments + 1)));
    let one = format!("{{ {} = 1 }}", path(segments));
    on_big_stack(move || parse_text(&one).unwrap());
    too_deep(format!("{{ {{ {} = 1 }} }}", path(segments)));
    // Far beyond the limit, a `let` path is stopped at its item.
    too_deep(format!("let {} = 1", path(200_000)));
}

#[test]
fn field_access_binds_tighter_than_everything() {
    assert_eq!(shape("a.b"), "(a . b)");
    assert_eq!(shape("-a.b"), "(-(a . b))");
    assert_eq!(shape("a.b.c"), "((a . b) . c)");
    assert_eq!(shape("a.b : Int"), "((a . b) : Int)");
    assert_eq!(shape("a.b + c.d"), "((a . b) + (c . d))");
    assert_eq!(shape("a.b == c.d"), "((a . b) == (c . d))");
    assert_eq!(shape("not a.b"), "(not (a . b))");
    assert_eq!(shape("a.\"b c\".d"), "((a . \"b c\") . d)");
    assert_eq!(shape("a . b"), "(a . b)");
    assert_eq!(shape("(a).b"), "([a] . b)");
    assert_eq!(shape("{ x = 1 }.x"), "({x = 1} . x)");
    assert_eq!(shape("1.x"), "(1 . x)");
    assert_eq!(shape("\"s\".x"), "(\"s\" . x)");
    assert_eq!(shape("a.\"x\""), "(a . x)");
    assert_eq!(shape("a.b * 2 - c.d.e"), "(((a . b) * 2) - ((c . d) . e))");
    // Spans: the node covers its record through its name.
    let ast = parse_text("-ab.\"c d\"").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Unary { operand, .. } = ast.get(root).kind else {
        panic!("expected a negation");
    };
    let access = ast.get(operand);
    assert_eq!(access.span, Span::new(1, 9));
    assert_eq!(access.member_span(&ast), Some(Span::new(3, 9)));
    assert_eq!(ast.get(root).member_span(&ast), None);
}

#[test]
fn field_access_errors() {
    let message = "expected a field name after `.`".to_owned();
    assert_eq!(error("x.1"), (message.clone(), Span::new(2, 3)));
    assert_eq!(error("x.1.5"), (message.clone(), Span::new(2, 5)));
    assert_eq!(error("x."), (message.clone(), Span::new(2, 2)));
    assert_eq!(error("x.let"), (message.clone(), Span::new(2, 5)));
    assert_eq!(error("x.(y)"), (message.clone(), Span::new(2, 3)));
    // Newlines are not skipped around `.`.
    assert_eq!(error("x.\ny"), (message, Span::new(2, 3)));
    assert_eq!(
        error("x\n.y"),
        ("expected expression, found `.`".to_owned(), Span::new(2, 3))
    );
}

#[test]
fn record_types() {
    assert_eq!(
        shape("let p: {x: Int, y: Int} = 1"),
        "let p: {x: Int, y: Int} = 1"
    );
    assert_eq!(shape("1 : {}"), "(1 : {})");
    assert_eq!(
        shape("1 : { \"a b\": String, nested: {n: Int}, }"),
        "(1 : {\"a b\": String, nested: {n: Int}})"
    );
    // Newlines anywhere inside the braces.
    assert_eq!(
        shape("let p: {\n  x:\n    Int\n  ,\n  y: {\n}\n\n} = 1"),
        "let p: {x: Int, y: {}} = 1"
    );
    // Repeated names are the checker's business.
    assert_eq!(shape("1 : {x: Int, x: Int}"), "(1 : {x: Int, x: Int})");
    // A record type ends an ascription as a name does.
    assert_eq!(shape("x : {a: Int} + 1"), "((x : {a: Int}) + 1)");
    let ast = parse_text("1 : {a: Int, \"b\": {}}").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Ascribe { ty, .. } = ast.get(root).kind else {
        panic!("expected an ascription");
    };
    let ty = ast.type_expr(ty);
    assert_eq!(ty.span, Span::new(4, 21));
    let TypeExprKind::Record(fields) = ty.kind else {
        panic!("expected a record type");
    };
    let fields = ast.field_tys(fields);
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].span, Span::new(5, 6));
    assert_eq!(fields[1].span, Span::new(13, 16));
    assert_eq!(
        fields[1].name().map(|name| ast.names().get(name)),
        Some("b")
    );
    assert_eq!(ast.type_expr(fields[1].ty).span, Span::new(18, 20));
}

#[test]
fn record_type_fields_may_have_defaults() {
    // Any expression, after any field, wherever a record type is
    // written: an annotation, a parameter's type, a return type, a
    // function type, nested, in a list or a union. What a default may
    // be is the checker's business.
    assert_eq!(
        shape("let p: {a: Int = 1, b: String, c: List<String> = []} = q"),
        "let p: {a: Int = 1, b: String, c: List<String> = list()} = q"
    );
    assert_eq!(
        shape("fn(e: {a: Int = -1}): {r: Bool = true} { e }"),
        "fn(e: {a: Int = (-1)}): {r: Bool = true} {e}"
    );
    assert_eq!(
        shape("let f: fn({a: Int = 1}): Int = g"),
        "let f: fn({a: Int = 1}): Int = g"
    );
    assert_eq!(
        shape("1 : {a: {b: Int = 1} = { b = 2 }, c: List<{d: String = \"x\"}>} | Int"),
        "(1 : ({a: {b: Int = 1} = {b = 2}, c: List<{d: String = \"x\"}>} | Int))"
    );
    assert_eq!(shape("x is {a: Int = k + 1}"), "(x is {a: Int = (k + 1)})");
    // Newlines around the `=`, and a trailing block in a default, the
    // braces being the type's.
    assert_eq!(
        shape("let p: {\n  a: Int\n  =\n  1,\n  b: Int = g { x = 1 }\n} = q"),
        "let p: {a: Int = 1, b: Int = g(){x = 1}} = q"
    );
    // A spread has none.
    assert_eq!(
        error("1 : {...T = 1}"),
        ("expected `,` or `}`".to_owned(), Span::new(10, 11))
    );
    assert_eq!(
        error("1 : {a: Int = }"),
        (
            "expected expression, found `}`".to_owned(),
            Span::new(14, 15)
        )
    );
    // A default is parsed after its field's type, before what follows
    // the type: the first node of a `let`'s block and of a function, as
    // a parameter's default is.
    let text = "fn(x: {a: Int = 1}, y: Int = 2): {r: Int = 3} { x }";
    let ast = parse_text(text).unwrap();
    let value = only_value(&ast);
    let first = ast.subtree(value).next().unwrap();
    assert_eq!(ast.get(first).span, Span::new(16, 17));
    assert_eq!(ast.subtree(value).count(), ast.len());
    let text = "{\n  let p: List<{a: Int = 7}> = q\n  = p\n}";
    let ast = parse_text(text).unwrap();
    let value = only_value(&ast);
    let first = ast.subtree(value).next().unwrap();
    assert_eq!(
        &text[ast.get(first).span.start as usize..],
        "7}> = q\n  = p\n}"
    );
    assert_eq!(ast.subtree(value).count(), ast.len());
    // The tree dump shows a default after its field's type.
    assert_eq!(
        parse_text("1 : {a: Int = -1, b: List<{c: String}> = [{ c = \"x\" }], d: Int = k}")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n  (expr @0..67\n    (: {a: Int = -1, b: List<{c: String}> = \
             [{c = \"x\"}], d: Int = ...} @0..67\n      (int 1 @0..1))))"
    );
}

#[test]
fn record_type_errors() {
    assert_eq!(
        error("1 : {1: Int}"),
        (
            "expected a field name in record type".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("1 : {x: Int,,}"),
        (
            "expected a field name in record type".to_owned(),
            Span::new(12, 13)
        )
    );
    assert_eq!(
        error("1 : {,}"),
        (
            "expected a field name in record type".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("1 : {x Int}"),
        ("expected `:` after field name".to_owned(), Span::new(7, 10))
    );
    assert_eq!(
        error("1 : {x := Int}"),
        ("expected `:` after field name".to_owned(), Span::new(7, 9))
    );
    assert_eq!(
        error("1 : {x: Int y: Int}"),
        ("expected `,` or `}`".to_owned(), Span::new(12, 13))
    );
    assert_eq!(
        error("1 : {x: Int"),
        ("expected `,` or `}`".to_owned(), Span::new(11, 11))
    );
    assert_eq!(
        error("1 : {x: }"),
        ("expected a type".to_owned(), Span::new(8, 9))
    );
    assert_eq!(
        error("1 : {"),
        (
            "expected a field name in record type".to_owned(),
            Span::new(5, 5)
        )
    );
}

#[test]
fn a_record_type_may_have_a_rest_field() {
    assert_eq!(shape("1 : {*: String}"), "(1 : {*: String})");
    // Anywhere among the fields and spreads, and with newlines around its
    // parts, as a field's; how many it may have is the checker's
    // business.
    assert_eq!(
        shape("let p: {\n  to: String,\n  *\n  :\n  Int,\n  ...T\n} = q"),
        "let p: {to: String, *: Int, ...T} = q"
    );
    assert_eq!(
        shape("1 : {*: Int, *: String}"),
        "(1 : {*: Int, *: String})"
    );
    let ast = parse_text("1 : {a: Int, *: {}}").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let ExprKind::Ascribe { ty, .. } = ast.get(root).kind else {
        panic!("expected an ascription");
    };
    let TypeExprKind::Record(fields) = ast.type_expr(ty).kind else {
        panic!("expected a record type");
    };
    let fields = ast.field_tys(fields);
    assert_eq!(fields[1].kind, FieldTyKind::Rest);
    assert_eq!(fields[1].name(), None);
    assert_eq!(fields[1].span, Span::new(13, 14));
    assert_eq!(ast.type_expr(fields[1].ty).span, Span::new(16, 18));
    // It has no default, and needs its `:`.
    assert_eq!(
        error("1 : {*: Int = 1}"),
        (
            "a rest-field cannot have a default".to_owned(),
            Span::new(12, 13)
        )
    );
    assert_eq!(
        error("1 : {* Int}"),
        ("expected `:` after `*`".to_owned(), Span::new(7, 10))
    );
}

#[test]
fn pretty_output_for_records() {
    let text = "let p: {x: Int, \"a b\": {}} = { x = 1; \"a b\" = {} }\n-p.x";
    let ast = parse_text(text).unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let p : {x: Int, \"a b\": {}} @0..50\n\
             \x20   (block @29..50\n\
             \x20     (field x @31..36\n\
             \x20       (int 1 @35..36))\n\
             \x20     (field \"a b\" @38..48\n\
             \x20       (block @46..48))))\n\
             \x20 (expr @51..55\n\
             \x20   (neg @51..55\n\
             \x20     (. x @52..55\n\
             \x20       (name p @52..53)))))"
    );
}

#[test]
fn field_chains_and_record_types_count_towards_depth() {
    // A chain of N accesses is N + 1 levels: each is one above its
    // record. This nests without recursion in the parser.
    let chain = |accesses: usize| format!("a{}", ".b".repeat(accesses));
    let deepest = chain(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(chain(MAX_DEPTH));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // An ascription is one above the taller of its expression and its
    // type, and a record type one above its tallest field type: N
    // nested record types are N + 1 levels.
    let nested = |levels: usize| format!("1 : {}Int{}", "{a: ".repeat(levels), "}".repeat(levels));
    let deepest = nested(MAX_DEPTH - 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(nested(MAX_DEPTH - 1));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // Far beyond the limit, record types are stopped by the recursion
    // count before they could overflow.
    let diagnostic = too_deep(nested(200_000));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    // In an annotation, the type counts towards its block's height.
    let annotated = |levels: usize| {
        format!(
            "{{ let x: {}Int{} = 1 }}",
            "{a: ".repeat(levels),
            "}".repeat(levels)
        )
    };
    let deepest = annotated(MAX_DEPTH - 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(annotated(MAX_DEPTH - 1));
    assert_eq!(diagnostic.message, "expression nested too deeply");
}

#[test]
fn tokens_without_eof_are_treated_as_ending() {
    let source = SourceFile::new("test.gcsl", "1 +").unwrap();
    let (tokens, names, strings) = tokenize(&source).unwrap();
    let without_eof = &tokens[..tokens.len() - 1];
    let diagnostic = parse(without_eof, names, strings, &source).unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected expression, found end of input"
    );
    assert_eq!(diagnostic.span, Span::new(3, 3));
}

// ----- Functions and calls -----

/// The expression of the file's only item, which must be an expression
/// or a `let`.
fn only_value(ast: &Ast) -> ExprId {
    match ast.item(ast.items(ast.root())[0]).kind {
        ItemKind::Expr(value) | ItemKind::Let { value, .. } => value,
        other => panic!("expected an expression or a let, got {other:?}"),
    }
}

#[test]
fn function_expressions() {
    assert_eq!(shape("fn(): Int { 1 }"), "fn(): Int {1}");
    assert_eq!(shape("fn(x: Int): Int { x }"), "fn(x: Int): Int {x}");
    assert_eq!(
        shape("fn(name: String, times: Int) -> String { name }"),
        "fn(name: String, times: Int) -> String {name}"
    );
    // A trailing comma, and a parameter list over several lines.
    assert_eq!(shape("fn(x: Int,): Int { x }"), "fn(x: Int): Int {x}");
    assert_eq!(
        shape("let f = fn(\n  x: Int,\n  y: Int,\n): Int { x + y }"),
        "let f = fn(x: Int, y: Int): Int {(x + y)}"
    );
    // Newlines after the marker, and a body over several lines.
    assert_eq!(
        shape("fn(n: Int) ->\n  {} {\n  let m = n\n  m\n}"),
        "fn(n: Int) -> {} {let m = n; m}"
    );
    assert_eq!(shape("fn():\nInt { 1 }"), "fn(): Int {1}");
    // A record return type followed by the body, and function types as
    // parameter and return types.
    assert_eq!(shape("fn(): {x: Int} { x = 1 }"), "fn(): {x: Int} {x = 1}");
    assert_eq!(
        shape("fn(f: fn(Int): Int): fn() -> Int { f }"),
        "fn(f: fn(Int): Int): fn() -> Int {f}"
    );
    // An operand like any other.
    assert_eq!(shape("fn(): Int { 1 } == g"), "(fn(): Int {1} == g)");
    // The details: parameters with their spans, the marker and purity.
    let ast = parse_text("fn(x: Int, yy: {a: Int}) -> Int { x }").unwrap();
    let expr = ast.get(only_value(&ast));
    assert_eq!(expr.span, Span::new(0, 37));
    let ExprKind::Fn(function) = expr.kind else {
        panic!("expected a function, got {expr:?}");
    };
    assert_eq!(ast.fn_count(), 1);
    let decl = ast.fn_decl(function);
    assert_eq!(decl.effect, Effect::Impure);
    assert_eq!(decl.marker, Span::new(25, 27));
    assert_eq!(ast.type_expr(decl.ret.unwrap()).span, Span::new(28, 31));
    assert_eq!(ast.get(decl.body).span, Span::new(32, 37));
    let spans: Vec<Span> = ast.params(decl.params).iter().map(|p| p.span).collect();
    assert_eq!(spans, vec![Span::new(3, 9), Span::new(11, 23)]);
    assert_eq!(decl.params.len(), 2);
    assert!(!decl.params.is_empty());
    let ast = parse_text("fn(): Int { 1 }").unwrap();
    let ExprKind::Fn(function) = ast.get(only_value(&ast)).kind else {
        panic!("expected a function");
    };
    assert_eq!(ast.fn_decl(function).effect, Effect::Pure);
    assert!(ast.fn_decl(function).params.is_empty());
}

#[test]
fn parameters_may_have_defaults() {
    // In any position, any expression, in a `fn` expression, a named
    // `fn`, a `def` and a def-type's `def`; what a default may be is the
    // checker's business.
    assert_eq!(
        shape("fn(a: Int = 1, b: String, c: List<Int> = []): Int { a }"),
        "fn(a: Int = 1, b: String, c: List<Int> = list()): Int {a}"
    );
    assert_eq!(
        shape("fn f(x: Int = -1) -> Int { x }"),
        "fn f(x: Int = (-1)) -> Int {x}"
    );
    assert_eq!(
        shape("def greet(name: String, greeting: String = \"hello\"): String { greeting }"),
        "def greet(name: String, greeting: String = \"hello\"): String {greeting}"
    );
    assert_eq!(
        shape("#[def_type]\ndef p(n: String, f: fn(): Int, v: String = r\"x\") -> Int { 1 }"),
        "def p(n: String, f: fn(): Int, v: String = \"x\") -> Int {1}"
    );
    assert_eq!(
        shape("fn(r: {a: Int} = { a = 1 }, s: Int = g(x) + 1): Int { 1 }"),
        "fn(r: {a: Int} = {a = 1}, s: Int = (g(x) + 1)): Int {1}"
    );
    // Newlines inside the parentheses, around the `=` too.
    assert_eq!(
        shape("fn(\n  a: Int\n  = 1,\n  b: Int =\n  2,\n): Int { a }"),
        "fn(a: Int = 1, b: Int = 2): Int {a}"
    );
    // The parameter's span is its name through its type; the default is
    // parsed before the body, and so is the first node of the
    // function's subtree.
    let text = "fn(x: Int, y: {a: Int} = { a = 1 }, z: Bool = true): Int { x }";
    let ast = parse_text(text).unwrap();
    let value = only_value(&ast);
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("expected a function");
    };
    let params = ast.params(ast.fn_decl(function).params);
    let spans: Vec<Span> = params.iter().map(|p| p.span).collect();
    assert_eq!(
        spans,
        vec![Span::new(3, 9), Span::new(11, 22), Span::new(36, 43)]
    );
    let defaults: Vec<Option<Span>> = params
        .iter()
        .map(|p| p.default.map(|default| ast.get(default).span))
        .collect();
    assert_eq!(
        defaults,
        vec![None, Some(Span::new(25, 34)), Some(Span::new(46, 50))]
    );
    let first = ast.subtree(value).next().unwrap();
    assert_eq!(ast.get(first).span, Span::new(31, 32));
    assert_eq!(ast.subtree(value).count(), ast.len());
    // The tree dump shows a default after its parameter's type.
    assert_eq!(
        parse_text("fn(a: Int, b: Int = 2): Int { a }")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n  (expr @0..33\n    (fn @0..33\n      (params (a Int) (b Int\n        \
             (int 2 @20..21)))\n      (ret pure Int)\n      (block @28..33\n        \
             (expr @30..31\n          (name a @30..31))))))"
    );
}

#[test]
fn a_default_is_written_only_in_a_function_expression() {
    // Not in a function type, named or not, nested or not, at its `=`.
    assert_eq!(
        error("let f: fn(a: Int = 1): Int = g"),
        (MISPLACED_DEFAULT.to_owned(), Span::new(17, 18))
    );
    assert_eq!(
        error("let f: fn(Int = 1): Int = g"),
        (MISPLACED_DEFAULT.to_owned(), Span::new(14, 15))
    );
    assert_eq!(
        error("fn(f: fn(x: Int = 1): Int): Int { 1 }"),
        (MISPLACED_DEFAULT.to_owned(), Span::new(16, 17))
    );
    // A function type as a parameter's type, followed by that
    // parameter's default.
    assert_eq!(
        shape("fn(f: fn(Int): Int = g): Int { 1 }"),
        "fn(f: fn(Int): Int = g): Int {1}"
    );
    // A definition's parameters are names alone, as before.
    assert_eq!(
        error("def p x(v = \"x\") { 1 }"),
        (
            "expected `,` or `)`, found `=`".to_owned(),
            Span::new(10, 11)
        )
    );
}

#[test]
fn memoised_functions() {
    // A function expression, a named item, function types and nested
    // function types, each with `->>`, which newlines may follow.
    assert_eq!(
        shape("fn(url: String) ->> String { url }"),
        "fn(url: String) ->> String {url}"
    );
    assert_eq!(
        shape("fn(n: Int) ->>\n  Int {\n  n\n}"),
        "fn(n: Int) ->> Int {n}"
    );
    assert_eq!(
        shape("fn(f: fn(Int) ->> Int): fn() ->> fn(x: Int) ->> Int { f }"),
        "fn(f: fn(Int) ->> Int): fn() ->> fn(x: Int) ->> Int {f}"
    );
    assert_eq!(
        shape("fn(f: fn(fn(Int) ->> Int) -> Int) ->> Int { 1 }"),
        "fn(f: fn(fn(Int) ->> Int) -> Int) ->> Int {1}"
    );
    let text = "fn fetch(url: String) ->> String { url }";
    let ast = parse_text(text).unwrap();
    let ItemKind::Let { value, .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a let");
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("expected a function");
    };
    let decl = ast.fn_decl(function);
    assert_eq!(decl.effect, Effect::Memo);
    // The marker spans the whole `->>`.
    assert_eq!(decl.marker, Span::new(22, 25));
    assert_eq!(&text[22..25], "->>");
    // A type annotation with a memoised function type.
    let ast = parse_text("let f: fn(Int) ->> Int = g").unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated let");
    };
    assert!(matches!(
        ast.type_expr(ty).kind,
        TypeExprKind::Fn {
            effect: Effect::Memo,
            ..
        }
    ));
    // Printed, it reads back as itself.
    let shown = shape("fn(f: fn(Int) ->> Int) ->> fn() ->> Int { f }");
    assert_eq!(shown, "fn(f: fn(Int) ->> Int) ->> fn() ->> Int {f}");
    assert_eq!(shape(&shown), shown);
    // The tree's debug form shows the effect, and the types as written.
    assert_eq!(
        parse_text("fn f(g: fn(Int) ->> Int) ->> Int { 1 }")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n\
             \x20 (fn f @0..38\n\
             \x20   (fn @0..38\n\
             \x20     (params (g fn(Int) ->> Int))\n\
             \x20     (ret memo Int)\n\
             \x20     (block @33..38\n\
             \x20       (expr @35..36\n\
             \x20         (int 1 @35..36))))))"
    );
    // `->>>` is the memo arrow and a `>`, which cannot start a type.
    assert_eq!(
        error("fn(x: Int) ->>> Int { x }"),
        ("expected a type".to_owned(), Span::new(14, 15))
    );
}

#[test]
fn function_expression_errors() {
    let diagnostic = parse_text("fn(x): Int { x }").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected `:` and a type after parameter `x`"
    );
    assert_eq!(diagnostic.span, Span::new(4, 5));
    assert_eq!(diagnostic.notes, vec!["parameters must be annotated"]);
    assert_eq!(
        error("fn(x, y: Int): Int { x }"),
        (
            "expected `:` and a type after parameter `x`".to_owned(),
            Span::new(4, 5)
        )
    );
    let diagnostic = parse_text("fn(x: Int) { x }").unwrap_err();
    assert_eq!(diagnostic.message, MISSING_RETURN_TYPE);
    assert_eq!(diagnostic.span, Span::new(11, 12));
    assert_eq!(diagnostic.notes, vec![RETURN_TYPE_NOTE]);
    assert_eq!(
        MISSING_RETURN_TYPE,
        "expected `:`, `->` or `->>` and a return type after the parameters"
    );
    assert_eq!(
        RETURN_TYPE_NOTE,
        "a function declares its return type: `: T` for a pure function, `-> T` for an \
             impure one, `->> T` for a memoised one"
    );
    assert_eq!(
        error("fn() = 1"),
        (MISSING_RETURN_TYPE.to_owned(), Span::new(5, 6))
    );
    // The body's `{` must follow the return type directly.
    assert_eq!(
        error("fn(): Int\n{ 1 }"),
        (
            "expected `{` to start the function body".to_owned(),
            Span::new(9, 10)
        )
    );
    assert_eq!(
        error("fn(): Int 1"),
        (
            "expected `{` to start the function body".to_owned(),
            Span::new(10, 11)
        )
    );
    assert_eq!(
        error("fn(): { 1 }").0,
        "expected a field name in record type"
    );
    // Parameters need plain names.
    assert_eq!(
        error("fn(\"x\": Int): Int { 1 }"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(3, 6))
    );
    assert_eq!(
        error("fn(\"{y}\": Int): Int { 1 }"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(3, 8))
    );
    assert_eq!(
        error("fn(1: Int): Int { 1 }"),
        (
            "expected a parameter name, found `1`".to_owned(),
            Span::new(3, 4)
        )
    );
    let diagnostic = parse_text("fn(x: Int y: Int): Int { x }").unwrap_err();
    assert_eq!(diagnostic.message, "expected `,` or `)`, found `y`");
    assert_eq!(diagnostic.span, Span::new(10, 11));
    assert_eq!(labels(&diagnostic), vec![(Span::new(2, 3), "unclosed `(`")]);
    assert_eq!(
        error("fn(x: Int"),
        (
            "expected `,` or `)`, found end of input".to_owned(),
            Span::new(9, 9)
        )
    );
    assert_eq!(
        error("fn { 1 }"),
        (
            "expected `(` after `fn`, found `{`".to_owned(),
            Span::new(3, 4)
        )
    );
    assert_eq!(error("fn(x: 1): Int { 1 }").0, "expected a type");
}

/// The labels of a diagnostic as `(span, text)` pairs.
fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
    diagnostic
        .labels
        .iter()
        .map(|label| (label.span, label.text.as_str()))
        .collect()
}

#[test]
fn a_def_is_a_field_of_a_function_expression() {
    let ast = parse_text("def add(a: Int, b: Int): Int { a + b }").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    let ItemKind::Field {
        path,
        ty: None,
        value,
        function: true,
    } = item.kind
    else {
        panic!("expected a def, got {item:?}");
    };
    assert_eq!(item.span, Span::new(0, 38));
    assert_eq!(ast.path(path)[0].span, Span::new(4, 7));
    assert_eq!(ast.get(value).span, Span::new(0, 38));
    // Each marker, as `fn` has them.
    for (marker, printed) in [(":", "():"), ("->>", "() ->>"), ("->", "() ->")] {
        let text = format!("def f(){marker} Int {{ 1 }}");
        assert_eq!(shape(&text), format!("def f{printed} Int {{1}}"));
    }
    // In a record block, among fields, in order.
    assert_eq!(
        shape("{ def f(): Int { 1 }; a = 2 }"),
        "{def f(): Int {1}; a = 2}"
    );
    assert!(
        ast.pretty()
            .to_string()
            .starts_with("(file\n  (def add @0..38\n    (fn @0..38\n")
    );
}

#[test]
fn the_four_forms_of_a_function_binding_print_as_written() {
    for (text, printed) in [
        ("fn f(): Int { 1 }", "fn f(): Int {1}"),
        ("def f(): Int { 1 }", "def f(): Int {1}"),
        ("let f = fn(): Int { 1 }", "let f = fn(): Int {1}"),
        ("f = fn(): Int { 1 }", "f = fn(): Int {1}"),
    ] {
        assert_eq!(shape(text), printed);
        assert_eq!(shape(printed), printed);
    }
    let heads = ["(fn f @", "(def f @", "(let f @", "(field f @"];
    for (text, head) in [
        "fn f(): Int { 1 }",
        "def f(): Int { 1 }",
        "let f = fn(): Int { 1 }",
        "f = fn(): Int { 1 }",
    ]
    .into_iter()
    .zip(heads)
    {
        let pretty = parse_text(text).unwrap().pretty().to_string();
        assert!(pretty.starts_with(&format!("(file\n  {head}")), "{pretty}");
    }
}

#[test]
fn a_def_needs_one_plain_name_and_the_grammar_of_fn() {
    assert_eq!(
        error("def a.\"b\"(): Int { 1 }"),
        (
            "a `def`'s path is made of identifiers".to_owned(),
            Span::new(6, 9)
        )
    );
    assert_eq!(
        error("def \"a b\"(): Int { 1 }"),
        (
            "a `def` is named by an identifier, not a string".to_owned(),
            Span::new(4, 9)
        )
    );
    assert_eq!(
        error("def (): Int { 1 }"),
        (
            "expected the function's name after `def`, found `(`".to_owned(),
            Span::new(4, 5)
        )
    );
    // Without a return type or a body, what `fn` reports, one byte on.
    for rest in ["f() { 1 }", "f(): Int", "f()"] {
        let (message, span) = error(&format!("fn {rest}"));
        let shifted = Span::new(span.start + 1, span.end + 1);
        assert_eq!(error(&format!("def {rest}")), (message, shifted), "{rest}");
    }
    // At the start of an item it always begins a `def`, so a field
    // named `def` is one without a name.
    assert_eq!(
        error("def = 1"),
        (
            "expected the function's name after `def`, found `=`".to_owned(),
            Span::new(4, 5)
        )
    );
    // As a name elsewhere, it is reported as any keyword is.
    for (with_def, with_let) in [("let def = 1", "let let = 1"), ("x = def", "x = let")] {
        assert_eq!(
            error(with_def).0.replace("`def`", "`let`"),
            error(with_let).0,
            "{with_def}"
        );
    }
}

#[test]
fn a_def_may_be_named_by_a_path_of_identifiers() {
    let text = "def dev.peios.hello(n: Int): Int { n }";
    let ast = parse_text(text).unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    let ItemKind::Field {
        path,
        ty: None,
        value,
        function: true,
    } = item.kind
    else {
        panic!("expected a def, got {item:?}");
    };
    // The dotted field of the path, its value the function from its
    // `(` on, spanning the item.
    let segments = ast.path(path);
    assert_eq!(ast.show_path(segments).to_string(), "dev.peios.hello");
    let spans: Vec<Span> = segments.iter().map(|segment| segment.span).collect();
    assert_eq!(
        spans,
        [Span::new(4, 7), Span::new(8, 13), Span::new(14, 19)]
    );
    assert_eq!(item.span, Span::new(0, 38));
    assert_eq!(ast.get(value).span, Span::new(0, 38));
    // Printed as written, with each marker, and in a record block.
    assert_eq!(shape(text), "def dev.peios.hello(n: Int): Int {n}");
    assert_eq!(shape("def a.b() -> Int { 1 }"), "def a.b() -> Int {1}");
    assert_eq!(
        shape("{ def a.b(): Int { 1 }; c = 2 }"),
        "{def a.b(): Int {1}; c = 2}"
    );
    assert!(
        ast.pretty()
            .to_string()
            .starts_with("(file\n  (def dev.peios.hello @0..38\n    (fn @0..38\n"),
        "{}",
        ast.pretty()
    );
    // A quoted or index segment after the first, and anything else
    // where a name is expected after a `.`, is an error at it.
    assert_eq!(
        error("def a.b.\"c d\"(): Int { 1 }"),
        (
            "a `def`'s path is made of identifiers".to_owned(),
            Span::new(8, 13)
        )
    );
    assert_eq!(
        error("def a[0](): Int { 1 }"),
        (
            "a `def`'s path is made of identifiers".to_owned(),
            Span::new(5, 8)
        )
    );
    assert_eq!(
        error("def a.b[i + 1](): Int { 1 }"),
        (
            "a `def`'s path is made of identifiers".to_owned(),
            Span::new(7, 14)
        )
    );
    assert_eq!(
        error("def a.(): Int { 1 }"),
        (
            "expected the function's name after `.`, found `(`".to_owned(),
            Span::new(6, 7)
        )
    );
    // Without a return type or a body, what `fn` reports.
    for rest in ["() { 1 }", "(): Int", "()"] {
        let (message, span) = error(&format!("fn f{rest}"));
        let shifted = Span::new(span.start + 5, span.end + 5);
        assert_eq!(
            error(&format!("def a.b.f{rest}")),
            (message, shifted),
            "{rest}"
        );
    }
    // Two levels taller for each segment after the first, as a dotted
    // field is.
    let deep = format!("def a{}(): Int {{ 1 }}", ".b".repeat(crate::MAX_DEPTH));
    assert!(
        error(&deep).0.contains("nested too deeply"),
        "{:?}",
        error(&deep)
    );
}

#[test]
fn named_functions_are_lets_of_function_expressions() {
    assert_eq!(
        shape("fn greet(name: String): String { \"hello {name}\" }"),
        "fn greet(name: String): String {\"hello <name>\"}"
    );
    assert_eq!(shape("fn f() -> {} {}\nf()"), "fn f() -> {} {}; f()");
    // The item and its value both span `fn` through the body; the path
    // is the name.
    let ast = parse_text("  fn add(a: Int, b: Int): Int { a + b }").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(2, 39));
    let ItemKind::Let {
        path, ty, value, ..
    } = item.kind
    else {
        panic!("expected a let, got {item:?}");
    };
    assert!(ty.is_none());
    assert_eq!(ast.get(value).span, Span::new(2, 39));
    let segments = ast.path(path);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].span, Span::new(5, 8));
    assert_eq!(ast.names().get(segments[0].name().unwrap()), "add");
    assert!(matches!(ast.get(value).kind, ExprKind::Fn(_)));
    // In a block, and as the pretty printer shows it.
    assert_eq!(
        shape("{ fn f(): Int { 1 }; = f }"),
        "{fn f(): Int {1}; = f}"
    );
    assert_eq!(
        parse_text("fn f(): Int { 1 }")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n\
             \x20 (fn f @0..17\n\
             \x20   (fn @0..17\n\
             \x20     (params)\n\
             \x20     (ret pure Int)\n\
             \x20     (block @12..17\n\
             \x20       (expr @14..15\n\
             \x20         (int 1 @14..15))))))"
    );
    // Without a name, `fn` starts an expression item, and an item is
    // never a field or a rebind of a function expression.
    assert_eq!(shape("fn(): Int { 1 }"), "fn(): Int {1}");
    assert_eq!(
        error("fn(): Int { 1 } = 2"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(16, 17)
        )
    );
}

#[test]
fn a_named_function_is_an_item_not_an_expression() {
    let diagnostic = parse_text("let f = fn greet(): Int { 1 }").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected `(` after `fn`, found identifier `greet`"
    );
    assert_eq!(diagnostic.span, Span::new(11, 16));
    assert_eq!(
        diagnostic.notes,
        vec![
            "a named function `fn NAME(...)` is an item; write it on a line of its own, or \
                 drop the name"
        ]
    );
    assert_eq!(
        error("1 + fn f(): Int { 1 }").0,
        "expected `(` after `fn`, found identifier `f`"
    );
    assert_eq!(
        error("fn 1"),
        (
            "expected `(` after `fn`, found `1`".to_owned(),
            Span::new(3, 4)
        )
    );
}

#[test]
fn calls() {
    assert_eq!(shape("f()"), "f()");
    assert_eq!(shape("f(1)"), "f(1)");
    assert_eq!(shape("f(1, 2,)"), "f(1, 2)");
    assert_eq!(shape("f(y = 2)"), "f(y = 2)");
    assert_eq!(shape("f(1, y = 2)"), "f(1, y = 2)");
    assert_eq!(shape("f(y = 2, 1)"), "f(y = 2, 1)");
    assert_eq!(shape("f(x == 1, y = x + 1)"), "f((x == 1), y = (x + 1))");
    // Arguments may span lines inside the parentheses, and a block or a
    // function body among them keeps its newlines.
    assert_eq!(shape("f(\n  1,\n  y = 2,\n)"), "f(1, y = 2)");
    assert_eq!(
        shape("f(fn(n: Int): Int {\n  let m = n\n  m\n}, {\n  x = 1\n  y = 2\n})"),
        "f(fn(n: Int): Int {let m = n; m}, {x = 1; y = 2})"
    );
    // Trailing blocks, with and without an argument list.
    assert_eq!(shape("f {}"), "f(){}");
    assert_eq!(shape("f() {}"), "f(){}");
    assert_eq!(shape("f(1) {}"), "f(1){}");
    assert_eq!(
        shape("service(\"web\") { port = 80; image = \"nginx\" }"),
        "service(\"web\"){port = 80; image = \"nginx\"}"
    );
    assert_eq!(shape("run {\n  1\n}"), "run(){1}");
    // A call binds as tightly as field access.
    assert_eq!(shape("math.square(4)"), "(math . square)(4)");
    assert_eq!(shape("a.b(1).c"), "((a . b)(1) . c)");
    assert_eq!(shape("-f(1)"), "(-f(1))");
    assert_eq!(shape("not f(1)"), "(not f(1))");
    assert_eq!(shape("f(1)(2)"), "f(1)(2)");
    assert_eq!(shape("f(1) + g(2) * 3"), "(f(1) + (g(2) * 3))");
    assert_eq!(shape("f(1) : Int"), "(f(1) : Int)");
    assert_eq!(shape("(f)(1)"), "[f](1)");
    assert_eq!(shape("{ x }(1)"), "{x}(1)");
    // Any operand followed by `(` is a call; the checker rejects calling
    // what is not a function.
    assert_eq!(shape("2 (3)"), "2(3)");
    // A newline ends the item before a `{` or `(`.
    assert_eq!(shape("f\n{ 1 }"), "f; {1}");
    assert_eq!(shape("f(1)\n{ 1 }"), "f(1); {1}");
    assert_eq!(shape("f\n(1)"), "f; [1]");
    // The trailing block is the last argument.
    let ast = parse_text("f(1, y = 2) { 3 }").unwrap();
    let call = ast.get(only_value(&ast));
    assert_eq!(call.span, Span::new(0, 17));
    assert_eq!(call.close_span(), Some(Span::new(16, 17)));
    let ExprKind::Call { callee, args } = call.kind else {
        panic!("expected a call, got {call:?}");
    };
    assert_eq!(ast.get(callee).span, Span::new(0, 1));
    let args = ast.args(args);
    let y = ast.names().lookup("y").unwrap();
    assert_eq!(args[0].kind, ArgKind::Positional);
    assert_eq!(
        args[1].kind,
        ArgKind::Named {
            name: y,
            span: Span::new(5, 6),
            shorthand: false,
        }
    );
    assert_eq!(args[2].kind, ArgKind::Trailing);
    assert_eq!(args[1].span(&ast), Span::new(5, 10));
    assert_eq!(args[2].span(&ast), Span::new(12, 17));
    let ast = parse_text("f(1)").unwrap();
    let call = ast.get(only_value(&ast));
    assert_eq!(call.span, Span::new(0, 4));
    assert_eq!(call.close_span(), Some(Span::new(3, 4)));
    assert_eq!(ast.get(callee).close_span(), None);
}

#[test]
fn call_errors() {
    let diagnostic = parse_text("f(1 2)").unwrap_err();
    assert_eq!(diagnostic.message, "expected `,` or `)`, found `2`");
    assert_eq!(diagnostic.span, Span::new(4, 5));
    assert_eq!(labels(&diagnostic), vec![(Span::new(1, 2), "unclosed `(`")]);
    assert_eq!(
        error("f(1"),
        (
            "expected `,` or `)`, found end of input".to_owned(),
            Span::new(3, 3)
        )
    );
    assert_eq!(
        error("f(,)"),
        ("expected expression, found `,`".to_owned(), Span::new(2, 3))
    );
    assert_eq!(
        error("f(y = )"),
        ("expected expression, found `)`".to_owned(), Span::new(6, 7))
    );
    // Only an expression may be an argument.
    assert_eq!(error("f(let x = 1)").0, "expected expression, found `let`");
    // A call has one trailing block at most; a second is a call of the
    // first call's result, as `f(1)(2)` is.
    assert_eq!(shape("f {} {}"), "f(){}(){}");
}

#[test]
fn an_if_condition_takes_no_trailing_block() {
    assert_eq!(shape("if x { 1 }"), "(if x {1})");
    assert_eq!(shape("if a and b { 1 }"), "(if (a and b) {1})");
    assert_eq!(shape("if not a.b { 1 }"), "(if (not (a . b)) {1})");
    assert_eq!(shape("if f(1) { 2 }"), "(if f(1) {2})");
    assert_eq!(
        shape("if a { 1 } else if x { 2 }"),
        "(if a {1} else (if x {2}))"
    );
    assert_eq!(
        shape("if a { 1 } else if b and c { 2 } else { 3 }"),
        "(if a {1} else (if (b and c) {2} else {3}))"
    );
    // Lifted inside parentheses, argument lists, blocks and
    // interpolations, and restored after them.
    assert_eq!(shape("if (f { }) { 1 }"), "(if [f(){}] {1})");
    assert_eq!(shape("if g(f { }) { 1 }"), "(if g(f(){}) {1})");
    assert_eq!(shape("if { f { } } { 1 }"), "(if {f(){}} {1})");
    assert_eq!(shape("if \"{f { }}\" { 1 }"), "(if \"<f(){}>\" {1})");
    assert_eq!(shape("if (a) and b { 1 }"), "(if ([a] and b) {1})");
    // After the condition, blocks are trailing blocks again.
    assert_eq!(shape("if a { f { } }"), "(if a {f(){}})");
    assert_eq!(
        shape("if a { 1 } else { 2 }\nf { }"),
        "(if a {1} else {2}); f(){}"
    );
    // A function expression in a condition has its body, since the
    // body is not a trailing block.
    assert_eq!(
        shape("if fn(): Bool { true }() { 1 }"),
        "(if fn(): Bool {true}() {1})"
    );
}

#[test]
fn function_types() {
    assert_eq!(
        shape("let f: fn(Int, String): Bool = g"),
        "let f: fn(Int, String): Bool = g"
    );
    assert_eq!(
        shape("let f: fn(name: String, times: Int) -> String = g"),
        "let f: fn(name: String, times: Int) -> String = g"
    );
    assert_eq!(
        shape("let f: fn(): {x: Int} = g"),
        "let f: fn(): {x: Int} = g"
    );
    assert_eq!(
        shape("let f: fn(fn(Int): Int): Int = g"),
        "let f: fn(fn(Int): Int): Int = g"
    );
    assert_eq!(
        shape("let r: {f: fn(Int): Int, g: fn() -> {}} = h"),
        "let r: {f: fn(Int): Int, g: fn() -> {}} = h"
    );
    assert_eq!(shape("g : fn(Int): Int"), "(g : fn(Int): Int)");
    assert_eq!(
        shape("let f: fn(x: Int, Int,): fn() -> Int = g"),
        "let f: fn(x: Int, Int): fn() -> Int = g"
    );
    // Newlines inside the parentheses and after the marker.
    assert_eq!(
        shape("let f: fn(\n  Int,\n  x: Int\n) ->\n  Int = g"),
        "let f: fn(Int, x: Int) -> Int = g"
    );
    // A name followed by `:` names the parameter; a name alone is a type.
    let ast = parse_text("g : fn(x: Int, Int): Int").unwrap();
    let ExprKind::Ascribe { ty, .. } = ast.get(only_value(&ast)).kind else {
        panic!("expected an ascription");
    };
    let ty = ast.type_expr(ty);
    assert_eq!(ty.span, Span::new(4, 24));
    let TypeExprKind::Fn {
        params,
        ret,
        effect,
        ..
    } = ty.kind
    else {
        panic!("expected a function type, got {ty:?}");
    };
    assert_eq!(effect, Effect::Pure);
    assert_eq!(ast.type_expr(ret).span, Span::new(21, 24));
    let params = ast.param_tys(params);
    assert_eq!(params[0].name, ast.names().lookup("x"));
    assert_eq!(params[1].name, None);
    assert_eq!(ast.type_expr(params[1].ty).span, Span::new(15, 18));
    // Errors.
    let diagnostic = parse_text("let f: fn(Int) Int = g").unwrap_err();
    assert_eq!(diagnostic.message, MISSING_RETURN_TYPE);
    assert_eq!(diagnostic.span, Span::new(15, 18));
    assert_eq!(diagnostic.notes, vec![RETURN_TYPE_NOTE]);
    assert_eq!(
        error("let f: fn Int = g"),
        (
            "expected `(` after `fn`, found `Int`".to_owned(),
            Span::new(10, 13)
        )
    );
    assert_eq!(
        error("let f: fn(Int Int): Int = g").0,
        "expected `,` or `)`, found `Int`"
    );
    assert_eq!(error("let f: fn(x:): Int = g").0, "expected a type");
}

#[test]
fn pretty_output_for_functions_and_calls() {
    let text = "fn f(x: Int, y: fn(Int) -> String) -> {} {}\nf(1, y = g) { 2 }";
    let ast = parse_text(text).unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (fn f @0..43\n\
             \x20   (fn @0..43\n\
             \x20     (params (x Int) (y fn(Int) -> String))\n\
             \x20     (ret impure {})\n\
             \x20     (block @41..43)))\n\
             \x20 (expr @44..61\n\
             \x20   (call @44..61\n\
             \x20     (name f @44..45)\n\
             \x20     (int 1 @46..47)\n\
             \x20     (named y\n\
             \x20       (name g @53..54))\n\
             \x20     (trailing\n\
             \x20       (block @56..61\n\
             \x20         (expr @58..59\n\
             \x20           (int 2 @58..59)))))))"
    );
}

#[test]
fn nested_calls_and_functions_count_towards_depth() {
    // N nested calls around a literal are N + 1 levels: each call is one
    // above its argument.
    let calls = |depth: usize| format!("{}1{}", "f(".repeat(depth), ")".repeat(depth));
    let deepest = calls(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(calls(MAX_DEPTH));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    assert_eq!(
        too_deep(calls(10_000)).message,
        "expression nested too deeply"
    );
    // Far beyond the limit, stopped by the recursion count.
    assert_eq!(
        too_deep(calls(200_000)).message,
        "expression nested too deeply"
    );
    // Each nested function expression is two levels, itself and its
    // body; with the innermost body's literal, N of them are 2N + 1.
    let functions =
        |depth: usize| format!("{}1{}", "fn(): Int { ".repeat(depth), " }".repeat(depth));
    let deepest = functions((MAX_DEPTH - 1) / 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(functions((MAX_DEPTH - 1) / 2 + 1)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(functions(10_000)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(functions(200_000)).message,
        "expression nested too deeply"
    );
    // Function types nest by recursion too.
    let types = |depth: usize| format!("let f: {}Int = g", "fn(): ".repeat(depth));
    let deepest = types(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(types(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(types(200_000)).message,
        "expression nested too deeply"
    );
    // Trailing blocks nest as blocks do.
    let trailing = |depth: usize| format!("{}1{}", "f { ".repeat(depth), " }".repeat(depth));
    let deepest = trailing((MAX_DEPTH - 1) / 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(trailing(200_000)).message,
        "expression nested too deeply"
    );
}

// ----- Futures -----

#[test]
fn lazy_takes_the_longest_expression() {
    assert_eq!(shape("lazy a + b"), "(lazy (a + b))");
    assert_eq!(shape("lazy a and b"), "(lazy (a and b))");
    assert_eq!(
        shape("lazy a + b * c : Int"),
        "(lazy (a + (b * (c : Int))))"
    );
    assert_eq!(shape("lazy { x = 1 }"), "(lazy {x = 1})");
    assert_eq!(shape("lazy { run(compiler) }"), "(lazy {run(compiler)})");
    assert_eq!(shape("lazy f(x) { y = 1 }"), "(lazy f(x){y = 1})");
    assert_eq!(
        shape("lazy if a { 1 } else { 2 }"),
        "(lazy (if a {1} else {2}))"
    );
    assert_eq!(shape("lazy lazy 1"), "(lazy (lazy 1))");
    // As an operand, it still takes everything after it.
    assert_eq!(shape("1 + lazy 2 * 3"), "(1 + (lazy (2 * 3)))");
    assert_eq!(shape("let x = lazy 5 + 5"), "let x = (lazy (5 + 5))");
    assert_eq!(
        shape("build = lazy { run(compiler) }"),
        "build = (lazy {run(compiler)})"
    );
    // A block after a condition is still the `if`'s.
    assert_eq!(shape("if lazy a { 1 }"), "(if (lazy a) {1})");
    // The body is a parameterless, impure function without a return
    // type, whose marker is the keyword; the expression spans the
    // keyword through the operand.
    let ast = parse_text("  lazy a + b").unwrap();
    let expr = ast.get(only_value(&ast));
    assert_eq!(expr.span, Span::new(2, 12));
    let ExprKind::Lazy(function) = expr.kind else {
        panic!("expected a lazy, got {expr:?}");
    };
    let decl = ast.fn_decl(function);
    assert!(decl.params.is_empty());
    assert_eq!(decl.ret, None);
    assert_eq!(decl.effect, Effect::Impure);
    assert_eq!(decl.marker, Span::new(2, 6));
    assert_eq!(ast.get(decl.body).span, Span::new(7, 12));
    assert!(matches!(ast.get(decl.body).kind, ExprKind::Binary { .. }));
    assert_eq!(expr.operator_span(), None);
    // Lazies and functions share one table.
    let ast = parse_text("let f = fn(): Future<Int> { lazy 1 }\nlazy f").unwrap();
    assert_eq!(ast.fn_count(), 3);
    assert_eq!(
        error("lazy"),
        (
            "expected expression, found end of input".to_owned(),
            Span::new(4, 4)
        )
    );
}

#[test]
fn await_binds_as_tightly_as_negation() {
    assert_eq!(shape("await x + 1"), "((await x) + 1)");
    assert_eq!(shape("await f()"), "(await f())");
    assert_eq!(shape("await x.y"), "(await (x . y))");
    assert_eq!(shape("-await x"), "(-(await x))");
    assert_eq!(shape("not await x"), "(not (await x))");
    assert_eq!(shape("await -x"), "(await (-x))");
    assert_eq!(shape("await x : Int"), "((await x) : Int)");
    assert_eq!(shape("await await x"), "(await (await x))");
    assert_eq!(shape("await x * await y"), "((await x) * (await y))");
    // A bare `await` item.
    assert_eq!(shape("await conf.build"), "(await (conf . build))");
    let ast = parse_text("await conf.build").unwrap();
    let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an expression item");
    };
    let expr = ast.get(root);
    assert!(matches!(
        expr.kind,
        ExprKind::Unary {
            op: UnaryOp::Await,
            ..
        }
    ));
    assert_eq!(expr.span, Span::new(0, 16));
    assert_eq!(expr.operator_span(), Some(Span::new(0, 5)));
    assert_eq!(
        error("await"),
        (
            "expected expression, found end of input".to_owned(),
            Span::new(5, 5)
        )
    );
}

#[test]
fn applied_and_union_types() {
    assert_eq!(shape("let x: Future<Int> = y"), "let x: Future<Int> = y");
    assert_eq!(
        shape("let x: Future<Future<Int>> = y"),
        "let x: Future<Future<Int>> = y"
    );
    assert_eq!(
        shape("let x: Future<{a: Int}> = y"),
        "let x: Future<{a: Int}> = y"
    );
    assert_eq!(shape("let x: Present<Int> = y"), "let x: Present<Int> = y");
    assert_eq!(
        shape("let x: Int | Future<Int> = y"),
        "let x: (Int | Future<Int>) = y"
    );
    // In parameters, return types, record fields and function types.
    assert_eq!(
        shape("fn(x: Future<Int>, y: Int | Future<Int>): Int | Future<Int> { x }"),
        "fn(x: Future<Int>, y: (Int | Future<Int>)): (Int | Future<Int>) {x}"
    );
    assert_eq!(
        shape("fn plan(): {build: Future<{}>} { build = lazy { run(compiler) } }"),
        "fn plan(): {build: Future<{}>} {build = (lazy {run(compiler)})}"
    );
    assert_eq!(
        shape("1 : {a: Int | Future<Int>, b: Present<{c: Int}>}"),
        "(1 : {a: (Int | Future<Int>), b: Present<{c: Int}>})"
    );
    assert_eq!(
        shape("let f: fn(Int | Future<Int>) -> Future<fn(): Int> = g"),
        "let f: fn((Int | Future<Int>)) -> Future<fn(): Int> = g"
    );
    // A function type's return type takes the union.
    assert_eq!(
        shape("let f: fn() -> Int | Future<Int> = g"),
        "let f: fn() -> (Int | Future<Int>) = g"
    );
    // Newlines after `<` and `|`, but not before `>`.
    assert_eq!(
        shape("let x: Future<\n  Int> = y"),
        "let x: Future<Int> = y"
    );
    assert_eq!(
        shape("let x: Int |\n  Future<Int> = y"),
        "let x: (Int | Future<Int>) = y"
    );
    // Unions of three are left-associative; the checker rejects them.
    assert_eq!(
        shape("let x: Int | Int | Future<Int> = y"),
        "let x: ((Int | Int) | Future<Int>) = y"
    );
    // Any name may be applied; `Future` alone is a name.
    assert_eq!(shape("let x: Foo<Int> = y"), "let x: Foo<Int> = y");
    assert_eq!(shape("let x: Future = y"), "let x: Future = y");
    // `Future` and `Present` are ordinary names in expressions.
    assert_eq!(shape("Future < Present"), "(Future < Present)");
    // Spans: the name, the whole, and the bar.
    let ast = parse_text("1 : Int | Future<Int>").unwrap();
    let ExprKind::Ascribe { ty, .. } = ast.get(only_value(&ast)).kind else {
        panic!("expected an ascription");
    };
    let union = ast.type_expr(ty);
    assert_eq!(union.span, Span::new(4, 21));
    let TypeExprKind::Union { lhs, rhs, bar } = union.kind else {
        panic!("expected a union, got {union:?}");
    };
    assert_eq!(bar, Span::new(8, 9));
    assert_eq!(ast.type_expr(lhs).span, Span::new(4, 7));
    let applied = ast.type_expr(rhs);
    assert_eq!(applied.span, Span::new(10, 21));
    let TypeExprKind::Applied { name, span, args } = applied.kind else {
        panic!("expected an applied type, got {applied:?}");
    };
    assert_eq!(ast.names().get(name), "Future");
    assert_eq!(span, Span::new(10, 16));
    let &[arg] = ast.type_args(args) else {
        panic!("expected one type argument");
    };
    assert_eq!(ast.type_expr(arg).span, Span::new(17, 20));
}

#[test]
fn parenthesised_types() {
    // A function type in parentheses is a member of the union, not a
    // function returning it.
    let text = "let f: (fn(): Int) | Future<fn(): Int> = g";
    assert_eq!(shape(text), "let f: (fn(): Int | Future<fn(): Int>) = g");
    let ast = parse_text(text).unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated let");
    };
    let TypeExprKind::Union { lhs, rhs, .. } = ast.type_expr(ty).kind else {
        panic!("expected a union");
    };
    assert!(matches!(
        ast.type_expr(lhs).kind,
        TypeExprKind::Fn {
            effect: Effect::Pure,
            ..
        }
    ));
    assert!(matches!(
        ast.type_expr(rhs).kind,
        TypeExprKind::Applied { .. }
    ));
    // The parentheses leave no node: the type inside stands for itself,
    // with its own span.
    assert_eq!(ast.type_expr(lhs).span, Span::new(8, 17));
    // The pretty printer shows the parentheses a function type in a
    // union needs.
    assert!(
        ast.pretty()
            .to_string()
            .contains("(let f : (fn(): Int) | Future<fn(): Int> @")
    );
    // A union in parentheses as a function type's return type is what
    // the return type takes without them.
    assert_eq!(
        shape("let f: fn() -> (Int | Future<Int>) = g"),
        "let f: fn() -> (Int | Future<Int>) = g"
    );
    assert_eq!(
        shape("let f: fn() -> (Int | Future<Int>) = g"),
        shape("let f: fn() -> Int | Future<Int> = g")
    );
    // Anywhere a type is, nested, and with newlines after `(`.
    assert_eq!(shape("let x: ((Int)) = y"), "let x: Int = y");
    assert_eq!(
        shape("1 : {a: (fn() -> Int) | Future<fn() -> Int>}"),
        "(1 : {a: (fn() -> Int | Future<fn() -> Int>)})"
    );
    assert_eq!(shape("let x: Future<(Int)> = y"), "let x: Future<Int> = y");
    assert_eq!(
        shape("let x: (\n  Int |\n  Future<Int>\n) = y"),
        "let x: (Int | Future<Int>) = y"
    );
}

#[test]
fn parenthesised_type_errors() {
    let diagnostic = parse_text("let x: (Int = 1").unwrap_err();
    assert_eq!(diagnostic.message, "expected `)` to close the type");
    assert_eq!(diagnostic.span, Span::new(12, 13));
    assert_eq!(labels(&diagnostic), vec![(Span::new(7, 8), "unclosed `(`")]);
    assert_eq!(
        error("let x: () = 1"),
        ("expected a type".to_owned(), Span::new(8, 9))
    );
    assert_eq!(
        error("let x: (Int | ) = 1"),
        ("expected a type".to_owned(), Span::new(14, 15))
    );
}

#[test]
fn applied_type_errors() {
    let diagnostic = parse_text("let x: Future<Int = 1").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "expected `>` to close the type arguments"
    );
    assert_eq!(diagnostic.span, Span::new(18, 19));
    assert_eq!(
        labels(&diagnostic),
        vec![(Span::new(13, 14), "unclosed `<`")]
    );
    // `>` must follow the argument directly.
    assert_eq!(
        error("let x: Future<Int\n> = 1"),
        (
            "expected `>` to close the type arguments".to_owned(),
            Span::new(17, 18)
        )
    );
    assert_eq!(
        error("let x: Future<> = 1"),
        ("expected a type".to_owned(), Span::new(14, 15))
    );
    assert_eq!(
        error("let x: Int | = 1"),
        ("expected a type".to_owned(), Span::new(13, 14))
    );
    // `|` is not an expression operator.
    assert_eq!(
        error("1 + | 2"),
        ("expected expression, found `|`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("| 2"),
        ("expected expression, found `|`".to_owned(), Span::new(0, 1))
    );
    assert_eq!(
        error("a | b"),
        (
            "expected end of item, found `|`".to_owned(),
            Span::new(2, 3)
        )
    );
}

#[test]
fn pretty_output_for_futures() {
    let ast = parse_text(
        "let x: Int | Future<Int> = lazy 1 + 2\nawait x\nlet p: Present<Future<Int>> = x",
    )
    .unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let x : Int | Future<Int> @0..37\n\
             \x20   (lazy @27..37\n\
             \x20     (+ @32..37\n\
             \x20       (int 1 @32..33)\n\
             \x20       (int 2 @36..37))))\n\
             \x20 (expr @38..45\n\
             \x20   (await @38..45\n\
             \x20     (name x @44..45)))\n\
             \x20 (let p : Present<Future<Int>> @46..77\n\
             \x20   (name x @76..77)))"
    );
}

#[test]
fn nested_futures_and_future_types_count_towards_depth() {
    // N nested lazies around a literal are N + 1 levels, and so are N
    // nested awaits.
    for keyword in ["lazy ", "await "] {
        let nested = |depth: usize| format!("{}1", keyword.repeat(depth));
        let deepest = nested(MAX_DEPTH - 1);
        on_big_stack(move || parse_text(&deepest).unwrap());
        assert_eq!(
            too_deep(nested(MAX_DEPTH)).message,
            "expression nested too deeply",
            "{keyword}"
        );
        assert_eq!(
            too_deep(nested(10_000)).message,
            "expression nested too deeply"
        );
        // Far beyond the limit, stopped by the recursion count.
        assert_eq!(
            too_deep(nested(200_000)).message,
            "expression nested too deeply"
        );
    }
    // N nested applied types around a name are N + 1 levels, in an
    // annotation, whose height is its item's.
    let applied = |depth: usize| {
        format!(
            "let x: {}Int{} = y",
            "Future<".repeat(depth),
            ">".repeat(depth)
        )
    };
    let deepest = applied(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(applied(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(applied(200_000)).message,
        "expression nested too deeply"
    );
    // So are N nested parentheses around a type, though they leave no
    // node of their own.
    let parenthesised =
        |depth: usize| format!("let x: {}Int{} = y", "(".repeat(depth), ")".repeat(depth));
    let deepest = parenthesised(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(parenthesised(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(parenthesised(200_000)).message,
        "expression nested too deeply"
    );
    // A union of N types is N levels, built without recursion.
    let union = |members: usize| format!("let x: Int{} = y", " | Int".repeat(members - 1));
    let deepest = union(MAX_DEPTH);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(union(MAX_DEPTH + 1)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(union(200_000)).message,
        "expression nested too deeply"
    );
}

// ----- Lists -----

#[test]
fn list_literals() {
    assert_eq!(shape("[]"), "list()");
    assert_eq!(shape("[1]"), "list(1)");
    assert_eq!(shape("[1, 2,]"), "list(1, 2)");
    assert_eq!(
        shape("[[1], [], [[2]]]"),
        "list(list(1), list(), list(list(2)))"
    );
    // A spread takes a whole element, parsed as any element is.
    assert_eq!(
        shape("[...xs, 1 + 2, ...a + b]"),
        "list(...xs, (1 + 2), ...(a + b))"
    );
    assert_eq!(shape("[...[...xs]]"), "list(...list(...xs))");
    // Across lines, with blocks and records inside.
    assert_eq!(shape("[\n  1,\n  2\n]"), "list(1, 2)");
    assert_eq!(
        shape("[\n  { a = 1\n    b = 2 },\n  {}\n]"),
        "list({a = 1; b = 2}, {})"
    );
    // An `if` condition may hold a list.
    assert_eq!(shape("if xs == [] { 1 }"), "(if (xs == list()) {1})");
    let ast = parse_text("[1, ...xs]").unwrap();
    let list = ast.get(only_value(&ast));
    assert_eq!(list.span, Span::new(0, 10));
    let ExprKind::List(elements) = list.kind else {
        panic!("expected a list, got {list:?}");
    };
    let [first, second] = ast.elements(elements) else {
        panic!("expected two elements");
    };
    assert_eq!(first.spread, None);
    assert_eq!(first.span(&ast), Span::new(1, 2));
    assert_eq!(second.spread, Some(Span::new(4, 7)));
    assert_eq!(second.span(&ast), Span::new(4, 9));
    assert_eq!(ast.get(second.value).span, Span::new(7, 9));
}

#[test]
fn indexes_chain_with_calls_and_field_access() {
    assert_eq!(shape("xs[0]"), "(xs at 0)");
    assert_eq!(shape("xs[0][1]"), "((xs at 0) at 1)");
    assert_eq!(shape("f(x)[0].name[1]"), "(((f(x) at 0) . name) at 1)");
    assert_eq!(shape("xs[i + 1]"), "(xs at (i + 1))");
    assert_eq!(shape("[1, 2][0]"), "(list(1, 2) at 0)");
    assert_eq!(shape("-xs[0] * 2"), "((-(xs at 0)) * 2)");
    assert_eq!(shape("xs[0](1)"), "(xs at 0)(1)");
    assert_eq!(shape("a : Int[0]"), "((a : Int) at 0)");
    // A position may span lines, as inside any brackets.
    assert_eq!(shape("xs[\n  0\n]"), "(xs at 0)");
    let ast = parse_text("xs [ 1 ]").unwrap();
    let index = ast.get(only_value(&ast));
    assert_eq!(index.span, Span::new(0, 8));
    assert_eq!(index.bracket_span(&ast), Some(Span::new(2, 8)));
    let ExprKind::Index { expr, index } = index.kind else {
        panic!("expected an index, got {index:?}");
    };
    assert_eq!(ast.get(expr).span, Span::new(0, 2));
    assert_eq!(ast.get(index).span, Span::new(5, 6));
}

#[test]
fn a_bracket_after_a_newline_starts_a_list() {
    // The item has ended, so the `[` starts the next one.
    assert_eq!(shape("xs\n[0]"), "xs; list(0)");
    assert_eq!(shape("let x = f(1)\n[1, 2]"), "let x = f(1); list(1, 2)");
    // Inside parentheses and brackets there are no newline tokens, so
    // it is still an index.
    assert_eq!(shape("(xs\n[0])"), "[(xs at 0)]");
    assert_eq!(shape("[xs\n[0]]"), "list((xs at 0))");
}

#[test]
fn list_types() {
    assert_eq!(shape("let x: List<Int> = y"), "let x: List<Int> = y");
    assert_eq!(shape("let x: Int[] = y"), "let x: (Int)[] = y");
    assert_eq!(shape("let x: Int[][] = y"), "let x: ((Int)[])[] = y");
    assert_eq!(shape("let x: {a: Int}[] = y"), "let x: ({a: Int})[] = y");
    assert_eq!(
        shape("let x: Future<Int>[] = y"),
        "let x: (Future<Int>)[] = y"
    );
    assert_eq!(shape("let x: List<Int>[] = y"), "let x: (List<Int>)[] = y");
    assert_eq!(
        shape("let x: (fn(): Int)[] = y"),
        "let x: (fn(): Int)[] = y"
    );
    assert_eq!(
        shape("let x: (Int | Future<Int>)[] = y"),
        "let x: ((Int | Future<Int>))[] = y"
    );
    // The postfix binds tighter than `|`, and belongs to a function
    // type's return type.
    assert_eq!(
        shape("let x: Int | Future<Int>[] = y"),
        "let x: (Int | (Future<Int>)[]) = y"
    );
    assert_eq!(shape("let x: fn(): Int[] = y"), "let x: fn(): (Int)[] = y");
    assert_eq!(
        shape("fn(xs: Int[]): Int[] { xs }"),
        "fn(xs: (Int)[]): (Int)[] {xs}"
    );
    let ast = parse_text("let x: Int[][] = y").unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated let");
    };
    let outer = ast.type_expr(ty);
    assert_eq!(outer.span, Span::new(7, 14));
    let TypeExprKind::List(inner) = outer.kind else {
        panic!("expected a list type, got {outer:?}");
    };
    assert_eq!(ast.type_expr(inner).span, Span::new(7, 12));
    // A `[` not directly followed by `]` is not part of the type.
    assert_eq!(
        error("let x: Int[3] = y"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(10, 11)
        )
    );
}

#[test]
fn pretty_output_for_lists() {
    let ast = parse_text(
        "let x: Int[] = [1, ...ys]\nx[0]\nlet f: (fn(): Int)[] = g\nlet u: (Int | \
             Future<Int>)[][] = v\nlet r: fn(): List<Int>[] = h",
    )
    .unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (let x : Int[] @0..25\n\
             \x20   (list @15..25\n\
             \x20     (int 1 @16..17)\n\
             \x20     (spread\n\
             \x20       (name ys @22..24))))\n\
             \x20 (expr @26..30\n\
             \x20   (index @26..30\n\
             \x20     (name x @26..27)\n\
             \x20     (int 0 @28..29)))\n\
             \x20 (let f : (fn(): Int)[] @31..55\n\
             \x20   (name g @54..55))\n\
             \x20 (let u : (Int | Future<Int>)[][] @56..90\n\
             \x20   (name v @89..90))\n\
             \x20 (let r : fn(): List<Int>[] @91..119\n\
             \x20   (name h @118..119)))"
    );
}

#[test]
fn list_errors() {
    // A spread anywhere an expression starts, but in a list or at the
    // start of an item.
    for (text, at) in [("f(...xs)", 2), ("let x = ...xs", 8), ("[(...xs)]", 2)] {
        assert_eq!(
            error(text),
            (MISPLACED_SPREAD.to_owned(), Span::new(at, at + 3)),
            "{text}"
        );
    }
    let diagnostic = parse_text("[1 2]").unwrap_err();
    assert_eq!(diagnostic.message, "expected `,` or `]`, found `2`");
    assert_eq!(diagnostic.span, Span::new(3, 4));
    assert_eq!(labels(&diagnostic), vec![(Span::new(0, 1), "unclosed `[`")]);
    assert_eq!(
        error("[1, 2"),
        (
            "expected `,` or `]`, found end of input".to_owned(),
            Span::new(5, 5)
        )
    );
    assert_eq!(
        error("[,]"),
        ("expected expression, found `,`".to_owned(), Span::new(1, 2))
    );
    assert_eq!(
        error("]"),
        ("expected expression, found `]`".to_owned(), Span::new(0, 1))
    );
    let diagnostic = parse_text("xs[0").unwrap_err();
    assert_eq!(diagnostic.message, "expected `]`, found end of input");
    assert_eq!(labels(&diagnostic), vec![(Span::new(2, 3), "unclosed `[`")]);
    assert_eq!(
        error("xs[]"),
        ("expected expression, found `]`".to_owned(), Span::new(3, 4))
    );
    assert_eq!(
        error("xs[0, 1]"),
        ("expected `]`, found `,`".to_owned(), Span::new(4, 5))
    );
}

#[test]
fn spread_items() {
    // A spread item in a block, first, last and alone.
    assert_eq!(shape("{ ...a; b = 1 }"), "{...a; b = 1}");
    assert_eq!(shape("{ b = 1; ...a }"), "{b = 1; ...a}");
    assert_eq!(shape("{ ...a }"), "{...a}");
    assert_eq!(shape("{\n  ...a\n  ...{ c = 2 }\n}"), "{...a; ...{c = 2}}");
    // At the top level, and in a function body.
    assert_eq!(shape("...a\nb = 1"), "...a; b = 1");
    assert_eq!(shape("...a"), "...a");
    assert_eq!(shape("fn(): {} { ...a }"), "fn(): {} {...a}");
    // Its expression is parsed as a field's value is, at minimum 0, so
    // an ascription belongs to it.
    assert_eq!(shape("{ ...a : {b: Int} }"), "{...(a : {b: Int})}");
    assert_eq!(shape("{ ...a.b + c }"), "{...((a . b) + c)}");
    assert_eq!(shape("{ ...f(x) }"), "{...f(x)}");
    // A newline after `...` does not end the item.
    assert_eq!(shape("{ ...\n  a }"), "{...a}");
    // A block with a result parses; that is the checker's error.
    assert_eq!(shape("{ ...a; = 1 }"), "{...a; = 1}");
    // What is printed parses back to itself.
    for text in [
        "{...a; b = 1}",
        "...a; ...{c = 2}",
        "fn(): {} {x = 1; ...a}",
    ] {
        assert_eq!(shape(text), text);
    }
    // The item spans `...` through its expression.
    let ast = parse_text("{ ...ab : T }").unwrap();
    let ExprKind::Block { items, .. } = ast.get(only_value(&ast)).kind else {
        panic!("expected a block");
    };
    let item = ast.item(ast.items(items)[0]);
    assert_eq!(item.span, Span::new(2, 11));
    let ItemKind::Spread(value) = item.kind else {
        panic!("expected a spread, got {item:?}");
    };
    assert_eq!(ast.get(value).span, Span::new(5, 11));
    // A `...` inside an item's expression is still misplaced.
    for (text, at) in [
        ("x = ...xs", 4),
        ("{ (...xs) }", 3),
        ("{ ...xs + ...ys }", 10),
    ] {
        assert_eq!(
            error(text),
            (MISPLACED_SPREAD.to_owned(), Span::new(at, at + 3)),
            "{text}"
        );
    }
    assert_eq!(
        error("{ ... }"),
        ("expected expression, found `}`".to_owned(), Span::new(6, 7))
    );
    // A spread item nests as a field item does: each block is two
    // levels of recursion, as `nesting_blocks_is_limited` counts them.
    let spreads = |depth: usize| format!("{}1{}", "{...".repeat(depth), "}".repeat(depth));
    let deepest = spreads((MAX_DEPTH - 1) / 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(spreads((MAX_DEPTH - 1) / 2 + 1)).message,
        "expression nested too deeply"
    );
}

#[test]
fn pretty_output_for_spread_items() {
    let ast = parse_text("...a\n{ ...b }").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (spread @0..4\n\
             \x20   (name a @3..4))\n\
             \x20 (expr @5..13\n\
             \x20   (block @5..13\n\
             \x20     (spread @7..11\n\
             \x20       (name b @10..11)))))"
    );
}

#[test]
fn lists_and_indexes_count_towards_depth() {
    // N nested literals around a literal are N + 1 levels.
    let nested = |depth: usize| format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let deepest = nested(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(nested(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(nested(200_000)).message,
        "expression nested too deeply"
    );
    // A spread is as tall as its expression.
    let spreads = |depth: usize| format!("{}1{}", "[...".repeat(depth), "]".repeat(depth));
    let deepest = spreads(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(spreads(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    // A chain of N indexes on a name is N + 1 levels, built without
    // recursion; so is N nested in each other's positions.
    let chain = |depth: usize| format!("x{}", "[0]".repeat(depth));
    let deepest = chain(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(chain(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(chain(200_000)).message,
        "expression nested too deeply"
    );
    let positions = |depth: usize| format!("{}0{}", "x[".repeat(depth), "]".repeat(depth));
    let deepest = positions(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(positions(200_000)).message,
        "expression nested too deeply"
    );
    // N `[]`s on a name are N + 1 levels of type.
    let ty = |depth: usize| format!("let x: Int{} = y", "[]".repeat(depth));
    let deepest = ty(MAX_DEPTH - 1);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(ty(MAX_DEPTH)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(ty(200_000)).message,
        "expression nested too deeply"
    );
}

// ----- Ranges and `for` -----

#[test]
fn ranges_bind_between_comparison_and_addition() {
    assert_eq!(shape("0..10"), "(0 .. 10)");
    assert_eq!(shape("0..=10"), "(0 ..= 10)");
    assert_eq!(shape("a..b"), "(a .. b)");
    assert_eq!(shape("0 .. 1"), "(0 .. 1)");
    // Looser than `+` and `-`, on either side.
    assert_eq!(shape("0..len(xs) - 1"), "(0 .. (len(xs) - 1))");
    assert_eq!(shape("a + 1..=b * 2"), "((a + 1) ..= (b * 2))");
    // Tighter than the comparisons, on either side.
    assert_eq!(shape("a..b == c"), "((a .. b) == c)");
    assert_eq!(shape("c == a..=b"), "(c == (a ..= b))");
    assert_eq!(shape("a..b < c..d"), "((a .. b) < (c .. d))");
    assert_eq!(shape("x and a..b != c"), "(x and ((a .. b) != c))");
    assert_eq!(shape("not a..b"), "(not (a .. b))");
    // Negation binds tighter still.
    assert_eq!(shape("-1..1"), "((-1) .. 1)");
    assert_eq!(shape("0..-1"), "(0 .. (-1))");
    assert_eq!(shape("-a..=-b"), "((-a) ..= (-b))");
    // Postfix operators apply to the bound, and an ascription too.
    assert_eq!(shape("a.b..c[0]"), "((a . b) .. (c at 0))");
    assert_eq!(shape("0..n : Int"), "(0 .. (n : Int))");
    assert_eq!(shape("(0..n)[1]"), "([(0 .. n)] at 1)");
    // Newlines after the operator are skipped, as after any binary
    // operator, and not before it.
    assert_eq!(shape("0..\n\n  10"), "(0 .. 10)");
    assert_eq!(shape("0..=\n10"), "(0 ..= 10)");
    assert_eq!(
        error("0\n..10"),
        (
            "expected expression, found `..`".to_owned(),
            Span::new(2, 4)
        )
    );
    // A range is an operand like any other: in a list, as an argument,
    // spread.
    assert_eq!(shape("[0..2, ...1..=3]"), "list((0 .. 2), ...(1 ..= 3))");
    assert_eq!(shape("f(0..n, m = 1..2)"), "f((0 .. n), m = (1 .. 2))");
    // Neither dot of a range is part of a number.
    assert_eq!(shape("1.5..2"), "(1.5 .. 2)");
}

#[test]
fn ranges_cannot_be_chained() {
    let diagnostic = parse_text("a..b..c").unwrap_err();
    assert_eq!(diagnostic.message, "`..` cannot be chained");
    assert_eq!(diagnostic.span, Span::new(4, 6));
    assert!(diagnostic.notes.is_empty());
    assert_eq!(
        error("a..=b..=c"),
        ("`..=` cannot be chained".to_owned(), Span::new(5, 8))
    );
    // With each other, either way round.
    assert_eq!(
        error("a..b..=c"),
        ("`..=` cannot be chained".to_owned(), Span::new(4, 7))
    );
    assert_eq!(
        error("a..=b..c"),
        ("`..` cannot be chained".to_owned(), Span::new(5, 7))
    );
    // After a right operand that is a sum, and on the right of a
    // comparison.
    assert_eq!(
        error("0..n + 1..m"),
        ("`..` cannot be chained".to_owned(), Span::new(8, 10))
    );
    assert_eq!(
        error("x == 0..1..2"),
        ("`..` cannot be chained".to_owned(), Span::new(9, 11))
    );
    // Parentheses separate them.
    assert_eq!(shape("(a..b)..c"), "([(a .. b)] .. c)");
    assert_eq!(shape("a..(b..c)"), "(a .. [(b .. c)])");
}

#[test]
fn for_shapes() {
    assert_eq!(shape("for x in xs { x }"), "for x in xs {x}");
    assert_eq!(shape("for x in xs {}"), "for x in xs {}");
    // The guard.
    assert_eq!(
        shape("for x in xs if x > 1 { x }"),
        "for x in xs if (x > 1) {x}"
    );
    assert_eq!(
        shape("for x in xs if a and not b { x }"),
        "for x in xs if (a and (not b)) {x}"
    );
    // An iterable that is a call, an index, a field, a range, an `if`,
    // a block, a list.
    assert_eq!(shape("for x in f(y) { x }"), "for x in f(y) {x}");
    assert_eq!(shape("for x in xs[0] { x }"), "for x in (xs at 0) {x}");
    assert_eq!(shape("for x in r.xs { x }"), "for x in (r . xs) {x}");
    assert_eq!(shape("for i in 0..n { i }"), "for i in (0 .. n) {i}");
    assert_eq!(
        shape("for i in (0..=n) if i != 2 { i }"),
        "for i in [(0 ..= n)] if (i != 2) {i}"
    );
    assert_eq!(
        shape("for x in if a { xs } else { ys } { x }"),
        "for x in (if a {xs} else {ys}) {x}"
    );
    assert_eq!(
        shape("for x in if a { xs } else { ys } if x { x }"),
        "for x in (if a {xs} else {ys}) if x {x}"
    );
    assert_eq!(shape("for x in { xs } { x }"), "for x in {xs} {x}");
    assert_eq!(shape("for x in [1, 2] { x }"), "for x in list(1, 2) {x}");
    // A `{` after the iterable or the guard is the body, never a
    // trailing block; in parentheses it is one.
    assert_eq!(shape("for x in f { x }"), "for x in f {x}");
    assert_eq!(shape("for x in xs if g { x }"), "for x in xs if g {x}");
    assert_eq!(shape("for x in (f { 1 }) { x }"), "for x in [f(){1}] {x}");
    assert_eq!(shape("for x in f(g { 1 }) { x }"), "for x in f(g(){1}) {x}");
    // Nested, and in the body.
    assert_eq!(
        shape("for x in xs { for y in x { y } }"),
        "for x in xs {for y in x {y}}"
    );
    assert_eq!(
        shape("for x in for y in ys { y } { x }"),
        "for x in for y in ys {y} {x}"
    );
    assert_eq!(
        shape("for x in xs {\n  let y = x * 2\n  y\n}"),
        "for x in xs {let y = (x * 2); y}"
    );
    // A `for` is an operand like any other prefix expression: in a
    // list, as an argument, spread, in a `let`, followed by an index.
    assert_eq!(
        shape("[for x in xs { x }, ...for y in ys { y }]"),
        "list(for x in xs {x}, ...for y in ys {y})"
    );
    assert_eq!(
        shape("f(for x in xs { x }, n = for y in ys { y })"),
        "f(for x in xs {x}, n = for y in ys {y})"
    );
    assert_eq!(
        shape("let ys = for x in xs { x }"),
        "let ys = for x in xs {x}"
    );
    assert_eq!(shape("for x in xs { x }[0]"), "(for x in xs {x} at 0)");
    assert_eq!(shape("1 + for x in xs { x }"), "(1 + for x in xs {x})");
    assert_eq!(shape("{ ...for x in xs { x } }"), "{...for x in xs {x}}");
    // What is printed parses back to itself.
    for text in [
        "for x in xs {x}",
        "for x in xs if g {x}",
        "let ys = for x in xs {for y in x {y}}",
        "f(for x in xs {x}, n = for y in ys {y})",
    ] {
        assert_eq!(shape(text), text);
    }
}

#[test]
fn a_for_spans_its_keyword_through_its_body() {
    let ast = parse_text("for x in xs if g { x }").unwrap();
    let root = only_value(&ast);
    let expr = ast.get(root);
    assert_eq!(expr.span, Span::new(0, 22));
    assert_eq!(expr.keyword_span(), Some(Span::new(0, 3)));
    let ExprKind::For(id) = expr.kind else {
        panic!("expected a for, got {expr:?}");
    };
    let decl = ast.for_decl(id);
    let ForHead::Each {
        name,
        name_span,
        iterable,
        guard,
    } = decl.head
    else {
        panic!("expected an iteration, got {decl:?}");
    };
    assert_eq!(ast.names().get(name), "x");
    assert_eq!(name_span, Span::new(4, 5));
    assert_eq!(ast.get(iterable).span, Span::new(9, 11));
    assert_eq!(
        guard.map(|guard| ast.get(guard).span),
        Some(Span::new(15, 16))
    );
    assert_eq!(ast.get(decl.body).span, Span::new(17, 22));
    assert_eq!(ast.for_count(), 1);
    // Without a guard.
    let ast = parse_text("for x in xs {}").unwrap();
    let ExprKind::For(id) = ast.get(only_value(&ast)).kind else {
        panic!("expected a for");
    };
    let ForHead::Each { guard, .. } = ast.for_decl(id).head else {
        panic!("expected an iteration");
    };
    assert_eq!(guard, None);
}

#[test]
fn for_errors() {
    // A body on the next line is the error an `if`'s is.
    assert_eq!(
        error("for x in xs\n{ x }"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(11, 12)
        )
    );
    assert_eq!(
        error("for x in xs if g\n{ x }"),
        (
            "expected `{` after `for` guard".to_owned(),
            Span::new(16, 17)
        )
    );
    assert_eq!(
        error("for x in xs x"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(13, 13)
        )
    );
    assert_eq!(
        error("for x in xs"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(11, 11)
        )
    );
    // The variable is a plain name.
    assert_eq!(
        error("for 1 in xs {}"),
        ("expected a name after `for`".to_owned(), Span::new(4, 5))
    );
    assert_eq!(
        error("for \"x\" in xs {}"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(4, 7))
    );
    assert_eq!(
        error("for a.b in xs {}"),
        (
            "expected `in` after the `for` variable, found `.`".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("for x 1 {}"),
        (
            "expected `in` after the `for` variable, found `1`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("for x in {}"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(11, 11)
        )
    );
    assert_eq!(
        error("for x in xs if {}"),
        (
            "expected `{` after `for` guard".to_owned(),
            Span::new(17, 17)
        )
    );
    assert_eq!(
        error("in"),
        (
            "expected expression, found `in`".to_owned(),
            Span::new(0, 2)
        )
    );
    assert_eq!(
        error("for x in xs { x } 2"),
        (
            "expected end of item, found `2`".to_owned(),
            Span::new(18, 19)
        )
    );
    // `for` and `in` are keywords, not names.
    assert_eq!(
        error("let in = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 6))
    );
}

#[test]
fn pretty_output_for_ranges_and_fors() {
    let ast = parse_text("for i in 0..=n if i != 1 { i }\nfor x in xs {}").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..30\n\
             \x20   (for i @0..30\n\
             \x20     (..= @9..14\n\
             \x20       (int 0 @9..10)\n\
             \x20       (name n @13..14))\n\
             \x20     (guard\n\
             \x20       (!= @18..24\n\
             \x20         (name i @18..19)\n\
             \x20         (int 1 @23..24)))\n\
             \x20     (block @25..30\n\
             \x20       (expr @27..28\n\
             \x20         (name i @27..28)))))\n\
             \x20 (expr @31..45\n\
             \x20   (for x @31..45\n\
             \x20     (name xs @40..42)\n\
             \x20     (block @43..45))))"
    );
}

#[test]
fn fors_count_towards_depth() {
    // A `for` is one above the tallest of its parts, as an `if` is:
    // nested in each other's bodies, N `for`s around a literal are 2N +
    // 1 levels, and two levels of parser recursion each.
    let nested =
        |depth: usize| format!("{}1{}", "for x in xs { ".repeat(depth), " }".repeat(depth));
    let deepest = nested((MAX_DEPTH - 1) / 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(nested((MAX_DEPTH - 1) / 2 + 1)).message,
        "expression nested too deeply"
    );
    assert_eq!(
        too_deep(nested(200_000)).message,
        "expression nested too deeply"
    );
    // Nested in each other's iterables, N `for`s are N + 2 levels, the
    // innermost being one above its body, a block around a name; and in
    // each other's guards.
    let iterables =
        |depth: usize| format!("{}xs{}", "for x in ".repeat(depth), " { x }".repeat(depth));
    let deepest = iterables(MAX_DEPTH - 2);
    on_big_stack(move || parse_text(&deepest).unwrap());
    assert_eq!(
        too_deep(iterables(MAX_DEPTH - 1)).message,
        "expression nested too deeply"
    );
    let guards = |depth: usize| {
        format!(
            "{}g{}",
            "for x in xs if ".repeat(depth),
            " {}".repeat(depth)
        )
    };
    assert_eq!(
        too_deep(guards(200_000)).message,
        "expression nested too deeply"
    );
    // A chain of ranges is an error, but ranges nest in parentheses.
    let ranges = |depth: usize| format!("{}0{}", "(0..".repeat(depth), ")".repeat(depth));
    assert_eq!(
        too_deep(ranges(200_000)).message,
        "expression nested too deeply"
    );
}

/// Whether each item of the file's body is a `let mut`.
fn mutability(text: &str) -> Vec<bool> {
    let ast = parse_text(text).unwrap();
    ast.items(ast.root())
        .iter()
        .map(|&id| matches!(ast.item(id).kind, ItemKind::Let { mutable: true, .. }))
        .collect()
}

/// The message, span and notes of the syntax error in `text`.
fn error_with_notes(text: &str) -> (String, Span, Vec<String>) {
    let diagnostic = parse_text(text).unwrap_err();
    (diagnostic.message, diagnostic.span, diagnostic.notes)
}

#[test]
fn mutable_lets() {
    // Both forms, which are `let`s, marked as mutable.
    assert_eq!(
        mutability("let mut x = 1\nlet y = 2\nlet mut z: Float = 3\nz := 4\nfn f(): Int { 1 }"),
        vec![true, false, true, false, false]
    );
    assert_eq!(shape("let mut x = 1"), "let x = 1");
    assert_eq!(shape("let mut x: Int = 1"), "let x: Int = 1");
    // A newline may follow `mut`, as it may follow `let`.
    assert_eq!(mutability("let mut\n  x = 1"), vec![true]);
    assert_eq!(mutability("let\nmut\nx =\n1"), vec![true]);
    assert_eq!(mutability("let mut x:\n  Int = 1"), vec![true]);
    // The item spans `let` through its value, and its name is its path.
    let ast = parse_text("  let mut total = 0").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(2, 19));
    let ItemKind::Let { path, ty, .. } = item.kind else {
        panic!("expected a let");
    };
    assert!(ty.is_none());
    let spans: Vec<Span> = ast.path(path).iter().map(|s| s.span).collect();
    assert_eq!(spans, vec![Span::new(10, 15)]);
    // In blocks and function bodies too, each marked on its own.
    let ast = parse_text("{ let a = 1; let mut b = 2; fn(): Int { let mut c = 3; c } }").unwrap();
    let printed = ast.pretty().to_string();
    for head in ["(let a @", "(let mut b @", "(let mut c @"] {
        assert!(printed.contains(head), "{head}: {printed}");
    }
}

#[test]
fn pretty_output_for_mutable_lets() {
    let text = "let mut x = 1\nlet y = 2\nlet mut z: Float = 3";
    let expected = "(file\n\
                        \x20 (let mut x @0..13\n\
                        \x20   (int 1 @12..13))\n\
                        \x20 (let y @14..23\n\
                        \x20   (int 2 @22..23))\n\
                        \x20 (let mut z : Float @24..44\n\
                        \x20   (int 3 @43..44)))";
    assert_eq!(parse_text(text).unwrap().pretty().to_string(), expected);
    // The printed tree says all the source does: laid out otherwise, the
    // same items print the same but for their spans, and a `let` that is
    // not mutable prints without `mut`.
    let spread = "let\nmut\nx = 1\nlet y = 2\nlet mut\nz: Float = 3";
    let printed = parse_text(spread).unwrap().pretty().to_string();
    let heads = |printed: &str| -> Vec<String> {
        printed
            .lines()
            .map(|line| line.split(" @").next().unwrap_or(line).to_owned())
            .collect()
    };
    assert_eq!(heads(&printed), heads(expected));
    let plain = parse_text("let x = 1").unwrap().pretty().to_string();
    assert_eq!(plain, "(file\n  (let x @0..9\n    (int 1 @8..9)))");
}

#[test]
fn mut_is_only_allowed_after_let() {
    let misplaced = |at: u32, note: Option<&str>| {
        (
            MISPLACED_MUT.to_owned(),
            Span::new(at, at + 3),
            note.map(str::to_owned).into_iter().collect::<Vec<_>>(),
        )
    };
    // A field, with a note: dotted, quoted or plain.
    assert_eq!(
        error_with_notes("mut x = 1"),
        misplaced(0, Some(MUT_FIELD_NOTE))
    );
    assert_eq!(
        error_with_notes("{ mut a.\"b c\".d = 1 }"),
        misplaced(2, Some(MUT_FIELD_NOTE))
    );
    assert_eq!(
        error_with_notes("mut \"q\" = 1"),
        misplaced(0, Some(MUT_FIELD_NOTE))
    );
    assert_eq!(
        MUT_FIELD_NOTE,
        "a field is defined once; declare a variable with `let mut x = ...` and define the \
             field from it"
    );
    // A parameter, with a note.
    assert_eq!(
        error_with_notes("fn(mut n: Int): Int { n }"),
        misplaced(3, Some(MUT_PARAM_NOTE))
    );
    assert_eq!(
        error_with_notes("fn f(a: Int, mut b: Int): Int { a }"),
        misplaced(13, Some(MUT_PARAM_NOTE))
    );
    assert_eq!(
        MUT_PARAM_NOTE,
        "copy the parameter into a mutable variable, as in `let mut n = n`"
    );
    // A `for`'s variable, a named function item and a function
    // expression, without one.
    for (text, at) in [
        ("for mut x in xs { x }", 4),
        ("fn mut f(): Int { 1 }", 3),
        ("let g = fn mut(): Int { 1 }", 11),
    ] {
        assert_eq!(error_with_notes(text), misplaced(at, None), "{text}");
    }
    // Anywhere else an item, an expression, a name or a type starts, or
    // an item ends.
    for (text, at) in [
        ("mut x := 1", 0),
        ("mut x", 0),
        ("mut", 0),
        ("1 + mut", 4),
        ("f(mut x)", 2),
        ("f(mut = 1)", 2),
        ("[mut]", 1),
        ("x := mut", 5),
        ("let x: mut = 1", 7),
        ("let x: List<mut> = []", 12),
        ("let f: fn(mut: Int): Int = g", 10),
        ("a.mut", 2),
        ("a.mut = 1", 2),
        ("let r: {mut: Int} = r", 8),
        ("let mut mut x = 1", 8),
        ("let x = 1 mut", 10),
        ("x mut", 2),
        ("if mut { 1 }", 3),
        ("\"{mut}\"", 2),
    ] {
        assert_eq!(error_with_notes(text), misplaced(at, None), "{text}");
    }
    // Where a separator or a closing bracket is expected inside an
    // expression, the usual error stands.
    assert_eq!(
        error("f(x mut)"),
        (
            "expected `,` or `)`, found `mut`".to_owned(),
            Span::new(4, 7)
        )
    );
}

#[test]
fn a_mutable_let_needs_a_plain_name() {
    let note = vec![DOTTED_MUT_NOTE.to_owned()];
    assert_eq!(
        error_with_notes("let mut a.b = 1"),
        (DOTTED_MUT.to_owned(), Span::new(8, 11), note.clone())
    );
    assert_eq!(
        error_with_notes("let mut a.\"b c\".d: Int = 1"),
        (DOTTED_MUT.to_owned(), Span::new(8, 17), note)
    );
    assert_eq!(
        DOTTED_MUT_NOTE,
        "a dotted `let` updates a variable; declare the variable with `let mut a = ...` and \
             update it with `a.b := ...`"
    );
    // A plain name, as after `let`.
    assert_eq!(
        error("let mut \"x\" = 1"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(8, 11))
    );
    assert_eq!(
        error("let mut = 1"),
        ("expected a name after `mut`".to_owned(), Span::new(8, 9))
    );
    assert_eq!(
        error("let = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 5))
    );
}

#[test]
fn index_segments_in_rebind_paths() {
    for (text, expected) in [
        // First, last, mixed with names, and two in a row.
        ("xs[0] := 1", "xs[0] := 1"),
        ("xs[0].a := 1", "xs[0].a := 1"),
        ("a.b[0] := 1", "a.b[0] := 1"),
        ("conf.hosts[0].up := true", "conf.hosts[0].up := true"),
        ("grid[y][x] := 1", "grid[y][x] := 1"),
        ("a.\"b c\"[0].d := 1", "a.\"b c\"[0].d := 1"),
        // Any expression as the position: a call, arithmetic, another
        // index, a block, even one holding a path of its own.
        ("xs[f(1, y = 2)] := 1", "xs[f(1, y = 2)] := 1"),
        ("xs[i + 1 * 2] := 1", "xs[(i + (1 * 2))] := 1"),
        ("xs[ys[0]] := 1", "xs[(ys at 0)] := 1"),
        ("xs[{ let i = 1; i }] := 2", "xs[{let i = 1; i}] := 2"),
        (
            "xs[{ ys[0] := 1; 0 }][a.b] := 2",
            "xs[{ys[0] := 1; 0}][(a . b)] := 2",
        ),
        // Inside brackets, newlines do not end the item.
        ("xs[\n  i\n] := 1", "xs[i] := 1"),
        ("xs[0] :=\n  1", "xs[0] := 1"),
        // In a block.
        ("{ xs[0] := 1 }", "{xs[0] := 1}"),
    ] {
        assert_eq!(shape(text), expected, "{text}");
    }
    // An index read, compared or accessed is an expression, as ever.
    assert_eq!(shape("xs[0]"), "(xs at 0)");
    assert_eq!(shape("xs[0] == 1"), "((xs at 0) == 1)");
    assert_eq!(shape("xs[0].a"), "((xs at 0) . a)");
    // A path is read from its tokens: its positions are its only nodes.
    assert_eq!(parse_text("xs[i].a[0] := 1").unwrap().len(), 3);
}

#[test]
fn index_segments_span_their_brackets() {
    let text = "  xs[i + 1].name[ 0 ] := v";
    let ast = parse_text(text).unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(2, 26));
    let ItemKind::Rebind { path, .. } = item.kind else {
        panic!("expected a rebind");
    };
    let segments = ast.path(path);
    let spans: Vec<Span> = segments.iter().map(|segment| segment.span).collect();
    assert_eq!(
        spans,
        vec![
            Span::new(2, 4),
            Span::new(4, 11),
            Span::new(12, 16),
            Span::new(16, 21)
        ]
    );
    assert_eq!(path_span(segments), Span::new(2, 21));
    // Each position is an expression of the tree, and the path shows it
    // as it is written between the brackets.
    let positions: Vec<Span> = segments
        .iter()
        .filter_map(PathSegment::position)
        .map(|position| ast.get(position).span)
        .collect();
    assert_eq!(positions, vec![Span::new(5, 10), Span::new(18, 19)]);
    assert_eq!(ast.show_path(segments).to_string(), "xs[i + 1].name[0]");
    assert_eq!(ast.show_path(&segments[..2]).to_string(), "xs[i + 1]");
    assert_eq!(segments[0].name(), ast.names().lookup("xs"));
    assert_eq!(segments[1].name(), None);
    // A block whose first item has a position starts with it.
    let ast = parse_text("{ xs[i] := v }").unwrap();
    let ItemKind::Expr(block) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected a block");
    };
    let nodes: Vec<Span> = ast.subtree(block).map(|id| ast.get(id).span).collect();
    assert_eq!(
        nodes,
        vec![Span::new(5, 6), Span::new(11, 12), Span::new(0, 14)]
    );
}

#[test]
fn an_index_on_the_next_line_is_not_a_segment() {
    // As in an expression, the `[` starts the next item, a list literal,
    // which is not a path.
    assert_eq!(shape("xs\n[0]"), "xs; list(0)");
    assert_eq!(
        error("xs\n[0] := 1"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(7, 9)
        )
    );
    assert_eq!(
        error("{ a.b\n[0] = 1 }"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(10, 11)
        )
    );
    assert_eq!(
        error("let xs\n[0] = 1"),
        (
            "expected `=` after variable name".to_owned(),
            Span::new(6, 7)
        )
    );
}

#[test]
fn only_a_rebind_path_may_index_a_list() {
    for (text, message, (start, end), path) in [
        ("let xs[0] = 1", LET_INDEX, (6, 9), "xs[0]"),
        (
            "let a.b[i + 1].c: Int = 1",
            LET_INDEX,
            (7, 14),
            "a.b[i + 1].c",
        ),
        ("let mut xs[0] = 1", LET_INDEX, (10, 13), "xs[0]"),
        ("xs[0] = 1", FIELD_INDEX, (2, 5), "xs[0]"),
        ("{ a.xs[0].b = 1 }", FIELD_INDEX, (6, 9), "a.xs[0].b"),
        ("\"q\"[0] = 1", FIELD_INDEX, (3, 6), "\"q\"[0]"),
        // A path written over several lines is shown on one.
        (
            "let xs[\n  i +\n  1\n] = 1",
            LET_INDEX,
            (6, 19),
            "xs[ i + 1 ]",
        ),
    ] {
        assert_eq!(
            error_with_notes(text),
            (
                message.to_owned(),
                Span::new(start, end),
                vec![format!("update the element with `{path} := ...`")]
            ),
            "{text}"
        );
    }
    assert_eq!(LET_INDEX, "a `let` path cannot index a list");
    assert_eq!(FIELD_INDEX, "a field's path cannot index a list");
    // A rebind's first name is a variable's, as ever.
    assert_eq!(
        error("\"x\"[0] := 1"),
        (PLAIN_VARIABLE_NAME.to_owned(), Span::new(0, 3))
    );
    // A mistake in a position is reported first.
    assert_eq!(
        error("let xs[) = 1"),
        ("expected expression, found `)`".to_owned(), Span::new(7, 8))
    );
    // Anything else before `:=` is still not a path.
    let found_colon_eq = |at: u32| {
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(at, at + 2),
        )
    };
    assert_eq!(error("xs[0] + 1 := 2"), found_colon_eq(10));
    assert_eq!(error("(xs)[0] := 2"), found_colon_eq(8));
    assert_eq!(error("f()[0] := 2"), found_colon_eq(7));
}

#[test]
fn pretty_output_for_index_paths() {
    let text = "xs[i + 1].up := true\ngrid[y][0] := 1";
    let expected = "(file\n\
                        \x20 (rebind xs[i + 1].up @0..20\n\
                        \x20   (at\n\
                        \x20     (+ @3..8\n\
                        \x20       (name i @3..4)\n\
                        \x20       (int 1 @7..8)))\n\
                        \x20   (bool true @16..20))\n\
                        \x20 (rebind grid[y][0] @21..36\n\
                        \x20   (at\n\
                        \x20     (name y @26..27))\n\
                        \x20   (at\n\
                        \x20     (int 0 @29..30))\n\
                        \x20   (int 1 @35..36)))";
    assert_eq!(parse_text(text).unwrap().pretty().to_string(), expected);
    // Laid out otherwise, the same items print the same but for their
    // spans.
    let spread = "xs[\n  i +\n  1\n].up :=\n  true\ngrid[ y ][0] := 1";
    let heads = |printed: &str| -> Vec<String> {
        printed
            .lines()
            .map(|line| line.split(" @").next().unwrap_or(line).to_owned())
            .collect()
    };
    let printed = parse_text(spread).unwrap().pretty().to_string();
    assert_eq!(heads(&printed), heads(expected));
    // A path shown as it is written reads back as the same path.
    for path in [
        "conf.hosts[i + 1].up",
        "grid[y][f(x, 2)]",
        "a.\"b c\"[ys[0]].d",
    ] {
        let text = format!("{path} := 1");
        let ast = parse_text(&text).unwrap();
        let ItemKind::Rebind { path: range, .. } = ast.item(ast.items(ast.root())[0]).kind else {
            panic!("expected a rebind");
        };
        let shown = ast.show_path(ast.path(range)).to_string();
        assert_eq!(shown, path);
        assert_eq!(shape(&format!("{shown} := 1")), shape(&text));
    }
}

#[test]
fn index_positions_count_towards_height() {
    // A chain of N terms is N levels: as a position of a path of two
    // segments, the item is two levels more, at the limit with one fewer
    // than the limit allows a chain alone.
    let chain = |terms: usize| format!("0{}", " + 0".repeat(terms - 1));
    let deepest = format!("xs[{}] := 1", chain(MAX_DEPTH - 2));
    on_big_stack(move || parse_text(&deepest).unwrap());
    let diagnostic = too_deep(format!("xs[{}] := 1", chain(MAX_DEPTH - 1)));
    assert_eq!(diagnostic.message, "expression nested too deeply");
    assert_eq!(diagnostic.span.start, 0);
    // Taller than the value, the position decides the height.
    let deep = format!("{{ xs[{}] := 1 }}", chain(MAX_DEPTH - 3));
    on_big_stack(move || parse_text(&deep).unwrap());
    too_deep(format!("{{ xs[{}] := 1 }}", chain(MAX_DEPTH - 2)));
    // Positions nested in each other recurse, and are stopped as any
    // expression nested too deeply is.
    let nested = format!(
        "xs[{}0{}] := 1",
        "ys[".repeat(MAX_DEPTH),
        "]".repeat(MAX_DEPTH)
    );
    assert_eq!(too_deep(nested).message, "expression nested too deeply");
}

#[test]
fn every_loop_form_parses_and_prints_back_to_itself() {
    for text in [
        "for x in xs {x}",
        "for x in xs if g {x}",
        "for c {x}",
        "for {break}",
        "for let i = 0; c; i := n {i}",
        "for let i = 0; let j: Int = 1; c {i}",
        "for c; i := n; xs[i] := 1 {i}",
        "for x := 1; c {x}",
        "for let r.a = 1; c {r}",
        "do {x} for c",
        "do'again {break'again 1} for c",
        "for'rows r in rows {for c in r {continue'rows 0}}",
        "for'spin {break'spin}",
        "for x in xs {break x}",
        "for x in xs {continue}",
        "for x in xs {continue x; break}",
    ] {
        assert_eq!(shape(text), text);
    }
    assert_eq!(
        shape("let first = for x in xs { if x > 5 { break x }; continue }"),
        "let first = for x in xs {(if (x > 5) {break x}); continue}"
    );
    // The summary's examples.
    assert_eq!(
        shape("for n < 3 { n := n + 1 }"),
        "for (n < 3) {n := (n + 1)}"
    );
    assert_eq!(
        shape("let sq = for let i = 0; i < 4; i := i + 1 { i * i }"),
        "let sq = for let i = 0; (i < 4); i := (i + 1) {(i * i)}"
    );
    assert_eq!(
        shape("let tries = do { attempt() } for not ok()"),
        "let tries = do {attempt()} for (not ok())"
    );
}

#[test]
fn a_name_before_in_begins_an_iteration_and_anything_else_a_condition() {
    let head = |text: &str| {
        let ast = parse_text(text).unwrap();
        let ExprKind::For(id) = ast.get(only_value(&ast)).kind else {
            panic!("{text}: expected a loop");
        };
        ast.for_decl(id).head
    };
    assert!(matches!(head("for x in xs {}"), ForHead::Each { .. }));
    assert!(matches!(head("for'a x in xs {}"), ForHead::Each { .. }));
    // A condition that starts with a name.
    for text in ["for x {}", "for x < 3 {}", "for x.done {}", "for x(1) {}"] {
        assert!(
            matches!(head(text), ForHead::While { cond: Some(_), .. }),
            "{text}"
        );
    }
    assert!(matches!(head("for {}"), ForHead::While { cond: None, .. }));
    assert!(matches!(head("do {} for x"), ForHead::Do { .. }));
    // A `let mut` in a header.
    let ast = parse_text("for let mut i = 0; c {}").unwrap();
    assert!(matches!(
        ast.items(ast.root()).first().map(|&id| ast.item(id).kind),
        Some(ItemKind::Expr(_))
    ));
    assert!(ast.pretty().to_string().contains("(let mut i @4..17"));
    // `in` is a keyword, so `for x in` is always an iteration's start.
    assert_eq!(
        error("for x in {}"),
        (
            "expected `{` or `if` after `for` iterable".to_owned(),
            Span::new(11, 11)
        )
    );
}

#[test]
fn a_header_splits_into_items_and_one_condition() {
    let parts = |text: &str| {
        let ast = parse_text(text).unwrap();
        let ExprKind::For(id) = ast.get(only_value(&ast)).kind else {
            panic!("{text}: expected a loop");
        };
        let ForHead::While {
            before,
            cond,
            after,
        } = ast.for_decl(id).head
        else {
            panic!("{text}: expected a condition loop");
        };
        (
            ast.items(before).len(),
            cond.map(|cond| ast.get(cond).span),
            ast.items(after).len(),
        )
    };
    assert_eq!(parts("for c {}"), (0, Some(Span::new(4, 5)), 0));
    assert_eq!(
        parts("for let i = 0; c {}"),
        (1, Some(Span::new(15, 16)), 0)
    );
    assert_eq!(parts("for c; i := 1 {}"), (0, Some(Span::new(4, 5)), 1));
    assert_eq!(
        parts("for let i = 0; let j = 0; c; i := 1; j := 2 {}"),
        (2, Some(Span::new(26, 27)), 2)
    );
    // A `:=` may come before the condition too.
    assert_eq!(parts("for i := 0; c {}"), (1, Some(Span::new(12, 13)), 0));
    // A newline may follow a `;`.
    assert_eq!(
        parts("for let i = 0;\n  c;\n  i := 1 {}"),
        (1, Some(Span::new(17, 18)), 1)
    );
    // The loop spans its keyword through its body, its header's items
    // being items of the tree.
    let ast = parse_text("for let i = 0; c; i := 1 {}").unwrap();
    let expr = ast.get(only_value(&ast));
    assert_eq!(expr.span, Span::new(0, 27));
    assert_eq!(ast.loop_keyword(only_value(&ast)), Span::new(0, 3));
}

#[test]
fn header_errors() {
    // Two conditions, at the second, labelled at the first.
    let diagnostic = parse_text("for a; b {}").unwrap_err();
    assert_eq!(diagnostic.message, "a `for` header has one condition");
    assert_eq!(diagnostic.span, Span::new(7, 8));
    assert_eq!(diagnostic.labels[0].span, Span::new(4, 5));
    assert_eq!(diagnostic.labels[0].text, "the condition is here");
    // Items alone.
    let diagnostic = parse_text("for let i = 0; i := 1 {}").unwrap_err();
    assert_eq!(diagnostic.message, "a `for` header needs a condition");
    assert_eq!(diagnostic.span, Span::new(4, 21));
    assert_eq!(diagnostic.notes, ["write `true` to loop until a `break`"]);
    // A `let` after the condition.
    let diagnostic = parse_text("for c; let i = 0 {}").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "a `let` after the condition would be new on every iteration and seen by nothing"
    );
    assert_eq!(diagnostic.span, Span::new(7, 16));
    assert_eq!(
        diagnostic.notes,
        ["declare it before the condition and rebind it here with `:=`"]
    );
    // No field, spread or named function.
    assert_eq!(
        error("for x = 1; c {}"),
        (
            "a `for` header cannot define a field".to_owned(),
            Span::new(4, 9)
        )
    );
    assert_eq!(error("for ...x; c {}").0, MISPLACED_SPREAD);
    assert_eq!(
        error("for c\n{}"),
        (
            "expected `;` or `{` in a `for` header, found newline".to_owned(),
            Span::new(5, 6)
        )
    );
    assert_eq!(
        error("for a < b 1 {}"),
        (
            "expected `;` or `{` in a `for` header, found `1`".to_owned(),
            Span::new(10, 11)
        )
    );
    // A block as a header's value or condition is the body, so the
    // header is in error; in parentheses it is a value.
    let diagnostic = parse_text("for let r = { a = 1 }; r.a < 3; r.a := r.a + 1 { }").unwrap_err();
    assert_eq!(
        diagnostic.message,
        "a `{` in a `for` header begins its body"
    );
    assert_eq!(diagnostic.span, Span::new(12, 13));
    assert_eq!(
        shape("for let r = ({ a = 1 }); r.a < 3; r.a := r.a + 1 { }"),
        "for let r = [{a = 1}]; ((r . a) < 3); r.a := ((r . a) + 1) {}"
    );
    // `for` directly followed by `{` is `for BLOCK`, an operand like any
    // other.
    assert_eq!(shape("for {x} < 3; c {}"), "(for {x} < 3); c(){}");
    // No `{` is looked for ahead: a block after a `;` is a part.
    assert_eq!(
        error("for c; { }").0,
        "a `{` in a `for` header begins its body"
    );
    // Inside brackets and calls a block is a block again.
    assert_eq!(
        shape("for let x = f({ 1 }); [{ 2 }] != x {}"),
        "for let x = f({1}); (list({2}) != x) {}"
    );
}

#[test]
fn do_and_its_errors() {
    let ast = parse_text("do { x } for c").unwrap();
    let root = only_value(&ast);
    assert_eq!(ast.get(root).span, Span::new(0, 14));
    assert_eq!(ast.loop_keyword(root), Span::new(0, 2));
    // No `for` after the body.
    assert_eq!(
        error("do { x }"),
        (MISSING_DO_FOR.to_owned(), Span::new(8, 8))
    );
    assert_eq!(
        error("do { x } c"),
        (MISSING_DO_FOR.to_owned(), Span::new(9, 10))
    );
    // `for` on the next line.
    assert_eq!(
        error("do { x }\nfor c"),
        (MISSING_DO_FOR.to_owned(), Span::new(8, 9))
    );
    // Only a condition after the `for`.
    for (text, span) in [
        ("do {} for let i = 0", Span::new(10, 13)),
        ("do {} for x in xs", Span::new(10, 14)),
        ("do {} for'a c", Span::new(9, 11)),
    ] {
        assert_eq!(error(text), (DO_CONDITION_ONLY.to_owned(), span), "{text}");
    }
    // A `{` after the condition is not a body: the condition ends as an
    // expression statement does, where a `{` on its line starts a
    // trailing block.
    assert_eq!(shape("do {} for c {}"), "do {} for c(){}");
    // On the next line, it is the next item.
    assert_eq!(shape("do {} for c\n{}"), "do {} for c; {}");
    assert_eq!(
        error("do x for c"),
        (
            "expected `{` after `do`, found `x`".to_owned(),
            Span::new(3, 4)
        )
    );
}

/// The `break` or `continue` that `text` consists of: its kind, as
/// written, its label's name, its value's span and its span.
fn jump(text: &str) -> (&'static str, Option<String>, Option<Span>, Span) {
    let ast = parse_text(text).unwrap();
    let [id] = ast.items(ast.root()) else {
        panic!("{text}: expected one item");
    };
    let item = ast.item(*id);
    let (keyword, jump) = match item.kind {
        ItemKind::Break(jump) => ("break", jump),
        ItemKind::Continue(jump) => ("continue", jump),
        ItemKind::Return(jump) => ("return", jump),
        other => panic!("{text}: expected a jump, got {other:?}"),
    };
    (
        keyword,
        jump.label
            .map(|label| ast.names().get(label.name).to_owned()),
        jump.value.map(|value| ast.get(value).span),
        item.span,
    )
}

#[test]
fn break_and_continue_with_and_without_a_value_and_a_label() {
    assert_eq!(jump("break"), ("break", None, None, Span::new(0, 5)));
    assert_eq!(jump("continue"), ("continue", None, None, Span::new(0, 8)));
    assert_eq!(
        jump("break x + 1"),
        ("break", None, Some(Span::new(6, 11)), Span::new(0, 11))
    );
    assert_eq!(
        jump("break'rows"),
        ("break", Some("rows".to_owned()), None, Span::new(0, 10))
    );
    assert_eq!(
        jump("continue'rows 0"),
        (
            "continue",
            Some("rows".to_owned()),
            Some(Span::new(14, 15)),
            Span::new(0, 15)
        )
    );
    // A newline, a `;` or a `}` after the keyword means no value.
    assert_eq!(shape("break\n1"), "break; 1");
    assert_eq!(shape("continue'a\nx"), "continue'a; x");
    assert_eq!(shape("break; 1"), "break; 1");
    assert_eq!(shape("{ break }"), "{break}");
    // They are items of any plain block, an operand or not.
    assert_eq!(
        shape("let x = if c { break } else { 1 }"),
        "let x = (if c {break} else {1})"
    );
    assert_eq!(shape("f({ continue })"), "f({continue})");
}

#[test]
fn break_and_continue_are_never_expressions() {
    for (text, found, span) in [
        ("x or break", "break", Span::new(5, 10)),
        ("1 + continue", "continue", Span::new(4, 12)),
        ("f(break)", "break", Span::new(2, 7)),
        ("[continue]", "continue", Span::new(1, 9)),
        ("let x = break", "break", Span::new(8, 13)),
        ("= break", "break", Span::new(2, 7)),
    ] {
        assert_eq!(
            error(text),
            (format!("expected expression, found `{found}`"), span),
            "{text}"
        );
    }
}

#[test]
fn a_label_is_written_directly_after_its_keyword() {
    for (text, keyword, span) in [
        ("for 'a x in xs {}", "for", Span::new(4, 6)),
        ("do 'a {} for c", "do", Span::new(3, 5)),
        ("break 'a", "break", Span::new(6, 8)),
        ("continue 'a", "continue", Span::new(9, 11)),
    ] {
        assert_eq!(
            error(text),
            (
                format!("a label is written directly after `{keyword}`, with no space"),
                span
            ),
            "{text}"
        );
    }
    // Anywhere else, a label is misplaced.
    for (text, span) in [
        ("'a", Span::new(0, 2)),
        ("x 'a", Span::new(2, 4)),
        ("1 + 'a", Span::new(4, 6)),
        ("f('a)", Span::new(2, 4)),
        ("let 'a = 1", Span::new(4, 6)),
        ("for'a'b x in xs {}", Span::new(5, 7)),
    ] {
        assert_eq!(error(text), (MISPLACED_LABEL.to_owned(), span), "{text}");
    }
    // Where a particular token is expected, the usual error stands.
    assert_eq!(
        error("if c 'a {}"),
        (
            "expected `{` after `if` condition".to_owned(),
            Span::new(5, 7)
        )
    );
}

#[test]
fn do_break_and_continue_are_not_names() {
    assert_eq!(
        error("let do = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 6))
    );
    assert_eq!(
        error("let break = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 9))
    );
    assert_eq!(
        error("fn(continue: Int): Int { 1 }").0,
        "expected a parameter name, found `continue`"
    );
    assert_eq!(error("x.do").0, "expected a field name after `.`");
    // A quoted field name may still be any text.
    assert_eq!(
        shape("\"do\" = 1; \"break\".x = 2"),
        "\"do\" = 1; \"break\".x = 2"
    );
}

#[test]
fn pretty_output_for_loops_and_jumps() {
    let ast = parse_text(
        "for'w let i = 0; i < 2; i := i + 1 { continue'w i }\n\
             do { break } for c\nfor {}",
    )
    .unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..51\n\
             \x20   (for'w @0..51\n\
             \x20     (let i @6..15\n\
             \x20       (int 0 @14..15))\n\
             \x20     (cond\n\
             \x20       (< @17..22\n\
             \x20         (name i @17..18)\n\
             \x20         (int 2 @21..22)))\n\
             \x20     (rebind i @24..34\n\
             \x20       (+ @29..34\n\
             \x20         (name i @29..30)\n\
             \x20         (int 1 @33..34)))\n\
             \x20     (block @35..51\n\
             \x20       (continue'w @37..49\n\
             \x20         (name i @48..49)))))\n\
             \x20 (expr @52..70\n\
             \x20   (do @52..70\n\
             \x20     (block @55..64\n\
             \x20       (break @57..62))\n\
             \x20     (cond\n\
             \x20       (name c @69..70))))\n\
             \x20 (expr @71..77\n\
             \x20   (for @71..77\n\
             \x20     (block @75..77))))"
    );
}

#[test]
fn return_with_and_without_a_value_and_a_label() {
    assert_eq!(jump("return"), ("return", None, None, Span::new(0, 6)));
    assert_eq!(
        jump("return x + 1"),
        ("return", None, Some(Span::new(7, 12)), Span::new(0, 12))
    );
    assert_eq!(
        jump("return'pick"),
        ("return", Some("pick".to_owned()), None, Span::new(0, 11))
    );
    assert_eq!(
        jump("return'pick 2"),
        (
            "return",
            Some("pick".to_owned()),
            Some(Span::new(12, 13)),
            Span::new(0, 13)
        )
    );
    // A newline, a `;` or a `}` after the keyword means no value.
    assert_eq!(shape("return\n1"), "return; 1");
    assert_eq!(shape("return'a\nx"), "return'a; x");
    assert_eq!(shape("return; 1"), "return; 1");
    assert_eq!(shape("{ return }"), "{return}");
    // An item of any plain block, an operand or not.
    assert_eq!(
        shape("let x = if c { return 1 } else { 2 }"),
        "let x = (if c {return 1} else {2})"
    );
    assert_eq!(shape("f(g({ return 1 }))"), "f(g({return 1}))");
}

#[test]
fn every_return_and_labelled_block_form_prints_back_to_itself() {
    for text in [
        "return",
        "return 1",
        "return'a",
        "return'a x",
        "{'pick return'pick 1; 3}",
        "{'a {'b return'a 1}}",
        "for x in xs {'it return'it 0; x}",
        "for'scan x in xs {return'scan x}",
        "for'spin {return'spin r}",
        "do'd {return'd 1} for c",
        "f(){'t 1}",
    ] {
        assert_eq!(shape(text), text);
    }
    assert_eq!(
        shape("if c {'t 1} else {'e return'e 2}"),
        "(if c {'t 1} else {'e return'e 2})"
    );
    assert_eq!(shape("lazy {'l return 1}"), "(lazy {'l return 1})");
    // A function's body may be labelled, as any block.
    assert_eq!(
        shape("fn f(): Int {'body return'body 1}"),
        "fn f(): Int {'body return'body 1}"
    );
    // The summary's examples.
    assert_eq!(
        shape("let y = {'pick if a { return'pick 1 }; if b { return'pick 2 }; 3 }"),
        "let y = {'pick (if a {return'pick 1}); (if b {return'pick 2}); 3}"
    );
    assert_eq!(
        shape("fn sign(n: Int): Int { if n < 0 { return -1 }; 0 }"),
        "fn sign(n: Int): Int {(if (n < 0) {return (-1)}); 0}"
    );
    assert_eq!(
        shape("let v: Int = for'spin { let r = poll(); if r > 0 { return'spin r } }"),
        "let v: Int = for'spin {let r = poll(); (if (r > 0) {return'spin r})}"
    );
}

#[test]
fn return_is_never_an_expression() {
    for (text, span) in [
        ("x or return 1", Span::new(5, 11)),
        ("1 + return", Span::new(4, 10)),
        ("f(return)", Span::new(2, 8)),
        ("[return 1]", Span::new(1, 7)),
        ("let x = return", Span::new(8, 14)),
        ("= return", Span::new(2, 8)),
    ] {
        assert_eq!(
            error(text),
            ("expected expression, found `return`".to_owned(), span),
            "{text}"
        );
    }
}

#[test]
fn a_block_label_is_written_directly_after_its_brace() {
    for (text, keyword, span) in [
        ("{ 'a 1 }", "{", Span::new(2, 4)),
        ("return 'a", "return", Span::new(7, 9)),
        ("fn f(): Int { 'b 1 }", "{", Span::new(14, 16)),
    ] {
        assert_eq!(
            error(text),
            (
                format!("a label is written directly after `{keyword}`, with no space"),
                span
            ),
            "{text}"
        );
    }
    // A label on the next line, or in an interpolation's braces, is
    // misplaced.
    for (text, span) in [
        ("{\n'a 1 }", Span::new(2, 4)),
        ("\"x{'a 1}\"", Span::new(3, 5)),
        ("{'a'b 1}", Span::new(3, 5)),
    ] {
        assert_eq!(error(text), (MISPLACED_LABEL.to_owned(), span), "{text}");
    }
    // A block in an interpolation may be labelled.
    assert_eq!(shape("\"x{ {'a return'a 1} }\""), "\"x<{'a return'a 1}>\"");
    assert_eq!(
        MISPLACED_LABEL,
        "a label names a loop or a block: it follows `for`, `do`, `break`, `continue`, \
             `return` or `{`"
    );
}

#[test]
fn a_record_block_cannot_be_labelled() {
    for (text, span) in [
        ("{'a x = 1 }", Span::new(1, 3)),
        ("{'a ...r }", Span::new(1, 3)),
        ("let r = {'rec a.b = 1; c = 2 }", Span::new(9, 13)),
    ] {
        assert_eq!(
            error(text),
            ("a record block cannot be labelled".to_owned(), span),
            "{text}"
        );
    }
    // Without its label, or with only a `let`, it is fine.
    assert_eq!(shape("{x = 1}"), "{x = 1}");
    assert_eq!(shape("{'a let x = 1; x}"), "{'a let x = 1; x}");
}

#[test]
fn return_is_not_a_name() {
    assert_eq!(
        error("let return = 1"),
        ("expected a name after `let`".to_owned(), Span::new(4, 10))
    );
    assert_eq!(
        error("fn(return: Int): Int { 1 }").0,
        "expected a parameter name, found `return`"
    );
    assert_eq!(error("x.return").0, "expected a field name after `.`");
    assert_eq!(shape("\"return\" = 1"), "\"return\" = 1");
}

#[test]
fn pretty_output_for_returns_and_labelled_blocks() {
    let ast = parse_text("{'pick return'pick 1 }\nreturn").unwrap();
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n\
             \x20 (expr @0..22\n\
             \x20   (block'pick @0..22\n\
             \x20     (return'pick @7..20\n\
             \x20       (int 1 @19..20))))\n\
             \x20 (return @23..29))"
    );
    // A label's height counts nothing; a value's counts as an item's.
    let ast = parse_text("{'a return'a 1 + 2 }").unwrap();
    let ExprKind::Block { items, label } = ast.get(only_value(&ast)).kind else {
        panic!("expected a block");
    };
    assert_eq!(
        label.map(|label| ast.label(label).span),
        Some(Span::new(1, 3))
    );
    assert_eq!(ast.items(items).len(), 1);
}

#[test]
fn a_name_alone_after_a_named_argument_is_shorthand() {
    let kinds = |text: &str| -> Vec<String> {
        let ast = parse_text(text).unwrap();
        let ExprKind::Call { args, .. } = ast.get(only_value(&ast)).kind else {
            panic!("expected a call");
        };
        ast.args(args)
            .iter()
            .map(|arg| match arg.kind {
                ArgKind::Positional => "positional".to_owned(),
                ArgKind::Named {
                    name, shorthand, ..
                } => {
                    let name = ast.names().get(name);
                    if shorthand {
                        format!("shorthand {name}")
                    } else {
                        format!("named {name}")
                    }
                }
                ArgKind::Trailing => "trailing".to_owned(),
            })
            .collect()
    };
    // After a named argument, at the end of the list or before a `,`.
    assert_eq!(
        kinds("f(x, out = 1, compiler)"),
        ["positional", "named out", "shorthand compiler"]
    );
    assert_eq!(
        kinds("f(out = 1, a, b,)"),
        ["named out", "shorthand a", "shorthand b"]
    );
    // Before any named argument, a name alone is positional.
    assert_eq!(kinds("f(a, b = 1)"), ["positional", "named b"]);
    // After one, anything else is positional still.
    assert_eq!(
        kinds("f(a = 1, b.c, 2, g(b))"),
        ["named a", "positional", "positional", "positional"]
    );
    // Its value is the read of its name, at its name.
    let ast = parse_text("f(a = 1, compiler)").unwrap();
    let ExprKind::Call { args, .. } = ast.get(only_value(&ast)).kind else {
        panic!("expected a call");
    };
    let arg = ast.args(args)[1];
    assert_eq!(ast.get(arg.value).span, Span::new(9, 17));
    assert!(matches!(ast.get(arg.value).kind, ExprKind::Name(_)));
    assert_eq!(arg.span(&ast), Span::new(9, 17));
    assert_eq!(shape("f(a = 1, compiler)"), "f(a = 1, compiler)");
}

#[test]
fn every_import_form_prints_as_written() {
    for text in [
        "import lib",
        "import tools/build",
        "import tools/build as builder",
        "import com from lib",
        "import com as libs from lib",
        "import a, b as c, * from lib/sub",
        "import * from lib",
        "import *, a from lib",
        "import lib; x = 1; import lib/y as y",
        "import pkgs/net.busybox.busybox as busybox",
        "import pkgs/dev.peios.live-root as live",
        "import org/gnu/gmp-c++/v6.3/x_1 as gmp",
        "import kernel from pkgs/dev.peios.kernel",
        "import 2fa as two",
    ] {
        assert_eq!(shape(text), text, "{text}");
    }
    // `from` is a name everywhere outside an import.
    assert_eq!(shape("from = 1; x = from"), "from = 1; x = from");
}

#[test]
fn an_import_item_spans_its_keyword_through_its_path_or_alias() {
    let ast = parse_text("import a/b as c").unwrap();
    let &[id] = ast.items(ast.root()) else {
        panic!("expected one item");
    };
    assert_eq!(ast.item(id).span, Span::new(0, 15));
    let ast = parse_text("import x, * from a/b").unwrap();
    let &[id] = ast.items(ast.root()) else {
        panic!("expected one item");
    };
    assert_eq!(ast.item(id).span, Span::new(0, 20));
    let ItemKind::Import(import) = ast.item(id).kind else {
        panic!("expected an import");
    };
    assert_eq!(import.index(), 0);
}

#[test]
fn import_is_a_keyword() {
    assert_eq!(
        message("import = 1"),
        "expected a module path after `import`, found `=`"
    );
    assert_eq!(message("x = import"), "expected expression, found `import`");
    assert_eq!(message("let import = 1"), "expected a name after `let`");
}

#[test]
fn import_only_at_the_top_level_of_a_file() {
    let text = "x = { import a }";
    assert_eq!(error(text), (MISPLACED_IMPORT.to_owned(), Span::new(6, 12)));
    assert_eq!(message("fn f(): Int { import a }"), MISPLACED_IMPORT);
    assert_eq!(message("def f(): Int { import a }"), MISPLACED_IMPORT);
    assert_eq!(message("x = lazy { import a }"), MISPLACED_IMPORT);
    assert_eq!(message("for i in [1] { import a }"), MISPLACED_IMPORT);
}

#[test]
fn import_path_errors() {
    assert_eq!(
        error("import \"github.com/x\""),
        (QUOTED_MODULE_PATH.to_owned(), Span::new(7, 21))
    );
    assert_eq!(message("import x from \"lib\""), QUOTED_MODULE_PATH);
    assert_eq!(
        message("import from as f"),
        "expected a path after `from`, found `as`"
    );
    assert_eq!(
        message("import x from"),
        "expected a path after `from`, found end of input"
    );
    assert_eq!(
        message("import x from from"),
        "expected a path after `from`, found `from`"
    );
    assert_eq!(
        message("import a as"),
        "expected a name after `as`, found end of input"
    );
    // An empty segment, at its `/`: one that ends the path or follows
    // another.
    assert_eq!(
        error("import lib/ as l"),
        (EMPTY_MODULE_SEGMENT.to_owned(), Span::new(10, 11))
    );
    assert_eq!(
        error("import lib//x"),
        (EMPTY_MODULE_SEGMENT.to_owned(), Span::new(11, 12))
    );
    // A segment that starts or ends with what only goes inside one, or has
    // `..` in it.
    assert_eq!(
        error("import lib/x. as y"),
        (
            format!("`x.` cannot name a module: {MODULE_SEGMENT}"),
            Span::new(11, 13)
        )
    );
    assert_eq!(
        message("import a..b as c"),
        format!("`a..b` cannot name a module: {MODULE_SEGMENT}")
    );
    assert_eq!(
        message("import a/../b"),
        format!("`..` cannot name a module: {MODULE_SEGMENT}")
    );
}

#[test]
fn a_module_path_is_the_run_of_glued_tokens() {
    // Each segment is named by its text, whatever its tokens: dots, `-`,
    // `+` and digits included.
    let ast = parse_text("import pkgs/net.busybox.busybox/x-1+2 as b").unwrap();
    let &[id] = ast.items(ast.root()) else {
        panic!("expected one item");
    };
    let ItemKind::Import(import) = ast.item(id).kind else {
        panic!("expected an import");
    };
    let segments = ast.path(ast.import_decl(import).path);
    let texts: Vec<&str> = segments
        .iter()
        .map(|segment| ast.names().get(segment.name().unwrap()))
        .collect();
    assert_eq!(texts, ["pkgs", "net.busybox.busybox", "x-1+2"]);
    let spans: Vec<Span> = segments.iter().map(|segment| segment.span).collect();
    assert_eq!(
        spans,
        [Span::new(7, 11), Span::new(12, 31), Span::new(32, 37)]
    );
    // A space ends the path, so what follows is the next token: `as`, or
    // an error.
    assert_eq!(message("import lib /x"), "expected end of item, found `/`");
}

#[test]
fn a_whole_import_needs_a_name_its_path_does_not_give() {
    let only = parse_text("import pkgs/net.busybox.busybox").unwrap_err();
    assert_eq!(
        only.message,
        "`net.busybox.busybox` is not a name, so the import needs one"
    );
    assert_eq!(only.span, Span::new(12, 31));
    // A name of identifiers joined by `.` may be the old form of a path.
    assert!(
        only.notes
            .iter()
            .any(|note| note.contains("write `net/busybox/busybox`"))
    );
    // A last segment that is a keyword names nothing either.
    assert_eq!(
        message("import lib/for"),
        "`for` is not a name, so the import needs one"
    );
    assert_eq!(
        message("import 2fa"),
        "`2fa` is not a name, so the import needs one"
    );
    // With `as`, any segment will do.
    assert!(parse_text("import pkgs/net.busybox.busybox as b").is_ok());
}

#[test]
fn import_member_errors() {
    assert_eq!(
        error("import a/b, c from p"),
        (SINGLE_MEMBER.to_owned(), Span::new(7, 10))
    );
    assert_eq!(
        error("import c, a.b from p"),
        (SINGLE_MEMBER.to_owned(), Span::new(10, 13))
    );
    assert_eq!(message("import a/b from p"), SINGLE_MEMBER);
    assert_eq!(message("import a/b from p"), SINGLE_MEMBER);
    assert_eq!(message("import c, a/b from p"), SINGLE_MEMBER);
    assert_eq!(
        error("import *"),
        (
            "`import *` needs `from` and a path".to_owned(),
            Span::new(7, 8)
        )
    );
    assert_eq!(message("import a, *"), "`import *` needs `from` and a path");
    assert_eq!(
        error("import *, a, * from p"),
        (
            "`*` is imported more than once".to_owned(),
            Span::new(13, 14)
        )
    );
    assert_eq!(
        message("import a, b c"),
        "expected `,` or `from`, found `c`"
    );
}

/// The items of `text`'s body marked `#[shadowable]`, by position, and
/// whether the file is marked `#![no_prelude]`.
fn attributes(text: &str) -> (Vec<usize>, Option<Span>) {
    let ast = parse_text(text).unwrap();
    let marked = ast
        .items(ast.root())
        .iter()
        .enumerate()
        .filter(|&(_, &item)| ast.is_shadowable(item))
        .map(|(position, _)| position)
        .collect();
    (marked, ast.no_prelude())
}

#[test]
fn attributes_mark_items_and_the_file() {
    // On its item's line or a line before it, comments and blank lines
    // between; a field, a dotted field, a `def` and an import of each
    // form.
    let text = "# a comment\n\n#![no_prelude]\n#[shadowable] a = 1\nb = 2\n#[shadowable]\n\n\
                    # between\nc.d = 3\n#[shadowable] def f(): Int { 1 }\n\
                    #[shadowable] import x\n#[shadowable] import * from y\n\
                    #[shadowable] import z as w\n";
    assert_eq!(
        attributes(text),
        (vec![0, 2, 3, 4, 5, 6], Some(Span::new(13, 27)))
    );
    assert_eq!(attributes("a = 1"), (vec![], None));
    // The printer shows them.
    assert_eq!(
        parse_text("#![no_prelude]\n#[shadowable] a = 1\nb = 2")
            .unwrap()
            .pretty()
            .to_string(),
        "(file #![no_prelude]\n  #[shadowable] (field a @29..34\n    (int 1 @33..34))\n  \
             (field b @35..40\n    (int 2 @39..40)))"
    );
}

#[test]
fn misplaced_and_unknown_attributes_are_errors() {
    let placed = MISPLACED_SHADOWABLE.to_owned();
    // An outer one that is none of the parser's is a use, which the
    // checker resolves, and applies only to a function.
    assert_eq!(
        error("#[sorted] a = 1"),
        (MISPLACED_USE.to_owned(), Span::new(0, 9))
    );
    assert_eq!(
        error("#![sorted]"),
        ("unknown attribute `sorted`".to_owned(), Span::new(0, 10))
    );
    assert_eq!(
        error("#![shadowable]\na = 1"),
        (
            "`shadowable` applies to a declaration: write `#[shadowable]`".to_owned(),
            Span::new(0, 14)
        )
    );
    assert_eq!(
        error("#[no_prelude]\na = 1"),
        (
            "`no_prelude` applies to the whole file: write `#![no_prelude]`".to_owned(),
            Span::new(0, 13)
        )
    );
    assert_eq!(
        error("#[shadowable] #[shadowable] a = 1"),
        (
            "`#[shadowable]` is written twice".to_owned(),
            Span::new(14, 27)
        )
    );
    assert_eq!(
        error("#![no_prelude]\n#![no_prelude]"),
        (
            "`#![no_prelude]` is written twice".to_owned(),
            Span::new(15, 29)
        )
    );
    for (text, at) in [
        ("a = 1\n#![no_prelude]", Span::new(6, 20)),
        ("#[shadowable]\n#![no_prelude]\na = 1", Span::new(14, 28)),
        ("a = { #![no_prelude] }", Span::new(6, 20)),
    ] {
        assert_eq!(error(text), (MISPLACED_NO_PRELUDE.to_owned(), at), "{text}");
    }
    // Before a `let`, a `let mut` or a `fn`, with the note.
    for text in [
        "#[shadowable] let a = 1",
        "#[shadowable] let mut a = 1",
        "#[shadowable] fn f(): Int { 1 }",
    ] {
        let diagnostic = parse_text(text).unwrap_err();
        assert_eq!(
            (diagnostic.message.as_str(), diagnostic.span),
            (MISPLACED_SHADOWABLE, Span::new(0, 13)),
            "{text}"
        );
        assert_eq!(
            diagnostic.notes,
            vec![LET_SHADOWS_NOTE.to_owned()],
            "{text}"
        );
    }
    // Before a `:=`, an expression, a spread, a result or a `break`, or
    // in an inner block or a `for` header, without it.
    for (text, at) in [
        ("let a = 1\n#[shadowable] a := 2", Span::new(10, 23)),
        ("#[shadowable] f()", Span::new(0, 13)),
        ("#[shadowable] ...{}", Span::new(0, 13)),
        ("#[shadowable] = 1", Span::new(0, 13)),
        ("x = { #[shadowable] a = 1 }", Span::new(6, 19)),
        ("x = { #[shadowable] let a = 1; = a }", Span::new(6, 19)),
        ("for #[shadowable] let i = 0; i < 1; {}", Span::new(4, 17)),
    ] {
        let diagnostic = parse_text(text).unwrap_err();
        assert_eq!(
            (diagnostic.message.clone(), diagnostic.span),
            (placed.clone(), at),
            "{text}"
        );
        let note = text.contains("let a = 1;");
        assert_eq!(diagnostic.notes.is_empty(), !note, "{text}");
    }
    // With nothing after it.
    for (text, at) in [
        ("a = 1\n#[shadowable]", Span::new(6, 19)),
        ("#[shadowable]\n# a comment\n", Span::new(0, 13)),
        ("x = { a = 1; #[shadowable] }", Span::new(13, 26)),
    ] {
        assert_eq!(
            error(text),
            ("`#[shadowable]` is not followed by anything".to_owned(), at),
            "{text}"
        );
    }
    // Where an expression starts, as any token is.
    assert_eq!(
        message("a = #[shadowable] 1"),
        "expected expression, found `#[shadowable]`"
    );
}

#[test]
fn type_items_parse_and_print_back() {
    assert_eq!(shape("type N = Int"), "type N = Int");
    assert_eq!(
        shape("type P = {\n  version: String,\n  arch: String,\n}"),
        "type P = {version: String, arch: String}"
    );
    assert_eq!(
        shape("type Package = { ...PackageDef, name: String }"),
        "type Package = {...PackageDef, name: String}"
    );
    assert_eq!(
        shape("type Q = {...t.P, ...{a: Int}}"),
        "type Q = {...t.P, ...{a: Int}}"
    );
    assert_eq!(
        shape("type Steps =\n  List<fn() -> String>"),
        "type Steps = List<fn() -> String>"
    );
    // A qualified name in any type position.
    assert_eq!(
        shape("x : a.b.T; fn f(p: t.P): t.Q { p }"),
        "(x : a.b.T); fn f(p: t.P): t.Q {p}"
    );
    let ast = parse_text("type P = {a: Int}").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(0, 17));
    let ItemKind::Type { name_span, .. } = item.kind else {
        panic!("expected a type item");
    };
    assert_eq!(name_span, Span::new(5, 6));
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n  (type P {a: Int} @0..17))"
    );
}

#[test]
fn type_is_a_contextual_keyword() {
    // Only `type` followed on its line by an identifier, at the start
    // of an item, begins a `type` item: everywhere else it is a name,
    // as it was.
    assert_eq!(shape("type = 1"), "type = 1");
    assert_eq!(shape("let type = 1; type"), "let type = 1; type");
    assert_eq!(shape("type.x = 1"), "type.x = 1");
    assert_eq!(shape("a = { type = 1 }.type"), "a = ({type = 1} . type)");
    assert_eq!(
        shape("fn f(type: Int): Int { type }"),
        "fn f(type: Int): Int {type}"
    );
    assert_eq!(shape("type\nFoo"), "type; Foo");
    assert_eq!(shape("type: Int = 1"), "type: Int = 1");
}

#[test]
fn type_item_errors() {
    for (text, at) in [
        ("x = { type T = Int }", Span::new(6, 10)),
        ("fn f(): Int { type T = Int; 1 }", Span::new(14, 18)),
        ("for { type T = Int }", Span::new(6, 10)),
    ] {
        assert_eq!(error(text), (MISPLACED_TYPE.to_owned(), at), "{text}");
    }
    assert_eq!(
        error("type Box<T, T> = {v: T}"),
        (
            "type parameter `T` is declared twice".to_owned(),
            Span::new(12, 13)
        )
    );
    assert_eq!(
        error("type T Int"),
        (
            "expected `=` after the type's name, found `Int`".to_owned(),
            Span::new(7, 10)
        )
    );
    assert_eq!(
        error("#[shadowable] type T = Int"),
        (MISPLACED_SHADOWABLE.to_owned(), Span::new(0, 13))
    );
    assert_eq!(
        error("x : t."),
        (
            "expected a type's name after `.`".to_owned(),
            Span::new(6, 6)
        )
    );
    assert_eq!(
        error("type T = {...}"),
        ("expected a type".to_owned(), Span::new(13, 14))
    );
}

#[test]
fn annotated_fields_parse() {
    assert_eq!(shape("x: Int = 1"), "x: Int = 1");
    assert_eq!(shape("a.b: Int | String = 1"), "a.b: (Int | String) = 1");
    assert_eq!(shape("\"a b\": Float = 2"), "\"a b\": Float = 2");
    assert_eq!(
        shape("x: {\n  a: Int,\n} = { a = 1 }"),
        "x: {a: Int} = {a = 1}"
    );
    assert_eq!(shape("x:\n  Int = 1"), "x: Int = 1");
    assert_eq!(shape("x: Int |\n  String = 1"), "x: (Int | String) = 1");
    assert_eq!(shape("{ x: Int = 1 }"), "{x: Int = 1}");
    // Ascriptions as they were: a block's result, a statement, an item
    // ending before an `=` on the next line.
    assert_eq!(shape("{ x : Int }"), "{(x : Int)}");
    assert_eq!(shape("x : {}"), "(x : {})");
    assert_eq!(shape("x : Int\ny = 2"), "(x : Int); y = 2");
    assert_eq!(shape("x : Int { a = 1 }"), "(x : Int)(){a = 1}");
    // An annotation is followed by `=`; a rebind is not annotated.
    assert_eq!(
        error("x: Int := 1"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(7, 9)
        )
    );
    // The annotation counts towards the item's height, as a `let`'s.
    let ast = parse_text("a.b: Int = 1").unwrap();
    let ItemKind::Field { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated field");
    };
    assert_eq!(ast.type_expr(ty).span, Span::new(5, 8));
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n  (field a.b : Int @0..12\n    (int 1 @11..12)))"
    );
}

#[test]
fn def_types_are_marked_defined_and_referred_to() {
    // `#[def_type]` on a `def` of one name or a path, with or without
    // `#[shadowable]`, in either order.
    let ast = parse_text(
        "#[def_type] def p(n: String, f: fn(): Int): Int { 1 }\n\
             #[shadowable]\n#[def_type]\ndef t.q(n: String, f: fn(): Int): Int { 1 }\n\
             #[def_type] #[shadowable] def r(n: String, f: fn(): Int): Int { 1 }",
    )
    .unwrap();
    let items = ast.items(ast.root());
    let marks: Vec<(bool, bool)> = items
        .iter()
        .map(|&item| (ast.is_def_type(item), ast.is_shadowable(item)))
        .collect();
    assert_eq!(marks, [(true, false), (true, true), (true, true)]);
    // A definition: its def-type a name or a path, its path a path, its
    // parameters by name alone; a reference, a def-type and a path on
    // one line, called or not, binding as a field access does.
    assert_eq!(
        shape("def package dev.peios.testpkg() { version = \"1\" }"),
        "def package dev.peios.testpkg() {version = \"1\"}"
    );
    assert_eq!(
        shape("def t.build x(a, b) { a }"),
        "def t.build x(a, b) {a}"
    );
    assert_eq!(
        shape("result = package dev.peios.testpkg()"),
        "result = (package dev.peios.testpkg)()"
    );
    assert_eq!(shape("let f = t.p a.b"), "let f = (t.p a.b)");
    assert_eq!(shape("x = -p a"), "x = (-(p a))");
    assert_eq!(shape("x = p a(1).y"), "x = ((p a)(1) . y)");
    // Two names on two lines are two items, or, inside parentheses, the
    // error they were.
    assert_eq!(shape("x = p\na"), "x = p; a");
    assert_eq!(
        error("f(p\na)"),
        ("expected `,` or `)`, found `a`".to_owned(), Span::new(4, 5))
    );
    // The dump shows them.
    assert_eq!(
        parse_text(
            "#[def_type] def p(n: String, f: fn(a: Int): Int): Int { 1 }\n\
                        def p x(a) { a }\ny = p x"
        )
        .unwrap()
        .pretty()
        .to_string(),
        "(file\n  #[def_type] (def p @12..59\n    (fn @12..59\n      \
             (params (n String) (f fn(a: Int): Int))\n      (ret pure Int)\n      \
             (block @54..59\n        (expr @56..57\n          (int 1 @56..57)))))\n  \
             (def-type-def p x @60..76\n    (fn @66..76\n      (params a)\n      \
             (block @71..76\n        (expr @73..74\n          (name a @73..74)))))\n  \
             (field y @77..84\n    (def-type-ref p x @81..84)))"
    );
}

#[test]
fn def_type_errors() {
    let misplaced = MISPLACED_DEF_TYPE.to_owned();
    assert_eq!(
        error("#[def_type] a = 1"),
        (misplaced.clone(), Span::new(0, 11))
    );
    assert_eq!(
        error("#[def_type] def p x() { 1 }"),
        (misplaced.clone(), Span::new(0, 11))
    );
    assert_eq!(
        error("#[def_type] fn f(): Int { 1 }"),
        (misplaced.clone(), Span::new(0, 11))
    );
    assert_eq!(
        error("x = { #[def_type] def f(): Int { 1 } }"),
        (misplaced, Span::new(6, 17))
    );
    assert_eq!(
        error("#[def_type]\n#[def_type] def f(): Int { 1 }"),
        (
            "`#[def_type]` is written twice".to_owned(),
            Span::new(12, 23)
        )
    );
    assert_eq!(
        error("#![def_type]\na = 1"),
        (
            "`def_type` applies to a declaration: write `#[def_type]`".to_owned(),
            Span::new(0, 12)
        )
    );
    assert_eq!(
        error("a = 1\n#[def_type]"),
        (
            "`#[def_type]` is not followed by anything".to_owned(),
            Span::new(6, 17)
        )
    );
    // A definition's own errors.
    assert_eq!(
        error("def p x(a: Int) { a }"),
        (
            "the parameter types of a `p` definition come from `p`".to_owned(),
            Span::new(9, 10)
        )
    );
    assert_eq!(
        error("def t.p x(): Int { 1 }"),
        (
            "the result type of a `t.p` definition comes from `t.p`".to_owned(),
            Span::new(11, 12)
        )
    );
    assert_eq!(
        error("def p x.\"y\"() { 1 }"),
        (DEF_PATH.to_owned(), Span::new(8, 11))
    );
    assert_eq!(
        error("def p x { 1 }"),
        (
            "expected `(` after the definition's path, found `{`".to_owned(),
            Span::new(8, 9)
        )
    );
    assert_eq!(
        error("def p x() 1"),
        (
            "expected `{` to start the definition's body, found `1`".to_owned(),
            Span::new(10, 11)
        )
    );
    assert_eq!(
        error("x = { def p x() { 1 } }"),
        (MISPLACED_DEF_TYPE_DEF.to_owned(), Span::new(6, 9))
    );
    // A reference's path is made of identifiers too.
    assert_eq!(
        error("x = p a.\"b\""),
        (DEF_PATH.to_owned(), Span::new(8, 11))
    );
    // But an index ends it, indexing what it reads; a definition's path
    // and a meta's have none.
    assert_eq!(shape("x = p a.b[0].c"), "x = (((p a.b) at 0) . c)");
    assert_eq!(
        error("def p a[0]() { 1 }"),
        (DEF_PATH.to_owned(), Span::new(7, 10))
    );
    assert_eq!(
        error("defmeta p a[0] { b = 1 }"),
        (DEF_PATH.to_owned(), Span::new(11, 14))
    );
    // No source writes a namespace's key.
    assert_eq!(
        error("\"<p#1.2>\" = 1"),
        (
            "a field cannot be named `\"<p#1.2>\"`: names of that form are def-types' \
                 namespaces"
                .to_owned(),
            Span::new(0, 9)
        )
    );
}

/// The meta of the outermost function type of the annotation of the
/// only item of `text`, a `let`: its marker's effect and its type's
/// span.
fn annotation_meta(text: &str) -> Option<(Effect, Span)> {
    let ast = parse_text(text).unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated `let`");
    };
    let TypeExprKind::Fn { meta, .. } = ast.type_expr(ty).kind else {
        panic!("expected a function type");
    };
    meta.map(|meta| (meta.effect, ast.type_expr(meta.ty).span))
}

#[test]
fn a_function_type_may_have_a_meta() {
    // After the return type, a union included, `META` and a marker.
    assert_eq!(
        shape("let f: fn(v: Int): Int META: {a: Int} = g"),
        "let f: fn(v: Int): Int META: {a: Int} = g"
    );
    assert_eq!(
        shape("let f: fn() -> Int | String META-> {a: Int} = g"),
        "let f: fn() -> (Int | String) META-> {a: Int} = g"
    );
    assert_eq!(
        annotation_meta("let f: fn() ->> Int META->> Future<{a: Int}> = g"),
        Some((Effect::Memo, Span::new(28, 44)))
    );
    assert_eq!(
        annotation_meta("let f: fn(): Int META-> {a: Int} = g"),
        Some((Effect::Impure, Span::new(24, 32)))
    );
    // A meta after the return type of a function type returned belongs
    // to the inner one, unless it is in parentheses.
    assert_eq!(
        annotation_meta("let f: fn(): fn(): Int META: {a: Int} = g"),
        None
    );
    assert_eq!(
        annotation_meta("let f: fn(): (fn(): Int) META: {a: Int} = g"),
        Some((Effect::Pure, Span::new(31, 39)))
    );
    // Newlines inside parentheses are skipped, after the marker too.
    assert_eq!(
        annotation_meta("let f: (fn(): Int\nMETA:\n{a: Int}) = g"),
        Some((Effect::Pure, Span::new(24, 32)))
    );
    // The dump prints it as written.
    assert_eq!(
        parse_text("x : fn(): (fn(): Int) META-> A")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n  (expr @0..30\n    (: fn(): (fn(): Int) META-> A @0..30\n      \
             (name x @0..1))))"
    );
    // `META` is a name everywhere else.
    assert_eq!(shape("META = 1\nlet y = META"), "META = 1; let y = META");
    assert_eq!(shape("let f: fn(): META = g"), "let f: fn(): META = g");
    // A meta's marker anywhere but after a function type's return type.
    assert_eq!(
        error("x = fn(): Int META: {a: Int} { 1 }"),
        (MISPLACED_META.to_owned(), Span::new(14, 19))
    );
    assert_eq!(
        error("def f() -> Int META-> {a: Int} { 1 }"),
        (MISPLACED_META.to_owned(), Span::new(15, 21))
    );
    assert_eq!(
        error("def p x() META->> {a: Int} { 1 }"),
        (MISPLACED_META.to_owned(), Span::new(10, 17))
    );
}

#[test]
fn a_defmeta_is_a_function_of_its_body() {
    assert_eq!(
        shape("defmeta p dev.x { a = 1 }"),
        "defmeta p dev.x {a = 1}"
    );
    assert_eq!(
        shape("defmeta t.p x = lazy { a = 1 }"),
        "defmeta t.p x {= (lazy {a = 1})}"
    );
    // The dump: its function, of no parameters, its body the block, or
    // a block whose result is the expression.
    assert_eq!(
        parse_text("defmeta p x = 1\ndefmeta p y { a = 2 }")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n  (defmeta p x @0..15\n    (fn @10..15\n      (params)\n      \
             (block @14..15\n        (result @12..15\n          (int 1 @14..15)))))\n  \
             (defmeta p y @16..37\n    (fn @26..37\n      (params)\n      \
             (block @28..37\n        (field a @30..35\n          (int 2 @34..35))))))"
    );
    // `defmeta` is a name everywhere else: a field, a variable, and the
    // def-type of a reference.
    assert_eq!(
        shape("defmeta = 1\nlet y = defmeta\nz = defmeta p\nw = defmeta p { 1 }"),
        "defmeta = 1; let y = defmeta; z = (defmeta p); w = (defmeta p)(){1}"
    );
    // Its errors.
    assert_eq!(
        error("x = { defmeta p a { b = 1 } }"),
        (MISPLACED_DEFMETA.to_owned(), Span::new(6, 13))
    );
    assert_eq!(
        error("#[shadowable] defmeta p a { b = 1 }"),
        (
            "`#[shadowable]` does not apply to a `defmeta`".to_owned(),
            Span::new(0, 13)
        )
    );
    assert_eq!(
        error("#[def_type] defmeta p a { b = 1 }"),
        (MISPLACED_DEF_TYPE.to_owned(), Span::new(0, 11))
    );
    assert_eq!(
        error("defmeta p a b"),
        (
            "expected `{` or `=` after the meta's path, found `b`".to_owned(),
            Span::new(12, 13)
        )
    );
    assert_eq!(
        error("defmeta p a.\"b\" { }"),
        (DEF_PATH.to_owned(), Span::new(12, 15))
    );
    assert_eq!(
        error("defmeta p a META: {b: Int} { }"),
        (MISPLACED_META.to_owned(), Span::new(12, 17))
    );
}

/// The path of each definition, meta and reference made with a def-type
/// in `text`, in the order they are parsed: each name's text, as its
/// source spells it, and its span.
fn def_type_paths(text: &str) -> Vec<Vec<(String, Span)>> {
    let ast = parse_text(text).unwrap();
    ast.def_types()
        .map(|(_, def)| {
            ast.path(def.path)
                .iter()
                .map(|segment| {
                    let name = segment.name().unwrap();
                    (ast.names().get(name).to_owned(), segment.span)
                })
                .collect()
        })
        .collect()
}

#[test]
fn a_definition_s_path_may_have_plus_in_its_names() {
    // The names of real packages, each a definition, a meta and a
    // reference, called and read past, with one name of the source text
    // of its run of tokens.
    for path in [
        "org.gnu.gmp-c++",
        "org.gnu.gcc-c++",
        "org.gnu.gcc-libstdc++",
        "org.gnu.gcc-libstdc++-devel",
        "dev.peios.build-essentials-c++",
    ] {
        let text = format!(
            "defmeta package {path} {{ versions = [] }}\n\
                 def package {path}(v) {{ v }}\n\
                 x = package {path}(\"1\")\n\
                 y = package {path}.versions"
        );
        assert_eq!(
            shape(&text),
            format!(
                "defmeta package {path} {{versions = list()}}; def package {path}(v) {{v}}; \
                     x = (package {path})(\"1\"); y = (package {path}.versions)"
            )
        );
        let (_, last) = path.rsplit_once('.').unwrap();
        let paths = def_type_paths(&text);
        let lengths: Vec<usize> = paths.iter().map(Vec::len).collect();
        assert_eq!(lengths, [3, 3, 3, 4]);
        for names in &paths {
            let (name, span) = &names[2];
            assert_eq!(name, last);
            assert_eq!(&text[span.start as usize..span.end as usize], last);
        }
        assert_eq!(paths[3][3].0, "versions");
    }
    // Any name with `+` after its first character, `+` ending it or not,
    // and names after it.
    assert_eq!(
        def_type_paths("def p a+b() { 1 }\nx = p c++11\ny = p a+b.c+d.e"),
        [
            vec![("a+b".to_owned(), Span::new(6, 9))],
            vec![("c++11".to_owned(), Span::new(24, 29))],
            vec![
                ("a+b".to_owned(), Span::new(36, 39)),
                ("c+d".to_owned(), Span::new(40, 43)),
                ("e".to_owned(), Span::new(44, 45)),
            ],
        ]
    );
    // The rule is of the characters, whatever the tokens are: a keyword,
    // an integer with `_` and a float with an exponent are glued too.
    assert_eq!(
        def_type_paths("x = p a+in.c++1_0.c++1e5"),
        [vec![
            ("a+in".to_owned(), Span::new(6, 10)),
            ("c++1_0".to_owned(), Span::new(11, 17)),
            ("c++1e5".to_owned(), Span::new(18, 24)),
        ]]
    );
    // A `-` is glued only before what it is directly followed by, as in
    // an identifier: before `(`, it is the negation it was.
    assert_eq!(shape("x = p a+-b"), "x = (p a+-b)");
    assert_eq!(shape("x = p a+-1"), "x = (p a+-1)");
    assert_eq!(
        error("x = p a+-(1)"),
        (
            "expected end of item, found `-(`".to_owned(),
            Span::new(8, 10)
        )
    );
}

#[test]
fn plus_ends_a_definition_s_path_where_it_did_and_is_added_elsewhere() {
    // Anything not directly after the segment ends it: `+` with a space
    // before it, or after it, is the addition it was.
    assert_eq!(shape("x = p a + 1"), "x = ((p a) + 1)");
    assert_eq!(shape("x = p a +1"), "x = ((p a) + 1)");
    assert_eq!(shape("x = p a.b + c"), "x = ((p a.b) + c)");
    // But a reference directly followed by `+` reads it into its path,
    // where it was the addition of the reference and what followed.
    assert_eq!(
        error("x = p a+ 1"),
        (
            "expected end of item, found `1`".to_owned(),
            Span::new(9, 10)
        )
    );
    assert_eq!(shape("x = p a+(1)"), "x = (p a+)(1)");
    // A path's name starts with an identifier, so a `+` after `.` is the
    // error it was, and so is one after `def` and a name.
    assert_eq!(
        error("x = p a.+b"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(8, 9)
        )
    );
    assert_eq!(
        error("def p a.+b() { 1 }"),
        (
            "expected `(` after the definition's path, found `.`".to_owned(),
            Span::new(7, 8)
        )
    );
    assert_eq!(
        error("def a+b() { 1 }"),
        (
            "expected `(` after `fn`, found `+`".to_owned(),
            Span::new(5, 6)
        )
    );
    // Anywhere else, `a+b` is the addition it was: a field's path, a
    // variable, a `def`'s name, a def-type's.
    assert_eq!(shape("c = a+b"), "c = (a + b)");
    assert_eq!(shape("let y = x.a+b"), "let y = ((x . a) + b)");
    assert_eq!(shape("x = p+q a"), "x = (p + (q a))");
    assert_eq!(
        error("x.a+b = 1"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(6, 7)
        )
    );
}

#[test]
fn a_definition_s_path_may_have_names_starting_with_a_digit() {
    // The names of real toolchain lanes, each a definition, a meta and a
    // reference, called and read past, the last name a number's.
    for path in ["org.golang.go-stage0-1.24", "org.rust-lang.rust-1.83"] {
        let text = format!(
            "defmeta package {path} {{ versions = [] }}\n\
                 def package {path}(v) {{ v }}\n\
                 x = package {path}(\"1\")\n\
                 y = package {path}.versions"
        );
        assert_eq!(
            shape(&text),
            format!(
                "defmeta package {path} {{versions = list()}}; def package {path}(v) {{v}}; \
                     x = (package {path})(\"1\"); y = (package {path}.versions)"
            )
        );
        let (_, last) = path.rsplit_once('.').unwrap();
        let paths = def_type_paths(&text);
        let lengths: Vec<usize> = paths.iter().map(Vec::len).collect();
        assert_eq!(lengths, [4, 4, 4, 5]);
        for names in &paths {
            let (name, span) = &names[3];
            assert_eq!(name, last);
            assert_eq!(&text[span.start as usize..span.end as usize], last);
        }
        assert_eq!(paths[3][4].0, "versions");
    }
    // A float is split at its `.`, each part a name with its own span;
    // an integer glues on what touches it; a name is its source text.
    assert_eq!(
        def_type_paths(
            "def p x.1.83.0() { 1 }\nx = p x.24rc1\ny = p x.1.83rc1.versions\nz = p x.1_0"
        ),
        [
            vec![
                ("x".to_owned(), Span::new(6, 7)),
                ("1".to_owned(), Span::new(8, 9)),
                ("83".to_owned(), Span::new(10, 12)),
                ("0".to_owned(), Span::new(13, 14)),
            ],
            vec![
                ("x".to_owned(), Span::new(29, 30)),
                ("24rc1".to_owned(), Span::new(31, 36)),
            ],
            vec![
                ("x".to_owned(), Span::new(43, 44)),
                ("1".to_owned(), Span::new(45, 46)),
                ("83rc1".to_owned(), Span::new(47, 52)),
                ("versions".to_owned(), Span::new(53, 61)),
            ],
            vec![
                ("x".to_owned(), Span::new(68, 69)),
                ("1_0".to_owned(), Span::new(70, 73)),
            ],
        ]
    );
    // A float with an exponent: whole without a `.`, split with one, the
    // exponent's sign kept.
    assert_eq!(
        def_type_paths("x = p a.1e5.2.5e-3.7.5E+3"),
        [vec![
            ("a".to_owned(), Span::new(6, 7)),
            ("1e5".to_owned(), Span::new(8, 11)),
            ("2".to_owned(), Span::new(12, 13)),
            ("5e-3".to_owned(), Span::new(14, 18)),
            ("7".to_owned(), Span::new(19, 20)),
            ("5E+3".to_owned(), Span::new(21, 25)),
        ]]
    );
    // A float written with `_` is split the same way.
    assert_eq!(
        def_type_paths("x = p a.1_0.2_5"),
        [vec![
            ("a".to_owned(), Span::new(6, 7)),
            ("1_0".to_owned(), Span::new(8, 11)),
            ("2_5".to_owned(), Span::new(12, 15)),
        ]]
    );
    // `+` glues as it does after an identifier, and `-` before a name.
    assert_eq!(
        def_type_paths("x = p a.1+2.c.3a-b.007"),
        [vec![
            ("a".to_owned(), Span::new(6, 7)),
            ("1+2".to_owned(), Span::new(8, 11)),
            ("c".to_owned(), Span::new(12, 13)),
            ("3a-b".to_owned(), Span::new(14, 18)),
            ("007".to_owned(), Span::new(19, 22)),
        ]]
    );
    // A float in a run, not after a `.`, is not split: the segment ends
    // before it, as it did after `c++`.
    assert_eq!(
        error("def p x.24+1.5() { 1 }"),
        (
            "expected `(` after the definition's path, found `1.5`".to_owned(),
            Span::new(11, 14)
        )
    );
    assert_eq!(
        error("def p x.c++1.5() { 1 }"),
        (
            "expected `(` after the definition's path, found `1.5`".to_owned(),
            Span::new(11, 14)
        )
    );
}

#[test]
fn a_digit_starts_a_name_nowhere_else() {
    // The first segment of a definition's path is what makes it one, an
    // identifier: a digit there is the error it was.
    assert_eq!(
        error("def p 1.x() { 1 }"),
        (
            "expected `(` after `fn`, found `1`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("x = p 1.x"),
        (
            "expected end of item, found `1`".to_owned(),
            Span::new(6, 7)
        )
    );
    // `x.0` is the error it was in a field's path, an expression, a
    // `def`'s name and a def-type.
    assert_eq!(
        error("c = a.0"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("let y = x.1.5"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(10, 13)
        )
    );
    assert_eq!(
        error("def x.0() { 1 }"),
        (
            "expected the function's name after `.`, found `0`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("x = p.1 a"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(6, 7)
        )
    );
    // After a reference's path, what follows its last name is read as it
    // was, so a range of one.
    assert_eq!(shape("x = p a.1..3"), "x = ((p a.1) .. 3)");
    // A quoted name or an index after a number is the error it is after
    // an identifier.
    assert_eq!(
        error("def p x.1.\"a\"() { 1 }"),
        (DEF_PATH.to_owned(), Span::new(10, 13))
    );
    assert_eq!(
        error("def p x.1[0]() { 1 }"),
        (DEF_PATH.to_owned(), Span::new(9, 12))
    );
}

// ----- Errors and recovery -----

/// Every error of `text`, lexical and syntax, as its message and the
/// source it is at.
fn all_errors(text: &str) -> Vec<(String, String)> {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (_, diagnostics) = parse_source(&source);
    diagnostics
        .iter()
        .map(|d| (d.message.clone(), source.slice(d.span).to_owned()))
        .collect()
}

/// The tree recovered from `text`, as [`shape`] writes it, an error
/// node as `<error>`, and how far it can be trusted.
fn recovered(text: &str) -> (String, Syntax) {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (ast, _) = parse_source(&source);
    let mut out = String::new();
    body(&ast, ast.root(), &mut out);
    (out, ast.syntax())
}

/// `pairs` as [`all_errors`] gives them.
fn errors_of(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|&(message, at)| (message.to_owned(), at.to_owned()))
        .collect()
}

#[test]
fn a_valid_file_parses_as_it_did_with_no_error() {
    let text = "x = f(1, [2, 3])\ny: {a: Int} = {a = 1}\ns = \"a{x}b\"";
    assert!(all_errors(text).is_empty());
    let (shape_of, syntax) = recovered(text);
    assert_eq!(shape_of, shape(text));
    assert_eq!(syntax, Syntax::Clean);
}

#[test]
fn items_recover_at_their_newline_or_semicolon() {
    // An item that fails having gone on past a newline, after an
    // operator or `=`, to a line that is an item of its own, ends at
    // that newline: the line is the next item.
    assert_eq!(
        all_errors("let a = 1 +\nlet b: Int = \"x\"\nlet c = f(1,, 2)\nlet e = 3"),
        errors_of(&[
            ("expected expression, found newline", "\n"),
            ("expected expression, found `,`", ","),
        ])
    );
    assert_eq!(
        recovered("let a = 1 +\nlet b: Int = \"x\"\nlet c = f(1,, 2)\nlet e = 3").0,
        "let a = <error>; let b: Int = \"x\"; let c = <error>; let e = 3"
    );
    assert_eq!(
        all_errors(
            "x = \ny = 2\nz: Int =\n#[shadowable] w = 1\nfn f(): Int { 1 }\nv =\nfn g(): Int { 2 }"
        ),
        errors_of(&[
            ("expected expression, found newline", "\n"),
            ("expected expression, found newline", "\n"),
            ("expected expression, found newline", "\n"),
        ])
    );
    assert_eq!(
        recovered("x = \ny = 2\nz: Int =\n#[shadowable] w = 1\nv =\nfn g(): Int { 2 }").0,
        "x = <error>; y = 2; z: Int = <error>; w = 1; v = <error>; fn g(): Int {2}"
    );
    // In a block, and for an item that fails before its value.
    assert_eq!(
        recovered("f = {\n    a = 1 +\n    b = 2\n}\ntype T =\nc = 3").0,
        "f = {a = <error>; b = 2}; <error>; c = 3"
    );
    assert_eq!(
        all_errors("type T =\nc = 3"),
        errors_of(&[("expected a type", "\n")])
    );
    // A line that may go on the line before is not an item of its own:
    // it is the operand the line before was missing, and the error
    // after it the item's one error.
    assert_eq!(
        all_errors("x = 1 +\n2 3\ny = 4"),
        errors_of(&[("expected end of item, found `3`", "3")])
    );
    assert_eq!(recovered("x = 1 +\n2 3\ny = 4").0, "x = <error>; y = 4");
    // A line inside brackets left open is no item of the body: the
    // lexer emits no newline there, so the rest of the file is the
    // list's.
    assert_eq!(
        all_errors("d = [1, 2\ne = 3\nf = 4"),
        errors_of(&[("expected `,` or `]`, found `e`", "e")])
    );
    assert_eq!(recovered("d = [1, 2\ne = 3").0, "d = <error>");
    // A value may be missing, its separator ending it still.
    assert_eq!(
        all_errors("x = ; y = 1"),
        errors_of(&[("expected expression, found `;`", ";")])
    );
    assert_eq!(recovered("x = ; y = 1").0, "x = <error>; y = 1");
    let expected = errors_of(&[
        ("expected end of item, found `2`", "2"),
        ("expected end of item, found `5`", "5"),
    ]);
    assert_eq!(all_errors("x = 1 2\ny = 3\nz = 4 5"), expected);
    assert_eq!(all_errors("x = 1 2; y = 3; z = 4 5"), expected);
    // The items keep their names, their values are errors.
    let (tree, syntax) = recovered("x = 1 2\ny = 3\nz = 4 5");
    assert_eq!(tree, "x = <error>; y = 3; z = <error>");
    assert_eq!(syntax, Syntax::Recovered);
    // An item that fails before its value is an error item.
    assert_eq!(
        all_errors("x . = 1\ny = 2\n) z"),
        errors_of(&[
            ("expected a field name after `.`", "="),
            ("expected expression, found `)`", ")"),
        ])
    );
    assert_eq!(
        recovered("x . = 1\ny = 2\n) z").0,
        "<error>; y = 2; <error>"
    );
}

#[test]
fn a_block_s_items_recover_at_its_closing_brace() {
    assert_eq!(
        all_errors("f = { a = 1 2 }\ng = { b = 3 4 }"),
        errors_of(&[
            ("expected end of item, found `2`", "2"),
            ("expected end of item, found `4`", "4"),
        ])
    );
    assert_eq!(
        recovered("f = { a = 1 2 }\ng = { b = 3 4 }").0,
        "f = {a = <error>}; g = {b = <error>}"
    );
    // An error inside a block is that block's item's alone.
    assert_eq!(
        all_errors("f = {\n    a = 1\n    b = = 2\n    c = 3\n}\ng = 4 4"),
        errors_of(&[
            ("expected expression, found `=`", "="),
            ("expected end of item, found `4`", "4"),
        ])
    );
    assert_eq!(
        recovered("f = {\n    a = 1\n    b = = 2\n    c = 3\n}").0,
        "f = {a = 1; b = <error>; c = 3}"
    );
}

#[test]
fn lists_of_arguments_parameters_and_elements_recover_at_their_commas() {
    assert_eq!(
        all_errors("x = f(1 2, 3, 4 5)"),
        errors_of(&[
            ("expected `,` or `)`, found `2`", "2"),
            ("expected `,` or `)`, found `5`", "5"),
        ])
    );
    assert_eq!(
        all_errors("x = f(+, 2, *)"),
        errors_of(&[
            ("expected expression, found `+`", "+"),
            ("expected expression, found `*`", "*"),
        ])
    );
    assert_eq!(
        all_errors("fn f(a: Int b: Int, c: Int, d: ): Int { 1 }"),
        errors_of(&[
            ("expected `,` or `)`, found `b`", "b"),
            ("expected a type", ")"),
        ])
    );
    assert_eq!(
        all_errors("x = [1 2, 3, 4 5]"),
        errors_of(&[
            ("expected `,` or `]`, found `2`", "2"),
            ("expected `,` or `]`, found `5`", "5"),
        ])
    );
    assert_eq!(
        all_errors("t: fn(Int Int, String, ): Int = g\nu = fn(+): Int { 1 }"),
        errors_of(&[
            ("expected `,` or `)`, found `Int`", "Int"),
            ("expected a parameter name, found `+`", "+"),
        ])
    );
    // The value of an item whose list has an error is an error.
    assert_eq!(recovered("x = f(1 2, 3)\ny = 2").0, "x = <error>; y = 2");
    // A named function whose parameters fail declares nothing.
    assert_eq!(recovered("fn f(a: ): Int { 1 }\ny = 2").0, "<error>; y = 2");
}

#[test]
fn record_types_recover_at_their_commas() {
    assert_eq!(
        all_errors("x: {a: Int b: Int, c: Int, d: } = {}"),
        errors_of(&[("expected `,` or `}`", "b"), ("expected a type", "}"),])
    );
    assert_eq!(
        all_errors("type T = {\n    a: Int\n    b Int\n    c: String,\n    1: Int\n}"),
        errors_of(&[
            ("expected `,` or `}`", "b"),
            ("expected a field name in record type", "1"),
        ])
    );
}

#[test]
fn an_interpolation_recovers_at_its_closing_brace() {
    assert_eq!(
        all_errors("s = \"a{1 +}b{f(}c{2 3}\""),
        errors_of(&[
            ("expected expression, found `}`", "}"),
            ("expected expression, found `}`", "}"),
            ("expected `}`, found `3`", "3"),
        ])
    );
    assert_eq!(recovered("s = \"a{1 +}b\"\nt = 2").0, "s = <error>; t = 2");
}

#[test]
fn unclosed_brackets_and_strings_at_the_end_of_the_file() {
    // A `{`: the block ends with the file; the tree is unbalanced.
    assert_eq!(
        all_errors("x = 1\nf = {\n    a = 2"),
        errors_of(&[("expected `}`, found end of input", "")])
    );
    assert_eq!(recovered("x = 1\nf = {\n    a = 2").1, Syntax::Unbalanced);
    // Nested ones are reported once, for the innermost.
    assert_eq!(
        all_errors("f = {\n    g = {\n        a = 2"),
        errors_of(&[("expected `}`, found end of input", "")])
    );
    // A `(` and a `[`, inside which there are no newlines: the rest of
    // the file is their item's. An unclosed call keeps what it parsed.
    assert_eq!(
        all_errors("x = 1\ny = f(1, 2\nz = 3"),
        errors_of(&[("expected `,` or `)`, found `z`", "z")])
    );
    assert_eq!(
        recovered("x = 1\ny = f(1, 2\nz = 3").0,
        "x = 1; y = <unclosed f(1, 2)>"
    );
    assert_eq!(
        all_errors("x = [1, 2"),
        errors_of(&[("expected `,` or `]`, found end of input", "")])
    );
    // A string: nothing after its quote was lexed.
    assert_eq!(
        all_errors("x = 1\ny = \"abc\nz = (\n"),
        errors_of(&[("unterminated string literal", "\"")])
    );
    assert_eq!(recovered("x = 1\ny = \"abc").0, "x = 1; y = <error>");
    assert_eq!(
        all_errors("y = \"a{1\nz = 2"),
        errors_of(&[("unterminated interpolation", "{")])
    );
}

#[test]
fn a_field_access_with_no_name_keeps_its_record() {
    // At the end of a line, and of the file: one error, the record kept.
    for (text, tree) in [
        ("x = a.\ny = 2", "x = (a . <missing>); y = 2"),
        ("x = a.b.", "x = ((a . b) . <missing>)"),
        ("x = f(1).\n", "x = (f(1) . <missing>)"),
        ("f = {\n    x = a.\n}", "f = {x = (a . <missing>)}"),
        ("x = g(a.)", "x = g((a . <missing>))"),
        ("x = g(a., 1)", "x = g((a . <missing>), 1)"),
        ("x = [a.]", "x = list((a . <missing>))"),
    ] {
        let errors = all_errors(text);
        assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
        assert_eq!(errors[0].0, "expected a field name after `.`", "{text:?}");
        assert_eq!(
            recovered(text),
            (tree.to_owned(), Syntax::Recovered),
            "{text:?}"
        );
    }
    // Where something else follows, as before: the value is an error.
    for text in ["x = a.)", "x = a. + 1", "x = a.}"] {
        assert!(recovered(text).0.starts_with("x = <error>"), "{text:?}");
    }
    // At the end of the file in a bracket, or before the `}` that
    // closes the bracket: the bracket's error is not reported again.
    for text in ["x = g(a.", "x = (a.", "x = [a.", "f = {\n    x = g(a.\n}"] {
        let errors = all_errors(text);
        assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
    }
}

#[test]
fn an_unclosed_call_keeps_its_callee_and_arguments() {
    for (text, tree, error) in [
        (
            "x = f(",
            "x = <unclosed f()>",
            "expected expression, found end of input",
        ),
        (
            "x = f(1, y = 2",
            "x = <unclosed f(1, y = 2)>",
            "expected `,` or `)`, found end of input",
        ),
        (
            "x = f(a.",
            "x = <unclosed f((a . <missing>))>",
            "expected a field name after `.`",
        ),
        (
            "x = g(f(1, ",
            "x = <unclosed g(<unclosed f(1)>)>",
            "expected expression, found end of input",
        ),
        (
            "fn h(): Int {\n    let y = f(1,\n}",
            "fn h(): Int {let y = <unclosed f(1)>}",
            "expected expression, found `}`",
        ),
    ] {
        let errors = all_errors(text);
        assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
        assert_eq!(errors[0].0, error, "{text:?}");
        assert_eq!(
            recovered(text),
            (tree.to_owned(), Syntax::Recovered),
            "{text:?}"
        );
    }
    // A list that ends at its `)` fails as before.
    assert_eq!(recovered("x = f(1 2)").0, "x = <error>");
}

#[test]
fn a_closing_brace_that_closes_nothing_is_reported_and_unbalances_the_tree() {
    assert_eq!(
        all_errors("x = 1\n}\ny = 2 2"),
        errors_of(&[
            ("unexpected `}`", "}"),
            ("expected end of item, found `2`", "2"),
        ])
    );
    let (tree, syntax) = recovered("x = 1\n}\ny = 2");
    assert_eq!(tree, "x = 1; y = 2");
    assert_eq!(syntax, Syntax::Unbalanced);
}

#[test]
fn lexical_errors_are_reported_once_by_the_lexer() {
    // Each error token is the lexer's, and what it is part of fails
    // silently.
    assert_eq!(
        all_errors("x = 1 @ 2\ny = $\nz = f(1, ?)"),
        errors_of(&[
            ("unexpected character `@`", "@"),
            ("unexpected character `$`", "$"),
            ("unexpected character `?`", "?"),
        ])
    );
    assert_eq!(
        recovered("x = 1 @ 2\ny = $\nz = 3").0,
        "x = <error>; y = <error>; z = 3"
    );
    // A malformed number is one token, however it goes on.
    assert_eq!(
        all_errors("x = 1__0.5e+3x + 1\ny = 2"),
        errors_of(&[("consecutive `_` in number literal", "__")])
    );
}

#[test]
fn errors_are_capped() {
    let text = "x = 1 1\n".repeat(150);
    let errors = all_errors(&text);
    assert_eq!(errors.len(), MAX_ERRORS + 1);
    assert_eq!(errors[MAX_ERRORS - 1].0, "expected end of item, found `1`");
    assert_eq!(errors[MAX_ERRORS].0, "too many errors");
    let text = "@\n".repeat(150);
    assert_eq!(all_errors(&text).len(), MAX_ERRORS + 1);
}

#[test]
fn parse_still_gives_the_first_error() {
    assert_eq!(
        message("x = 1 2\ny = 3 4"),
        "expected end of item, found `2`"
    );
}

#[test]
fn attributes_and_misplaced_items_recover() {
    // A misplaced attribute makes the item after it an error item.
    assert_eq!(
        all_errors("#[bogus]\nx = 1\ny = 2 2"),
        errors_of(&[
            (MISPLACED_USE, "#[bogus]"),
            ("expected end of item, found `2`", "2"),
        ])
    );
    assert_eq!(recovered("#[bogus]\nx = 1\ny = 2").0, "<error>; y = 2");
    // An import in a block, an error item.
    assert_eq!(
        recovered("f = {\n    import x\n    a = 1\n}").0,
        "f = {<error>; a = 1}"
    );
}

#[test]
fn nothing_hangs_on_any_prefix_of_a_program() {
    let text = "#![no_prelude]\nimport a/b as c\ntype T = {a: Int, b: String[] = []}\n\
                    #[shadowable] def f(x: Int, y: T = {a = 1}): Int -> Int {\n\
                    \x20   let mut z = [x, ...[1, 2]]\n    for i in 0..3 if i > 0 { z := z }\n\
                    \x20   do { break } for false\n    \"s{x}t{y.a}\" \n    = x\n}\n\
                    defmeta p q.r = 1\nv = p q.r(1)[0].x : Int as Int is Int";
    for end in (0..=text.len()).filter(|&end| text.is_char_boundary(end)) {
        let source = SourceFile::new("t.gcsl", &text[..end]).unwrap();
        let _ = parse_source(&source);
    }
}

#[test]
fn a_number_glued_to_a_name_that_is_no_unit_is_an_unknown_unit() {
    let error = parse_text("x = 30sec").unwrap_err();
    assert_eq!(error.message, "unknown unit `sec`");
    assert_eq!(error.span, Span::new(6, 9));
    assert_eq!(error.notes, [crate::lexer::UNITS_NOTE]);
    // A module path is no expression: `7zip` is its segment.
    assert!(parse_text("import pkgs/7zip as zip\nx = zip.v").is_ok());
    // A quantity is a literal, negated as a number is.
    assert_eq!(shape("x = -1h30m"), "x = (-1h30m)");
}

#[test]
fn attribute_uses_are_kept_on_the_function_items_they_apply_to() {
    let text = "#[attribute] type hermetic = {on: Bool = true}\n\
                #[hermetic] #[retry(3)]\nfn f(): Int { 1 }\n\
                #[shadowable] #[timeout(30s)] def g(): Int {\n    #[retry(1)] fn h(): Int { 2 }\n    h()\n}\n\
                #[hermetic(false)] def test x() { 1 }";
    let ast = parse_text(text).unwrap();
    let shown = ast.pretty().to_string();
    assert!(shown.contains("#[attribute] (type hermetic"), "{shown}");
    assert!(shown.contains("#[hermetic] #[retry(…)] (fn f"), "{shown}");
    assert!(
        shown.contains("#[shadowable] #[timeout(…)] (def g"),
        "{shown}"
    );
    assert!(shown.contains("#[retry(…)] (fn h"), "{shown}");
    assert!(shown.contains("#[hermetic(…)] (def-type-def"), "{shown}");
    // Each use, by its item, a nested `fn` before the item around it, its
    // name read as a type's.
    let uses = ast.all_attribute_uses();
    let named: Vec<&str> = uses.iter().map(|used| ast.names().get(used.name)).collect();
    assert_eq!(named, ["hermetic", "retry", "retry", "timeout", "hermetic"]);
    for used in uses {
        assert_eq!(ast.type_expr(used.ty).span, used.name_span);
        assert_eq!(
            &text[used.name_span.start as usize..used.name_span.end as usize],
            ast.names().get(used.name)
        );
    }
}

#[test]
fn misplaced_and_malformed_attribute_uses_are_errors() {
    assert_eq!(
        all_errors("#[retry(3)]\ny = 1"),
        errors_of(&[(MISPLACED_USE, "#[retry(3)]")])
    );
    assert_eq!(
        all_errors("#[retry(3)] #[retry(4)]\nfn f(): Int { 1 }"),
        errors_of(&[("`#[retry]` is written twice", "#[retry(4)]")])
    );
    assert_eq!(
        all_errors("#[shadowable(1)]\nx = 1"),
        errors_of(&[("`#[shadowable]` takes no arguments", "#[shadowable")])
    );
    assert_eq!(
        all_errors("#![x(1)]\nx = 1"),
        errors_of(&[("an inner attribute takes no arguments", "#![x")])
    );
    assert_eq!(
        all_errors("#[retry(3) x]\nfn f(): Int { 1 }"),
        errors_of(&[(
            "expected `]` after the attribute's arguments, found `x`",
            "x"
        )])
    );
    assert_eq!(
        all_errors("#[attribute] x = 1"),
        errors_of(&[(MISPLACED_ATTRIBUTE, "#[attribute]")])
    );
    assert_eq!(
        all_errors("#[retry(3)]"),
        errors_of(&[("`#[retry]` is not followed by anything", "#[retry(3)]")])
    );
    // A malformed attribute makes the item after it an error item, and the
    // next item is parsed as ever; one whose brackets do not balance leaves
    // them open, as a list's would, so what follows is part of it.
    assert_eq!(
        all_errors("#[retry(3) x]\nfn f(): Int { 1 }\ny = 2 2"),
        errors_of(&[
            (
                "expected `]` after the attribute's arguments, found `x`",
                "x"
            ),
            ("expected end of item, found `2`", "2"),
        ])
    );
    assert_eq!(
        all_errors("#[retry(3]\nfn f(): Int { 1 }\ny = 2 2"),
        errors_of(&[("expected `,` or `)`, found `]`", "]")])
    );
}

#[test]
fn a_named_function_takes_type_parameters() {
    let ast = parse_text("fn id<T>(x: T): T { x }\ndef pair<A, B>(a: A, b: B): A { a }").unwrap();
    let all: Vec<Vec<&str>> = ast
        .all_generics()
        .iter()
        .map(|generics| {
            generics
                .params
                .iter()
                .map(|&(name, _)| ast.names().get(name))
                .collect()
        })
        .collect();
    assert_eq!(all, [vec!["T"], vec!["A", "B"]]);
    // In scope in the signature and the body only.
    let text = "fn id<T>(x: T): T { let y: T = x\n y }\nz: T = 1";
    let ast = parse_text(text).unwrap();
    let scoped: Vec<bool> = ast
        .type_ids()
        .map(|id| match ast.type_expr(id).kind {
            TypeExprKind::Named(name) => ast.type_param(id, name).is_some(),
            _ => false,
        })
        .collect();
    assert_eq!(scoped, [true, true, true, false]);
    for (text, message) in [
        (
            "fn f<T, T>(x: T): T { x }",
            "type parameter `T` is declared twice",
        ),
        (
            "fn f<>(x: Int): Int { x }",
            "expected a type parameter's name, found `>`",
        ),
        (
            "fn f<T,>(x: T): T { x }",
            "expected a type parameter's name, found `>`",
        ),
        (
            "fn f<T U>(x: T): T { x }",
            "expected `,` or `>` after a type parameter, found `U`",
        ),
    ] {
        assert_eq!(message_of(text), message, "{text}");
    }
}

fn message_of(text: &str) -> String {
    parse_text(text).unwrap_err().message
}

#[test]
fn applied_types_take_argument_lists_and_qualified_names() {
    assert_eq!(
        shape("let x: Pair<Int, String> = y"),
        "let x: Pair<Int, String> = y"
    );
    assert_eq!(
        shape("let x: t.Pair<Int,\n String> = y"),
        "let x: t.Pair<Int, String> = y"
    );
    assert_eq!(
        shape("let x: Box<Box<Int>[]> = y"),
        "let x: Box<(Box<Int>)[]> = y"
    );
    let ast = parse_text("type Pair<A, B> = {first: A, second: B}").unwrap();
    let item = ast.items(ast.root())[0];
    assert_eq!(ast.type_arity(item), 2);
    let scoped: Vec<bool> = ast
        .type_ids()
        .filter_map(|id| match ast.type_expr(id).kind {
            TypeExprKind::Named(name) => Some(ast.type_param(id, name).is_some()),
            _ => None,
        })
        .collect();
    assert_eq!(scoped, [true, true]);
    assert_eq!(
        error("let x: Pair<Int String> = y").0,
        "expected `>` to close the type arguments"
    );
}

#[test]
fn a_method_is_a_function_binding_with_its_receiver_first() {
    let ast =
        parse_text("fn (x: Int) twice(): Int { x * 2 }\ndef (xs: List<T>) first<T>(f: T): T { f }")
            .unwrap();
    let methods: Vec<(&str, bool)> = ast
        .methods()
        .iter()
        .map(|method| (ast.names().get(method.name), method.public))
        .collect();
    assert_eq!(methods, [("twice", false), ("first", true)]);
    let method = ast.methods()[1];
    assert_eq!(ast.names().get(method.binding), "<method first@0>");
    let ItemKind::Let {
        value,
        function: true,
        ..
    } = ast.item(method.item).kind
    else {
        panic!("expected a function binding");
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        panic!("expected a function");
    };
    let params: Vec<&str> = ast
        .params(ast.fn_decl(function).params)
        .iter()
        .map(|param| ast.names().get(param.name))
        .collect();
    assert_eq!(params, ["xs", "f"]);
    // The type parameter is in scope in the receiver.
    let scoped = ast
        .type_ids()
        .filter(|&id| matches!(ast.type_expr(id).kind, TypeExprKind::Named(name) if ast.type_param(id, name).is_some()))
        .count();
    assert_eq!(scoped, 3);
    // A function expression is still one.
    assert!(
        parse_text("f = fn (x: Int): Int { x }")
            .unwrap()
            .methods()
            .is_empty()
    );
}

#[test]
fn method_errors() {
    for (text, message) in [
        (
            "x = { fn (y: Int) m(): Int { y } }",
            "a method is declared only at the top level of a file",
        ),
        ("fn () m(): Int { 1 }", "expected the method's receiver"),
        (
            "fn (x: Int, y: Int) m(): Int { y }",
            "a method has one receiver",
        ),
        (
            "fn (x: Int = 1) m(): Int { x }",
            "a receiver takes no default",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}

#[test]
fn enum_items_and_variants_parse() {
    assert_eq!(
        shape(
            "enum Shape extensible<{*: Int}>\nenum Shape::Square<{side: Int}>\nenum t.Shape::Dot"
        ),
        "enum Shape extensible<{*: Int}>; enum Shape::Square<{side: Int}>; enum t.Shape::Dot"
    );
    assert_eq!(shape("x = Shape::Square"), "x = Shape::Square");
    assert_eq!(
        shape("x = t.Shape::Square({ side = 1 })"),
        "x = t.Shape::Square({side = 1})"
    );
    // `enum` is a field's name elsewhere.
    assert_eq!(shape("enum = 1"), "enum = 1");
    for (text, message) in [
        (
            "x = { enum Shape extensible }",
            "`enum` is declared only at the top level of a file",
        ),
        (
            "enum Shape",
            "expected `extensible` after the enum's name, found end of input",
        ),
        (
            "enum Shape::",
            "expected the variant's name after `::`, found end of input",
        ),
        (
            "enum Shape extensible<Int",
            "expected `>` to close the type",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}

#[test]
fn definitions_and_references_made_with_a_variant_parse() {
    assert_eq!(
        shape("def Toolchain::C gcc() { 1 }"),
        "def Toolchain::C gcc() {1}"
    );
    assert_eq!(
        shape("def t.Toolchain::C dev.gcc(x) { x }"),
        "def t.Toolchain::C dev.gcc(x) {x}"
    );
    assert_eq!(shape("r = Toolchain::C gcc()"), "r = (Toolchain::C gcc)()");
    assert_eq!(
        shape("r = t.Toolchain::C gcc.meta"),
        "r = (t.Toolchain::C gcc.meta)"
    );
    assert_eq!(
        shape("defmeta Toolchain::C gcc = 1"),
        "defmeta Toolchain::C gcc {= 1}"
    );
    // A variant followed by no name on its line is a variant as a value.
    assert_eq!(
        shape("r = Toolchain::C\ngcc = 1"),
        "r = Toolchain::C; gcc = 1"
    );
    for (text, message) in [
        (
            "def Toolchain::C() { 1 }",
            "expected the definition's path after `Toolchain::C`, found `(`",
        ),
        (
            "def Toolchain::(x) { 1 }",
            "expected the variant's name after `::`, found `(`",
        ),
        (
            "def Toolchain::C gcc(x: Int) { 1 }",
            "the parameter types of a `Toolchain::C` definition come from `Toolchain::C`",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}

#[test]
fn contexts_reads_withs_and_context_parameters_parse() {
    assert_eq!(
        shape("context arch: String = \"x86_64\"\ncontext toolchains: List<Int>"),
        "context arch: String = {= \"x86_64\"}; context toolchains: List<Int>"
    );
    assert_eq!(shape("r = ^arch"), "r = ^arch");
    assert_eq!(shape("r = ^platform.arch"), "r = (^platform . arch)");
    assert_eq!(
        shape("r = f() with { arch = \"x\"; platform.os = \"y\" }"),
        "r = (f() with {arch = \"x\"; platform.os = \"y\"})"
    );
    assert_eq!(
        shape("r = a + f() with { arch }"),
        "r = (a + (f() with {arch = arch}))"
    );
    assert_eq!(
        shape("r = with { arch = \"x\" } { f() }"),
        "r = ({f()} with {arch = \"x\"})"
    );
    assert_eq!(
        shape("fn f(^arch, ^os: String): String { arch }"),
        "fn f(arch: ^arch = ^arch, os: String = ^os): String {arch}"
    );
    // `context` and `with` are names everywhere else.
    assert_eq!(
        shape("context = 1\nwith = 2\nr = with"),
        "context = 1; with = 2; r = with"
    );
    // A `with` on the next line is a new item.
    assert_eq!(
        error("r = f()\nwith { arch = \"x\" }").0,
        "expected `{` to start the body of `with`, found end of input"
    );
    for (text, message) in [
        (
            "x = { context y: Int = 1 }",
            "`context` is declared only at the top level of a file",
        ),
        ("r = ^1", "expected a context's name after `^`, found `1`"),
        (
            "r = f() with { 1 }",
            "expected a context's name in `with`, found `1`",
        ),
        (
            "r = f() with { platform.arch }",
            "expected `=` after `platform.arch`, found `}`",
        ),
        (
            "r = f() with { a = 1 b = 2 }",
            "expected `;` or a line break after a binding, found `b`",
        ),
        (
            "fn f(^arch = \"x\"): String { arch }",
            "a context parameter's default is its context",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}

#[test]
fn assertions_and_recovers_parse() {
    assert_eq!(shape("assert x == 1"), "assert (x == 1), \"`x == 1`\"");
    assert_eq!(
        shape("assert ok, \"it failed\""),
        "assert ok, \"it failed\""
    );
    assert_eq!(
        shape("r = recover { f() } else { 0 }"),
        "r = recover {f()} else {0}"
    );
    assert_eq!(
        shape("r = recover { f() }\nelse (e) { e.message }"),
        "r = recover {f()} else (e) {(e . message)}"
    );
    // `assert` and `recover` are names everywhere else.
    assert_eq!(
        shape("assert = 1\nrecover = 2\nr = recover"),
        "assert = 1; recover = 2; r = recover"
    );
    for (text, message) in [
        (
            "r = recover { 1 } 2",
            "expected `else` after the body of `recover`, found `2`",
        ),
        (
            "r = recover { 1 } else (1) { 2 }",
            "expected the failure's name after `(`, found `1`",
        ),
        (
            "r = recover { 1 } else (e { 2 }",
            "expected `)` after the failure's name, found `{`",
        ),
        (
            "r = recover { 1 } else 2",
            "expected `{` to start the fallback of `recover`, found `2`",
        ),
    ] {
        assert_eq!(error(text).0, message, "{text}");
    }
}
