//! The parser: tokens to an [`Ast`].
//!

use crate::ast::{
    Arg, Ast, AstBuilder, AttributeUse, BinaryOp, BuilderMark, Element, ExprId, ExprKind, FieldTy,
    ItemId, ItemRange, Param, ParamTy, PathSegment, StrPart, Syntax, TypeExprId,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{self, MAX_ERRORS, Token, TokenKind, lex};
use crate::names::Names;
use crate::source::{SourceFile, Span};
use crate::strings::Strings;

/// Minimum binding power of the operand of prefix `not`: the left power of
/// comparison, so the operand takes in a comparison but not `and` or `or`.
const NOT_BP: u8 = 5;

/// Minimum binding power of the operand of prefix `-`: tighter than every
/// binary operator and than ascription.
const PREFIX_BP: u8 = 13;

/// Left binding power of postfix `:` (type ascription) and of postfix `as`:
/// tighter than every binary operator, looser than prefix `-`.
const ASCRIBE_BP: u8 = 11;

/// Left binding power of postfix `is` (a type test): the left power of
/// comparison, which it binds as.
const IS_BP: u8 = 5;

/// Left binding power of postfix `.` (field access): tighter than
/// everything, prefix `-` included.
const FIELD_BP: u8 = 15;

/// Height of a leaf node, and of a type written as a name.
const LEAF_HEIGHT: u32 = 1;

/// The error for a raw or interpolated string literal where a field name is
/// expected.
const PLAIN_FIELD_NAME: &str = "a field name must be a plain string literal";

/// The error for a string literal where a variable's name is expected.
const PLAIN_VARIABLE_NAME: &str = "variables need plain names";

/// The error for a quoted or an index segment of a `def`'s path, at the
/// segment.
const DEF_PATH: &str = "a `def`'s path is made of identifiers";

/// The error for a `def` named by a string, at the string.
const DEF_STRING_NAME: &str = "a `def` is named by an identifier, not a string";

/// The error for a function, or a function type, without a return marker.
const MISSING_RETURN_TYPE: &str =
    "expected `:`, `->` or `->>` and a return type after the parameters";

/// The error for a `...` that starts neither an element of a list literal
/// nor an item, where an expression starts. (A spread item is recognised
/// by `Parser::parse_item` before any expression is parsed, so it never
/// reaches this.)
const MISPLACED_SPREAD: &str = "`...` is only allowed in a list or at the start of an item";

/// The error for an `import` item anywhere but in a file's outermost body.
const MISPLACED_IMPORT: &str = "`import` is only allowed at the top level of a file";

/// The contextual keyword that begins a `type` item.
const TYPE: &str = "type";

/// The word that starts an `enum` item, which is no keyword (see
/// [Enums](crate::parser::enums#enums)).
const ENUM: &str = "enum";

/// The word after an extensible enum's name, which is no keyword (see
/// [Enums](crate::parser::enums#enums)).
const EXTENSIBLE: &str = "extensible";

/// The error for an `enum` anywhere but at a file's top level, at its
/// `enum`.
const MISPLACED_ENUM: &str = "`enum` is declared only at the top level of a file";

/// The word that starts a `context` item, which is no keyword (see
/// [Context](crate::parser::context#context)).
const CONTEXT: &str = "context";

/// The word of `with`, which is no keyword (see [Context](crate::parser::context#context)).
const WITH: &str = "with";

/// The word that starts an `assert` statement, which is no keyword (see
/// [Failure](crate::parser::failure#failure)).
const ASSERT: &str = "assert";

/// The word that starts a `recover` expression, which is no keyword (see
/// [Failure](crate::parser::failure#failure)).
const RECOVER: &str = "recover";

/// The error for a `context` item anywhere but at a file's top level, at
/// its `context`.
const MISPLACED_CONTEXT: &str = "`context` is declared only at the top level of a file";

/// The error for a `type` item anywhere but in a file's outermost body.
const MISPLACED_TYPE: &str = "`type` is declared only at the top level of a file";

/// The error for a method anywhere but at a file's top level, at its
/// keyword.
const MISPLACED_METHOD: &str = "a method is declared only at the top level of a file";

/// The name of the outer attribute that marks a declaration shadowable.
const SHADOWABLE: &str = "shadowable";

/// The name of the inner attribute that turns the implicit prelude off.
const NO_PRELUDE: &str = "no_prelude";

/// The name of the outer attribute that makes a `def` a def-type.
const DEF_TYPE: &str = "def_type";

/// The error for `#[def_type]` before anything but a top-level `def`.
const MISPLACED_DEF_TYPE: &str = "`#[def_type]` applies only to a file's top-level `def`s";

/// The name of the outer attribute that makes a `type` an attribute.
const ATTRIBUTE: &str = "attribute";

/// The error for `#[attribute]` before anything but a top-level `type`.
const MISPLACED_ATTRIBUTE: &str = "`#[attribute]` applies only to a file's top-level `type`s";

/// The error for a use of an attribute before anything but a function item.
const MISPLACED_USE: &str = "attributes apply to functions: a `fn`, a `def` or a definition";

/// The error for a definition made with a def-type anywhere but in a file's
/// outermost body.
const MISPLACED_DEF_TYPE_DEF: &str =
    "a def-type definition is declared only at the top level of a file";

/// The text of the name a def-type reference's read of its definition
/// starts with until the workspace names the def-type's namespace: no
/// program can write it, `<` being in no identifier. A definition's read of
/// its meta starts with it, and its field access names it, until the
/// workspace names the namespace of the metas and the meta's leaf.
const UNRESOLVED_NAMESPACE: &str = "<def-type>";

/// The contextual keyword that begins a definition's meta (see
/// [Def-types](crate::parser::deftypes#def-types)).
const DEFMETA: &str = "defmeta";

/// The error for a definition's meta anywhere but in a file's outermost
/// body.
const MISPLACED_DEFMETA: &str = "a `defmeta` is declared only at the top level of a file";

/// The name that, after a function type's return type and followed by `:`,
/// `->` or `->>`, begins its meta (see [Types](crate::parser::types#types)).
const META: &str = "META";

/// The error for a meta's marker anywhere but after a function type's
/// return type.
const MISPLACED_META: &str = "a meta is written only in a function type";

/// The error for a parameter's default, `= EXPR`, in a function type, at
/// its `=`: only a function expression's parameter has one.
const MISPLACED_DEFAULT: &str = "a default is written only in a function expression's parameters";

/// The error for `#[shadowable]` before anything but a top-level field,
/// `def` or `import`.
const MISPLACED_SHADOWABLE: &str =
    "`#[shadowable]` applies only to a file's top-level fields, `def`s and imports";

/// The note on [`MISPLACED_SHADOWABLE`] before a `let`, `let mut` or `fn`.
const LET_SHADOWS_NOTE: &str = "a `let` or `fn` already shadows";

/// The error for `#![no_prelude]` anywhere but before the file's first
/// item.
const MISPLACED_NO_PRELUDE: &str = "`#![no_prelude]` must come before anything else in the file";

/// The error for a module path written as a string.
const QUOTED_MODULE_PATH: &str =
    "a module path is written without quotes, its segments separated by `/`";

/// The error for an empty segment of a module path, at its `/`.
const EMPTY_MODULE_SEGMENT: &str = "expected a module name before and after each `/`";

/// What a module path's segment may be made of, for the error when one is
/// not.
const MODULE_SEGMENT: &str =
    "a segment starts with a letter, a digit or `_`, ends with one or `+`, and has no `..`";

/// The error for a member of a `from` import that is not a single name.
const SINGLE_MEMBER: &str = "a name imported with `from` is a single name";

/// The error for a `mut` anywhere but directly after `let`.
const MISPLACED_MUT: &str = "`mut` is only allowed after `let`";

/// The error for a `let mut` whose path is dotted.
const DOTTED_MUT: &str = "`let mut` needs a plain name";

/// The note on [`DOTTED_MUT`].
const DOTTED_MUT_NOTE: &str = "a dotted `let` updates a variable; declare the variable with `let \
     mut a = ...` and update it with `a.b := ...`";

/// The error for an index segment in the path of a `let`.
const LET_INDEX: &str = "a `let` path cannot index a list";

/// The error for an index segment in the path of a field item.
const FIELD_INDEX: &str = "a field's path cannot index a list";

/// The error for a label anywhere but directly after the keyword of what it
/// belongs to.
const MISPLACED_LABEL: &str = "a label names a loop or a block: it follows `for`, `do`, `break`, \
     `continue`, `return` or `{`";

/// The error for a label on a record block, which nothing can leave by
/// it.
const LABELLED_RECORD: &str = "a record block cannot be labelled";

/// The error for a `do` loop whose body is not followed by `for`.
const MISSING_DO_FOR: &str = "expected `for` and a condition after the body of a `do`";

/// The error for anything but a condition after the `for` of a `do` loop.
const DO_CONDITION_ONLY: &str = "only a condition follows the `for` of a `do`";

/// The error for a `{` where an expression starts in a `for` header.
const HEADER_BRACE: &str = "a `{` in a `for` header begins its body";

/// The note on [`HEADER_BRACE`].
const HEADER_BRACE_NOTE: &str =
    "put a block in a `for` header in parentheses, as in `for let r = ({ a = 1 }); ...`";

/// The note on [`MISPLACED_MUT`] before a field.
const MUT_FIELD_NOTE: &str = "a field is defined once; declare a variable with `let mut x = ...` \
     and define the field from it";

/// The note on [`MISPLACED_MUT`] before a parameter.
const MUT_PARAM_NOTE: &str = "copy the parameter into a mutable variable, as in `let mut n = n`";

/// The note on [`MISSING_RETURN_TYPE`].
const RETURN_TYPE_NOTE: &str = "a function declares its return type: `: T` for a pure function, `-> T` for an impure one, \
     `->> T` for a memoised one";

/// Parses `tokens`, lexed from `source` with its names interned into
/// `names` and its string literals' text stored in `strings`, as a file: a
/// body of items. The returned [`Ast`] owns `names` and `strings`.
///
/// All tokens must be consumed. `tokens` should end with
/// [`TokenKind::Eof`], as [`crate::lexer::tokenize`] guarantees; if it does
/// not, running out of tokens is treated as end of input.
///
/// # Errors
///
/// Returns the first syntax error, or, for tokens with a
/// [`TokenKind::Error`] and no syntax error, "invalid token" at the first.
/// [`parse_all`] gives every error, with the tree recovered from them.
pub fn parse(
    tokens: &[Token],
    mut names: Names,
    mut strings: Strings,
    source: &SourceFile,
) -> Result<Ast, Diagnostic> {
    let (builder, root, diagnostics) = parse_parts(tokens, source, (&mut names, &mut strings), 0);
    if let Some(first) = diagnostics.into_iter().next() {
        return Err(first);
    }
    if let Some(token) = tokens.iter().find(|token| token.kind == TokenKind::Error) {
        return Err(Diagnostic::error("invalid token", token.span));
    }
    Ok(builder.finish(root, names, strings))
}

/// Parses `tokens` as [`parse`] does, but going on past every syntax error
/// (see [Errors and recovery](crate::parser::recovery#errors-and-recovery)): returns the tree,
/// with an error node in place of each part of the source that could not
/// be parsed ([`Ast::syntax`] says whether it has any), and every error,
/// `lexical`, the lexer's ([`crate::lexer::lex`]), among them, in source
/// order and capped ([`capped`]).
#[must_use]
pub fn parse_all(
    tokens: &[Token],
    mut names: Names,
    mut strings: Strings,
    source: &SourceFile,
    lexical: Vec<Diagnostic>,
) -> (Ast, Vec<Diagnostic>) {
    let (builder, root, syntax) =
        parse_parts(tokens, source, (&mut names, &mut strings), lexical.len());
    (
        builder.finish(root, names, strings),
        capped(lexical, syntax),
    )
}

/// Lexes and parses `source` going on past every error ([`lex`],
/// [`parse_all`]): its tree, recovered from the errors, and every lexical
/// and syntax error, in source order and capped.
#[must_use]
pub fn parse_source(source: &SourceFile) -> (Ast, Vec<Diagnostic>) {
    let mut names = Names::new();
    let lexed = lex(source, &mut names);
    parse_all(
        &lexed.tokens,
        names,
        lexed.strings,
        source,
        lexed.diagnostics,
    )
}

/// The errors of one source, its lexer's, `lexical`, and its parser's,
/// `syntax`, each in the order found, merged in source order (a lexical
/// error first among those that start at one place) and capped: past
/// [`MAX_ERRORS`], the first after them is replaced by "too many errors",
/// at it, and the rest are left out.
#[must_use]
pub fn capped(lexical: Vec<Diagnostic>, syntax: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut all = lexical;
    all.extend(syntax);
    all.sort_by_key(|diagnostic| diagnostic.span.start);
    if all.len() > MAX_ERRORS {
        all.truncate(MAX_ERRORS + 1);
        if let Some(last) = all.last_mut() {
            *last = Diagnostic::error("too many errors", last.span).with_note(format!(
                "only the first {MAX_ERRORS} errors of this file are reported"
            ));
        }
    }
    all
}

/// Like [`parse_all`], returning the unfinished arenas and the file's body
/// instead of an [`Ast`], and borrowing the tables rather than taking them,
/// so that a caller lending its [`Names`] hands them over only once the
/// tree is finished, with the parser's errors alone, `lexical` being the
/// number of the lexer's. The text of every quoted field name is interned
/// into `names`, those of source that could not be parsed among them. The
/// source text of the position of every index segment of a path is added
/// to `strings` (see [`crate::ast::SegmentKind::Index`]).
pub(crate) fn parse_parts(
    tokens: &[Token],
    source: &SourceFile,
    (names, strings): (&mut Names, &mut Strings),
    lexical: usize,
) -> (AstBuilder, ItemRange, Vec<Diagnostic>) {
    // A lexical error whose token runs to the end of the source, an
    // unterminated string literal, leaves nothing after it to parse: what
    // is missing there is no error of its own.
    let quiet_from = match tokens {
        [.., last, eof]
            if last.kind == TokenKind::Error
                && eof.kind == TokenKind::Eof
                && last.span.end == source.len() =>
        {
            last.span.start
        }
        _ => u32::MAX,
    };
    let mut parser = Parser {
        tokens,
        pos: 0,
        source,
        names,
        strings,
        // There are never more nodes than tokens, so these never
        // reallocate: every node consumes a token of its own except the
        // empty else block of an `if` without `else`, and that `if`'s own
        // block consumes two tokens, `{` and `}`, for one node; and a call
        // with only a trailing block, whose block likewise consumes two.
        // (A field access consumes two, `.` and the name, a call with an
        // argument list its `(` and `)`, and a function expression at least
        // `fn`, `(`, `)`, a marker and a type besides its body.) An error
        // node takes the place of the nodes of the source it stands for,
        // which is at least the token it is at, but at the end of the input.
        builder: AstBuilder::with_capacity(tokens.len()),
        heights: Vec::with_capacity(tokens.len()),
        depth: 0,
        item_stack: Vec::new(),
        part_stack: Vec::new(),
        field_ty_stack: Vec::new(),
        param_stack: Vec::new(),
        param_ty_stack: Vec::new(),
        arg_stack: Vec::new(),
        element_stack: Vec::new(),
        path: Vec::new(),
        braces: Braces::Free,
        recovery: Recovery::Body { top: true },
        quiet_from,
        unclosed: false,
        deep: false,
        line_end: None,
        diagnostics: Vec::new(),
    };
    if lexical > 0 {
        parser.builder.set_syntax(Syntax::Recovered);
    }
    let (root, _) = parser.parse_body(None);
    (parser.builder, root, parser.diagnostics)
}

/// What an item's height needs to know of its path: its number of segments,
/// each after the first adding two levels, and the height of its tallest
/// index position, 0 if it has none.
#[derive(Clone, Copy, Debug)]
struct PathShape {
    /// The number of segments.
    segments: u32,
    /// The height of the tallest position of an index segment, or 0.
    positions: u32,
}

impl PathShape {
    /// The shape of a single name, and of an item without a path.
    const SINGLE: PathShape = PathShape {
        segments: 1,
        positions: 0,
    };
}

/// The outer attributes read before an item of a body, waiting for it (see
/// [Attributes](crate::parser::attributes#attributes)): where each is written, if it is.
#[derive(Clone, Debug, Default)]
struct Pending {
    /// `#[shadowable]`.
    shadowable: Option<Span>,
    /// `#[def_type]`.
    def_type: Option<Span>,
    /// `#[attribute]`.
    attribute: Option<Span>,
    /// The uses of attributes that are none of the parser's, in source
    /// order, each with the item it applies to left for the item to give.
    uses: Vec<AttributeUse>,
}

impl Pending {
    /// Whether no attribute is waiting.
    fn is_empty(&self) -> bool {
        self.shadowable.is_none()
            && self.def_type.is_none()
            && self.attribute.is_none()
            && self.uses.is_empty()
    }

    /// The first attribute written of those waiting, by its name and where
    /// it is written.
    fn first(&self, names: &Names) -> Option<(String, Span)> {
        let builtin = [
            (SHADOWABLE, self.shadowable),
            (DEF_TYPE, self.def_type),
            (ATTRIBUTE, self.attribute),
        ];
        let builtin = builtin
            .into_iter()
            .filter_map(|(name, span)| Some((name.to_owned(), span?)));
        let uses = self
            .uses
            .iter()
            .map(|used| (names.get(used.name).to_owned(), used.span));
        builtin.chain(uses).min_by_key(|&(_, span)| span.start)
    }
}

/// What a `{` means where the parser is (see [Trailing
/// blocks](crate::parser::expressions#trailing-blocks)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Braces {
    /// A `{` after an expression on its line starts a trailing block, and
    /// one where an expression starts, a block.
    Free,
    /// A `{` after an expression never starts a trailing block, since it
    /// begins the block of an `if`, or the body of a `for`; one where an
    /// expression starts is a block.
    NoTrailing,
    /// As [`Braces::NoTrailing`], and a `{` where an expression starts is
    /// the body too, and so an error: in the header of a `for`.
    Header,
}

