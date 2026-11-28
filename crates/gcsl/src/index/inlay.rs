//! Inlay hints: the types a file does not write, shown where they would be
//! written (see the documentation of [`crate::index`], "Inlay hints").

use crate::ast::{Ast, ExprId, ExprKind, FnId, ForHead, ItemKind, TypeExprKind, UnaryOp};
use crate::source::Span;
use crate::types::{TyId, TyKind};

use super::{Index, LocalKind, MAX_NESTING, known, unparen};

/// A type shown where the source does not write it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayHint {
    /// The byte offset it is shown at: the end of a name, or of a
    /// definition's parameter list.
    pub offset: u32,
    /// What is shown: `: Int` after a name, and after a parameter list the
    /// result as GCSL writes it after the `)`, `: Int`, ` ->> Int` or ` ->
    /// Int`, as the function's marker is.
    pub label: String,
}

/// The longest type a hint shows, in characters: a type longer than that
/// has no hint, as one cut short would say too little to be worth its
/// room.
pub const MAX_HINT: usize = 40;

impl Index {
    /// The inlay hints of the file of input number `file` whose offsets are
    /// in `range`, its ends included, in source order (see the module
    /// documentation, "Inlay hints"): none in a file not indexed.
    pub fn inlay_hints(&mut self, file: u32, range: Span) -> Vec<InlayHint> {
        let mut found = self.unwritten(file);
        found.retain(|&(offset, _, _)| range.start <= offset && offset <= range.end);
        found.sort_by_key(|&(offset, _, _)| offset);
        found.dedup_by_key(|&mut (offset, _, _)| offset);
        found
            .into_iter()
            .filter_map(|(offset, marker, ty)| {
                Some(InlayHint {
                    offset,
                    label: format!("{marker}{}", self.hinted(ty)?),
                })
            })
            .collect()
    }

    /// The types the file of input number `file` does not write, each by
    /// the offset its hint is at, what its hint shows before it, and the
    /// type.
    fn unwritten(&self, file: u32) -> Vec<(u32, &'static str, TyId)> {
        let (Some(entry), Some(indexed)) = (
            self.scan.files.get(file as usize),
            self.files.get(file as usize),
        ) else {
            return Vec::new();
        };
        if !indexed.checked {
            return Vec::new();
        }
        let ast = &entry.ast;
        let mut hints = Vec::new();
        // The type of the binding whose name is at `name`.
        let binding = |name: Span| {
            let local = indexed.locals.get(&name)?;
            Some((name.end, ": ", local.ty))
        };
        for id in ast.item_ids() {
            match ast.item(id).kind {
                ItemKind::Let {
                    path,
                    ty: None,
                    value,
                    function: false,
                    ..
                } => {
                    if let [only] = ast.path(path)
                        && !spelled(ast, value)
                        && indexed.locals.get(&only.span).is_some_and(|local| {
                            matches!(local.what, LocalKind::Variable | LocalKind::Mutable)
                        })
                    {
                        hints.extend(binding(only.span));
                    }
                }
                ItemKind::DefTypeDef { def, value } => {
                    let ExprKind::Fn(function) = ast.get(value).kind else {
                        continue;
                    };
                    let decl = ast.fn_decl(function);
                    for param in ast.params(decl.params) {
                        hints.extend(binding(param.span));
                    }
                    let use_ = ast.def_type(def);
                    // Its own, written with no type, whose defaults are the
                    // arguments of its body's function's call.
                    for (param, default) in own_untyped(ast, use_.body, decl.body) {
                        if !spelled(ast, default) {
                            hints.extend(binding(param));
                        }
                    }
                    let (dt, body) = (use_.dt, ast.get(decl.body).span);
                    hints.extend(self.result(file, (dt, value), body));
                }
                _ => {}
            }
        }
        for id in ast.expr_ids() {
            match ast.get(id).kind {
                ExprKind::For(at) => {
                    if let ForHead::Each { name_span, .. } = ast.for_decl(at).head {
                        hints.extend(binding(name_span));
                    }
                }
                // A parameter written with no type, as a `let` is.
                ExprKind::Fn(function) => {
                    for param in ast.params(ast.fn_decl(function).params) {
                        if ast.written_type(param.ty) == TypeExprKind::Inferred
                            && let Some(default) = param.default
                            && !spelled(ast, default)
                        {
                            hints.extend(binding(param.span));
                        }
                    }
                }
                _ => {}
            }
        }
        hints
    }

