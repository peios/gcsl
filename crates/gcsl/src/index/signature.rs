//! Signature help: the signature of the call whose arguments a place is
//! among, and the parameter it is at (see the documentation of
//! [`crate::index`], "Signature help").

use crate::ast::{ArgKind, Ast, ExprId, ExprKind};
use crate::check::Builtin;
use crate::lexer::TokenKind;
use crate::types::{TyId, TyKind};

use super::complete::{Bracket, Tokens};
use super::{Index, LocalKind, Symbol, known};

/// A call's signature, as signature help shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// The callee's signature: `fn name(a: Int, b: String): Int`, `def
    /// name(...)` for a `def`.
    pub label: String,
    /// Where each parameter is written in the label, `a: Int`, as byte
    /// offsets, in order.
    pub params: Vec<(u32, u32)>,
    /// The parameter the place is at, by its position: `None` if it is at
    /// none.
    pub active: Option<u32>,
    /// The callee's doc comment, Markdown, if it has one.
    pub documentation: Option<String>,
}

/// Where in a call a place is.
#[derive(Clone, Copy, Debug)]
enum Within {
    /// In its argument list, whose `(` is this token.
    Arguments(usize),
    /// In its trailing block.
    Trailing,
}

/// A parameter as a signature shows it.
#[derive(Clone, Debug)]
pub(super) struct Shown {
    /// Its name, if it has one.
    pub(super) name: Option<String>,
    /// Its type, as its source writes it.
    pub(super) ty: String,
    /// Its default, as its source writes it, if it has one.
    pub(super) default: Option<String>,
    /// Whether it is the rest parameter, shown `...name: T`.
    pub(super) rest: bool,
}

/// What a signature shows of a callee ([`Index::callee`]).
#[derive(Clone, Debug)]
pub(super) struct Callee {
    /// Its parameters, in order.
    pub(super) params: Vec<Shown>,
    /// What its signature ends with after them: `): Int`.
    pub(super) tail: String,
    /// Its type's parameters, `<T>` for `len` and `show`, built-ins whose
    /// arguments are never named; else empty.
    pub(super) generic: &'static str,
    /// The symbol the callee is, if it is one.
    symbol: Option<Symbol>,
    /// Its name, if it is no path: what the occurrence at its end names.
    name: Option<String>,
}

impl Index {
    /// The signature of the innermost call whose argument list, or trailing
    /// block, byte `offset` of the file of input number `file` is in, with
    /// the parameter it is at (see the module documentation, "Signature
    /// help"): `None` if it is in none, or the callee's type is no function
    /// type the check found.
    #[must_use]
    pub fn signature(&self, file: u32, offset: u32) -> Option<Signature> {
        if !self.checked(file) {
            return None;
        }
        let source = self.sources.get(file as usize)?.as_ref()?;
        let tokens = Tokens::new(source);
        let ast = &self.scan.files.get(file as usize)?.ast;
        let mut before = tokens.starting_before(offset);
        // Out through the brackets open, to the first that is a call's.
        loop {
            let (open, bracket) = tokens.open_before(before)?;
            match bracket {
                Bracket::Paren => {
                    if let Some(callee) = call_of_paren(ast, &tokens, open) {
                        let within = Within::Arguments(open);
                        return self.call_signature(file, (&tokens, ast), callee, (within, offset));
                    }
                    before = open;
                }
                Bracket::Square => before = open,
                Bracket::Brace => {
                    let callee = call_of_block(ast, &tokens, open)?;
                    let within = (Within::Trailing, offset);
                    return self.call_signature(file, (&tokens, ast), callee, within);
                }
                Bracket::Str => return None,
            }
        }
    }