/// Proof that an error is on record: pushed to the parser's sink, or, for
/// one at a [`TokenKind::Error`], the lexer's. Only [`Parser::report`]
/// creates one, and [`Parser::report_close`], for an error on record at its
/// place already.
#[derive(Debug)]
struct Reported;

type ParseResult<T> = Result<T, Reported>;

/// How an argument of a call failed: its value, or the separator after it,
/// the argument parsed.
#[derive(Debug)]
enum Entry {
    /// The value.
    Value(Reported),
    /// The separator after it.
    Separator(Reported),
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    source: &'a SourceFile,
    /// Where quoted field names are interned.
    names: &'a mut Names,
    /// The text of the string literals, for quoted field names, to which the
    /// source text of each index segment's position is added.
    strings: &'a mut Strings,
    builder: AstBuilder,
    /// The height of each allocated expression node, indexed like the
    /// builder's arena: 1 for a leaf, one more than the tallest child
    /// otherwise. A block's children are its items' expressions.
    heights: Vec<u32>,
    /// Current recursion depth: nested [`Parser::parse_expr`] calls and
    /// nested block bodies.
    depth: usize,
    /// The items of the bodies being parsed, innermost last. Each body
    /// pushes its items here as it parses them and moves them to the
    /// builder's item-list pool when it ends, so nested bodies need no
    /// allocation of their own.
    item_stack: Vec<ItemId>,
    /// The parts of the interpolated strings being parsed, innermost last,
    /// moved to the builder's part pool as each string ends, as for
    /// `item_stack`.
    part_stack: Vec<StrPart>,
    /// The fields of the record types being parsed, innermost last, moved
    /// to the builder's field-type pool as each type ends, as for
    /// `item_stack`.
    field_ty_stack: Vec<FieldTy>,
    /// The parameters of the function expressions being parsed, moved to
    /// the builder's parameter pool as each parameter list ends, as for
    /// `item_stack`.
    param_stack: Vec<Param>,
    /// The parameters of the function types being parsed, innermost last,
    /// moved to the builder's parameter-type pool as each type ends, as for
    /// `item_stack`.
    param_ty_stack: Vec<ParamTy>,
    /// The arguments of the calls being parsed, innermost last, moved to
    /// the builder's argument pool as each call ends, as for `item_stack`.
    arg_stack: Vec<Arg>,
    /// The elements of the list literals being parsed, innermost last,
    /// moved to the builder's element pool as each literal ends, as for
    /// `item_stack`.
    element_stack: Vec<Element>,
    /// The segments of the paths being read, innermost last, moved to the
    /// builder's path pool as each path is complete, as for `item_stack`:
    /// the position of an index segment may hold a block, whose items read
    /// paths of their own.
    path: Vec<PathSegment>,
    /// What a `{` means where the parser is: whether one after an
    /// expression is kept from starting a trailing block, as it is while
    /// parsing the condition of an `if`, so that its block is not taken for
    /// one, and, in the header of a `for`, whether one that starts an
    /// expression is the body too. Lifted inside parentheses, argument
    /// lists, list literals, indexes, blocks and interpolations.
    braces: Braces,
    /// Whether the value of an item being parsed recovers from an error in
    /// it, as a body's items' do, and in which body (see [Errors and
    /// recovery](crate::parser::recovery#errors-and-recovery)).
    recovery: Recovery,
    /// Where the lexical error whose token runs to the end of the source
    /// starts, if one does: no error is reported from there on, since
    /// nothing after it was lexed. `u32::MAX` if none does.
    quiet_from: u32,
    /// Whether the end of the input inside a block has been reported: it is
    /// reported once, at the innermost block.
    unclosed: bool,
    /// Whether the error being recovered from is nesting too deep, which
    /// no list and no interpolation recovers from: the item, or its value,
    /// does.
    deep: bool,
    /// The newline token, by its index, past which no newline is skipped:
    /// set while an item that failed is parsed again up to the end of its
    /// line ([`Parser::item_line`]).
    line_end: Option<usize>,
    /// Errors reported so far, in the order found: at most [`MAX_ERRORS`]
    /// and one more, after which the parser skips to the end of the input.
    diagnostics: Vec<Diagnostic>,
}