    /// The result of the function expression `function` of file `file`, a
    /// definition's made with the def-type `dt` reads, whose body is at
    /// `body`, as [`Index::unwritten`] gives it: after the `)` its body
    /// follows, shown after its marker as GCSL writes a result. It is the
    /// one the def-type declares for its definitions, its definition
    /// parameter's (its second); the function's as the check found it, if
    /// that is not known.
    fn result(
        &self,
        file: u32,
        (dt, function): (ExprId, ExprId),
        body: Span,
    ) -> Option<(u32, &'static str, TyId)> {
        let declared = self.expr_type(file, dt).and_then(|ty| {
            let def_type = self.types.fn_sig(self.types.strip(ty))?;
            let &(_, definition) = def_type.params.get(1)?;
            Some(definition)
        });
        let ty = declared
            .filter(|&ty| self.types.fn_sig(self.types.strip(ty)).is_some())
            .or_else(|| self.expr_type(file, function))?;
        let sig = self.types.fn_sig(self.types.strip(ty))?;
        let before = self.text(file).get(..body.start as usize)?;
        let close = before.trim_end_matches([' ', '\t']);
        if !close.ends_with(')') {
            return None;
        }
        let marker = sig.effect.params_end();
        let marker = marker.strip_prefix(')').unwrap_or(marker);
        Some((u32::try_from(close.len()).ok()?, marker, sig.ret))
    }

    /// `ty` as a hint shows it, as a source writes it (see
    /// [`Index::written`]), on one line: `None` for a type the check did
    /// not find, that is erroneous anywhere in it, or that is longer than
    /// [`MAX_HINT`] characters.
    fn hinted(&mut self, ty: TyId) -> Option<String> {
        let ty = known(ty)?;
        if contains_error(self, ty, 0) {
            return None;
        }
        let ty = self.written(ty, 0);
        let shown = self.one_line(ty);
        let line = shown.trim();
        let fits = !line.is_empty() && !line.contains('\n') && line.chars().count() <= MAX_HINT;
        fits.then(|| line.to_owned())
    }

    /// `ty`, `depth` levels into a type, as a source writes it: without the
    /// either shape, `T | Future<T>`, which an impure function's result,
    /// and a parameter a function only passes on, have in place of the `T`
    /// written, nor a provisional layer, `T?`, anywhere but in a function
    /// type, which is left as it is. Interns the types it makes.
    fn written(&mut self, ty: TyId, depth: usize) -> TyId {
        if depth > MAX_NESTING {
            return ty;
        }
        match self.types.kind(ty).clone() {
            TyKind::Either(inner) | TyKind::Provisional { inner, .. } => self.written(inner, depth),
            TyKind::Record(record) => {
                let fields = record
                    .fields
                    .iter()
                    .map(|&(name, field)| (name, self.written(field, depth + 1)))
                    .collect();
                let rest = record.rest().map(|rest| self.written(rest, depth + 1));
                self.types.record_like(ty, fields, rest)
            }
            TyKind::List(element) => {
                let element = self.written(element, depth + 1);
                self.types.list(element)
            }
            TyKind::Future(inner) => {
                let inner = self.written(inner, depth + 1);
                self.types.future(inner)
            }
            TyKind::Union(members) => {
                let members: Vec<TyId> = members
                    .iter()
                    .map(|&member| self.written(member, depth + 1))
                    .collect();
                self.types.union(members)
            }
            _ => ty,
        }
    }
}

/// The parameters of a definition's own written with no type, each by where
/// its name is written, with its default: those of `inner`, its body's
/// function, if it has one, whose defaults are the arguments of the call
/// its body `body` results in (see the parser's
/// [Def-types](crate::parser::deftypes#def-types)).
fn own_untyped(ast: &Ast, inner: Option<FnId>, body: ExprId) -> Vec<(Span, ExprId)> {
    let Some(inner) = inner else {
        return Vec::new();
    };
    let ExprKind::Block { items, .. } = ast.get(body).kind else {
        return Vec::new();
    };
    let [item] = ast.items(items) else {
        return Vec::new();
    };
    let ItemKind::Result(call) = ast.item(*item).kind else {
        return Vec::new();
    };
    let ExprKind::Call { args, .. } = ast.get(call).kind else {
        return Vec::new();
    };
    ast.params(ast.fn_decl(inner).params)
        .iter()
        .zip(ast.args(args))
        .filter(|(param, _)| ast.written_type(param.ty) == TypeExprKind::Inferred)
        .map(|(param, arg)| (param.span, arg.value))
        .collect()
}