    /// The signature of the call of `callee`, of file `file`, whose tokens
    /// and tree are these, the place being at `offset` `within` it.
    fn call_signature(
        &self,
        file: u32,
        (tokens, ast): (&Tokens, &Ast),
        callee: ExprId,
        (within, offset): (Within, u32),
    ) -> Option<Signature> {
        let Callee {
            params,
            tail,
            generic,
            symbol,
            name,
        } = self.callee(file, ast, callee)?;
        let keyword = match symbol {
            Some(Symbol::Node(node)) if self.def_of(node).is_some() => "def",
            _ => "fn",
        };
        let written = tokens_text(tokens, ast.get(callee).span);
        let name = written.or(name).unwrap_or_default();
        let mut label = format!("{keyword} {name}{generic}(");
        let mut places = Vec::with_capacity(params.len());
        for (at, param) in params.iter().enumerate() {
            if at > 0 {
                label.push_str(", ");
            }
            let start = u32::try_from(label.len()).unwrap_or(u32::MAX);
            if param.rest {
                label.push_str("...");
            }
            if let Some(name) = &param.name {
                label.push_str(name);
                label.push_str(": ");
            }
            label.push_str(&param.ty);
            if let Some(default) = &param.default {
                label.push_str(" = ");
                label.push_str(default);
            }
            places.push((start, u32::try_from(label.len()).unwrap_or(u32::MAX)));
        }
        label.push_str(&tail);
        let labels: Vec<Option<&str>> = params.iter().map(|param| param.name.as_deref()).collect();
        let rest = params.iter().position(|param| param.rest);
        let active = match within {
            Within::Trailing => labels.len().checked_sub(1),
            Within::Arguments(open) => active_parameter(tokens, open, offset, (&labels, rest)),
        };
        Some(Signature {
            label,
            params: places,
            active: active.and_then(|at| u32::try_from(at).ok()),
            documentation: symbol.and_then(|symbol| self.documentation_of(symbol)),
        })
    }

    /// What a signature shows of `callee`, an expression of file `file`,
    /// whose tree is `ast`: its parameters, named as its source names them,
    /// typed as written, each with its default as written (see
    /// [`Shown`]), and the rest of its signature. `None` if its type is no
    /// function type the check found.
    pub(super) fn callee(&self, file: u32, ast: &Ast, callee: ExprId) -> Option<Callee> {
        let span = ast.get(callee).span;
        let found = self
            .at(file, span.end.saturating_sub(1))
            .filter(|found| found.span.end == span.end)
            .copied();
        let symbol = found.and_then(|found| found.symbol);
        // The function's own type, as declared, not the call's, whose
        // parameters a call of it may pass futures to.
        let ty = symbol
            .and_then(|symbol| self.declared_type(symbol))
            .or_else(|| self.expr_type(file, callee))
            .or_else(|| found.and_then(|found| known(found.ty)))?;
        let names = &self.scan.names;
        let generic = |name: &str, ty: &str, ret: &str| Callee {
            params: vec![Shown {
                name: Some(name.to_owned()),
                ty: ty.to_owned(),
                default: None,
                rest: false,
            }],
            tail: format!("): {ret}"),
            generic: "<T>",
            symbol,
            name: None,
        };
        let mut shown = match self.types.kind(ty) {
            TyKind::Builtin(Builtin::Len) => generic("list", "List<T>", "Int"),
            TyKind::Builtin(Builtin::Show) => generic("value", "T", "String"),
            TyKind::Builtin(Builtin::JsonDecode) => generic("text", "String", "T"),
            _ => {
                let sig = self.types.fn_sig(self.types.strip(ty))?;
                let written = self.written_params(symbol);
                let params = sig
                    .params
                    .iter()
                    .enumerate()
                    .map(|(at, &(label, param))| {
                        let (name, default) = match written.get(at) {
                            Some((name, default)) => (Some(*name), default.clone()),
                            None => (label, None),
                        };
                        Shown {
                            name: name.map(|name| names.get(name).to_owned()),
                            ty: self
                                .types
                                .display(self.as_written(param), names)
                                .to_string(),
                            default,
                            rest: sig.rest.map(usize::from) == Some(at),
                        }
                    })
                    .collect();
                Callee {
                    params,
                    tail: self.sig_tail(sig),
                    generic: "",
                    symbol,
                    name: None,
                }
            }
        };
        shown.name = found
            .and_then(|found| found.name)
            .map(|name| names.get(name).to_owned());
        Some(shown)
    }

    /// The parameter type `ty` as its source writes it: without the
    /// either shape, `T | Future<T>`, a parameter the function only passes
    /// on accepts, nor a provisional layer, `T?`.
    pub(super) fn as_written(&self, mut ty: TyId) -> TyId {
        while let TyKind::Either(inner) | TyKind::Provisional { inner, .. } = *self.types.kind(ty) {
            ty = inner;
        }
        ty
    }