/// Whether the value of an item recovers from an error in it (see [Errors
/// and recovery](crate::parser::recovery#errors-and-recovery)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recovery {
    /// It does, as an item of a body does: of the file's body if `top`, of
    /// a block's otherwise.
    Body {
        /// Whether the body is the file's.
        top: bool,
    },
    /// It does not, as a part of a `for` header does not: its error is its
    /// header's.
    None,
}

/// Where the parser's stacks and the builder's arenas were, with its depth
/// and what a `{` meant: what a recovery point puts back when what it
/// parses fails, so that nothing of the source it skips is left in the tree
/// (see [Errors and recovery](crate::parser::recovery#errors-and-recovery)).
#[derive(Clone, Copy, Debug)]
struct Marks {
    builder: BuilderMark,
    heights: usize,
    items: usize,
    parts: usize,
    field_tys: usize,
    params: usize,
    param_tys: usize,
    args: usize,
    elements: usize,
    path: usize,
    depth: usize,
    braces: Braces,
    /// The number of errors reported, the tree's syntax and whether the end
    /// of the input in a block was reported: what [`Parser::rewind`], but
    /// not [`Parser::restore`], puts back.
    diagnostics: usize,
    syntax: Syntax,
    unclosed: bool,
}

/// What ends a skip over source that could not be parsed (see [Errors and
/// recovery](crate::parser::recovery#errors-and-recovery)).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sync {
    /// An item of a body, or its value: a newline or `;` ends it, and so
    /// does a `}` that closes the body, which is a block's but if `top`.
    Item {
        /// Whether the body is the file's, which no `}` closes.
        top: bool,
    },
    /// An entry of a list that `close` closes: a `,` or `close` ends it.
    Entry {
        /// The token that closes the list.
        close: TokenKind,
    },
    /// The expression of an interpolation: the string's piece after it
    /// ends it.
    Interpolation,
}

