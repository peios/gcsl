//! Completion: what may be written at a place of a file, and the tokens
//! that tell what the place is (see the documentation of [`crate::index`],
//! "Completion").

use std::cmp::Reverse;
use std::collections::HashSet;

use crate::ast::{Ast, ExprKind, ForHead, ItemKind, PathSegment};
use crate::check::{BUILTIN_TYPE_NAMES, Fileroot};
use crate::lexer::{self, Comments, Token, TokenKind};
use crate::mentions::Resolve;
use crate::names::{self, Name, Names};
use crate::source::{SourceFile, Span};
use crate::strings::FieldName;
use crate::types::{TyId, TyKind};
use crate::workspace::{BUILTINS, FileScope, STANDARD_DIR, Target, target_roots};

use super::signature::{Arguments, call_of_paren};
use super::{Index, LocalKind, Symbol, item_value, known, scope_symbol, standard_stem, unparen};

/// What a completion offers: one thing that may be written at the place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// Its name, as a list shows it.
    pub label: String,
    /// What it is.
    pub kind: CompletionKind,
    /// Its type, on one line, cut short; or, for a module, where it is.
    pub detail: Option<String>,
    /// Its doc comment, Markdown, if it has one.
    pub documentation: Option<String>,
    /// What the list is ordered by: its group's number, then its label.
    pub sort: String,
    /// What is written, when it is not the label: a name that is no
    /// identifier, quoted; a named argument's `name = `.
    pub insert: Option<String>,
}

/// What a [`Completion`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    /// A variable, a parameter, a `for` variable.
    Variable,
    /// A field: of a record, or a node of the workspace.
    Field,
    /// A function, or a field or variable holding one.
    Function,
    /// A directory, a file, a module of the standard library.
    Module,
    /// A type.
    Type,
    /// A keyword.
    Keyword,
    /// A named argument of the call whose argument list the place is in,
    /// `name = `.
    Argument,
}

/// The groups a list is ordered by, nearest first: the named arguments of
/// the call the place is in, locals, then the file's own declarations, its
/// directories', what it imports, the standard library's, and keywords.
const NAMED: u8 = 0;
/// See [`NAMED`].
const LOCAL: u8 = 1;
/// See [`NAMED`].
const FILE: u8 = 2;
/// See [`NAMED`].
const DIRECTORY: u8 = 3;
/// See [`NAMED`].
const IMPORTED: u8 = 4;
/// See [`NAMED`].
const STANDARD: u8 = 5;
/// See [`NAMED`].
const KEYWORD: u8 = 6;

/// The keywords an expression may start with, or a block's item.
const KEYWORDS: [&str; 13] = [
    "if", "for", "do", "fn", "let", "lazy", "await", "not", "true", "false", "return", "break",
    "continue",
];

/// The file that marks a workspace's root, which no `import` names.
const WORKSPACE: &str = "workspace.gcsl";

/// The built-in types that take arguments, after which a `<` opens a type.
const CONSTRUCTORS: [&str; 3] = ["List", "Future", "Present"];

/// What a place of a file is, as its tokens tell (see the documentation of
/// [`crate::index`], "Completion").
#[derive(Clone, Debug, PartialEq, Eq)]
enum Place {
    /// Where nothing is offered: in a string or a comment, where a new
    /// name is declared, after what ends an expression.
    Nothing,
    /// A name of an expression.
    Name,
    /// After a `.`, at `dot`: a field of what is before it. `chain` is the
    /// names before it, `a.b.` if that is what it is.
    Member {
        /// Where the `.` is.
        dot: Span,
        /// The names of the path before it, if it is one.
        chain: Option<Vec<String>>,
    },
    /// A type.
    Type,
    /// A type of the module the names before a `.` name, in a type.
    QualifiedType(Vec<String>),
    /// The path of an `import`, after these segments.
    Import(Vec<String>),
    /// The path of a def-type reference, `DT a.b.`, after these segments.
    Definition {
        /// The def-type's name, and where it is written.
        def_type: (String, Span),
        /// The segments before the place.
        prefix: Vec<String>,
    },
}

/// A bracket open before a place, as [`Tokens::open_before`] finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bracket {
    /// `(`
    Paren,
    /// `[`
    Square,
    /// `{`
    Brace,
    /// An interpolation of a string.
    Str,
}

/// A file's text lexed, to tell what a place of it is.
pub(super) struct Tokens<'s> {
    /// The text.
    text: &'s str,
    /// The tokens, but the end of the input.
    lexed: Vec<Token>,
    /// The comments.
    comments: Comments,
}

impl<'s> Tokens<'s> {
    /// The tokens of `source`.
    pub(super) fn new(source: &'s SourceFile) -> Self {
        let (lexed, comments) = lexer::lex_with_comments(source, &mut Names::new());
        let mut tokens = lexed.tokens;
        if tokens.last().is_some_and(|t| t.kind == TokenKind::Eof) {
            tokens.pop();
        }
        Self {
            text: source.text(),
            lexed: tokens,
            comments,
        }
    }

    /// The kind of token number `at`: the end of the input past the last.
    pub(super) fn kind(&self, at: usize) -> TokenKind {
        self.lexed.get(at).map_or(TokenKind::Eof, |t| t.kind)
    }

    /// The span of token number `at`.
    pub(super) fn span(&self, at: usize) -> Span {
        self.lexed.get(at).map_or(Span::new(0, 0), |t| t.span)
    }