    /// The type `symbol` is declared with, if the check found it: a node's
    /// cell's, a `def`'s function's; a local function's function's, any
    /// other local's as it was declared.
    fn declared_type(&self, symbol: Symbol) -> Option<TyId> {
        let function = |file: u32, function: ExprId| {
            let types = &self.files.get(file as usize)?.types;
            types.get(function.index()).copied().and_then(known)
        };
        match symbol {
            Symbol::Node(node) => self
                .cells
                .get(node as usize)
                .copied()
                .and_then(known)
                .or_else(|| self.def_of(node).and_then(|(file, f)| function(file, f))),
            Symbol::Local { file, name } => {
                let local = *self.files.get(file as usize)?.locals.get(&name)?;
                match local.what {
                    LocalKind::Function(f) => function(file, f),
                    _ => known(local.ty),
                }
            }
            _ => None,
        }
    }

    /// The names of the parameters of the function `symbol` is, as its
    /// source writes them, each with its default as written, if it has one:
    /// a local function's, a `def`'s; none for anything else.
    fn written_params(&self, symbol: Option<Symbol>) -> Vec<(crate::names::Name, Option<String>)> {
        let function = match symbol {
            Some(Symbol::Local { file, name }) => {
                let local = self
                    .files
                    .get(file as usize)
                    .and_then(|f| f.locals.get(&name));
                match local.map(|local| local.what) {
                    Some(LocalKind::Function(function)) => Some((file, function)),
                    _ => None,
                }
            }
            Some(Symbol::Node(node)) => self.def_of(node),
            _ => None,
        };
        let Some((file, function)) = function else {
            return Vec::new();
        };
        let Some(ast) = self.scan.files.get(file as usize).map(|f| &f.ast) else {
            return Vec::new();
        };
        let text = self.text(file);
        match ast.get(function).kind {
            ExprKind::Fn(id) => ast
                .params(ast.fn_decl(id).params)
                .iter()
                .map(|param| {
                    let default = param.default.and_then(|default| {
                        let span = ast.get(default).span;
                        let written = text.get(span.start as usize..span.end as usize)?;
                        Some(written.to_owned()).filter(|written| !written.is_empty())
                    });
                    (param.name, default)
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// The callee of the call whose argument list token number `open`, a `(`,
/// opens: the call of `ast` whose callee ends at the token before it.
pub(super) fn call_of_paren(ast: &Ast, tokens: &Tokens, open: usize) -> Option<ExprId> {
    let paren = tokens.span(open);
    let end = tokens.span(open.checked_sub(1)?).end;
    ast.expr_ids().find_map(|id| {
        let (ExprKind::Call { callee, .. } | ExprKind::Unclosed { callee, .. }) = ast.get(id).kind
        else {
            return None;
        };
        let span = ast.get(id).span;
        (ast.get(callee).span.end == end && span.start <= paren.start && paren.end <= span.end)
            .then_some(callee)
    })
}

/// The callee of the call whose trailing block token number `open`, a
/// `{`, opens, if it opens one.
fn call_of_block(ast: &Ast, tokens: &Tokens, open: usize) -> Option<ExprId> {
    let brace = tokens.span(open);
    ast.expr_ids().find_map(|id| {
        let ExprKind::Call { callee, args } = ast.get(id).kind else {
            return None;
        };
        let last = ast.args(args).last()?;
        (last.kind == ArgKind::Trailing && ast.get(last.value).span.start == brace.start)
            .then_some(callee)
    })
}

/// The text of the tokens of `span`, joined, if it is a path short enough
/// to show, `lib.f`: `None` for anything else.
fn tokens_text(tokens: &Tokens, span: crate::source::Span) -> Option<String> {
    let first = tokens.starting_before(span.start);
    let mut text = String::new();
    let mut at = first;
    while at < tokens.len() && tokens.span(at).end <= span.end {
        match tokens.kind(at) {
            TokenKind::Ident(_) | TokenKind::Dot => text.push_str(tokens.word(at)),
            _ => return None,
        }
        at += 1;
    }
    (!text.is_empty() && text.len() <= 60).then_some(text)
}

/// The parameter the place at `offset` is at in the argument list token
/// number `open`, a `(`, opens, of a function whose parameters are named
/// `named`: the parameter a named argument names, when the place is in
/// one, past its `=`; else the next parameter that no argument before it
/// filled, from the left, none filled by a named argument anywhere in the
/// list, nor the last by a trailing block.
fn active_parameter(
    tokens: &Tokens,
    open: usize,
    offset: u32,
    (named, rest): (&[Option<&str>], Option<usize>),
) -> Option<usize> {
    let arguments = Arguments::of(tokens, open, offset);
    // In a named argument, past its `=`: the parameter it names.
    if let Some(&argument) = arguments.spans.get(arguments.current)
        && let Some((name, after)) = name_of(tokens, argument)
        && after <= offset
    {
        return named.iter().position(|&n| n == Some(name));
    }
    arguments
        .filled(tokens, named, rest)
        .iter()
        .position(|&filled| !filled)
}

/// The arguments of an argument list, as its tokens have them.
pub(super) struct Arguments {
    /// The first token of each argument and the token after its last, at
    /// depth 0 in the list.
    spans: Vec<(usize, usize)>,
    /// The argument the place is in, by its position.
    current: usize,
    /// The `)` that closes the list, if one does.
    close: Option<usize>,
}

impl Arguments {
    /// The arguments of the list token number `open`, a `(`, opens, the
    /// place being at `offset`.
    pub(super) fn of(tokens: &Tokens, open: usize, offset: u32) -> Self {
        let mut spans: Vec<(usize, usize)> = Vec::new();
        let mut current = 0;
        let mut depth = 0usize;
        let mut start = open + 1;
        let mut at = open + 1;
        let mut close = None;
        loop {
            let kind = tokens.kind(at);
            match kind {
                TokenKind::Eof => {
                    spans.push((start, at));
                    break;
                }
                TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::StrStart(_) => depth += 1,
                TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::StrEnd(_) => {
                    if depth == 0 {
                        spans.push((start, at));
                        close = (kind == TokenKind::RParen).then_some(at);
                        break;
                    }
                    depth -= 1;
                }
                TokenKind::Comma if depth == 0 => {
                    spans.push((start, at));
                    if tokens.span(at).end <= offset {
                        current = spans.len();
                    }
                    start = at + 1;
                }
                _ => {}
            }
            at += 1;
        }
        Self {
            spans,
            current,
            close,
        }
    }

    /// Which of the parameters, named `named`, the arguments but the one the
    /// place is in fill: each a named argument names, anywhere in the list;
    /// the last, if a trailing block follows the list; and, from the left,
    /// those not filled so, one for each positional argument before the
    /// place, up to the rest parameter, number `rest`, if there is one,
    /// which every positional argument from there fills, and which is never
    /// filled up.
    pub(super) fn filled(
        &self,
        tokens: &Tokens,
        named: &[Option<&str>],
        rest: Option<usize>,
    ) -> Vec<bool> {
        let position = |name: &str| named.iter().position(|&n| n == Some(name));
        let mut filled = vec![false; named.len()];
        for (at, &argument) in self.spans.iter().enumerate() {
            if at != self.current
                && let Some(slot) = name_of(tokens, argument).and_then(|(name, _)| position(name))
                && let Some(slot) = filled.get_mut(slot)
            {
                *slot = true;
            }
        }
        let trailing = self
            .close
            .is_some_and(|close| tokens.kind(close + 1) == TokenKind::LBrace);
        if trailing && let Some(last) = filled.last_mut() {
            *last = true;
        }
        let positional = self
            .spans
            .iter()
            .take(self.current)
            .filter(|&&argument| name_of(tokens, argument).is_none())
            .count();
        let before_rest = rest.unwrap_or(filled.len());
        for slot in filled
            .iter_mut()
            .take(before_rest)
            .filter(|done| !**done)
            .take(positional)
        {
            *slot = true;
        }
        filled
    }
}

/// The name of the argument whose first token and the token after its last
/// are these, if it is a named one, `name = ...`, with where its `=` ends.
fn name_of<'s>(tokens: &Tokens<'s>, (first, end): (usize, usize)) -> Option<(&'s str, u32)> {
    let named = first + 1 < end
        && matches!(tokens.kind(first), TokenKind::Ident(_))
        && tokens.kind(first + 1) == TokenKind::Eq;
    named.then(|| (tokens.word(first), tokens.span(first + 1).end))
}

#[cfg(test)]
mod tests {
    use crate::source::SourceFile;
    use crate::workspace::index_with_standard;

    /// The signature help at the `|` of `text`, the file `app.gcsl` of a
    /// workspace with `others`, as its label, the active parameter's text,
    /// and its documentation.
    fn help(
        text: &str,
        others: &[(&str, &str)],
    ) -> Option<(String, Option<String>, Option<String>)> {
        let at = text.find('|').unwrap();
        let mut sources = vec![
            SourceFile::new("app.gcsl", text.replacen('|', "", 1)).unwrap(),
            SourceFile::new("workspace.gcsl", "").unwrap(),
        ];
        for &(path, other) in others {
            sources.push(SourceFile::new(path, other).unwrap());
        }
        sources.sort_by(|a, b| a.name().cmp(b.name()));
        let index = index_with_standard(&sources, &[], "/").index.unwrap();
        let file = index.file("app.gcsl").unwrap();
        let found = index.signature(file, u32::try_from(at).unwrap())?;
        let active = found.active.map(|active| {
            let (start, end) = found.params[active as usize];
            found.label[start as usize..end as usize].to_owned()
        });
        Some((found.label, active, found.documentation))
    }

    #[test]
    fn every_positional_argument_from_a_rest_parameter_fills_it() {
        let text = "w = 0\ndef f(a: Int, ...more: String, end: Bool = true): Int { a }\n";
        let label = "def f(a: Int, ...more: String, end: Bool = true): Int";
        for (call, active) in [
            ("r = f(|1, \"s\")", Some("a: Int")),
            ("r = f(1, |\"s\")", Some("...more: String")),
            ("r = f(1, \"s\", \"t\", |\"u\")", Some("...more: String")),
            ("r = f(1, \"s\", end = |true)", Some("end: Bool = true")),
        ] {
            let (found, at, _) =
                help(&format!("{text}{call}\n"), &[]).unwrap_or_else(|| panic!("{call}"));
            assert_eq!(found, label, "{call}");
            assert_eq!(at.as_deref(), active, "{call}");
        }
    }

    /// A `def` of three parameters, documented, below the file's first line,
    /// whose comment would be the file's header.
    const F: &str = "w = 0\n# Makes an f.\ndef f(a: Int, b: String, c: Bool): Int { a }\n";

    #[test]
    fn positional_and_named_arguments_fill_their_parameters() {
        let label = "def f(a: Int, b: String, c: Bool): Int";
        for (call, active) in [
            ("r = f(|1, \"s\", true)", Some("a: Int")),
            ("r = f(1, |\"s\", true)", Some("b: String")),
            ("r = f(1, \"s\", tr|ue)", Some("c: Bool")),
            // A named argument fills its own; positional ones from the left.
            ("r = f(c = true, |1, \"s\")", Some("a: Int")),
            ("r = f(a = 1, |\"s\", true)", Some("b: String")),
            ("r = f(b = \"s\", 1, |true)", Some("c: Bool")),
            ("r = f(1, c = |true, b = \"s\")", Some("c: Bool")),
            ("r = f(1, |b = \"s\", c = true)", Some("b: String")),
            ("r = f(1, c = true, |)", Some("b: String")),
            // Past the parameters: none.
            ("r = f(1, \"s\", true, |2)", None),
        ] {
            let text = format!("{F}{call}\n");
            let (found, at, doc) = help(&text, &[]).unwrap_or_else(|| panic!("{call}"));
            assert_eq!(found, label, "{call}");
            assert_eq!(at.as_deref(), active, "{call}");
            assert_eq!(doc.as_deref(), Some("Makes an f."), "{call}");
        }
        // Outside a call's parentheses: none.
        for call in ["r = |f(1, \"s\", true)", "r = f(1, \"s\", true)|"] {
            assert_eq!(help(&format!("{F}{call}\n"), &[]), None, "{call}");
        }
    }

    #[test]
    fn a_trailing_block_fills_the_last_parameter() {
        let t = "def t(n: Int, body: {x: Int}): Int { n + body.x }\n";
        let text = format!("{t}r = t(1) {{\n    x = |2\n}}\n");
        let (label, active, _) = help(&text, &[]).unwrap();
        assert_eq!(label, "def t(n: Int, body: {x: Int}): Int");
        assert_eq!(active.as_deref(), Some("body: {x: Int}"));
        // In its parentheses, the block fills the last: the first is next.
        let text = format!("{t}r = t(|1) {{\n    x = 2\n}}\n");
        assert_eq!(help(&text, &[]).unwrap().1.as_deref(), Some("n: Int"));
    }

    #[test]
    fn the_innermost_call_wins() {
        let text = format!("{F}fn g(n: Int): Int {{ n }}\nr = f(g(|1), \"s\", true)\n");
        let (label, active, _) = help(&text, &[]).unwrap();
        assert_eq!(label, "fn g(n: Int): Int");
        assert_eq!(active.as_deref(), Some("n: Int"));
        // Out of the inner call's parentheses, the outer one's.
        let text = format!("{F}fn g(n: Int): Int {{ n }}\nr = f(g(1), |\"s\", true)\n");
        assert_eq!(help(&text, &[]).unwrap().1.as_deref(), Some("b: String"));
        // In a parenthesised argument, the call it is an argument of.
        let text = format!("{F}r = f((1 + |2), \"s\", true)\n");
        assert_eq!(help(&text, &[]).unwrap().1.as_deref(), Some("a: Int"));
    }

    #[test]
    fn a_standard_and_a_built_in_function_have_their_signatures() {
        let (label, active, _) = help("import path\nr = path.join(\"a\", |\"b\")\n", &[]).unwrap();
        assert_eq!(label, "def path.join(base: String, rel: String): String");
        assert_eq!(active.as_deref(), Some("rel: String"));
        let (label, active, _) = help("r = len(|[1])\n", &[]).unwrap();
        assert_eq!(label, "fn len<T>(list: List<T>): Int");
        assert_eq!(active.as_deref(), Some("list: List<T>"));
        let (label, _, _) = help("import builtins\nr = builtins.int(|1.5)\n", &[]).unwrap();
        assert_eq!(label, "fn builtins.int(x: Float): Int");
    }

    #[test]
    fn defaults_are_shown_as_written() {
        let g = "w = 0\ndef g(a: Int, mid: String = \"é\", last: Bool = 1 > 2): Int { a }\n";
        let (label, active, _) = help(&format!("{g}r = g(1, |)\n"), &[]).unwrap();
        assert_eq!(
            label,
            "def g(a: Int, mid: String = \"é\", last: Bool = 1 > 2): Int"
        );
        assert_eq!(active.as_deref(), Some("mid: String = \"é\""));
        let (_, active, _) = help(&format!("{g}r = g(1, \"s\", |)\n"), &[]).unwrap();
        assert_eq!(active.as_deref(), Some("last: Bool = 1 > 2"));
    }

    #[test]
    fn a_trailing_block_left_open_has_its_signature() {
        let pair = "w = 0\ndef pair(a: String, b: String, body: {x: Int}): String { a }\n";
        for rest in [
            "",
            "\n    x = 1\n",
            "\ny = 2\nz = pair(\"c\", \"d\") {\n    x = 2\n}\n",
        ] {
            let text = format!("{pair}r = pair(\"a\", \"b\") {{|{rest}");
            let at = u32::try_from(text.find('|').unwrap()).unwrap();
            let sources = vec![
                SourceFile::new("app.gcsl", text.replacen('|', "", 1)).unwrap(),
                SourceFile::new("workspace.gcsl", "").unwrap(),
            ];
            let index = index_with_standard(&sources, &[], "/").index.unwrap();
            let file = index.file("app.gcsl").unwrap();
            // The file's braces do not match: it is not indexed.
            assert_eq!(index.signature(file, at), None, "{rest:?}");
            let found = closed(&index, file, at)
                .signature(at)
                .unwrap_or_else(|| panic!("{rest:?}"));
            assert_eq!(
                found.label,
                "def pair(a: String, b: String, body: {x: Int}): String"
            );
            assert_eq!(found.active, Some(2), "{rest:?}");
        }
        // A block that is no trailing block, left open: none.
        let text = format!("{pair}fn h(): Int {{|\n");
        let at = u32::try_from(text.find('|').unwrap()).unwrap();
        let sources = vec![
            SourceFile::new("app.gcsl", text.replacen('|', "", 1)).unwrap(),
            SourceFile::new("workspace.gcsl", "").unwrap(),
        ];
        let index = index_with_standard(&sources, &[], "/").index.unwrap();
        let file = index.file("app.gcsl").unwrap();
        assert_eq!(closed(&index, file, at).signature(at), None);
    }

    /// The workspace of `index` indexed again with the file of input number
    /// `file` balanced for a query at `at`.
    fn closed(index: &crate::index::Index, file: u32, at: u32) -> crate::index::Closed {
        let closing = index.closing(file, at).unwrap();
        closing
            .index(index, |files| index_with_standard(files, &[], "/"))
            .unwrap()
    }

    #[test]
    fn a_call_in_a_broken_file_has_its_signature() {
        for text in [
            format!("{F}r = f(1, |"),
            format!("{F}r = f(1, |\ny = 2\n"),
            format!("{F}r = f(1, |)\nbroken = (1 2)\n"),
            format!("{F}fn h(): Int {{\n    let y = f(1, |\n}}\n"),
            format!("broken = 1 + \"a\"\n{F}r = f(1, |\"s\", true)\n"),
        ] {
            let (label, active, _) = help(&text, &[]).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(label, "def f(a: Int, b: String, c: Bool): Int", "{text}");
            assert_eq!(active.as_deref(), Some("b: String"), "{text}");
        }
    }
}