/// Where a skip stopped, at a token it did not consume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    /// At the separator after an item or an entry: a newline, a `;` or a
    /// `,`.
    Separator,
    /// At what closes the body, the list or the interpolation.
    Close,
    /// At the end of the input, or at what closes something around the list.
    Outer,
}

/// A bracket opened in the source a skip goes over: what the lexer keeps
/// on its stack, which a skip follows (see [`nest`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    /// `(`
    Paren,
    /// `[`
    Square,
    /// `{`
    Brace,
    /// The start of an interpolated string, which its last piece closes.
    Str,
}

impl Parser<'_> {
    /// Skips item separators: newlines and `;`.
    fn skip_separators(&mut self) {
        while matches!(self.peek().kind, TokenKind::Newline | TokenKind::Semi) {
            self.advance();
        }
    }

    /// Skips newlines where an item continues on the next line.
    fn skip_newlines(&mut self) {
        while self.peek().kind == TokenKind::Newline && self.line_end != Some(self.pos) {
            self.advance();
        }
    }

    /// The current token. Past the end of a token slice that lacks a final
    /// `Eof`, this synthesises one at the end of the source.
    fn peek(&self) -> Token {
        self.peek_at(0)
    }

    /// The token `ahead` places after the current one, or a synthesised
    /// `Eof` past the end.
    fn peek_at(&self, ahead: usize) -> Token {
        self.token_at(self.pos + ahead)
    }

    /// The token at `index`, or a synthesised `Eof` past the end.
    fn token_at(&self, index: usize) -> Token {
        self.tokens.get(index).copied().unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.source.len(), self.source.len()),
        })
    }

    /// Moves past the current token. Never moves past `Eof`, which callers
    /// never consume.
    fn advance(&mut self) {
        self.pos += 1;
    }

    fn span_of(&self, id: ExprId) -> Span {
        self.builder.get(id).span
    }

    fn type_span(&self, id: TypeExprId) -> Span {
        self.builder.type_expr(id).span
    }

    /// Describes a token for "found ..." messages: literals and names by
    /// their source text, operators and keywords by their symbol, both in
    /// backticks. The piece of a string after an interpolation is found
    /// where an expression's tokens end, so it is described by the `}` that
    /// ends the interpolation, as [`error_span`] points at.
    fn describe(&self, token: Token) -> String {
        match (token.kind, token.kind.symbol()) {
            (_, Some(symbol)) => format!("`{symbol}`"),
            (TokenKind::Eof, None) => "end of input".to_owned(),
            (TokenKind::Newline, None) => "newline".to_owned(),
            (TokenKind::StrMid(_) | TokenKind::StrEnd(_), None) => "`}`".to_owned(),
            (_, None) => format!("`{}`", self.source.slice(token.span)),
        }
    }

    /// `kind`, the literal of the number token `number`, unless a name is
    /// glued to it, as in `30sec`: the lexer makes a number glued to a unit
    /// a duration or a size, and leaves it a number before any other name,
    /// so that a module path's `pkgs/7zip` is still a path. Where an
    /// expression is expected, that name is "unknown unit `sec`", noted with
    /// the units there are (see [Expressions](crate::parser::expressions#expressions)).
    fn unitless(&mut self, number: Token, kind: ExprKind) -> ParseResult<ExprKind> {
        let next = self.peek_at(1);
        if next.span.start != number.span.end || !matches!(next.kind, TokenKind::Ident(_)) {
            return Ok(kind);
        }
        let name = self.source.slice(next.span);
        let error = Diagnostic::error(format!("unknown unit `{name}`"), next.span)
            .with_note(lexer::UNITS_NOTE);
        Err(self.report(error))
    }

    /// Records an error in the sink and returns the proof that it was
    /// recorded.
    fn error(&mut self, message: impl Into<String>, span: Span) -> Reported {
        self.report(Diagnostic::error(message, span))
    }

    /// Records a diagnostic in the sink and returns the proof that it was
    /// recorded, the tree then being [`Syntax::Recovered`] at least. Not one
    /// at a [`TokenKind::Error`], which is the lexer's to report, nor one
    /// after a lexical error that runs to the end of the input, after which
    /// nothing was lexed; nor past [`MAX_ERRORS`] and one more.
    fn report(&mut self, diagnostic: Diagnostic) -> Reported {
        self.builder.set_syntax(Syntax::Recovered);
        let token = self.peek();
        let lexical = (token.kind == TokenKind::Error && diagnostic.span.start == token.span.start)
            || diagnostic.span.start >= self.quiet_from;
        if !lexical && self.diagnostics.len() <= MAX_ERRORS {
            self.diagnostics.push(diagnostic);
        }
        Reported
    }
}