    /// The text of token number `at`.
    pub(super) fn word(&self, at: usize) -> &'s str {
        let span = self.span(at);
        self.text
            .get(span.start as usize..span.end as usize)
            .unwrap_or_default()
    }

    /// The number of tokens, but the end of the input.
    pub(super) fn len(&self) -> usize {
        self.lexed.len()
    }

    /// The number of tokens that start before `offset`.
    pub(super) fn starting_before(&self, offset: u32) -> usize {
        self.lexed.partition_point(|t| t.span.start < offset)
    }

    /// Whether token number `at` is a word: an identifier or a keyword, as
    /// a half-typed name may be.
    fn is_word(&self, at: usize) -> bool {
        let word = self.word(at);
        word.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    /// Whether token number `at` is an identifier.
    fn is_ident(&self, at: usize) -> bool {
        matches!(self.kind(at), TokenKind::Ident(_))
    }

    /// Whether `offset` is in a comment, or in a string's text.
    pub(super) fn in_text(&self, offset: u32) -> bool {
        let spans = self.comments.spans();
        let after = spans.partition_point(|c| c.end < offset);
        if spans
            .get(after)
            .is_some_and(|c| c.start < offset && offset <= c.end)
        {
            return true;
        }
        let at = self.starting_before(offset);
        let Some(last) = at.checked_sub(1) else {
            return false;
        };
        let span = self.span(last);
        match self.kind(last) {
            TokenKind::Str(_)
            | TokenKind::StrStart(_)
            | TokenKind::StrMid(_)
            | TokenKind::StrEnd(_) => span.start < offset && offset < span.end,
            // An unterminated string, or what is no token.
            TokenKind::Error => offset <= span.end,
            _ => false,
        }
    }

    /// The innermost bracket open before token number `before`, and the
    /// number of the token that opens it, the lexer's way: a `}` closes the
    /// `(` and `[` open inside its block too.
    pub(super) fn open_before(&self, before: usize) -> Option<(usize, Bracket)> {
        let mut closed: Vec<Bracket> = Vec::new();
        for at in (0..before.min(self.len())).rev() {
            match self.kind(at) {
                TokenKind::RParen => closed.push(Bracket::Paren),
                TokenKind::RBracket => closed.push(Bracket::Square),
                TokenKind::RBrace => closed.push(Bracket::Brace),
                TokenKind::StrEnd(_) => closed.push(Bracket::Str),
                TokenKind::StrMid(_) => {
                    if closed.last() == Some(&Bracket::Str) {
                        closed.pop();
                    } else if closed.is_empty() {
                        return Some((at, Bracket::Str));
                    }
                    closed.push(Bracket::Str);
                }
                TokenKind::StrStart(_) => {
                    if closed.last() == Some(&Bracket::Str) {
                        closed.pop();
                    } else if closed.is_empty() {
                        return Some((at, Bracket::Str));
                    }
                }
                kind @ (TokenKind::LParen | TokenKind::LBracket) => {
                    let bracket = if kind == TokenKind::LParen {
                        Bracket::Paren
                    } else {
                        Bracket::Square
                    };
                    if closed.last() == Some(&bracket) {
                        closed.pop();
                    } else if closed.is_empty() {
                        return Some((at, bracket));
                    }
                }
                TokenKind::LBrace => {
                    // Its `}`, and what it closes with it, a `)` or `]` of
                    // nothing open.
                    let mut matched = false;
                    while let Some(last) = closed.pop() {
                        if last == Bracket::Brace {
                            matched = true;
                            break;
                        }
                    }
                    if !matched {
                        return Some((at, Bracket::Brace));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The first token of the line token number `at` is on, after any `;`.
    fn line_start(&self, at: usize) -> usize {
        let mut start = at;
        while let Some(before) = start.checked_sub(1) {
            if matches!(self.kind(before), TokenKind::Newline | TokenKind::Semi) {
                break;
            }
            start = before;
        }
        start
    }

    /// Whether token number `word` is a word directly followed by a `-` of
    /// its own that ends at `offset`, `libcap-`: a name half-typed up to a
    /// `-`, which the lexer takes for a `-` that is ambiguous.
    fn dangling_dash(&self, word: usize, offset: u32) -> bool {
        let dash = word + 1;
        self.kind(dash) == TokenKind::Error
            && self.word(dash) == "-"
            && self.span(dash).end == offset
            && self.is_word(word)
            && self.span(word).end == self.span(dash).start
    }

    /// The number of the token the place at `offset` is after, the word
    /// half-typed at it set aside (the client filters by it), `libcap-`
    /// among them: `Ok(None)` for none, at the start of the file; `Err` for
    /// a place in a token that is no word, where nothing is completed.
    pub(super) fn before_word(&self, offset: u32) -> Result<Option<usize>, ()> {
        let starting = self.starting_before(offset);
        if let Some(word) = starting.checked_sub(2)
            && self.dangling_dash(word, offset)
        {
            return Ok(word.checked_sub(1));
        }
        match starting.checked_sub(1) {
            Some(last) if self.span(last).end >= offset && self.is_word(last) => {
                Ok(last.checked_sub(1))
            }
            Some(last) if self.span(last).end > offset => Err(()),
            other => Ok(other),
        }
    }

    /// What the place at `offset` is (see [`Place`]).
    fn place(&self, offset: u32) -> Place {
        let dashed = self
            .starting_before(offset)
            .checked_sub(2)
            .is_some_and(|word| self.dangling_dash(word, offset));
        if !dashed && self.in_text(offset) {
            return Place::Nothing;
        }
        let Ok(previous) = self.before_word(offset) else {
            return Place::Nothing;
        };
        let Some(previous) = previous else {
            return Place::Name;
        };
        if let Some(import) = self.import_place(previous) {
            return import;
        }
        let kind = self.kind(previous);
        if kind == TokenKind::Dot {
            return self.dot_place(previous);
        }
        if self.is_type_place(previous) {
            return Place::Type;
        }
        match kind {
            TokenKind::Let
            | TokenKind::Mut
            | TokenKind::Def
            | TokenKind::Fn
            | TokenKind::For
            | TokenKind::Label(_)
            | TokenKind::Attribute(_)
            | TokenKind::InnerAttribute(_)
            | TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::Str(_)
            | TokenKind::StrEnd(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace
            | TokenKind::Error => Place::Nothing,
            // A `type` item's name is declared.
            TokenKind::Ident(_)
                if self.word(previous) == "type" && self.line_start(previous) == previous =>
            {
                Place::Nothing
            }
            // A name, then on its line another: a def-type reference.
            TokenKind::Ident(_) => Place::Definition {
                def_type: (self.word(previous).to_owned(), self.span(previous)),
                prefix: Vec::new(),
            },
            TokenKind::LParen | TokenKind::Comma if self.declares_parameters(previous) => {
                Place::Nothing
            }
            _ => Place::Name,
        }
    }

    /// The place after token number `previous` if it is in the path of an
    /// `import`: its segments before it; `None` if it is in no `import`,
    /// and [`Place::Nothing`] where the path is not what is written.
    fn import_place(&self, previous: usize) -> Option<Place> {
        let mut first = self.line_start(previous);
        while matches!(
            self.kind(first),
            TokenKind::Attribute(_) | TokenKind::InnerAttribute(_)
        ) && first < previous
        {
            first += 1;
        }
        if self.kind(first) != TokenKind::Import {
            return None;
        }
        let written: Vec<usize> = (first + 1..=previous).collect();
        if written.iter().any(|&at| self.kind(at) == TokenKind::As) {
            return Some(Place::Nothing);
        }
        let path = match written.iter().rposition(|&at| self.word(at) == "from") {
            Some(from) => written.get(from + 1..).unwrap_or_default(),
            None => written.as_slice(),
        };
        let mut prefix = Vec::new();
        for pair in path.chunks(2) {
            match *pair {
                [name, dot] if self.is_ident(name) && self.kind(dot) == TokenKind::Dot => {
                    prefix.push(self.word(name).to_owned());
                }
                _ => return Some(Place::Nothing),
            }
        }
        Some(Place::Import(prefix))
    }

    /// The place after the `.` of token number `dot`.
    fn dot_place(&self, dot: usize) -> Place {
        let mut chain = Vec::new();
        let mut start = dot;
        while let Some(name) = start.checked_sub(1) {
            if !self.is_ident(name) {
                break;
            }
            chain.push(self.word(name).to_owned());
            start = name;
            match name.checked_sub(1) {
                Some(before) if self.kind(before) == TokenKind::Dot => start = before,
                _ => break,
            }
        }
        chain.reverse();
        let before = start.checked_sub(1);
        if self.kind(start) == TokenKind::Dot || chain.is_empty() {
            // What is before it is no path: `f(x).`, `a[0].`.
            return Place::Member {
                dot: self.span(dot),
                chain: None,
            };
        }
        match before {
            Some(before) if self.is_type_place(before) => Place::QualifiedType(chain),
            Some(before) if self.is_ident(before) => Place::Definition {
                def_type: (self.word(before).to_owned(), self.span(before)),
                prefix: chain,
            },
            _ => Place::Member {
                dot: self.span(dot),
                chain: Some(chain),
            },
        }
    }

    /// Whether a type is written after token number `at`: after a `:`, a
    /// return marker, `is`, `as` or `|`, after `List<`, or after the `=` of
    /// a `type` item.
    fn is_type_place(&self, at: usize) -> bool {
        match self.kind(at) {
            TokenKind::Colon
            | TokenKind::Arrow
            | TokenKind::MemoArrow
            | TokenKind::Is
            | TokenKind::As
            | TokenKind::Pipe => true,
            TokenKind::Lt => at
                .checked_sub(1)
                .is_some_and(|name| CONSTRUCTORS.contains(&self.word(name))),
            TokenKind::Eq => {
                let start = self.line_start(at);
                at == start + 2 && self.word(start) == "type" && self.is_ident(start + 1)
            }
            _ => false,
        }
    }

    /// Whether token number `at`, a `(` or a `,`, starts a parameter of a
    /// function's or a definition's parameter list, a name declared there.
    fn declares_parameters(&self, at: usize) -> bool {
        let open = if self.kind(at) == TokenKind::LParen {
            Some(at)
        } else {
            match self.open_before(at) {
                Some((open, Bracket::Paren)) => Some(open),
                _ => None,
            }
        };
        let Some(open) = open else {
            return false;
        };
        let mut before = open;
        while let Some(at) = before.checked_sub(1) {
            match self.kind(at) {
                TokenKind::Ident(_) | TokenKind::Dot => before = at,
                TokenKind::Fn | TokenKind::Def => return true,
                _ => return false,
            }
        }
        false
    }
}

impl Index {
    /// What may be written at byte `offset` of the file of input number
    /// `file` (see the module documentation, "Completion"), in the order
    /// [`Completion::sort`] gives: none in a string or a comment, nor where
    /// a name is declared, nor in a file not indexed.
    #[must_use]
    pub fn completion(&self, file: u32, offset: u32) -> Vec<Completion> {
        if !self.checked(file) {
            return Vec::new();
        }
        let Some(source) = self.sources.get(file as usize).and_then(Option::as_ref) else {
            return Vec::new();
        };
        let tokens = Tokens::new(source);
        let mut found = match tokens.place(offset) {
            Place::Nothing => Vec::new(),
            Place::Name => {
                let mut found = self.named_arguments(file, &tokens, offset);
                found.extend(self.names_at(file, offset));
                found
            }
            Place::Member { dot, chain } => self.members(file, offset, dot, chain.as_deref()),
            Place::Type => self.types_at(file),
            Place::QualifiedType(chain) => self.qualified_types(file, &chain),
            Place::Import(prefix) => self.modules_under(file, &prefix),
            Place::Definition { def_type, prefix } => self.definitions(file, &def_type, &prefix),
        };
        found.sort_by(|a, b| a.sort.cmp(&b.sort));
        found
    }

    /// The named arguments that may be written at `offset` of file `file`,
    /// whose tokens are `tokens`, if an argument starts there, in a call's
    /// argument list: after its `(` or a `,` of it, the word half-typed set
    /// aside. One, `name = `, for each parameter of the callee that has a
    /// name and that no other argument fills (see [`Arguments::filled`]),
    /// typed as its signature types it; none for `len` and `show`.
    fn named_arguments(&self, file: u32, tokens: &Tokens<'_>, offset: u32) -> Vec<Completion> {
        let Some(ast) = self.scan.files.get(file as usize).map(|f| &f.ast) else {
            return Vec::new();
        };
        let Ok(Some(mut previous)) = tokens.before_word(offset) else {
            return Vec::new();
        };
        while tokens.kind(previous) == TokenKind::Newline {
            let Some(before) = previous.checked_sub(1) else {
                return Vec::new();
            };
            previous = before;
        }
        let open = match tokens.kind(previous) {
            TokenKind::LParen => previous,
            TokenKind::Comma => match tokens.open_before(previous) {
                Some((open, Bracket::Paren)) => open,
                _ => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        let Some(callee) = call_of_paren(ast, tokens, open)
            .and_then(|callee| self.callee(file, ast, callee))
            .filter(|callee| callee.generic.is_empty())
        else {
            return Vec::new();
        };
        let named: Vec<Option<&str>> = callee
            .params
            .iter()
            .map(|param| param.name.as_deref())
            .collect();
        let filled = Arguments::of(tokens, open, offset).filled(tokens, &named);
        callee
            .params
            .iter()
            .zip(filled)
            .filter(|(_, filled)| !filled)
            .filter_map(|(param, _)| {
                let name = param.name.as_deref()?;
                Some(Completion {
                    label: format!("{name} ="),
                    kind: CompletionKind::Argument,
                    detail: Some(param.ty.clone()),
                    documentation: None,
                    sort: sort_text(NAMED, name),
                    insert: Some(format!("{name} = ")),
                })
            })
            .collect()
    }

    /// Everything a name may name at `offset` of file `file`, each name
    /// once, the nearest: locals, then the file's own declarations, its
    /// directories', what it imports, the standard library's, then the
    /// keywords.
    fn names_at(&self, file: u32, offset: u32) -> Vec<Completion> {
        let Some(entry) = self.scan.files.get(file as usize) else {
            return Vec::new();
        };
        let ast = &entry.ast;
        let mut seen: HashSet<Name> = HashSet::new();
        let mut found = Vec::new();
        for (name, at) in self.locals_at(ast, offset) {
            if seen.insert(name) {
                found.push(self.local_completion(file, name, at, LOCAL));
            }
        }
        // The file's own `let`s, before the place, and `fn`s, anywhere.
        for &item in ast.items(ast.root()) {
            let it = ast.item(item);
            let ItemKind::Let { path, function, .. } = it.kind else {
                continue;
            };
            if !function && it.span.end >= offset {
                continue;
            }
            let Some((name, at)) = first_segment(ast.path(path)) else {
                continue;
            };
            if seen.insert(name) {
                found.push(self.local_completion(file, name, at, FILE));
            }
        }
        // Its view: its own roots, its directories', what it imports.
        let scope = FileScope::new(&self.scan, entry);
        let tree = &entry.mentions.tree;
        let own = tree
            .roots
            .iter()
            .filter_map(|&root| tree.nodes.get(root as usize).map(|n| (n.name, FILE)));
        let directories = self.scan.chain(entry.dir).flat_map(|dir| {
            target_roots(&self.scan, Target::Dir(dir))
                .into_iter()
                .map(|name| (name, DIRECTORY))
        });
        let imported = entry.aliases.iter().map(|alias| {
            let group = if alias.implicit { STANDARD } else { IMPORTED };
            (alias.name, group)
        });
        let view: Vec<(Name, u8)> = own.chain(directories).chain(imported).collect();
        // The top-level field whose value the place is in is being declared:
        // it is not offered (a `def`, which may call itself, is).
        if let Some(declaring) = declared_at(ast, offset) {
            seen.insert(declaring);
        }
        for (name, group) in view {
            if names::is_namespace_key(self.scan.names.get(name)) || !seen.insert(name) {
                continue;
            }
            let symbol = scope.root(name).and_then(|n| scope_symbol(&scope, n));
            found.push(self.view_completion(name, symbol, group));
        }
        for keyword in KEYWORDS {
            found.push(Completion {
                label: keyword.to_owned(),
                kind: CompletionKind::Keyword,
                detail: None,
                documentation: None,
                sort: sort_text(KEYWORD, keyword),
                insert: None,
            });
        }
        found
    }

    /// The locals declared in `ast` in scope at `offset`, innermost and
    /// latest first, each name once: the names of the `let`s, `fn`s and
    /// fields of each block around it before the item it is in, the
    /// parameters of each function whose body it is in, the variable of
    /// each `for` whose body or guard it is in, and the `let`s of each
    /// condition loop's header before it. The file's own top level is not
    /// among them.
    fn locals_at(&self, ast: &Ast, offset: u32) -> Vec<(Name, Span)> {
        let mut scopes: Vec<(u32, Vec<(Name, Span)>)> = Vec::new();
        for id in ast.expr_ids() {
            let expr = ast.get(id);
            let span = expr.span;
            if !(span.start < offset && offset <= span.end) {
                continue;
            }
            match expr.kind {
                ExprKind::Block { items, .. } if offset < span.end || span.start == span.end => {
                    scopes.push((span.start, items_before(ast, ast.items(items), offset)));
                }
                ExprKind::Fn(function) => {
                    let decl = ast.fn_decl(function);
                    let body = ast.get(decl.body).span;
                    if body.start < offset && offset <= body.end {
                        let params = ast
                            .params(decl.params)
                            .iter()
                            .map(|param| (param.name, self.param_name(param.name, param.span)))
                            .collect();
                        scopes.push((span.start, params));
                    }
                }
                ExprKind::For(id) => {
                    let decl = ast.for_decl(id);
                    let body = ast.get(decl.body).span;
                    let in_body = body.start < offset && offset <= body.end;
                    match decl.head {
                        ForHead::Each {
                            name,
                            name_span,
                            guard,
                            ..
                        } => {
                            let in_guard = guard.is_some_and(|guard| {
                                let at = ast.get(guard).span;
                                at.start <= offset && offset <= at.end
                            });
                            if in_body || in_guard {
                                scopes.push((span.start, vec![(name, name_span)]));
                            }
                        }
                        ForHead::While { before, .. } => {
                            scopes.push((span.start, items_before(ast, ast.items(before), offset)));
                        }
                        ForHead::Do { .. } => {}
                    }
                }
                _ => {}
            }
        }
        scopes.sort_by_key(|&(start, _)| Reverse(start));
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for (_, declared) in scopes {
            for (name, at) in declared.into_iter().rev() {
                if seen.insert(name) {
                    found.push((name, at));
                }
            }
        }
        found
    }

    /// Where the name `name` of the parameter written at `span` is.
    fn param_name(&self, name: Name, span: Span) -> Span {
        let length = u32::try_from(self.scan.names.get(name).len()).unwrap_or(0);
        Span::new(span.start, span.start.saturating_add(length).min(span.end))
    }

    /// The completion of the local `name` of file `file` whose name is at
    /// `at`, in the group `group`.
    fn local_completion(&self, file: u32, name: Name, at: Span, group: u8) -> Completion {
        let text = self.scan.names.get(name);
        let local = self
            .files
            .get(file as usize)
            .and_then(|f| f.locals.get(&at))
            .copied();
        // As declared: a parameter's type is provisional until its body is
        // checked, `Int?`, which is no type a source writes.
        let ty = local.and_then(|local| match *self.types.kind(local.ty) {
            TyKind::Provisional { inner, .. } => known(inner),
            _ => known(local.ty),
        });
        let kind = match local.map(|local| local.what) {
            Some(LocalKind::Function(_)) => CompletionKind::Function,
            _ if ty.is_some_and(|ty| self.is_function(ty)) => CompletionKind::Function,
            Some(LocalKind::Field) => CompletionKind::Field,
            _ => CompletionKind::Variable,
        };
        Completion {
            label: text.to_owned(),
            kind,
            detail: ty.and_then(|ty| self.short(ty)),
            documentation: self.documentation_of(Symbol::Local { file, name: at }),
            sort: sort_text(group, text),
            insert: quoted(text),
        }
    }

    /// The completion of the name `name` of a file's view, which is
    /// `symbol`, in the group `group`.
    fn view_completion(&self, name: Name, symbol: Option<Symbol>, group: u8) -> Completion {
        self.symbol_completion(self.scan.names.get(name), symbol, group)
    }

    /// The completion of `text`, a name of a file's view or a module's
    /// member, which is `symbol`, in the group `group`.
    fn symbol_completion(&self, text: &str, symbol: Option<Symbol>, group: u8) -> Completion {
        let (kind, detail) = match symbol {
            Some(Symbol::Node(node)) => {
                let ty = self.cells.get(node as usize).copied().and_then(known);
                let function =
                    self.def_of(node).is_some() || ty.is_some_and(|ty| self.is_function(ty));
                let kind = if function {
                    CompletionKind::Function
                } else {
                    CompletionKind::Field
                };
                (kind, ty.and_then(|ty| self.short(ty)))
            }
            Some(Symbol::Module(target)) => match target {
                Target::Fileroot(Some(member)) => (
                    if member == Fileroot::Helper {
                        CompletionKind::Function
                    } else {
                        CompletionKind::Field
                    },
                    Some(fileroot_type(member).to_owned()),
                ),
                Target::Dir(_) | Target::File(_) | Target::Fileroot(None) => {
                    (CompletionKind::Module, self.module_detail(target))
                }
                _ => (CompletionKind::Field, None),
            },
            _ => (CompletionKind::Field, None),
        };
        Completion {
            label: text.to_owned(),
            kind,
            detail,
            documentation: symbol.and_then(|symbol| self.documentation_of(symbol)),
            sort: sort_text(group, text),
            insert: quoted(text),
        }
    }

    /// Where the module `target` is, as a completion's detail says it.
    fn module_detail(&self, target: Target) -> Option<String> {
        let scan = &self.scan;
        match target {
            Target::Dir(dir) if dir == scan.builtins_dir => Some("built-in module".to_owned()),
            Target::Dir(dir) => {
                let path = &scan.dirs.get(dir as usize)?.path;
                Some(format!("directory `{path}/`"))
            }
            Target::File(file) => {
                let path = &scan.files.get(file as usize)?.path;
                match standard_stem(path) {
                    Some(stem) => Some(format!("standard library `{stem}`")),
                    None => Some(format!("file `{path}`")),
                }
            }
            Target::Fileroot(None) => Some("built-in module".to_owned()),
            _ => None,
        }
    }

    /// Whether `ty` is a function's type, a built-in function's included.
    fn is_function(&self, ty: TyId) -> bool {
        matches!(self.types.kind(ty), TyKind::Builtin(_))
            || self.types.fn_sig(self.types.strip(ty)).is_some()
    }

    /// The fields of what is before the `.` at `dot` of file `file`, the
    /// place being at `offset`: the children of a node of the view, of a
    /// module read through an import, or of the `fileroot` module; else the
    /// fields of the record type of what is there. `chain` is the path
    /// before it, if it is one, which is followed when the tree has nothing
    /// of the access.
    fn members(
        &self,
        file: u32,
        offset: u32,
        dot: Span,
        chain: Option<&[String]>,
    ) -> Vec<Completion> {
        let Some(entry) = self.scan.files.get(file as usize) else {
            return Vec::new();
        };
        let ast = &entry.ast;
        let scope = FileScope::new(&self.scan, entry);
        // The access the `.` is of, and what it reads a field of.
        let base = ast
            .expr_ids()
            .filter_map(|id| {
                let (ExprKind::Field { expr, .. } | ExprKind::NoField(expr)) = ast.get(id).kind
                else {
                    return None;
                };
                let (span, of) = (ast.get(id).span, ast.get(expr).span);
                (of.end <= dot.start && span.start <= dot.start && dot.end <= span.end)
                    .then_some((of.end, expr))
            })
            .max_by_key(|&(end, _)| end)
            .map(|(_, expr)| expr);
        let names: Option<Vec<Name>> = match base {
            Some(base) => path_names(ast, base),
            None => chain.and_then(|chain| {
                chain
                    .iter()
                    .map(|text| self.scan.names.lookup(text))
                    .collect()
            }),
        };
        // A path whose first name is no local's: through the view.
        if let Some(names) = &names
            && let Some((&first, rest)) = names.split_first()
        {
            let local = self
                .locals_at(ast, offset)
                .into_iter()
                .find(|&(n, _)| n == first);
            let bound = base.and_then(|base| {
                let mut at = base;
                while let ExprKind::Field { expr, .. } = ast.get(at).kind {
                    at = expr;
                }
                self.at(file, ast.get(at).span.start)
                    .and_then(|found| found.symbol)
            });
            let is_local = local.is_some() || matches!(bound, Some(Symbol::Local { .. }));
            if !is_local {
                let mut number = scope.root(first);
                for &name in rest {
                    number = number.and_then(|n| scope.child(n, name));
                }
                if let Some(number) = number {
                    let found = self.children(&scope, number);
                    if !found.is_empty() {
                        return found;
                    }
                }
            }
            if base.is_none()
                && let Some((_, at)) = local
            {
                // No tree of the access: the local's type, followed.
                let mut ty = self
                    .files
                    .get(file as usize)
                    .and_then(|f| f.locals.get(&at))
                    .map_or(TyId::ERROR, |l| l.ty);
                for &name in rest {
                    ty = self.field_of(ty, name).unwrap_or(TyId::ERROR);
                }
                return self.record_members(known(ty), None);
            }
        }
        let Some(base) = base else {
            return Vec::new();
        };
        let at_end = self
            .at(file, ast.get(base).span.end.saturating_sub(1))
            .filter(|found| found.span.end == ast.get(base).span.end)
            .copied();
        let ty = self
            .expr_type(file, base)
            .or_else(|| at_end.and_then(|found| known(found.ty)))
            .or_else(|| match at_end.and_then(|found| found.symbol) {
                Some(Symbol::Node(node)) => self.cells.get(node as usize).copied().and_then(known),
                _ => None,
            });
        self.record_members(ty, at_end.and_then(|found| found.symbol))
    }

    /// The type of the field `name` of `ty`, if it is a record type with
    /// one.
    fn field_of(&self, ty: TyId, name: Name) -> Option<TyId> {
        let fields = self.types.record_fields(self.types.strip(ty))?;
        fields.iter().find(|&&(f, _)| f == name).map(|&(_, ty)| ty)
    }

    /// The fields of `ty`, if it is a record type, each with its doc
    /// comment if `of`, what has it, is a record block written as the
    /// value of a local or a node.
    fn record_members(&self, ty: Option<TyId>, of: Option<Symbol>) -> Vec<Completion> {
        let Some(fields) = ty.and_then(|ty| self.types.record_fields(self.types.strip(ty))) else {
            return Vec::new();
        };
        fields
            .iter()
            .map(|&(name, ty)| {
                let text = self.scan.names.get(name);
                let kind = if self.is_function(ty) {
                    CompletionKind::Function
                } else {
                    CompletionKind::Field
                };
                Completion {
                    label: text.to_owned(),
                    kind,
                    detail: known(ty).and_then(|ty| self.short(ty)),
                    documentation: of.and_then(|of| self.field_doc(of, name)),
                    sort: sort_text(FILE, text),
                    insert: quoted(text),
                }
            })
            .collect()
    }

    /// The doc comment of the field `name` of the record block written as
    /// the value of `of`, a local or a leaf node, if it is one.
    fn field_doc(&self, of: Symbol, name: Name) -> Option<String> {
        let (owner, value) = match of {
            Symbol::Local { file, name: at } => {
                let item = self.files.get(file as usize)?.locals.get(&at)?.item?;
                let ast = &self.scan.files.get(file as usize)?.ast;
                (file, item_value(ast, item)?)
            }
            Symbol::Node(node) => {
                let owner = self.scan.nodes.get(node as usize)?.leaf?;
                let file = self.scan.files.get(owner as usize)?;
                let own = file.global.iter().position(|&g| g == node)?;
                (owner, file.mentions.tree.nodes.get(own)?.leaf?.value)
            }
            _ => return None,
        };
        let ast = &self.scan.files.get(owner as usize)?.ast;
        let ExprKind::Block { items, .. } = ast.get(unparen(ast, value)).kind else {
            return None;
        };
        let item = ast.items(items).iter().copied().find(|&item| {
            matches!(ast.item(item).kind, ItemKind::Field { path, .. }
                if ast.path(path).first().and_then(PathSegment::name) == Some(name))
        })?;
        self.doc_comment(owner, ast.item(item).span.start)
    }

    /// The children of the number `number` of the file's view `scope`: of
    /// a node, a module, or the `fileroot` module's members.
    fn children(&self, scope: &FileScope<'_>, number: u32) -> Vec<Completion> {
        if scope.target(number) == Some(Target::Fileroot(None)) {
            return Fileroot::ALL
                .into_iter()
                .map(|member| {
                    let symbol = Some(Symbol::Module(Target::Fileroot(Some(member))));
                    self.symbol_completion(member.name(), symbol, FILE)
                })
                .collect();
        }
        scope
            .children(number)
            .into_iter()
            .filter(|&(name, _)| !names::is_namespace_key(self.scan.names.get(name)))
            .map(|(name, child)| self.view_completion(name, scope_symbol(scope, child), FILE))
            .collect()
    }

    /// The types a type may name in file `file`: its view's, then the
    /// built-in types.
    fn types_at(&self, file: u32) -> Vec<Completion> {
        let Some(entry) = self.scan.files.get(file as usize) else {
            return Vec::new();
        };
        let base = self.scan.type_base;
        let mut found: Vec<Completion> = entry
            .type_view
            .iter()
            .filter_map(|(&name, &number)| {
                let decl = number.checked_sub(base)?;
                let group = match self.scan.types.get(decl as usize)?.file {
                    owner if owner == file => FILE,
                    owner if owner < self.scan.standard() => STANDARD,
                    _ => IMPORTED,
                };
                Some(self.type_completion(name, decl, group))
            })
            .collect();
        for builtin in BUILTIN_TYPE_NAMES {
            found.push(Completion {
                label: builtin.to_owned(),
                kind: CompletionKind::Type,
                detail: Some("built-in type".to_owned()),
                documentation: None,
                sort: sort_text(STANDARD, builtin),
                insert: None,
            });
        }
        found
    }

    /// The types of the module the first of `chain` imports, in a type of
    /// file `file`: `lib.` of `lib.T`.
    fn qualified_types(&self, file: u32, chain: &[String]) -> Vec<Completion> {
        let Some(entry) = self.scan.files.get(file as usize) else {
            return Vec::new();
        };
        let [first] = chain else {
            return Vec::new();
        };
        let Some(target) = self
            .scan
            .names
            .lookup(first)
            .and_then(|name| FileScope::new(&self.scan, entry).alias(name))
        else {
            return Vec::new();
        };
        self.scan
            .target_types(target)
            .into_iter()
            .filter_map(|decl| {
                let name = self.scan.types.get(decl as usize)?.name;
                Some(self.type_completion(name, decl, FILE))
            })
            .collect()
    }

    /// The completion of the declared type `decl`, an index in
    /// [`crate::workspace::Scan::types`], named `name`, in the group
    /// `group`.
    fn type_completion(&self, name: Name, decl: u32, group: u8) -> Completion {
        let text = self.scan.names.get(name);
        let expansion = self.scan.types.get(decl as usize).and_then(|declared| {
            let declared_in = self.files.get(declared.file as usize)?;
            let &(_, ty) = declared_in
                .declared
                .iter()
                .find(|&&(item, _)| item == declared.item)?;
            known(ty)
        });
        Completion {
            label: text.to_owned(),
            kind: CompletionKind::Type,
            detail: expansion.and_then(|ty| self.short(ty)),
            documentation: self.documentation_of(Symbol::Type(decl)),
            sort: sort_text(group, text),
            insert: None,
        }
    }

    /// What an `import` of file `file` may name after the segments
    /// `prefix`: the directories and files under the directory they name;
    /// at the start, the root directory's and the standard library's
    /// modules (only those for a file outside any workspace).
    fn modules_under(&self, file: u32, prefix: &[String]) -> Vec<Completion> {
        let scan = &self.scan;
        let mut found = Vec::new();
        let mut module = |label: &str, detail: String, group: u8| {
            found.push(Completion {
                label: label.to_owned(),
                kind: CompletionKind::Module,
                detail: Some(detail),
                documentation: None,
                sort: sort_text(group, label),
                insert: None,
            });
        };
        if prefix.is_empty() {
            // Every standard file, loaded or not: a scan loads only those
            // its files import.
            for &(stem, _) in &crate::STANDARD {
                module(stem, format!("standard library `{stem}`"), STANDARD);
            }
            module(BUILTINS, "built-in module".to_owned(), STANDARD);
            module(Fileroot::MODULE, "built-in module".to_owned(), STANDARD);
        }
        if scan.is_lone() {
            return found;
        }
        let path = prefix.join("/");
        let Some(dir) = (0u32..)
            .zip(&scan.dirs)
            .find(|(_, d)| d.path == path)
            .map(|(at, _)| at)
        else {
            return found;
        };
        for d in &scan.dirs {
            if d.parent != Some(dir) || d.path.starts_with(STANDARD_DIR) {
                continue;
            }
            let name = d.path.rsplit('/').next().unwrap_or(&d.path);
            module(name, format!("directory `{}/`", d.path), DIRECTORY);
        }
        for (at, f) in (0u32..).zip(&scan.files) {
            if f.dir != dir || at < scan.standard() || at == file || f.path == WORKSPACE {
                continue;
            }
            let name = f.path.rsplit('/').next().unwrap_or(&f.path);
            if let Some(stem) = name.strip_suffix(".gcsl") {
                module(stem, format!("file `{}`", f.path), FILE);
            }
        }
        found
    }

    /// The definitions under the segments `prefix` of the def-type named
    /// `def_type` (with where it is written), in file `file`: the children
    /// of the node of the path in the def-type's namespace. None if the name
    /// is no def-type's.
    fn definitions(
        &self,
        file: u32,
        def_type: &(String, Span),
        prefix: &[String],
    ) -> Vec<Completion> {
        let Some(entry) = self.scan.files.get(file as usize) else {
            return Vec::new();
        };
        let Some(key) = self.namespace_key(file, def_type) else {
            return Vec::new();
        };
        let scope = FileScope::new(&self.scan, entry);
        let mut number = scope.root(key);
        for segment in prefix {
            let name = self.scan.names.lookup(segment);
            number = number.zip(name).and_then(|(n, name)| scope.child(n, name));
        }
        number.map_or_else(Vec::new, |number| self.children(&scope, number))
    }

    /// The key of the namespace of the def-type named as `def_type` is,
    /// written where it is, in file `file`: of a reference to or a
    /// definition made with it in the file, or else in any file, the
    /// def-type it names resolving to what the name does here.
    fn namespace_key(&self, file: u32, (text, _): &(String, Span)) -> Option<Name> {
        let name = self.scan.names.lookup(text)?;
        let entry = self.scan.files.get(file as usize)?;
        let named = |ast: &Ast, dt| matches!(ast.get(dt).kind, ExprKind::Name(n) if n == name);
        let own = entry
            .ast
            .def_types()
            .find_map(|(_, used)| used.key.filter(|_| named(&entry.ast, used.dt)));
        if own.is_some() {
            return own;
        }
        let scope = FileScope::new(&self.scan, entry);
        let symbol = scope.root(name).and_then(|n| scope_symbol(&scope, n))?;
        (0u32..).zip(&self.scan.files).find_map(|(at, other)| {
            other.ast.def_types().find_map(|(_, used)| {
                let key = used.key?;
                let dt = other.ast.get(used.dt).span;
                (self.at(at, dt.start)?.symbol? == symbol).then_some(key)
            })
        })
    }
}

/// The names declared by the items of `items` that end before `offset`:
/// each `let`'s, `fn`'s and field's first segment, with where it is.
fn items_before(ast: &Ast, items: &[crate::ast::ItemId], offset: u32) -> Vec<(Name, Span)> {
    let mut found = Vec::new();
    for &item in items {
        let it = ast.item(item);
        if it.span.end >= offset {
            break;
        }
        if let ItemKind::Let { path, .. } | ItemKind::Field { path, .. } = it.kind
            && let Some(first) = first_segment(ast.path(path))
        {
            found.push(first);
        }
    }
    found
}

/// The name of the top-level field of `ast`, named by one segment and not a
/// `def`, whose value the place at `offset` is in: the field being
/// declared there.
fn declared_at(ast: &Ast, offset: u32) -> Option<Name> {
    ast.items(ast.root()).iter().find_map(|&item| {
        let it = ast.item(item);
        let ItemKind::Field {
            path,
            value,
            function: false,
            ..
        } = it.kind
        else {
            return None;
        };
        let in_value = ast.get(value).span.start <= offset && offset <= it.span.end;
        match ast.path(path) {
            [only] if in_value => only.name(),
            _ => None,
        }
    })
}

/// The span of the name being completed at byte `offset` of `text`, by
/// GCSL's rules for a name: the run of letters, digits and `_` around the
/// place, with each `-` between two of them, and a `-` just before the
/// place after one, `libcap-`, a name half-typed; empty, at the place, if
/// there is none. What a completion's edit replaces (to its end) or inserts
/// over (to the place).
#[must_use]
pub fn name_around(text: &str, offset: u32) -> Span {
    let bytes = text.as_bytes();
    let offset = (offset as usize).min(bytes.len());
    let is_name = |at: usize| {
        bytes
            .get(at)
            .is_some_and(|&byte| byte.is_ascii_alphanumeric() || byte == b'_')
    };
    let mut start = offset;
    while let Some(before) = start.checked_sub(1) {
        let dash = bytes.get(before) == Some(&b'-') && before.checked_sub(1).is_some_and(&is_name);
        if is_name(before) || dash {
            start = before;
        } else {
            break;
        }
    }
    let mut end = offset;
    loop {
        if is_name(end) {
            end += 1;
        } else if end > start && bytes.get(end) == Some(&b'-') && is_name(end + 1) {
            end += 2;
        } else {
            break;
        }
    }
    let at = |offset: usize| u32::try_from(offset).unwrap_or(u32::MAX);
    Span::new(at(start), at(end))
}

/// The name of the first of `segments`, and where it is.
fn first_segment(segments: &[PathSegment]) -> Option<(Name, Span)> {
    let first = segments.first()?;
    Some((first.name()?, first.span))
}

/// The names of the path `id` is, `a.b.c`, if it is one.
fn path_names(ast: &Ast, id: crate::ast::ExprId) -> Option<Vec<Name>> {
    let mut names = Vec::new();
    let mut at = id;
    loop {
        match ast.get(at).kind {
            ExprKind::Field { expr, name } => {
                names.push(name);
                at = expr;
            }
            ExprKind::Name(name) => {
                names.push(name);
                names.reverse();
                return Some(names);
            }
            _ => return None,
        }
    }
}

/// The text a list is ordered by of `label` in the group `group`.
fn sort_text(group: u8, label: &str) -> String {
    format!("{group}:{label}")
}

/// `text` quoted as a field's name is when it is no identifier: `None`
/// when it is one.
fn quoted(text: &str) -> Option<String> {
    let shown = FieldName(text).to_string();
    (shown != text).then_some(shown)
}

/// The type of the member `member` of the `fileroot` module, as GCSL
/// writes it.
const fn fileroot_type(member: Fileroot) -> &'static str {
    match member {
        Fileroot::Helper => "fn(): {}",
        Fileroot::This | Fileroot::Workspace | Fileroot::Caller => "String",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;
    use crate::workspace::index_with_standard;

    /// The completions at the `|` of the file at `path` among `files`, the
    /// `|` taken out, each file a path and a text, with a `workspace.gcsl`
    /// if they have none.
    fn complete(files: &[(&str, &str)], path: &str) -> Vec<Completion> {
        let mut at = None;
        let mut sources: Vec<SourceFile> = files
            .iter()
            .map(|&(name, text)| {
                if name == path {
                    at = text.find('|');
                }
                SourceFile::new(name, text.replacen('|', "", usize::from(name == path))).unwrap()
            })
            .collect();
        if !files.iter().any(|&(name, _)| name == "workspace.gcsl") {
            sources.push(SourceFile::new("workspace.gcsl", "").unwrap());
        }
        sources.sort_by(|a, b| a.name().cmp(b.name()));
        let index = index_with_standard(&sources, &[], "/").index.unwrap();
        let file = index.file(path).unwrap();
        index.completion(file, u32::try_from(at.unwrap()).unwrap())
    }

    /// The labels of `found`.
    fn labels(found: &[Completion]) -> Vec<&str> {
        found.iter().map(|c| c.label.as_str()).collect()
    }

    /// The completion labelled `label` among `found`.
    fn named<'a>(found: &'a [Completion], label: &str) -> &'a Completion {
        found
            .iter()
            .find(|c| c.label == label)
            .unwrap_or_else(|| panic!("no {label:?} in {:?}", labels(found)))
    }

    #[test]
    fn after_a_dot_the_fields_of_a_record_a_node_and_a_module() {
        // A record's, at the end of a line and of the file, the record kept.
        for text in [
            "fn g(): Int {\n    let c = {port = 80; host = \"h\"}\n    let x = c.|\n    1\n}\nr = g()\n",
            "c = {port = 80; host = \"h\"}\nx = c.|",
            "c = {port = 80; host = \"h\"}\nx = c.ho|st\n",
        ] {
            let found = complete(&[("app.gcsl", text)], "app.gcsl");
            assert_eq!(labels(&found), ["host", "port"], "{text}");
            let port = named(&found, "port");
            assert_eq!(port.kind, CompletionKind::Field);
            assert_eq!(port.detail.as_deref(), Some("Int"));
        }
        // A node of the view: its children, from another file.
        let files = [
            ("app.gcsl", "r = cfg.|\n"),
            (
                "lib.gcsl",
                "w = 0\n# The port.\ncfg.port = 1\ncfg.run = fn(): Int { 1 }\n",
            ),
        ];
        let found = complete(&files, "app.gcsl");
        assert_eq!(labels(&found), ["port", "run"]);
        assert_eq!(
            named(&found, "port").documentation.as_deref(),
            Some("The port.")
        );
        assert_eq!(named(&found, "run").kind, CompletionKind::Function);
        // An imported module's members.
        let found = complete(&[("app.gcsl", "import path\nr = path.|\n")], "app.gcsl");
        let join = named(&found, "join");
        assert_eq!(join.kind, CompletionKind::Function);
        assert_eq!(
            join.detail.as_deref(),
            Some("fn(base: String, rel: String): String")
        );
        // `fileroot`'s.
        let found = complete(
            &[("app.gcsl", "import fileroot\nr = fileroot.|\n")],
            "app.gcsl",
        );
        assert_eq!(labels(&found), ["caller", "helper", "this", "workspace"]);
        // A name that is no identifier is written quoted.
        let found = complete(&[("app.gcsl", "c = {\"a b\" = 1}\nr = c.|\n")], "app.gcsl");
        assert_eq!(named(&found, "a b").insert.as_deref(), Some("\"a b\""));
    }

    #[test]
    fn after_a_dot_a_record_with_a_rest_field_offers_its_named_fields() {
        // The rest-field names no field, and those it covers are not read
        // by name: the named ones are offered, whatever the rest-field's
        // type, and never lost to it.
        for rest in ["String", "Int"] {
            let text = format!(
                "fn g(): Int {{\n    let f: {{to: String, from: String, *: {rest}}} = \
                 {{to = \"a\"; from = \"b\"; x = {value}}}\n    let x = f.|\n    1\n}}\nr = g()\n",
                value = if rest == "Int" { "1" } else { "\"c\"" },
            );
            let found = complete(&[("app.gcsl", &text)], "app.gcsl");
            assert_eq!(labels(&found), ["from", "to"], "{text}");
            assert_eq!(named(&found, "to").detail.as_deref(), Some("String"));
        }
        // A map names none.
        let text = "fn g(): Int {\n    let m: {*: String} = {a = \"x\"}\n    let x = m.|\n    1\n}\nr = g()\n";
        let found = complete(&[("app.gcsl", text)], "app.gcsl");
        assert_eq!(labels(&found), [] as [&str; 0]);
    }

    #[test]
    fn a_name_is_completed_from_what_is_in_scope_nearest_first() {
        let text = "import v from lib.x\n# The top.\ntop = 1\nfn helper(n: Int): Int { n }\n\
                    fn g(p: Int): String {\n    let x = 1\n    let x = \"s\"\n    let y = |\n    let later = 2\n    x\n}\n\
                    r = g(1)\n";
        let files = [
            ("app.gcsl", text),
            ("dir.gcsl", "shared = 2\n"),
            ("lib/x.gcsl", "v = 1\n"),
        ];
        let found = complete(&files, "app.gcsl");
        // Shadowing: only the nearer `x`.
        let xs: Vec<&Completion> = found.iter().filter(|c| c.label == "x").collect();
        assert_eq!(xs.len(), 1);
        assert_eq!(xs[0].detail.as_deref(), Some("String"));
        assert_eq!(xs[0].kind, CompletionKind::Variable);
        // A local declared after the place is not offered.
        assert!(!labels(&found).contains(&"later"));
        // A parameter, in its function's body.
        assert_eq!(named(&found, "p").detail.as_deref(), Some("Int"));
        // The file's own, its directory's, the standard library's, then
        // the keywords, in that order.
        let groups: Vec<&str> = ["x", "top", "helper", "shared", "v", "len", "if"]
            .iter()
            .map(|label| &named(&found, label).sort[..1])
            .collect();
        // (Group 0 is the named arguments of a call.)
        assert_eq!(groups, ["1", "2", "2", "3", "4", "5", "6"]);
        assert_eq!(
            named(&found, "top").documentation.as_deref(),
            Some("The top.")
        );
        assert_eq!(named(&found, "helper").kind, CompletionKind::Function);
        assert_eq!(named(&found, "len").kind, CompletionKind::Function);
        assert_eq!(named(&found, "if").kind, CompletionKind::Keyword);
        // Outside the function, its parameter and locals are not offered.
        let found = complete(
            &[("app.gcsl", "fn f(p: Int): Int { p }\nr = |\n")],
            "app.gcsl",
        );
        assert!(!labels(&found).contains(&"p"));
        assert!(labels(&found).contains(&"f"));
    }

    #[test]
    fn completion_works_in_a_file_with_another_error() {
        for broken in ["broken = (1 2)\n", "broken = 1 + \"a\"\n", "broken = q.\n"] {
            let text = format!(
                "{broken}c = {{port = 80}}\nfn g(): Int {{\n    let local = 1\n    c.|\n}}\n"
            );
            let found = complete(&[("app.gcsl", &text)], "app.gcsl");
            assert_eq!(labels(&found), ["port"], "{broken}");
            let text = format!("{broken}fn g(): Int {{\n    let local = 1\n    lo|\n}}\n");
            let found = complete(&[("app.gcsl", &text)], "app.gcsl");
            assert!(labels(&found).contains(&"local"), "{broken}");
        }
    }

    #[test]
    fn a_type_is_completed_from_the_types_in_scope() {
        let text = "w = 0\n# A version.\ntype V = {version: String}\nx: |V = {version = \"1\"}\n";
        let found = complete(&[("app.gcsl", text)], "app.gcsl");
        let v = named(&found, "V");
        assert_eq!(v.kind, CompletionKind::Type);
        assert_eq!(v.detail.as_deref(), Some("{version: String}"));
        assert_eq!(v.documentation.as_deref(), Some("A version."));
        assert!(labels(&found).contains(&"Int") && labels(&found).contains(&"List"));
        // After `is`, in a parameter, and in a `type` item.
        for text in [
            "type V = Int\nfn f(v: |): Int { 1 }\n",
            "type V = Int\nr = 1 is |\n",
            "type V = Int\ntype W = |\n",
        ] {
            let found = complete(&[("app.gcsl", text)], "app.gcsl");
            assert!(
                labels(&found).contains(&"V"),
                "{text}: {:?}",
                labels(&found)
            );
            assert!(!labels(&found).contains(&"if"), "{text}");
        }
    }

    #[test]
    fn an_import_s_path_is_completed_by_segment() {
        let files = [
            ("app.gcsl", "import lib.|\n"),
            ("lib/x.gcsl", "v = 1\n"),
            ("lib/sub/y.gcsl", "w = 1\n"),
        ];
        let found = complete(&files, "app.gcsl");
        assert_eq!(labels(&found), ["x", "sub"]);
        let files = [("app.gcsl", "import |\n"), ("lib/x.gcsl", "v = 1\n")];
        let found = complete(&files, "app.gcsl");
        for module in ["lib", "path", "fs", "prelude", "builtins", "fileroot"] {
            assert!(
                labels(&found).contains(&module),
                "{module}: {:?}",
                labels(&found)
            );
        }
        let files = [
            ("app.gcsl", "import v from lib.|\n"),
            ("lib/x.gcsl", "v = 1\n"),
        ];
        assert_eq!(labels(&complete(&files, "app.gcsl")), ["x"]);
    }

    #[test]
    fn nothing_is_offered_in_a_string_a_comment_or_where_a_name_is_declared() {
        for text in [
            "x = \"ab|c\"\n",
            "x = 1 # a com|ment\n",
            "x = 1\n# |\n",
            "let |",
            "fn |",
            "fn f(|) {}\n",
            "x = 1|\n",
        ] {
            let found = complete(&[("app.gcsl", text)], "app.gcsl");
            assert_eq!(labels(&found), Vec::<&str>::new(), "{text}");
        }
    }

    #[test]
    fn the_name_completed_is_found_by_gcsl_s_rules_for_names() {
        let around = |text: &str| {
            let at = text.find('|').unwrap();
            let text = text.replacen('|', "", 1);
            let span = name_around(&text, u32::try_from(at).unwrap());
            text[span.start as usize..span.end as usize].to_owned()
        };
        assert_eq!(around("x = odd.libcap-t|"), "libcap-t");
        assert_eq!(around("x = odd.libcap-|"), "libcap-");
        assert_eq!(around("x = odd.lib|cap-tools\n"), "libcap-tools");
        assert_eq!(around("x = c.|"), "");
        assert_eq!(around("x = a - b|"), "b");
        assert_eq!(around("x = |-a"), "");
        assert_eq!(around("x = dev.1pass|word"), "1password");
    }

    #[test]
    fn a_name_typed_up_to_a_dash_is_completed() {
        let text = "odd.libcap-tools = 1\nodd.other = 2\nr = odd.libcap-|\n";
        let found = complete(&[("app.gcsl", text)], "app.gcsl");
        assert_eq!(labels(&found), ["libcap-tools", "other"]);
    }

    #[test]
    fn the_field_being_declared_is_not_offered() {
        let found = complete(&[("app.gcsl", "para = 1\nb = p|\n")], "app.gcsl");
        assert!(labels(&found).contains(&"para"), "{:?}", labels(&found));
        assert!(!labels(&found).contains(&"b"), "{:?}", labels(&found));
        // Nor the field whose record block the place is in.
        let found = complete(
            &[("app.gcsl", "para = 1\ncfg = {\n    a = |\n}\n")],
            "app.gcsl",
        );
        assert!(!labels(&found).contains(&"cfg"), "{:?}", labels(&found));
        // A `def` and a `fn` in their own bodies are, and a dotted field's
        // root.
        for (text, name) in [
            ("def f(n: Int): Int { |n }\n", "f"),
            ("fn g(n: Int): Int { |n }\nr = g(1)\n", "g"),
            ("a.b = 1\na.c = |\n", "a"),
        ] {
            let found = complete(&[("app.gcsl", text)], "app.gcsl");
            assert!(
                labels(&found).contains(&name),
                "{text}: {:?}",
                labels(&found)
            );
        }
    }

    #[test]
    fn a_call_s_unfilled_parameters_are_offered_as_named_arguments() {
        let f = "def f(a: Int, b: String = \"x\", c: Bool = true): Int { a }\n";
        let found = complete(&[("app.gcsl", &format!("{f}r = f(1, |)\n"))], "app.gcsl");
        // Before the locals, those not filled by an argument.
        assert_eq!(labels(&found)[..2], ["b =", "c ="]);
        let b = named(&found, "b =");
        assert_eq!(b.kind, CompletionKind::Argument);
        assert_eq!(b.detail.as_deref(), Some("String"));
        assert_eq!(b.insert.as_deref(), Some("b = "));
        assert_eq!(b.sort, "0:b");
        // A named argument fills its own, anywhere; a word half-typed.
        for (call, offered) in [
            ("r = f(c = true, |)", vec!["a =", "b ="]),
            ("r = f(b = \"y\", |)", vec!["a =", "c ="]),
            ("r = f(1, c = true, x|", vec!["b ="]),
            ("r = f(\n    1,\n    |\n)", vec!["b =", "c ="]),
        ] {
            let found = complete(&[("app.gcsl", &format!("{f}{call}\n"))], "app.gcsl");
            let arguments: Vec<&str> = found
                .iter()
                .filter(|c| c.kind == CompletionKind::Argument)
                .map(|c| c.label.as_str())
                .collect();
            assert_eq!(arguments, offered, "{call}");
        }
        // Only where an argument starts; none for `len`.
        for call in ["r = f(a = |)", "r = f(1 + |)", "r = len(|)", "r = |"] {
            let found = complete(&[("app.gcsl", &format!("{f}{call}\n"))], "app.gcsl");
            assert!(
                found.iter().all(|c| c.kind != CompletionKind::Argument),
                "{call}: {:?}",
                labels(&found)
            );
        }
    }

    #[test]
    fn a_def_type_reference_s_path_lists_the_definitions_under_it() {
        let workspace =
            "#[def_type]\ndef package(name: String, func: fn(): String): String { func() }\n";
        let files = [
            ("app.gcsl", "r = package dev.|\n"),
            (
                "pkg.gcsl",
                "def package dev.x() { \"x\" }\ndef package dev.y() { \"y\" }\n",
            ),
            ("workspace.gcsl", workspace),
        ];
        assert_eq!(labels(&complete(&files, "app.gcsl")), ["x", "y"]);
        let files = [
            ("app.gcsl", "r = package |\n"),
            ("pkg.gcsl", "def package dev.x() { \"x\" }\n"),
            ("workspace.gcsl", workspace),
        ];
        assert_eq!(labels(&complete(&files, "app.gcsl")), ["dev"]);
    }
}