/// Whether `ty` is `Error` or has it in it, `depth` levels into it: in a
/// field, an element, a member, a parameter or a result. A type nested too
/// deeply to tell counts as one that has.
fn contains_error(index: &Index, ty: TyId, depth: usize) -> bool {
    if ty == TyId::ERROR || depth > MAX_NESTING {
        return true;
    }
    let inner = |ty: TyId| contains_error(index, ty, depth + 1);
    match index.types.kind(ty) {
        TyKind::Record(record) => record.types().any(inner),
        &(TyKind::Future(of)
        | TyKind::Either(of)
        | TyKind::List(of)
        | TyKind::Provisional { inner: of, .. }) => inner(of),
        TyKind::Union(members) => members.iter().any(|&member| inner(member)),
        TyKind::Fn(sig) => {
            sig.params.iter().any(|&(_, param)| inner(param))
                || inner(sig.ret)
                || sig.meta.is_some_and(|meta| inner(meta.ty))
        }
        _ => false,
    }
}

/// Whether the value `value` of `ast` spells its own type, so that a hint
/// of it would only repeat it: a literal (a negated number among them), a
/// list written out, a record block, a function expression, an ascription
/// or an `as`.
fn spelled(ast: &Ast, value: ExprId) -> bool {
    match ast.get(unparen(ast, value)).kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Duration(_)
        | ExprKind::Size(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Interp(_)
        | ExprKind::List(_)
        | ExprKind::Fn(_)
        | ExprKind::Ascribe { .. }
        | ExprKind::As { .. } => true,
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => matches!(
            ast.get(unparen(ast, operand)).kind,
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Duration(_) | ExprKind::Size(_)
        ),
        ExprKind::Block { items, .. } => {
            let items = ast.items(items);
            items.is_empty()
                || items.iter().any(|&item| {
                    matches!(
                        ast.item(item).kind,
                        ItemKind::Field { .. } | ItemKind::Spread(_)
                    )
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;
    use crate::workspace::index_with_standard;

    /// `text`, the file `app.gcsl` of a workspace with `others`, with its
    /// inlay hints in `range` (the whole file if `None`) written in, each
    /// in `⟨⟩`.
    fn hinted(text: &str, others: &[(&str, &str)], range: Option<Span>) -> String {
        let mut sources = vec![SourceFile::new("app.gcsl", text).unwrap()];
        if !others.iter().any(|&(path, _)| path == "workspace.gcsl") {
            sources.push(SourceFile::new("workspace.gcsl", "").unwrap());
        }
        for &(path, other) in others {
            sources.push(SourceFile::new(path, other).unwrap());
        }
        sources.sort_by(|a, b| a.name().cmp(b.name()));
        let mut index = index_with_standard(&sources, &[], "/").index.unwrap();
        let file = index.file("app.gcsl").unwrap();
        let whole = Span::new(0, u32::try_from(text.len()).unwrap());
        let mut shown = text.to_owned();
        for hint in index.inlay_hints(file, range.unwrap_or(whole)).iter().rev() {
            shown.insert_str(hint.offset as usize, &format!("⟨{}⟩", hint.label));
        }
        shown
    }

    #[test]
    fn a_let_and_a_for_variable_without_a_type_have_one() {
        let text = "fn f(): Int { 1 }\nlet top = f()\n\
                    fn g(xs: List<String>): Int {\n    let a = f()\n    let mut b = a + top\n    \
                    let ys = for x in xs { x }\n    b + len(ys)\n}\nr = g([\"s\"])\n";
        assert_eq!(
            hinted(text, &[], None),
            "fn f(): Int { 1 }\nlet top⟨: Int⟩ = f()\n\
             fn g(xs: List<String>): Int {\n    let a⟨: Int⟩ = f()\n    let mut b⟨: Int⟩ = a + top\n    \
             let ys⟨: List<String>⟩ = for x⟨: String⟩ in xs { x }\n    b + len(ys)\n}\nr = g([\"s\"])\n"
        );
    }

    #[test]
    fn what_spells_its_type_or_is_a_field_has_none() {
        let text = "fn f(): Int { 1 }\nfn g(): Int {\n    let a = 1\n    let s = \"s{a}\"\n    \
                    let b = true\n    let n = -1.5\n    let r = {a = 1}\n    let l = [1, 2]\n    \
                    let h = fn(): Int { 1 }\n    let t: Int = f()\n    let c = f() as Int\n    \
                    let p = (2)\n    a\n}\nx = f()\ny = {z = f()}\n";
        assert_eq!(hinted(text, &[], None), text);
    }

    #[test]
    fn an_erroneous_type_or_one_too_long_has_none() {
        // 40 characters, then 41.
        let fits = "{alpha: Int, beta: Int, deltaxx: String}";
        assert_eq!(fits.chars().count(), MAX_HINT);
        let text = "fn g(): Int {\n    let e = nope\n    let u = [nope]\n    1\n}\n\
                    fn fits(): {alpha: Int, beta: Int, deltaxx: String} {\n    \
                    {alpha = 1; beta = 2; deltaxx = \"d\"}\n}\n\
                    fn long(): {alpha: Int, beta: Int, deltaxxx: String} {\n    \
                    {alpha = 1; beta = 2; deltaxxx = \"d\"}\n}\n\
                    let f = fits()\nlet l = long()\n";
        let shown = hinted(text, &[], None);
        assert!(
            shown.contains("let e = nope\n    let u = [nope]"),
            "{shown}"
        );
        assert!(
            shown.contains(&format!("let f⟨: {fits}⟩ = fits()")),
            "{shown}"
        );
        assert!(shown.contains("let l = long()"), "{shown}");
    }

    #[test]
    fn a_definition_s_parameters_and_result_have_theirs() {
        let workspace = "#[def_type]\ndef package(name: String, func: fn(v: {version: String}): \
                         {arch: String}, version: String) -> String { name }\n";
        for (marker, result) in [(":", "⟨: {arch: String}⟩"), ("->", "⟨ -> {arch: String}⟩")]
        {
            let workspace = workspace.replace(
                "{version: String}): ",
                &format!("{{version: String}}){marker} "),
            );
            let text = "def package dev.x(v) {\n    arch = v.version\n}\n";
            assert_eq!(
                hinted(text, &[("workspace.gcsl", &workspace)], None),
                format!(
                    "def package dev.x(v⟨: {{version: String}}⟩){result} {{\n    arch = v.version\n}}\n"
                ),
                "{marker}"
            );
        }
    }

    #[test]
    fn a_parameter_written_with_no_type_has_its_default_s() {
        // As a `let` has: none where the default spells its type.
        let text = "fn machine(): {name: String} { name = \"vm\" }\n\
                    fn f(a = 2, m = machine(), xs = [1], b = a + 1, c: Int = a) -> Int { a + b }\n\
                    let g = fn(k = f()): Int { 1 }\n\
                    fn (x: Int) add(n = f()): Int { x }\n";
        assert_eq!(
            hinted(text, &[], None),
            "fn machine(): {name: String} { name = \"vm\" }\n\
             fn f(a = 2, m⟨: {name: String}⟩ = machine(), xs = [1], b⟨: Int⟩ = a + 1, c: Int = a) \
             -> Int { a + b }\n\
             let g = fn(k⟨: Int⟩ = f()): Int { 1 }\n\
             fn (x: Int) add(n⟨: Int⟩ = f()): Int { x }\n"
        );
        // A definition's own, beside those its def-type gives.
        let workspace = "#[def_type]\ndef test(name: String, f: fn(v: Int) -> String) -> String \
                         { f(1) }\ndef machine() -> {name: String} { name = \"vm\" }\n";
        let text = "def test t(v, m = machine(), n = 2, s: String = \"x\") {\n    m.name\n}\n";
        assert_eq!(
            hinted(text, &[("workspace.gcsl", workspace)], None),
            "def test t(v⟨: Int⟩, m⟨: {name: String}⟩ = machine(), n = 2, s: String = \"x\")⟨ -> \
             String⟩ {\n    m.name\n}\n"
        );
    }

    #[test]
    fn only_the_range_asked_for_is_answered() {
        let text = "fn f(): Int { 1 }\nlet a = f()\nlet b = f()\nlet c = f()\n";
        let line = |n: usize| -> u32 {
            let length: usize = text.split_inclusive('\n').take(n).map(str::len).sum();
            u32::try_from(length).unwrap()
        };
        assert_eq!(
            hinted(text, &[], Some(Span::new(line(2), line(3)))),
            "fn f(): Int { 1 }\nlet a = f()\nlet b⟨: Int⟩ = f()\nlet c = f()\n"
        );
    }
}