/// Follows the token of `kind` in the brackets `open`, those opened since a
/// skip began, innermost last, as the lexer follows its own (see
/// [`crate::lexer`]): an opening bracket, or the start of an interpolated
/// string, is pushed; a `)` or `]` closes the bracket of its kind on top of
/// the innermost interpolation's, or nothing; a `}` closes the nearest `{`
/// of the innermost interpolation, with everything opened after it; and the
/// piece of a string after an interpolation closes everything opened in
/// it, its last piece the string too. Returns whether the token closes
/// something opened before the skip began: a closing bracket or piece with
/// nothing of its own open, or a `}` with no `{` open outside an
/// interpolation.
fn nest(nesting: &mut Nesting, kind: TokenKind) -> bool {
    let Nesting { open, strings } = nesting;
    // Where the innermost interpolation's brackets start.
    let context = strings.last().map_or(0, |&at| at + 1);
    match kind {
        TokenKind::LParen => open.push(Open::Paren),
        TokenKind::LBracket => open.push(Open::Square),
        TokenKind::LBrace => open.push(Open::Brace),
        TokenKind::StrStart(_) => {
            strings.push(open.len());
            open.push(Open::Str);
        }
        TokenKind::RParen | TokenKind::RBracket => {
            let bracket = if kind == TokenKind::RParen {
                Open::Paren
            } else {
                Open::Square
            };
            if open.len() > context && open.last() == Some(&bracket) {
                open.pop();
            } else {
                return open.is_empty();
            }
        }
        TokenKind::RBrace => {
            let brace = open
                .get(context..)
                .and_then(|inner| inner.iter().rposition(|&bracket| bracket == Open::Brace));
            match brace {
                Some(at) => open.truncate(context + at),
                None => return context == 0,
            }
        }
        TokenKind::StrMid(_) | TokenKind::StrEnd(_) => {
            let Some(at) = context.checked_sub(1) else {
                return true;
            };
            if matches!(kind, TokenKind::StrMid(_)) {
                open.truncate(context);
            } else {
                open.truncate(at);
                strings.pop();
            }
        }
        _ => {}
    }
    false
}

/// The brackets opened in the source a skip goes over, innermost last
/// ([`nest`]), with where each interpolated string among them is, so that
/// the innermost is found at once.
#[derive(Debug, Default)]
struct Nesting {
    /// The brackets open, innermost last.
    open: Vec<Open>,
    /// The index in `open` of each [`Open::Str`], innermost last.
    strings: Vec<usize>,
}

/// The binary operator a token denotes, if any. `Neg` is not one: a `-`
/// directly followed by its operand is always a negation.
fn binary_op(kind: TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Plus => Some(BinaryOp::Add),
        TokenKind::Minus => Some(BinaryOp::Sub),
        TokenKind::Star => Some(BinaryOp::Mul),
        TokenKind::Slash => Some(BinaryOp::Div),
        TokenKind::SlashSlash => Some(BinaryOp::FloorDiv),
        TokenKind::Percent => Some(BinaryOp::Rem),
        TokenKind::EqEq => Some(BinaryOp::Eq),
        TokenKind::BangEq => Some(BinaryOp::Ne),
        TokenKind::EqEqEq => Some(BinaryOp::Same),
        TokenKind::BangEqEq => Some(BinaryOp::NotSame),
        TokenKind::Lt => Some(BinaryOp::Lt),
        TokenKind::Le => Some(BinaryOp::Le),
        TokenKind::Gt => Some(BinaryOp::Gt),
        TokenKind::Ge => Some(BinaryOp::Ge),
        TokenKind::And => Some(BinaryOp::And),
        TokenKind::Or => Some(BinaryOp::Or),
        TokenKind::DotDot => Some(BinaryOp::Range),
        TokenKind::DotDotEq => Some(BinaryOp::RangeInclusive),
        TokenKind::Int(_)
        | TokenKind::Float(_)
        | TokenKind::Duration(_)
        | TokenKind::Size(_)
        | TokenKind::Str(_)
        | TokenKind::StrStart(_)
        | TokenKind::StrMid(_)
        | TokenKind::StrEnd(_)
        | TokenKind::Ident(_)
        | TokenKind::Attribute(_)
        | TokenKind::InnerAttribute(_)
        | TokenKind::AttributeArgs(_)
        | TokenKind::InnerAttributeArgs(_)
        | TokenKind::Let
        | TokenKind::Mut
        | TokenKind::If
        | TokenKind::Else
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Not
        | TokenKind::Fn
        | TokenKind::Def
        | TokenKind::Import
        | TokenKind::Lazy
        | TokenKind::Await
        | TokenKind::For
        | TokenKind::Do
        | TokenKind::Break
        | TokenKind::Continue
        | TokenKind::Return
        | TokenKind::Label(_)
        | TokenKind::In
        | TokenKind::Is
        | TokenKind::As
        | TokenKind::Pipe
        | TokenKind::Neg
        | TokenKind::Colon
        | TokenKind::ColonEq
        | TokenKind::Caret
        | TokenKind::ColonColon
        | TokenKind::Arrow
        | TokenKind::MemoArrow
        | TokenKind::Eq
        | TokenKind::LParen
        | TokenKind::RParen
        | TokenKind::LBrace
        | TokenKind::RBrace
        | TokenKind::LBracket
        | TokenKind::RBracket
        | TokenKind::Semi
        | TokenKind::Dot
        | TokenKind::DotDotDot
        | TokenKind::Comma
        | TokenKind::Newline
        | TokenKind::Error
        | TokenKind::Eof => None,
    }
}

/// `(left, right)` binding powers of a binary operator. Right is left + 1,
/// which makes every level left-associative, except comparison and the
/// ranges, which [`Parser::parse_expr`] makes non-associative. The ranges
/// sit between comparison, whose right power is their left one, and `+`
/// and `-`, whose left power is their right one.
fn binding_power(op: BinaryOp) -> (u8, u8) {
    match op {
        BinaryOp::Or => (1, 2),
        BinaryOp::And => (3, 4),
        BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Same
        | BinaryOp::NotSame
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge => (5, 6),
        BinaryOp::Range | BinaryOp::RangeInclusive => (6, 7),
        BinaryOp::Add | BinaryOp::Sub => (7, 8),
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::FloorDiv | BinaryOp::Rem => (9, 10),
    }
}

/// `text`, the source of a position or a path, on one line: each line break,
/// with the spaces around it, is one space, so that a path shown in a
/// diagnostic or a dump stays on its line.
fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where to point an error about finding `token`: its span, except for the
/// piece of a string after an interpolation, where it is the `}` ending the
/// interpolation, not the text after it.
fn error_span(token: Token) -> Span {
    match token.kind {
        TokenKind::StrMid(_) | TokenKind::StrEnd(_) => {
            Span::new(token.span.start, token.span.start + 1)
        }
        _ => token.span,
    }
}

pub(crate) mod attributes;
pub(crate) mod bodies;
pub(crate) mod context;
pub(crate) mod deftypes;
pub(crate) mod enums;
pub(crate) mod expressions;
pub(crate) mod failure;
pub(crate) mod functions;
pub(crate) mod imports;
pub(crate) mod lists;
pub(crate) mod loops;
pub(crate) mod paths;
pub(crate) mod recovery;
pub(crate) mod types;

#[cfg(test)]
mod tests;
