//! The parser: tokens to an [`Ast`].
//!
//! # Items and bodies
//!
//! A file, like the inside of a block `{ ... }`, is a *body*: a sequence of
//! items separated by newlines or `;` (`Parser::parse_body`). Runs of
//! separators, and so blank and comment-only lines, are skipped. An item is
//! `let PATH (: TYPE)? = EXPR`, `let mut NAME (: TYPE)? = EXPR`, `PATH :=
//! EXPR`, a field `PATH = EXPR`, `= EXPR`, a spread `...EXPR`, a named
//! function `fn NAME(...) ...`, or an expression. After an item the next
//! token must be a separator or the end of the body; anything else is
//! "expected end of item".
//!
//! `let mut` declares a mutable variable: an ordinary `let` item, marked as
//! mutable (see [`ItemKind::Let`]). Its path must be a single name: `let mut a.b = e` is "`let mut` needs a
//! plain name", at the path, since a dotted `let` updates a variable rather
//! than declaring one. `mut` is allowed nowhere else, and is "`mut` is only
//! allowed after `let`" at the `mut` wherever an item, an expression, a
//! name, a type or a function's parameter list would start, or an item
//! would end: before a field (`mut x = 1`) and a parameter (`fn(mut n:
//! Int)`) with a note saying what to write instead, and before a `for`'s
//! variable, after `fn` and everywhere else of those without one. (Where a
//! particular token is expected after a complete part of an item or an
//! expression, a separator, a closing bracket, `=`, `in` or a block, as in
//! `f(x mut)` or `let x mut = 1`, the usual "expected ..., found `mut`"
//! stands.)
//!
//! A spread item is recognised by `...` at the start of an item, and its
//! expression is parsed as a field's value is, at minimum binding power 0,
//! so `...conf : {b: Int}` spreads `conf : {b: Int}`. Whether the block it
//! is in may have a result is the checker's business, as for a field.
//!
//! A named function is recognised by `fn` followed by an identifier at the
//! start of an item, and is parsed as `let NAME = <the function expression
//! from its '(' on>`, both spanning `fn` through the body's `}`. `fn`
//! followed by anything else starts an ordinary expression item.
//!
//! # Paths
//!
//! A path is one or more names separated by `.`: `x`, `server.port`,
//! `"log level".format`. After `let` the parser reads the path directly:
//! an identifier, then `.` and a name as many times as they follow. A field
//! item or a rebind is recognised afterwards instead: the parser parses an
//! expression and, when the next token is `=` or `:=`, converts that
//! expression to a path and drops its nodes, which were the last allocated.
//! The expression must be a name, a plain string literal, or a chain of
//! field accesses on one of those, which is exactly how a path reads as an
//! expression; anything else before the `=` or `:=` is "expected end of
//! item". A variable's name must be an identifier, so
//! the first segment of the path of a `let` or a `:=` cannot be a literal;
//! later segments may be quoted.
//!
//! A dotted path nests what its item declares: later stages build a record
//! for each segment but the last, updating one record inside another, so a
//! path of `N` segments makes its item `2(N - 1)` levels taller than its
//! value (see [`MAX_DEPTH`]).
//!
//! A newline ends an item except where the item cannot be complete: after
//! a binary operator, after `:` (in an annotation or an ascription, or as a
//! function's return marker), `->>` and `->`, and after `=`, `:=`, `let`,
//! the `mut` of a `let mut` and a spread item's `...`. There
//! the parser skips newlines before going on. Directly inside parentheses,
//! brackets and interpolations the lexer emits no newlines at all, so an
//! expression, a parameter list, an argument list, a list literal or an
//! index there may span lines freely; a
//! block inside them is newline-sensitive again, so a multi-line block or
//! function body may be passed as an argument.
//!
//! # Names
//!
//! A field's name, where it is declared, accessed or written in a record
//! type, may be an identifier or a plain string literal: one without the
//! `r` prefix or interpolations, whose text the parser interns into the
//! [`Names`] like an identifier, so `"x"` and `x` are the same name. A raw
//! or interpolated literal there is an error. A variable's name, and so a
//! parameter's, must be an identifier.
//!
//! # Types
//!
//! A type is a name, such as `Int`, a name applied to a type argument, such
//! as `Future<Int>` or `List<Int>`, a record type, a function type, a type
//! in parentheses, a list type written `T[]`, or a union of types. A
//! record type is `{` then `}`, or `NAME: TYPE` entries separated by `,`,
//! with an optional trailing `,`, then `}`. The lexer does not know it is
//! in a type, so it emits the newlines inside the braces, and the parser
//! skips them all: a record type may span lines freely. Repeated field names
//! are left to the checker. A function type is `fn`, `(`, parameter types
//! separated by `,` with an optional trailing `,`, `)`, the marker `:`,
//! `->>` or `->`, and the return type; a parameter type is `NAME: TYPE`,
//! recognised
//! by an identifier followed by `:`, or just `TYPE`.
//!
//! An applied type is any identifier directly followed by `<`, a type, and
//! `>`, which must follow the type directly; there is no `>>` token, so
//! `Future<Future<Int>>` closes with two `>`. Which names take an argument
//! is the checker's business. A union is two types separated by `|`, which
//! binds looser than everything else in a type: after any type, while a `|`
//! follows, the parser reads another and makes the union of the two, so a
//! union is left-associative. A function type's return type is a whole
//! type, so it takes a union that follows it: `fn() -> Int | Future<Int>`
//! returns the union. Parentheses group a type, `(` TYPE `)`, so that
//! `(fn(): Int) | Future<fn(): Int>` is the union of a function type and a
//! future; they leave no node of their own, the type inside standing for
//! itself, and a missing `)` is "expected `)` to close the type". Newlines
//! are skipped after `<`, `|` and `(`, as after `:`. In expression position
//! `|` is not an operator, so it is "expected expression".
//!
//! `T[]` is a postfix on any type that is not a union (a *member*, as
//! `Parser::parse_type_member` parses it), repeatable, so `Int[][]` is a
//! list of lists, `{a: Int}[]` and `(fn(): Int)[]` are lists, and `Int |
//! Future<Int>[]` is the union of `Int` and a future of a list, which the
//! checker rejects. A `[` after a type is this postfix only if a `]`
//! follows it directly; in `fn(): Int[]` it belongs to the return type,
//! which is parsed first.
//!
//! # Functions and calls
//!
//! A function expression is `fn`, `(`, parameters `NAME: TYPE` separated by
//! `,` with an optional trailing `,`, `)`, the return marker, the return
//! type, and the body, a block whose `{` must follow the return type
//! directly, as an `if`'s block follows its condition. Every parameter is
//! annotated. The marker is one of three, each an [`Effect`] of the
//! [`FnDecl`] and of a function type alike: `:` for a pure function, `->>`
//! for an impure one that is memoised, run at most once per distinct
//! arguments, and `->` for an impure one run on every call.
//!
//! A call is a postfix operator: after an operand, `(` starts an argument
//! list, whose arguments are separated by `,` with an optional trailing
//! `,`, each an expression or `NAME = EXPR` (recognised by an identifier
//! followed by `=`). After the `)`, or instead of the list, a `{` on the
//! same line starts a trailing block: an ordinary block, passed as the
//! call's last argument. The parser accepts arguments in any order; binding
//! them to parameters is the checker's business.
//!
//! # Lists
//!
//! A list literal is `[`, elements separated by `,` with an optional
//! trailing `,`, and `]`; `[]` is the empty list. An element is an
//! expression, or `...` and an expression, a spread, parsed at minimum
//! binding power 0 as any element is, so `...a + b` spreads `a + b`. A
//! `...` anywhere else that an expression starts, but at the start of an
//! item (see [Items and bodies](#items-and-bodies)), is an error, "`...` is
//! only allowed in a list or at the start of an item".
//! Directly inside `[ ]` the lexer emits no newlines, as inside `( )`, so a
//! literal may span lines.
//!
//! An index, `EXPR[INDEX]`, is a postfix operator, binding as tightly as a
//! call and a field access and chaining with them: `f(x)[0].name[1]`. Like a
//! trailing block, it must start on the line its operand ends on: after a
//! newline the item has ended, so a `[` starting a line begins a list
//! literal, the next item. Inside brackets, where there are no newline
//! tokens, a `[` on the next line is still an index.
//!
//! # Futures
//!
//! `lazy EXPR` is a prefix expression that takes the longest expression it
//! can: its operand is parsed at minimum binding power 0, so `lazy a + b`
//! defers the sum and `lazy { ... }` a block. The operand becomes the body
//! of a parameterless function in the function arena, with no return type,
//! impure, whose marker is the `lazy` keyword. `await` is a prefix operator
//! binding exactly as tightly as `-`: `await x + 1` is `(await x) + 1`,
//! `await f()` awaits the call's result, and `await x.y` the field.
//!
//! # Trailing blocks
//!
//! A trailing block would make `if x { ... }` a call of `x`, so while the
//! parser is in the condition of an `if` (or `else if`), or the iterable or
//! the guard of a `for`, a `{` after an expression never starts one: the
//! parser keeps a flag, `Parser::no_trailing_block`, that `Parser::parse_if`
//! sets around its condition and `Parser::parse_for` around its iterable
//! and guard. The flag applies to the whole condition at every operator, so
//! `if a and b { }` is an `if` with condition `a and b`. It is lifted, and
//! restored afterwards, inside parentheses, argument lists, blocks and
//! interpolations, so `if (f { }) { }` calls `f`. A newline before a `{`
//! ends the item as always, so it never starts a trailing block either.
//!
//! # Newlines after an `if`
//!
//! The one place a newline does not end an item even though the expression
//! before it could be complete is after the block of an `if`: an `else` may
//! start the next line, so the parser looks past newlines for one, and puts
//! them back for the item separator if it finds none.
//!
//! # Expressions
//!
//! Expressions are parsed by a Pratt parser, also called precedence
//! climbing. Instead of one grammar function per precedence level, a single
//! function, [`Parser::parse_expr`], parses any expression whose operators
//! bind at least as tightly as a given *minimum binding power*. It first
//! parses a *prefix* expression (a literal, a name, a string, a
//! parenthesised expression, a block, a list literal, an `if`, a `for`, a
//! function expression, a `lazy`, or a prefix operator applied to an
//! operand), which
//! becomes the
//! left-hand side. Then it loops: if the
//! next token is a binary operator whose left binding power is at least the
//! minimum, it consumes the operator, parses the right-hand side by calling
//! itself with the operator's right binding power as the new minimum, and
//! folds the result into a new left-hand side. Otherwise the operator
//! belongs to an enclosing call, and the loop stops.
//!
//! Precedence and associativity both fall out of the binding-power pairs:
//!
//! | operator                    | left bp | right bp / operand min bp  |
//! |-----------------------------|---------|----------------------------|
//! | `or`                        | 1       | 2                          |
//! | `and`                       | 3       | 4                          |
//! | prefix `not`                | (none)  | operand parsed with min 5  |
//! | `==` `!=` `===` `!==`       | 5       | 6 (non-associative)        |
//! | `<` `<=` `>` `>=`           | 5       | 6 (non-associative)        |
//! | `..` `..=`                  | 6       | 7 (non-associative)        |
//! | `+` `-`                     | 7       | 8                          |
//! | `*` `/` `//` `%`            | 9       | 10                         |
//! | postfix `:` TYPE            | 11      | (none: parses a type)      |
//! | prefix `-` and `await`      | (none)  | operand parsed with min 13 |
//! | postfix `.` NAME            | 15      | (none: takes a name)       |
//! | postfix call `(`...`)` `{`  | 15      | (none: arguments at min 0) |
//! | postfix index `[`...`]`     | 15      | (none: position at min 0)  |
//!
//! A tighter operator has higher powers, so in `1 + 2 * 3` the recursive
//! call for the right-hand side of `+` (minimum 8) still accepts `*` (left
//! power 9). Left associativity comes from making the right power one higher
//! than the left: in `8 - 3 - 2`, the right-hand side of the first `-` is
//! parsed with minimum 8, which rejects the second `-` (left power 7), so the
//! first subtraction completes before the second begins.
//!
//! Comparison is non-associative rather than left-associative: its right
//! operand stops before another comparison as usual, and the parser then
//! reports "comparison operators cannot be chained" if one follows, since
//! `a < b < c` would otherwise mean `(a < b) < c`.
//!
//! The ranges, `..` and `..=`, bind tighter than comparison and looser than
//! `+` and `-`: their left power is comparison's right power, so the right
//! operand of `==` takes in a range, and their right power is the left
//! power of `+` and `-`, so a range's right operand takes in a sum and
//! `0..len(xs) - 1` is `0..(len(xs) - 1)`, while `a..b == c` is `(a..b) ==
//! c`. They are non-associative as comparison is: a range directly
//! followed by another, `a..b..c` or `a..b..=c`, is "`..` cannot be
//! chained" (or "`..=` cannot be chained"), at the second operator.
//! Negation binds tighter, so `-1..1` is `(-1)..1`.
//!
//! `not` is a prefix operator whose operand is parsed with minimum 5, the
//! left power of comparison, so it takes in a comparison but not `and` or
//! `or`: `not a == b` is `not (a == b)`, and `not a and b` is
//! `(not a) and b`.
//!
//! Type ascription `EXPR : TYPE` is a postfix operator in the same loop: when
//! the next token is `:` and its left power 11 is at least the minimum, the
//! parser consumes it, parses a type, and wraps the left-hand side. It binds
//! tighter than every binary operator, so `1 + 2 : Float` is
//! `1 + (2 : Float)`, and looser than negation, whose operand is parsed with
//! minimum 13, so `-5 : Float` is `(-5) : Float`. It chains left to right.
//!
//! Field access `EXPR.NAME` is a postfix operator too, binding tighter than
//! everything: its left power 15 is above every minimum, so a `.` always
//! applies to the operand just parsed. `-a.b` is `-(a.b)`, `a.b : Int` is
//! `(a.b) : Int`, and `a.b.c` is `(a.b).c`. The parser does not skip
//! newlines around `.`, so a `.` starting a line is not a continuation.
//!
//! A call binds exactly as tightly as field access, in the same loop, so
//! `a.b(1).c` is `((a.b)(1)).c`, `-f(1)` is `-(f(1))` and `f(1)(2)` calls
//! the result of `f(1)`. Each argument is parsed at minimum 0. So does an
//! index, whose position is parsed at minimum 0 too.
//!
//! Negation is its own token, [`TokenKind::Neg`], distinct from subtraction's
//! [`TokenKind::Minus`]; the lexer tells them apart by spacing. `Neg` is only
//! accepted in prefix position and `Minus` only as a binary operator.
//!
//! An `if` is a prefix expression: `if`, a condition parsed at minimum 0, a
//! block, and optionally `else` followed by a block or another `if`. An `if`
//! without `else` gets an empty block as its else branch, allocated by the
//! parser with an empty span at the end of the `if`'s block.
//!
//! A `for` is a prefix expression too, usable wherever an `if` is: `for`,
//! the variable's name, an identifier, `in`, the iterable, parsed at
//! minimum 0 as an `if`'s condition is, optionally `if` and a guard, parsed
//! the same way, and the body, a block that must follow on the same line.
//! No operator starts with `if`, so an `if` after the complete iterable
//! always begins the guard; an iterable that is itself an `if` expression
//! starts with `if` and is parsed whole first. Its details are kept in the
//! tree's `for` arena (see [`crate::ast::ForDecl`]).
//!
//! An interpolated string arrives from the lexer as a
//! [`TokenKind::StrStart`], each expression's tokens, a
//! [`TokenKind::StrMid`] between each two expressions, and a
//! [`TokenKind::StrEnd`]. The parser parses each expression at minimum 0
//! and collects the parts on a stack shared by all the interpolations being
//! parsed, as it does for items, so nested interpolations need no
//! allocation of their own.
//!
//! # Errors and depth
//!
//! Errors are values. Parsing functions return `Result`, and a failure is
//! recorded in the parser's diagnostic sink before the `Err` propagates. The
//! parser stops at the first error for now; recovery can be added later by
//! catching an `Err` at a synchronisation point (such as the next item
//! separator) and continuing, with the sink then holding several
//! diagnostics.
//!
//! The parser enforces [`MAX_DEPTH`] in two places, because nesting arises in
//! two ways. Parentheses, prefix operators (`await` included), blocks, `if`s,
//! `for`s, interpolations, function expressions, `lazy`s, argument lists, list
//! literals, indexes, and
//! record, function, applied and parenthesised types nest by recursion, so
//! [`Parser::parse_expr`], the body of every nested block, every `else if`
//! and every record, function, applied and parenthesised type count their
//! own depth and stop before the recursion itself could overflow; a
//! function expression, a `lazy`, a `for`, an argument list, a list literal
//! and an index recurse only through those.
//! Left-associative chains such as `1 + 1 + 1`, `a.b.c`, `xs[0][0]`, a
//! union of many types or `Int[][]` nest without recursion, since the loop
//! folds each operator into the
//! left-hand side, so the parser also tracks the height of every node it
//! allocates, a block being one above the tallest of its items' expressions
//! and types, an `if` or an interpolation one above its tallest child, a
//! `for` one above the tallest of its iterable, its guard and its body, a
//! list literal one above its tallest element (a spread's being its
//! expression's), an index one above the taller of its list and its
//! position, a
//! field access one above its record, an ascription one above the taller of
//! its expression and its type (a record type being one above its tallest
//! field type, a function type one above the tallest of its parameter and
//! return types, an applied type one above its argument, a `T[]` one above
//! `T`, a parenthesised
//! type one above the type inside, though it leaves no node of its own, and
//! a union one above the taller of its two sides), a function expression
//! one above the tallest of its parameter types, its return type and its
//! body, a `lazy`
//! one above its operand, and a call one above the tallest of its callee and
//! arguments, and rejects a node that would exceed the limit. An item's height is its
//! value's, plus two levels for each segment of its path after the first,
//! or its annotation's if that is taller; an item taller than the limit is
//! rejected too, at the item. Together these guarantee that no tree handed
//! to a later stage is deeper than `MAX_DEPTH`.

use crate::MAX_DEPTH;
use crate::ast::{
    Arg, ArgKind, Ast, AstBuilder, BinaryOp, Effect, Element, Expr, ExprId, ExprKind, FieldTy,
    FnDecl, ForDecl, Item, ItemId, ItemKind, ItemRange, Param, ParamTy, PathRange, PathSegment,
    StrPart, TypeExpr, TypeExprId, TypeExprKind, UnaryOp, path_span,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::{Name, Names};
use crate::source::{SourceFile, Span};
use crate::strings::{StrId, Strings};

/// Minimum binding power of the operand of prefix `not`: the left power of
/// comparison, so the operand takes in a comparison but not `and` or `or`.
const NOT_BP: u8 = 5;

/// Minimum binding power of the operand of prefix `-`: tighter than every
/// binary operator and than ascription.
const PREFIX_BP: u8 = 13;

/// Left binding power of postfix `:` (type ascription): tighter than every
/// binary operator, looser than prefix `-`.
const ASCRIBE_BP: u8 = 11;

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

/// The error for a function, or a function type, without a return marker.
const MISSING_RETURN_TYPE: &str =
    "expected `:`, `->` or `->>` and a return type after the parameters";

/// The error for a `...` that starts neither an element of a list literal
/// nor an item, where an expression starts. (A spread item is recognised
/// by `Parser::parse_item` before any expression is parsed, so it never
/// reaches this.)
const MISPLACED_SPREAD: &str = "`...` is only allowed in a list or at the start of an item";

/// The error for a `mut` anywhere but directly after `let`.
const MISPLACED_MUT: &str = "`mut` is only allowed after `let`";

/// The error for a `let mut` whose path is dotted.
const DOTTED_MUT: &str = "`let mut` needs a plain name";

/// The note on [`DOTTED_MUT`].
const DOTTED_MUT_NOTE: &str = "a dotted `let` updates a variable; declare the variable with `let \
     mut a = ...` and update it with `a.b := ...`";

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
/// Returns the first syntax error.
pub fn parse(
    tokens: &[Token],
    mut names: Names,
    strings: Strings,
    source: &SourceFile,
) -> Result<Ast, Diagnostic> {
    let (builder, root) = parse_parts(tokens, source, &mut names, &strings)?;
    Ok(builder.finish(root, names, strings))
}

/// Like [`parse`], returning the unfinished arenas and the file's body
/// instead of an [`Ast`], and borrowing the tables rather than taking them,
/// so that a caller lending its [`Names`] only hands them over once parsing
/// has succeeded. The text of every quoted field name is interned into
/// `names`; on error, names met before the error remain interned.
pub(crate) fn parse_parts(
    tokens: &[Token],
    source: &SourceFile,
    names: &mut Names,
    strings: &Strings,
) -> Result<(AstBuilder, ItemRange), Diagnostic> {
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
        // `fn`, `(`, `)`, a marker and a type besides its body.)
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
        no_trailing_block: false,
        diagnostics: Vec::new(),
    };
    match parser.parse_body(None) {
        Ok((root, _)) => Ok((parser.builder, root)),
        Err(Reported) => Err(parser.diagnostics.into_iter().next().unwrap_or_else(|| {
            Diagnostic::error(
                "internal error: parser failed without reporting a diagnostic",
                Span::new(0, 0),
            )
        })),
    }
}

/// Proof that a diagnostic has been pushed to the parser's sink. Only
/// [`Parser::report`] creates one.
#[derive(Debug)]
struct Reported;

type ParseResult<T> = Result<T, Reported>;

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    source: &'a SourceFile,
    /// Where quoted field names are interned.
    names: &'a mut Names,
    /// The text of the string literals, for quoted field names.
    strings: &'a Strings,
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
    /// The segments of the path being read, moved to the builder's path
    /// pool once it is complete. Reading a path never starts another, so
    /// one buffer serves them all.
    path: Vec<PathSegment>,
    /// Whether a `{` after an expression is kept from starting a trailing
    /// block: set while parsing the condition of an `if`, so that its block
    /// is not taken for one, and lifted inside parentheses, argument lists,
    /// blocks and interpolations.
    no_trailing_block: bool,
    /// Errors reported so far. Only one is ever pushed until error recovery
    /// exists.
    diagnostics: Vec<Diagnostic>,
}

impl Parser<'_> {
    /// Parses a body up to its end: the `}` matching `open` (not consumed),
    /// or the end of input for the file's body, when `open` is `None`.
    /// Returns the body and the height of its tallest item expression (0 if
    /// it has no items).
    fn parse_body(&mut self, open: Option<Span>) -> ParseResult<(ItemRange, u32)> {
        let base = self.item_stack.len();
        let mut height = 0;
        loop {
            self.skip_separators();
            let token = self.peek();
            match (token.kind, open) {
                (TokenKind::Eof, None) | (TokenKind::RBrace, Some(_)) => break,
                (TokenKind::Eof, Some(open)) => {
                    let found = self.describe(token);
                    return Err(self.report(
                        Diagnostic::error(format!("expected `}}`, found {found}"), token.span)
                            .with_context(open, "unclosed `{`"),
                    ));
                }
                (TokenKind::RBrace, None) => {
                    return Err(self.error("unexpected `}`", token.span));
                }
                _ => {}
            }
            let (item, item_height) = self.parse_item()?;
            self.item_stack.push(item);
            height = height.max(item_height);
            self.expect_end_of_item()?;
        }
        let range = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok((range, height))
    }

    /// Parses one item. Returns it and its height: its expression's, plus
    /// two levels for each segment of its path after the first, or its
    /// annotation's if that is taller.
    fn parse_item(&mut self) -> ParseResult<(ItemId, u32)> {
        let token = self.peek();
        let name_token = self.peek_at(1);
        if token.kind == TokenKind::Fn
            && let TokenKind::Ident(name) = name_token.kind
        {
            return self.parse_named_fn(token, (name, name_token.span));
        }
        if token.kind == TokenKind::Mut {
            let note = self.starts_field(1).then_some(MUT_FIELD_NOTE);
            return Err(self.misplaced_mut(token, note));
        }
        let mut ty_height = 0;
        // The number of segments of the item's path, if it has one.
        let mut segments = 1;
        let (kind, value) = match token.kind {
            TokenKind::Let => {
                self.advance();
                self.skip_newlines();
                let mutable = self.peek().kind == TokenKind::Mut;
                let keyword = if mutable {
                    let keyword = self.peek();
                    self.advance();
                    self.skip_newlines();
                    keyword
                } else {
                    token
                };
                let (path, len) = self.parse_let_path(keyword)?;
                if mutable && len > 1 {
                    return Err(self.report(
                        Diagnostic::error(DOTTED_MUT, path_span(&self.path))
                            .with_note(DOTTED_MUT_NOTE),
                    ));
                }
                segments = len;
                let ty = if self.peek().kind == TokenKind::Colon {
                    self.advance();
                    self.skip_newlines();
                    let (ty, height) = self.parse_type()?;
                    ty_height = height;
                    Some(ty)
                } else {
                    None
                };
                let eq = self.peek();
                if eq.kind != TokenKind::Eq {
                    return Err(self.error("expected `=` after variable name", eq.span));
                }
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                let kind = ItemKind::Let {
                    path,
                    ty,
                    value,
                    mutable,
                };
                (kind, value)
            }
            TokenKind::Eq => {
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                (ItemKind::Result(value), value)
            }
            TokenKind::DotDotDot => {
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                (ItemKind::Spread(value), value)
            }
            _ => {
                let start = self.pos;
                let first_node = self.builder.expr_count();
                let expr = self.parse_expr(0)?;
                let operator = self.peek();
                let rebind = match operator.kind {
                    TokenKind::Eq => false,
                    TokenKind::ColonEq => true,
                    _ => return self.finish_item(token, (ItemKind::Expr(expr), expr), 0, 1),
                };
                let (path, len) = self.expr_to_path(start, expr, operator, rebind)?;
                segments = len;
                // The path's nodes were the last allocated, and it replaces
                // them.
                self.builder.truncate_exprs(first_node);
                self.heights.truncate(first_node);
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                let kind = if rebind {
                    ItemKind::Rebind { path, value }
                } else {
                    ItemKind::Field { path, value }
                };
                (kind, value)
            }
        };
        self.finish_item(token, (kind, value), ty_height, segments)
    }

    /// Whether the tokens from `ahead` places after the current one begin a
    /// field item: a path, a name or a plain string literal and then `.`
    /// and a name as many times as they follow, and then `=`.
    fn starts_field(&self, ahead: usize) -> bool {
        let is_name = |token: Token| matches!(token.kind, TokenKind::Ident(_) | TokenKind::Str(_));
        let mut at = ahead;
        if !is_name(self.peek_at(at)) {
            return false;
        }
        at += 1;
        while self.peek_at(at).kind == TokenKind::Dot && is_name(self.peek_at(at + 1)) {
            at += 2;
        }
        self.peek_at(at).kind == TokenKind::Eq
    }

    /// Reports `token`, a `mut` where it is not allowed, with `note` if one
    /// says what to write instead.
    fn misplaced_mut(&mut self, token: Token, note: Option<&str>) -> Reported {
        let error = Diagnostic::error(MISPLACED_MUT, token.span);
        self.report(match note {
            Some(note) => error.with_note(note),
            None => error,
        })
    }

    /// Parses a named function item, `fn NAME(...) ...`, starting at its
    /// `fn` keyword, `keyword`, followed by `name` and its span: a `let` of
    /// the name whose value is the function expression from `(` on, both
    /// spanning `fn` through the body's `}`.
    fn parse_named_fn(
        &mut self,
        keyword: Token,
        (name, name_span): (Name, Span),
    ) -> ParseResult<(ItemId, u32)> {
        self.advance();
        self.advance();
        let value = self.parse_fn(keyword)?;
        self.path.clear();
        self.path.push(PathSegment {
            name,
            span: name_span,
        });
        let (path, segments) = self.finish_path();
        let kind = ItemKind::Let {
            path,
            ty: None,
            value,
            mutable: false,
        };
        self.finish_item(keyword, (kind, value), 0, segments)
    }

    /// Allocates the item of `kind`, starting at `token` and ending with its
    /// expression `value`, whose annotation, if any, is `ty_height` tall and
    /// whose path has `segments` segments. Returns it and its height,
    /// failing if that exceeds [`MAX_DEPTH`].
    fn finish_item(
        &mut self,
        token: Token,
        (kind, value): (ItemKind, ExprId),
        ty_height: u32,
        segments: u32,
    ) -> ParseResult<(ItemId, u32)> {
        let span = Span::join(token.span, self.span_of(value));
        let path_height = segments.saturating_sub(1).saturating_mul(2);
        let height = self
            .height_of(value)
            .saturating_add(path_height)
            .max(ty_height);
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let item = self.builder.alloc_item(Item { kind, span });
        Ok((item, height))
    }

    /// Parses the path after `let`, or after the `mut` of a `let mut`,
    /// `keyword`, starting at its first segment: an identifier, then `.`
    /// and a name, plain or quoted, as many times as they follow. Returns
    /// it and its number of segments.
    fn parse_let_path(&mut self, keyword: Token) -> ParseResult<(PathRange, u32)> {
        let name_token = self.peek();
        let name = match name_token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) => {
                return Err(self.error(PLAIN_VARIABLE_NAME, name_token.span));
            }
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_VARIABLE_NAME, span));
            }
            TokenKind::Mut => return Err(self.misplaced_mut(name_token, None)),
            _ => {
                let after = self.describe(keyword);
                return Err(self.error(format!("expected a name after {after}"), name_token.span));
            }
        };
        self.advance();
        self.path.clear();
        self.path.push(PathSegment {
            name,
            span: name_token.span,
        });
        while self.peek().kind == TokenKind::Dot {
            self.advance();
            let segment = self.peek();
            let Some(name) = self.field_name()? else {
                return Err(self.error("expected a field name after `.`", error_span(segment)));
            };
            self.path.push(PathSegment {
                name,
                span: segment.span,
            });
        }
        Ok(self.finish_path())
    }

    /// Converts `expr`, parsed from the tokens starting at `start` and
    /// followed by `operator`, the `=` of a field item or the `:=` of a
    /// rebind as `rebind` says, to the path it spells: a name, a plain
    /// string literal, or a chain of field accesses on one of those. Returns
    /// it and its number of segments.
    ///
    /// Anything else is "expected end of item" at the operator. A raw or
    /// interpolated literal is not a field name, and no literal is a
    /// variable's name.
    fn expr_to_path(
        &mut self,
        start: usize,
        expr: ExprId,
        operator: Token,
        rebind: bool,
    ) -> ParseResult<(PathRange, u32)> {
        // Walk down to the base, collecting the accessed names from the last
        // segment to the second.
        self.path.clear();
        let mut node = expr;
        let base = loop {
            let inner = *self.builder.get(node);
            match inner.kind {
                ExprKind::Field { expr, name } => {
                    self.path.push(PathSegment {
                        name,
                        span: inner.span,
                    });
                    node = expr;
                }
                ExprKind::Name(name) => break name,
                ExprKind::Str(_) | ExprKind::Interp(_) if rebind => {
                    return Err(self.error(PLAIN_VARIABLE_NAME, inner.span));
                }
                ExprKind::Str(text) => {
                    let literal = self.token_at(start);
                    break self.quoted_name(literal, text)?;
                }
                ExprKind::Interp(_) => return Err(self.error(PLAIN_FIELD_NAME, inner.span)),
                _ => {
                    let found = self.describe(operator);
                    return Err(self.error(
                        format!("expected end of item, found {found}"),
                        operator.span,
                    ));
                }
            }
        };
        self.path.push(PathSegment {
            name: base,
            span: self.token_at(start).span,
        });
        self.path.reverse();
        // A path's tokens are exactly its base and then `.` and a name for
        // each access, since newlines are not skipped around `.`: the name
        // of segment `i` is token `start + 2i`.
        for (index, segment) in self.path.iter_mut().enumerate().skip(1) {
            if let Some(token) = self.tokens.get(start + 2 * index) {
                segment.span = token.span;
            }
        }
        Ok(self.finish_path())
    }

    /// Moves the path being read to the builder's path pool, returning its
    /// range and its number of segments.
    fn finish_path(&mut self) -> (PathRange, u32) {
        let len = u32::try_from(self.path.len()).unwrap_or(u32::MAX);
        (self.builder.push_path(&self.path), len)
    }

    /// The name a plain string literal `token`, of text `text`, gives a
    /// field: its text, interned. A raw literal is an error.
    fn quoted_name(&mut self, token: Token, text: StrId) -> ParseResult<Name> {
        if self.source.slice(token.span).starts_with('r') {
            return Err(self.error(PLAIN_FIELD_NAME, token.span));
        }
        Ok(self.names.intern(self.strings.get(text)))
    }

    /// Parses the name of a field after `.` or in a record type, the
    /// current token: an identifier or a plain string literal. Returns
    /// `None`, having consumed nothing, if the token is neither; a raw or
    /// interpolated literal is an error, and so is a `mut`.
    fn field_name(&mut self) -> ParseResult<Option<Name>> {
        let token = self.peek();
        let name = match token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(text) => self.quoted_name(token, text)?,
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_FIELD_NAME, span));
            }
            TokenKind::Mut => return Err(self.misplaced_mut(token, None)),
            _ => return Ok(None),
        };
        self.advance();
        Ok(Some(name))
    }

    /// Checks that the item just parsed is followed by a separator or the
    /// end of its body.
    fn expect_end_of_item(&mut self) -> ParseResult<()> {
        let token = self.peek();
        match token.kind {
            TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof => Ok(()),
            // `a -b`: the user most likely meant a subtraction. The lexer
            // only makes a `Neg` when an operand follows directly, so show
            // the `-` together with that operand's first token.
            TokenKind::Neg => {
                let span = Span::join(token.span, self.peek_at(1).span);
                let found = self.source.slice(span);
                Err(self.report(
                    Diagnostic::error(format!("expected end of item, found `{found}`"), span)
                        .with_note("write `a - b` with spaces for subtraction"),
                ))
            }
            TokenKind::Mut => Err(self.misplaced_mut(token, None)),
            _ => {
                let found = self.describe(token);
                Err(self.error(format!("expected end of item, found {found}"), token.span))
            }
        }
    }

    /// Parses a type expression: a name, an applied type, a record type, a
    /// function type or a parenthesised type, or a union of such types.
    /// Returns it and its height: 1 for a name, one more than its argument
    /// for an applied type, one more than its tallest field's type for a
    /// record type, one more than the tallest of its parameter and return
    /// types for a function type, one more than the type inside for a
    /// parenthesised type, and one more than the taller of its two sides
    /// for a union.
    fn parse_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let (mut ty, mut height) = self.parse_type_member()?;
        // A union is left-associative and built by this loop, not by
        // recursion, so its height is checked here.
        while self.peek().kind == TokenKind::Pipe {
            let bar = self.peek().span;
            self.advance();
            self.skip_newlines();
            let (rhs, rhs_height) = self.parse_type_member()?;
            let span = Span::join(self.type_span(ty), self.type_span(rhs));
            height = height.max(rhs_height).saturating_add(1);
            if height as usize > MAX_DEPTH {
                return Err(self.too_deep(span));
            }
            ty = self.builder.alloc_type(TypeExpr {
                kind: TypeExprKind::Union { lhs: ty, rhs, bar },
                span,
            });
        }
        Ok((ty, height))
    }

    /// Parses a type that is not a union, unless in parentheses: a name, an
    /// applied type, a record type, a function type or a parenthesised
    /// type, followed by any number of `[]`, each making a list type of the
    /// type before it. Returns it and its height, as [`Parser::parse_type`]
    /// does, one more for each `[]`.
    fn parse_type_member(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let (mut ty, mut height) = self.parse_type_atom()?;
        // Repeated `[]`s are built by this loop, not by recursion, so their
        // height is checked here.
        while self.peek().kind == TokenKind::LBracket && self.peek_at(1).kind == TokenKind::RBracket
        {
            let close = self.peek_at(1);
            self.advance();
            self.advance();
            let span = Span::join(self.type_span(ty), close.span);
            height = height.saturating_add(1);
            if height as usize > MAX_DEPTH {
                return Err(self.too_deep(span));
            }
            ty = self.builder.alloc_type(TypeExpr {
                kind: TypeExprKind::List(ty),
                span,
            });
        }
        Ok((ty, height))
    }

    /// Parses a type that is neither a union nor a list type written `T[]`,
    /// unless in parentheses: a name, an applied type, a record type, a
    /// function type or a parenthesised type. Returns it and its height, as
    /// [`Parser::parse_type`] does.
    fn parse_type_atom(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let token = self.peek();
        match token.kind {
            TokenKind::Ident(name) if self.peek_at(1).kind == TokenKind::Lt => {
                // Applied types nest by recursion, so they count their
                // depth.
                self.enter()?;
                let applied = self.parse_applied_type(name, token.span);
                self.depth -= 1;
                applied
            }
            TokenKind::Ident(name) => {
                self.advance();
                let id = self.builder.alloc_type(TypeExpr {
                    kind: TypeExprKind::Named(name),
                    span: token.span,
                });
                Ok((id, LEAF_HEIGHT))
            }
            // Record and function types nest by recursion, so they count
            // their depth.
            TokenKind::LBrace => {
                self.enter()?;
                let record = self.parse_record_type();
                self.depth -= 1;
                record
            }
            TokenKind::Fn => {
                self.enter()?;
                let function = self.parse_fn_type();
                self.depth -= 1;
                function
            }
            // Parenthesised types nest by recursion too.
            TokenKind::LParen => {
                self.enter()?;
                let inner = self.parse_paren_type();
                self.depth -= 1;
                inner
            }
            TokenKind::Mut => Err(self.misplaced_mut(token, None)),
            _ => Err(self.error("expected a type", error_span(token))),
        }
    }

    /// Parses a parenthesised type, starting at its `(`: the `(`, newlines,
    /// the type, and the `)`. Returns the type inside itself, with a height
    /// one more than its own, since the parentheses leave no node of their
    /// own.
    fn parse_paren_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let open = self.peek();
        self.advance();
        self.skip_newlines();
        let (inner, inner_height) = self.parse_type()?;
        let close = self.peek();
        if close.kind != TokenKind::RParen {
            return Err(self.report(
                Diagnostic::error("expected `)` to close the type", error_span(close))
                    .with_context(open.span, "unclosed `(`"),
            ));
        }
        self.advance();
        let height = inner_height.saturating_add(1);
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(Span::join(open.span, close.span)));
        }
        Ok((inner, height))
    }

    /// Parses the applied type whose name, `name` at `name_span`, is the
    /// current token and is followed by `<`: the `<`, newlines, the type
    /// argument, and the `>`, which must follow the argument directly.
    fn parse_applied_type(
        &mut self,
        name: Name,
        name_span: Span,
    ) -> ParseResult<(TypeExprId, u32)> {
        self.advance();
        let open = self.peek();
        self.advance();
        self.skip_newlines();
        let (arg, arg_height) = self.parse_type()?;
        let close = self.peek();
        if close.kind != TokenKind::Gt {
            return Err(self.report(
                Diagnostic::error("expected `>` to close the type argument", error_span(close))
                    .with_context(open.span, "unclosed `<`"),
            ));
        }
        self.advance();
        let span = Span::join(name_span, close.span);
        let height = arg_height.saturating_add(1);
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Applied {
                name,
                span: name_span,
                arg,
            },
            span,
        });
        Ok((id, height))
    }

    /// Parses a function type, starting at its `fn`: `(`, parameter types
    /// separated by `,` with an optional trailing `,`, each `NAME: TYPE` or
    /// `TYPE`, `)`, the marker `:` or `->`, and the return type.
    fn parse_fn_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let keyword = self.peek();
        self.advance();
        let open = self.expect_fn_paren()?;
        let base = self.param_ty_stack.len();
        let mut tallest = 0;
        loop {
            let token = self.peek();
            if token.kind == TokenKind::RParen {
                self.advance();
                break;
            }
            let name = match (token.kind, self.peek_at(1).kind) {
                (TokenKind::Ident(name), TokenKind::Colon) => {
                    self.advance();
                    self.advance();
                    Some(name)
                }
                _ => None,
            };
            let (ty, height) = self.parse_type()?;
            tallest = tallest.max(height);
            self.param_ty_stack.push(ParamTy { name, ty });
            if self.list_separator(open)? {
                break;
            }
        }
        let params = self.builder.push_param_tys(&self.param_ty_stack[base..]);
        self.param_ty_stack.truncate(base);
        let (effect, _) = self.return_marker()?;
        let (ret, ret_height) = self.parse_type()?;
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Fn {
                params,
                ret,
                effect,
            },
            span: Span::join(keyword.span, self.type_span(ret)),
        });
        Ok((id, tallest.max(ret_height) + 1))
    }

    /// Consumes the `(` after `fn`, returning it, or reports what is there
    /// instead.
    fn expect_fn_paren(&mut self) -> ParseResult<Token> {
        let open = self.peek();
        if open.kind == TokenKind::Mut {
            return Err(self.misplaced_mut(open, None));
        }
        if open.kind != TokenKind::LParen {
            let found = self.describe(open);
            return Err(self.error(
                format!("expected `(` after `fn`, found {found}"),
                error_span(open),
            ));
        }
        self.advance();
        Ok(open)
    }

    /// After an entry of a parenthesised list opened by `open`, consumes a
    /// `,` or the closing `)`, returning whether it was the `)`; anything
    /// else is an error.
    fn list_separator(&mut self, open: Token) -> ParseResult<bool> {
        let separator = self.peek();
        match separator.kind {
            TokenKind::Comma => {
                self.advance();
                Ok(false)
            }
            TokenKind::RParen => {
                self.advance();
                Ok(true)
            }
            _ => {
                let found = self.describe(separator);
                Err(self.report(
                    Diagnostic::error(
                        format!("expected `,` or `)`, found {found}"),
                        error_span(separator),
                    )
                    .with_context(open.span, "unclosed `(`"),
                ))
            }
        }
    }

    /// Consumes the return marker of a function or function type, `:` for a
    /// pure one, `->>` for a memoised one and `->` for an impure one, and
    /// the newlines after it. Returns the effect it declares, and the
    /// marker's span.
    fn return_marker(&mut self) -> ParseResult<(Effect, Span)> {
        let marker = self.peek();
        let effect = match marker.kind {
            TokenKind::Colon => Effect::Pure,
            TokenKind::MemoArrow => Effect::Memo,
            TokenKind::Arrow => Effect::Impure,
            _ => {
                return Err(self.report(
                    Diagnostic::error(MISSING_RETURN_TYPE, error_span(marker))
                        .with_note(RETURN_TYPE_NOTE),
                ));
            }
        };
        self.advance();
        self.skip_newlines();
        Ok((effect, marker.span))
    }

    /// Parses a record type, starting at its `{`. Newlines anywhere inside
    /// are skipped.
    fn parse_record_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let open = self.peek();
        self.advance();
        let base = self.field_ty_stack.len();
        let mut tallest = 0;
        let close = loop {
            self.skip_newlines();
            let token = self.peek();
            if token.kind == TokenKind::RBrace {
                self.advance();
                break token;
            }
            let Some(name) = self.field_name()? else {
                return Err(self.error("expected a field name in record type", error_span(token)));
            };
            self.skip_newlines();
            let colon = self.peek();
            if colon.kind != TokenKind::Colon {
                return Err(self.error("expected `:` after field name", error_span(colon)));
            }
            self.advance();
            self.skip_newlines();
            let (ty, height) = self.parse_type()?;
            tallest = tallest.max(height);
            self.field_ty_stack.push(FieldTy {
                name,
                span: token.span,
                ty,
            });
            self.skip_newlines();
            let separator = self.peek();
            match separator.kind {
                // A `}` after it ends the type at the top of the loop.
                TokenKind::Comma => self.advance(),
                TokenKind::RBrace => {
                    self.advance();
                    break separator;
                }
                _ => return Err(self.error("expected `,` or `}`", error_span(separator))),
            }
        };
        let fields = self.builder.push_field_tys(&self.field_ty_stack[base..]);
        self.field_ty_stack.truncate(base);
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Record(fields),
            span: Span::join(open.span, close.span),
        });
        Ok((id, tallest + 1))
    }

    /// Parses an expression whose binary operators all have a left binding
    /// power of at least `min_bp`.
    fn parse_expr(&mut self, min_bp: u8) -> ParseResult<ExprId> {
        self.enter()?;
        let result = self.parse_expr_inner(min_bp);
        self.depth -= 1;
        result
    }

    /// Records one more level of recursion, failing if it would exceed
    /// [`MAX_DEPTH`]. Every successful call is paired with a decrement by
    /// its caller.
    fn enter(&mut self) -> ParseResult<()> {
        if self.depth >= MAX_DEPTH {
            let span = self.peek().span;
            return Err(self.too_deep(span));
        }
        self.depth += 1;
        Ok(())
    }

    fn parse_expr_inner(&mut self, min_bp: u8) -> ParseResult<ExprId> {
        let mut lhs = self.parse_prefix()?;
        loop {
            let token = self.peek();
            let starts_call = match token.kind {
                TokenKind::LParen => true,
                TokenKind::LBrace => !self.no_trailing_block,
                _ => false,
            };
            if starts_call {
                if FIELD_BP < min_bp {
                    break;
                }
                lhs = self.parse_call(lhs)?;
                continue;
            }
            if token.kind == TokenKind::LBracket {
                if FIELD_BP < min_bp {
                    break;
                }
                lhs = self.parse_index(lhs)?;
                continue;
            }
            if token.kind == TokenKind::Dot {
                if FIELD_BP < min_bp {
                    break;
                }
                self.advance();
                let name_token = self.peek();
                let Some(name) = self.field_name()? else {
                    return Err(
                        self.error("expected a field name after `.`", error_span(name_token))
                    );
                };
                let span = Span::join(self.span_of(lhs), name_token.span);
                let height = self.height_of(lhs) + 1;
                lhs = self.alloc(ExprKind::Field { expr: lhs, name }, span, height)?;
                continue;
            }
            if token.kind == TokenKind::Colon {
                if ASCRIBE_BP < min_bp {
                    break;
                }
                self.advance();
                self.skip_newlines();
                let (ty, ty_height) = self.parse_type()?;
                let span = Span::join(self.span_of(lhs), self.type_span(ty));
                let height = self.height_of(lhs).max(ty_height) + 1;
                lhs = self.alloc(ExprKind::Ascribe { expr: lhs, ty }, span, height)?;
                continue;
            }
            let Some(op) = binary_op(token.kind) else {
                break;
            };
            let (left_bp, right_bp) = binding_power(op);
            if left_bp < min_bp {
                break;
            }
            let op_start = token.span.start;
            self.advance();
            self.skip_newlines();
            let rhs = self.parse_expr(right_bp)?;
            let span = Span::join(self.span_of(lhs), self.span_of(rhs));
            let height = self.height_of(lhs).max(self.height_of(rhs)) + 1;
            lhs = self.alloc(
                ExprKind::Binary {
                    op,
                    op_start,
                    lhs,
                    rhs,
                },
                span,
                height,
            )?;
            // The right operand stopped before any further comparison, since
            // its minimum is above comparison's left power; one following
            // directly would chain.
            if op.is_comparison() {
                let next = self.peek();
                if binary_op(next.kind).is_some_and(BinaryOp::is_comparison) {
                    return Err(self.report(
                        Diagnostic::error("comparison operators cannot be chained", next.span)
                            .with_note("write `a < b and b < c`, or add parentheses"),
                    ));
                }
            }
            // Likewise for a range, whose right operand stops before any
            // further range.
            if op.is_range() {
                let next = self.peek();
                if let Some(next_op) = binary_op(next.kind).filter(|next| next.is_range()) {
                    let message = format!("`{}` cannot be chained", next_op.symbol());
                    return Err(self.error(message, next.span));
                }
            }
        }
        Ok(lhs)
    }

    /// Parses a literal, a name, a string, a parenthesised expression, a
    /// block, a list literal, an `if`, a `for`, a function expression, a
    /// `lazy` or a prefix operator application. Postfix operators, `.`, calls and
    /// indexes included, are applied by the caller's loop. A `...` here is
    /// not the start of a list's element, and so an error.
    fn parse_prefix(&mut self) -> ParseResult<ExprId> {
        let token = self.peek();
        let kind = match token.kind {
            TokenKind::Int(value) => ExprKind::Int(value),
            TokenKind::Float(value) => ExprKind::Float(value),
            TokenKind::True => ExprKind::Bool(true),
            TokenKind::False => ExprKind::Bool(false),
            TokenKind::Str(id) => ExprKind::Str(id),
            TokenKind::Ident(name) => ExprKind::Name(name),
            TokenKind::StrStart(first) => return self.parse_interpolation(first),
            TokenKind::If => return self.parse_if(),
            TokenKind::For => return self.parse_for(),
            TokenKind::Fn => return self.parse_fn_expr(),
            TokenKind::Lazy => return self.parse_lazy(),
            TokenKind::Neg => return self.parse_unary(UnaryOp::Neg, PREFIX_BP),
            TokenKind::Await => return self.parse_unary(UnaryOp::Await, PREFIX_BP),
            TokenKind::Not => return self.parse_unary(UnaryOp::Not, NOT_BP),
            TokenKind::LParen => {
                self.advance();
                let outer = std::mem::replace(&mut self.no_trailing_block, false);
                let inner = self.parse_expr(0);
                self.no_trailing_block = outer;
                let inner = inner?;
                let close = self.peek();
                if close.kind != TokenKind::RParen {
                    let found = self.describe(close);
                    return Err(self.report(
                        Diagnostic::error(
                            format!("expected `)`, found {found}"),
                            error_span(close),
                        )
                        .with_context(token.span, "unclosed `(`"),
                    ));
                }
                self.advance();
                let height = self.height_of(inner) + 1;
                return self.alloc(
                    ExprKind::Paren(inner),
                    Span::join(token.span, close.span),
                    height,
                );
            }
            TokenKind::LBrace => return self.parse_block(),
            TokenKind::LBracket => return self.parse_list(),
            TokenKind::DotDotDot => return Err(self.error(MISPLACED_SPREAD, token.span)),
            TokenKind::Mut => return Err(self.misplaced_mut(token, None)),
            TokenKind::Minus
            | TokenKind::Plus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::SlashSlash
            | TokenKind::Percent
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::EqEqEq
            | TokenKind::BangEqEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Else
            | TokenKind::In
            | TokenKind::DotDot
            | TokenKind::DotDotEq
            | TokenKind::Let
            | TokenKind::Colon
            | TokenKind::ColonEq
            | TokenKind::Arrow
            | TokenKind::MemoArrow
            | TokenKind::Eq
            | TokenKind::RParen
            | TokenKind::RBrace
            | TokenKind::RBracket
            | TokenKind::StrMid(_)
            | TokenKind::StrEnd(_)
            | TokenKind::Semi
            | TokenKind::Dot
            | TokenKind::Comma
            | TokenKind::Pipe
            | TokenKind::Newline
            | TokenKind::Eof => {
                let found = self.describe(token);
                return Err(self.error(
                    format!("expected expression, found {found}"),
                    error_span(token),
                ));
            }
        };
        self.advance();
        self.alloc(kind, token.span, LEAF_HEIGHT)
    }

    /// Parses a function expression in expression position, starting at
    /// its `fn`. A name after the `fn` is an error here: a named function
    /// is an item.
    fn parse_fn_expr(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let next = self.peek();
        if let TokenKind::Ident(_) = next.kind {
            let found = self.source.slice(next.span);
            return Err(self.report(
                Diagnostic::error(
                    format!("expected `(` after `fn`, found identifier `{found}`"),
                    next.span,
                )
                .with_note(
                    "a named function `fn NAME(...)` is an item; write it on a line of its own, \
                     or drop the name",
                ),
            ));
        }
        self.parse_fn(keyword)
    }

    /// Parses the rest of a function expression whose `fn` keyword,
    /// `keyword`, has been consumed, from its `(`: the parameters, the
    /// return marker and type, and the body, which must start directly
    /// after the return type. The expression spans `keyword` through the
    /// body.
    fn parse_fn(&mut self, keyword: Token) -> ParseResult<ExprId> {
        let open = self.expect_fn_paren()?;
        let base = self.param_stack.len();
        let mut tallest = 0;
        loop {
            let token = self.peek();
            let name = match token.kind {
                TokenKind::RParen => {
                    self.advance();
                    break;
                }
                TokenKind::Ident(name) => name,
                TokenKind::Str(_) => return Err(self.error(PLAIN_VARIABLE_NAME, token.span)),
                TokenKind::StrStart(first) => {
                    let literal = self.parse_interpolation(first)?;
                    let span = self.span_of(literal);
                    return Err(self.error(PLAIN_VARIABLE_NAME, span));
                }
                TokenKind::Mut => {
                    return Err(self.misplaced_mut(token, Some(MUT_PARAM_NOTE)));
                }
                _ => {
                    let found = self.describe(token);
                    return Err(self.error(
                        format!("expected a parameter name, found {found}"),
                        error_span(token),
                    ));
                }
            };
            self.advance();
            let colon = self.peek();
            if colon.kind != TokenKind::Colon {
                let message = format!(
                    "expected `:` and a type after parameter `{}`",
                    self.names.get(name)
                );
                return Err(self.report(
                    Diagnostic::error(message, error_span(colon))
                        .with_note("parameters must be annotated"),
                ));
            }
            self.advance();
            let (ty, height) = self.parse_type()?;
            tallest = tallest.max(height);
            self.param_stack.push(Param {
                name,
                span: Span::join(token.span, self.type_span(ty)),
                ty,
            });
            if self.list_separator(open)? {
                break;
            }
        }
        let params = self.builder.push_params(&self.param_stack[base..]);
        self.param_stack.truncate(base);
        let (effect, marker) = self.return_marker()?;
        let (ret, ret_height) = self.parse_type()?;
        let brace = self.peek();
        if brace.kind != TokenKind::LBrace {
            return Err(self.error("expected `{` to start the function body", error_span(brace)));
        }
        let body = self.parse_block()?;
        let height = tallest.max(ret_height).max(self.height_of(body)) + 1;
        let span = Span::join(keyword.span, self.span_of(body));
        let id = self.builder.push_fn(FnDecl {
            params,
            ret: Some(ret),
            effect,
            marker,
            body,
        });
        self.alloc(ExprKind::Fn(id), span, height)
    }

    /// Parses `lazy EXPR`, starting at its `lazy` keyword: the operand,
    /// parsed at minimum binding power 0, becomes the body of a function
    /// with no parameters and no return type, impure, whose marker is the
    /// keyword. The expression spans the keyword through the operand.
    fn parse_lazy(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let body = self.parse_expr(0)?;
        let height = self.height_of(body) + 1;
        let span = Span::join(keyword.span, self.span_of(body));
        let params = self.builder.push_params(&[]);
        let id = self.builder.push_fn(FnDecl {
            params,
            ret: None,
            effect: Effect::Impure,
            marker: keyword.span,
            body,
        });
        self.alloc(ExprKind::Lazy(id), span, height)
    }

    /// Parses the call of `callee`, starting at its `(` or, for a call with
    /// only a trailing block, its `{`: the arguments and any trailing
    /// block. The trailing block must start on the line the argument list
    /// ends on, which the lexer's newline tokens ensure.
    fn parse_call(&mut self, callee: ExprId) -> ParseResult<ExprId> {
        let base = self.arg_stack.len();
        let mut tallest = self.height_of(callee);
        let open = self.peek();
        let mut end = open.span;
        if open.kind == TokenKind::LParen {
            self.advance();
            // An argument list is inside parentheses.
            let outer = std::mem::replace(&mut self.no_trailing_block, false);
            let close = self.parse_arguments(open, &mut tallest);
            self.no_trailing_block = outer;
            end = close?;
        }
        if self.peek().kind == TokenKind::LBrace && !self.no_trailing_block {
            let block = self.parse_block()?;
            tallest = tallest.max(self.height_of(block));
            end = self.span_of(block);
            self.arg_stack.push(Arg {
                kind: ArgKind::Trailing,
                value: block,
            });
        }
        let args = self.builder.push_args(&self.arg_stack[base..]);
        self.arg_stack.truncate(base);
        let span = Span::join(self.span_of(callee), end);
        self.alloc(ExprKind::Call { callee, args }, span, tallest + 1)
    }

    /// Parses the arguments of a call after its `(`, `open`, through its
    /// `)`, whose span it returns, pushing each on `arg_stack` and raising
    /// `tallest` to the tallest's height. An identifier directly followed
    /// by `=` names its argument.
    fn parse_arguments(&mut self, open: Token, tallest: &mut u32) -> ParseResult<Span> {
        loop {
            let token = self.peek();
            if token.kind == TokenKind::RParen {
                self.advance();
                return Ok(token.span);
            }
            let kind = match (token.kind, self.peek_at(1).kind) {
                (TokenKind::Ident(name), TokenKind::Eq) => {
                    self.advance();
                    self.advance();
                    ArgKind::Named {
                        name,
                        span: token.span,
                    }
                }
                _ => ArgKind::Positional,
            };
            let value = self.parse_expr(0)?;
            *tallest = (*tallest).max(self.height_of(value));
            self.arg_stack.push(Arg { kind, value });
            let close = self.peek();
            if self.list_separator(open)? {
                return Ok(close.span);
            }
        }
    }

    /// Parses a list literal, starting at its `[`: its elements, each an
    /// expression or `...` and one, separated by `,` with an optional
    /// trailing `,`, through its `]`.
    fn parse_list(&mut self) -> ParseResult<ExprId> {
        let open = self.peek();
        self.advance();
        let base = self.element_stack.len();
        let mut tallest = 0;
        // Elements are inside brackets.
        let outer = std::mem::replace(&mut self.no_trailing_block, false);
        let close = self.parse_elements(open, &mut tallest);
        self.no_trailing_block = outer;
        let close = close?;
        let elements = self.builder.push_elements(&self.element_stack[base..]);
        self.element_stack.truncate(base);
        self.alloc(
            ExprKind::List(elements),
            Span::join(open.span, close),
            tallest + 1,
        )
    }

    /// Parses the elements of a list literal after its `[`, `open`, through
    /// its `]`, whose span it returns, pushing each on `element_stack` and
    /// raising `tallest` to the tallest's height, a spread's being its
    /// expression's.
    fn parse_elements(&mut self, open: Token, tallest: &mut u32) -> ParseResult<Span> {
        loop {
            let token = self.peek();
            if token.kind == TokenKind::RBracket {
                self.advance();
                return Ok(token.span);
            }
            let spread = if token.kind == TokenKind::DotDotDot {
                self.advance();
                Some(token.span)
            } else {
                None
            };
            let value = self.parse_expr(0)?;
            *tallest = (*tallest).max(self.height_of(value));
            self.element_stack.push(Element { value, spread });
            let separator = self.peek();
            match separator.kind {
                TokenKind::Comma => self.advance(),
                TokenKind::RBracket => {
                    self.advance();
                    return Ok(separator.span);
                }
                _ => {
                    let found = self.describe(separator);
                    return Err(self.report(
                        Diagnostic::error(
                            format!("expected `,` or `]`, found {found}"),
                            error_span(separator),
                        )
                        .with_context(open.span, "unclosed `[`"),
                    ));
                }
            }
        }
    }

    /// Parses the index of `list`, starting at its `[`: the position,
    /// parsed at minimum binding power 0, and the `]`. The index spans
    /// `list` through the `]`.
    fn parse_index(&mut self, list: ExprId) -> ParseResult<ExprId> {
        let open = self.peek();
        self.advance();
        // The position is inside brackets.
        let outer = std::mem::replace(&mut self.no_trailing_block, false);
        let index = self.parse_expr(0);
        self.no_trailing_block = outer;
        let index = index?;
        let close = self.peek();
        if close.kind != TokenKind::RBracket {
            let found = self.describe(close);
            return Err(self.report(
                Diagnostic::error(format!("expected `]`, found {found}"), error_span(close))
                    .with_context(open.span, "unclosed `[`"),
            ));
        }
        self.advance();
        let height = self.height_of(list).max(self.height_of(index)) + 1;
        let span = Span::join(self.span_of(list), close.span);
        self.alloc(ExprKind::Index { expr: list, index }, span, height)
    }

    /// Parses a prefix operator `op`, the current token, and its operand,
    /// parsed with minimum binding power `operand_bp`.
    fn parse_unary(&mut self, op: UnaryOp, operand_bp: u8) -> ParseResult<ExprId> {
        let token = self.peek();
        self.advance();
        let operand = self.parse_expr(operand_bp)?;
        let span = Span::join(token.span, self.span_of(operand));
        let height = self.height_of(operand) + 1;
        self.alloc(ExprKind::Unary { op, operand }, span, height)
    }

    /// Parses a block `{ ... }`, starting at its `{`.
    fn parse_block(&mut self) -> ParseResult<ExprId> {
        let open = self.peek();
        self.advance();
        self.enter()?;
        let outer = std::mem::replace(&mut self.no_trailing_block, false);
        let body = self.parse_body(Some(open.span));
        self.no_trailing_block = outer;
        self.depth -= 1;
        let (items, tallest) = body?;
        // `parse_body` only returns successfully at the matching `}`.
        let close = self.peek();
        self.advance();
        self.alloc(
            ExprKind::Block(items),
            Span::join(open.span, close.span),
            tallest + 1,
        )
    }

    /// Parses an `if` expression, starting at its `if`.
    fn parse_if(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        // A `{` after the condition is the `if`'s block, not a trailing
        // block of a call.
        let outer = std::mem::replace(&mut self.no_trailing_block, true);
        let cond = self.parse_expr(0);
        self.no_trailing_block = outer;
        let cond = cond?;
        let open = self.peek();
        if open.kind != TokenKind::LBrace {
            return Err(self.error("expected `{` after `if` condition", error_span(open)));
        }
        let then = self.parse_block()?;
        // An `else` may start a later line. If none does, the newlines are
        // put back to end the item.
        let after_then = self.pos;
        self.skip_newlines();
        let (else_, has_else) = if self.peek().kind == TokenKind::Else {
            self.advance();
            let next = self.peek();
            let else_ = match next.kind {
                TokenKind::LBrace => self.parse_block()?,
                // `else if` recurses directly rather than through
                // `parse_expr`, so it counts its own depth.
                TokenKind::If => {
                    self.enter()?;
                    let nested = self.parse_if();
                    self.depth -= 1;
                    nested?
                }
                _ => {
                    return Err(self.error("expected `{` or `if` after `else`", error_span(next)));
                }
            };
            (else_, true)
        } else {
            self.pos = after_then;
            let end = self.span_of(then).end;
            let empty = self.builder.push_item_list(&[]);
            let else_ = self.alloc(ExprKind::Block(empty), Span::new(end, end), LEAF_HEIGHT)?;
            (else_, false)
        };
        let span = Span::join(keyword.span, self.span_of(else_));
        let height = self
            .height_of(cond)
            .max(self.height_of(then))
            .max(self.height_of(else_))
            + 1;
        self.alloc(
            ExprKind::If {
                cond,
                then,
                else_,
                has_else,
            },
            span,
            height,
        )
    }

    /// Parses a `for` expression, starting at its `for`: the variable's
    /// name, an identifier, `in`, the iterable, an optional guard, `if` and
    /// an expression, and the body, a block. The iterable and the guard are
    /// parsed as an `if`'s condition is, at minimum binding power 0 with no
    /// trailing block, so a `{` after either is the body; an `if` after the
    /// complete iterable, which no expression continues with, begins the
    /// guard. The expression spans `for` through the body.
    fn parse_for(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let name_token = self.peek();
        let name = match name_token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) => return Err(self.error(PLAIN_VARIABLE_NAME, name_token.span)),
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_VARIABLE_NAME, span));
            }
            TokenKind::Mut => return Err(self.misplaced_mut(name_token, None)),
            _ => {
                return Err(self.error("expected a name after `for`", error_span(name_token)));
            }
        };
        self.advance();
        let in_token = self.peek();
        if in_token.kind != TokenKind::In {
            let found = self.describe(in_token);
            return Err(self.error(
                format!("expected `in` after the `for` variable, found {found}"),
                error_span(in_token),
            ));
        }
        self.advance();
        // A `{` after the iterable or the guard is the body, not a trailing
        // block of a call.
        let outer = std::mem::replace(&mut self.no_trailing_block, true);
        let parts = self.parse_for_head();
        self.no_trailing_block = outer;
        let (iterable, guard) = parts?;
        let open = self.peek();
        if open.kind != TokenKind::LBrace {
            let message = match guard {
                Some(_) => "expected `{` after `for` guard",
                None => "expected `{` or `if` after `for` iterable",
            };
            return Err(self.error(message, error_span(open)));
        }
        let body = self.parse_block()?;
        let height = self
            .height_of(iterable)
            .max(guard.map_or(0, |guard| self.height_of(guard)))
            .max(self.height_of(body))
            + 1;
        let span = Span::join(keyword.span, self.span_of(body));
        let id = self.builder.push_for(ForDecl {
            name,
            name_span: name_token.span,
            iterable,
            guard,
            body,
        });
        self.alloc(ExprKind::For(id), span, height)
    }

    /// Parses the iterable of a `for` and its guard, if an `if` follows the
    /// iterable, each at minimum binding power 0.
    fn parse_for_head(&mut self) -> ParseResult<(ExprId, Option<ExprId>)> {
        let iterable = self.parse_expr(0)?;
        if self.peek().kind != TokenKind::If {
            return Ok((iterable, None));
        }
        self.advance();
        let guard = self.parse_expr(0)?;
        Ok((iterable, Some(guard)))
    }

    /// Parses an interpolated string, starting at its
    /// [`TokenKind::StrStart`], whose text is `first`.
    fn parse_interpolation(&mut self, first: StrId) -> ParseResult<ExprId> {
        let start = self.peek();
        self.advance();
        let base = self.part_stack.len();
        self.part_stack.push(StrPart::Lit(first));
        let mut tallest = 0;
        // The `{` the current interpolation opened with: the last byte of
        // the token before it.
        let mut open = start.span;
        let end = loop {
            let outer = std::mem::replace(&mut self.no_trailing_block, false);
            let expr = self.parse_expr(0);
            self.no_trailing_block = outer;
            let expr = expr?;
            tallest = tallest.max(self.height_of(expr));
            self.part_stack.push(StrPart::Expr(expr));
            let token = self.peek();
            match token.kind {
                TokenKind::StrMid(text) => {
                    self.advance();
                    self.part_stack.push(StrPart::Lit(text));
                    open = token.span;
                }
                TokenKind::StrEnd(text) => {
                    self.advance();
                    self.part_stack.push(StrPart::Lit(text));
                    break token.span;
                }
                _ => {
                    let found = self.describe(token);
                    let brace = Span::new(open.end.saturating_sub(1), open.end);
                    return Err(self.report(
                        Diagnostic::error(format!("expected `}}`, found {found}"), token.span)
                            .with_context(brace, "unclosed `{`"),
                    ));
                }
            }
        };
        let parts = self.builder.push_parts(&self.part_stack[base..]);
        self.part_stack.truncate(base);
        self.alloc(
            ExprKind::Interp(parts),
            Span::join(start.span, end),
            tallest + 1,
        )
    }

    /// Allocates a node of the given `height`, failing if that height
    /// exceeds [`MAX_DEPTH`].
    fn alloc(&mut self, kind: ExprKind, span: Span, height: u32) -> ParseResult<ExprId> {
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let id = self.builder.alloc(Expr { kind, span });
        self.heights.push(height);
        Ok(id)
    }

    fn height_of(&self, id: ExprId) -> u32 {
        // Every id came from `alloc`, which pushes a height for each node.
        self.heights.get(id.index()).copied().unwrap_or(LEAF_HEIGHT)
    }

    fn too_deep(&mut self, span: Span) -> Reported {
        self.report(
            Diagnostic::error("expression nested too deeply", span)
                .with_note(format!("the limit is {MAX_DEPTH} levels")),
        )
    }

    /// Skips item separators: newlines and `;`.
    fn skip_separators(&mut self) {
        while matches!(self.peek().kind, TokenKind::Newline | TokenKind::Semi) {
            self.advance();
        }
    }

    /// Skips newlines where an item continues on the next line.
    fn skip_newlines(&mut self) {
        while self.peek().kind == TokenKind::Newline {
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

    /// Records an error in the sink and returns the proof that it was
    /// recorded.
    fn error(&mut self, message: impl Into<String>, span: Span) -> Reported {
        self.report(Diagnostic::error(message, span))
    }

    /// Records a diagnostic in the sink and returns the proof that it was
    /// recorded.
    fn report(&mut self, diagnostic: Diagnostic) -> Reported {
        self.diagnostics.push(diagnostic);
        Reported
    }
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
        | TokenKind::Str(_)
        | TokenKind::StrStart(_)
        | TokenKind::StrMid(_)
        | TokenKind::StrEnd(_)
        | TokenKind::Ident(_)
        | TokenKind::Let
        | TokenKind::Mut
        | TokenKind::If
        | TokenKind::Else
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Not
        | TokenKind::Fn
        | TokenKind::Lazy
        | TokenKind::Await
        | TokenKind::For
        | TokenKind::In
        | TokenKind::Pipe
        | TokenKind::Neg
        | TokenKind::Colon
        | TokenKind::ColonEq
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::ItemKind;
    use crate::lexer::tokenize;
    use crate::strings::FieldName;

    fn parse_text(text: &str) -> Result<Ast, Diagnostic> {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source)?;
        parse(&tokens, names, strings, &source)
    }

    /// Renders a file compactly, making grouping explicit: items are joined
    /// by `; `, binary operations and prefix operations are parenthesised,
    /// parens show as `[ ]`, blocks as `{ }`, ascriptions as `(e : T)`,
    /// field accesses as `(e . x)`, fields as `x = e`, strings as `"text"`
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
            match ast.item(id).kind {
                ItemKind::Let {
                    path, ty, value, ..
                } => {
                    out.push_str("let ");
                    out.push_str(&ast.show_path(ast.path(path)).to_string());
                    if let Some(ty) = ty {
                        out.push_str(": ");
                        type_name(ast, ty, out);
                    }
                    out.push_str(" = ");
                    expr(ast, value, out);
                }
                ItemKind::Rebind { path, value } => {
                    out.push_str(&ast.show_path(ast.path(path)).to_string());
                    out.push_str(" := ");
                    expr(ast, value, out);
                }
                ItemKind::Field { path, value } => {
                    out.push_str(&ast.show_path(ast.path(path)).to_string());
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
            }
        }
    }

    fn type_name(ast: &Ast, id: TypeExprId, out: &mut String) {
        match ast.type_expr(id).kind {
            TypeExprKind::Named(name) => out.push_str(ast.names().get(name)),
            TypeExprKind::Record(fields) => {
                out.push('{');
                for (index, field) in ast.field_tys(fields).iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&FieldName(ast.names().get(field.name)).to_string());
                    out.push_str(": ");
                    type_name(ast, field.ty, out);
                }
                out.push('}');
            }
            TypeExprKind::Fn {
                params,
                ret,
                effect,
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
            }
            TypeExprKind::Applied { name, arg, .. } => {
                out.push_str(ast.names().get(name));
                out.push('<');
                type_name(ast, arg, out);
                out.push('>');
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

    fn expr(ast: &Ast, id: ExprId, out: &mut String) {
        use std::fmt::Write as _;
        match ast.get(id).kind {
            ExprKind::Int(value) => write!(out, "{value}").unwrap(),
            ExprKind::Float(value) => write!(out, "{value:?}").unwrap(),
            ExprKind::Bool(value) => write!(out, "{value}").unwrap(),
            ExprKind::Str(text) => write!(out, "\"{}\"", ast.strings().get(text)).unwrap(),
            ExprKind::Interp(parts) => {
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
                    assert!(
                        matches!(empty.kind, ExprKind::Block(items) if ast.items(items).is_empty())
                    );
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
            ExprKind::Block(items) => {
                out.push('{');
                body(ast, items, out);
                out.push('}');
            }
            ExprKind::Ascribe { expr: inner, ty } => {
                out.push('(');
                expr(ast, inner, out);
                out.push_str(" : ");
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
        }
    }

    /// A `for` as `for x in xs if g {..}`, without ` if g` if it has no
    /// guard: as it is written, so that it parses back to itself.
    fn for_shape(ast: &Ast, id: crate::ast::ForId, out: &mut String) {
        let decl = ast.for_decl(id);
        out.push_str("for ");
        out.push_str(ast.names().get(decl.name));
        out.push_str(" in ");
        expr(ast, decl.iterable, out);
        if let Some(guard) = decl.guard {
            out.push_str(" if ");
            expr(ast, guard, out);
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
    /// memoised and `-> Int` if impure.
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
            error("{ x y }"),
            (
                "expected end of item, found `y`".to_owned(),
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
        let diagnostic = parse_text("\"{a}{b c}\"").unwrap_err();
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
        let wrapped =
            |parens: usize| format!("\"{{{}x{}}}\"", "(".repeat(parens), ")".repeat(parens));
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
                ItemKind::Field { path, .. } => ast.path(path)[0].name,
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
        assert_eq!(error("a.b : Int = 1"), found_eq(10));
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
        assert_eq!(ast.names().get(fields[1].name), "b");
        assert_eq!(ast.type_expr(fields[1].ty).span, Span::new(18, 20));
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
        let nested =
            |levels: usize| format!("1 : {}Int{}", "{a: ".repeat(levels), "}".repeat(levels));
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
             \x20 (let f @0..38\n\
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
    fn named_functions_are_lets_of_function_expressions() {
        assert_eq!(
            shape("fn greet(name: String): String { \"hello {name}\" }"),
            "let greet = fn(name: String): String {\"hello <name>\"}"
        );
        assert_eq!(shape("fn f() -> {} {}\nf()"), "let f = fn() -> {} {}; f()");
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
        assert_eq!(ast.names().get(segments[0].name), "add");
        assert!(matches!(ast.get(value).kind, ExprKind::Fn(_)));
        // In a block, and as the pretty printer shows it.
        assert_eq!(
            shape("{ fn f(): Int { 1 }; = f }"),
            "{let f = fn(): Int {1}; = f}"
        );
        assert_eq!(
            parse_text("fn f(): Int { 1 }")
                .unwrap()
                .pretty()
                .to_string(),
            "(file\n\
             \x20 (let f @0..17\n\
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
                span: Span::new(5, 6)
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
             \x20 (let f @0..43\n\
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
            "let plan = fn(): {build: Future<{}>} {build = (lazy {run(compiler)})}"
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
        let TypeExprKind::Applied { name, span, arg } = applied.kind else {
            panic!("expected an applied type, got {applied:?}");
        };
        assert_eq!(ast.names().get(name), "Future");
        assert_eq!(span, Span::new(10, 16));
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
            "expected `>` to close the type argument"
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
                "expected `>` to close the type argument".to_owned(),
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
        let ExprKind::Block(items) = ast.get(only_value(&ast)).kind else {
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
        assert_eq!(ast.names().get(decl.name), "x");
        assert_eq!(decl.name_span, Span::new(4, 5));
        assert_eq!(ast.get(decl.iterable).span, Span::new(9, 11));
        assert_eq!(
            decl.guard.map(|guard| ast.get(guard).span),
            Some(Span::new(15, 16))
        );
        assert_eq!(ast.get(decl.body).span, Span::new(17, 22));
        assert_eq!(ast.for_count(), 1);
        // Without a guard.
        let ast = parse_text("for x in xs {}").unwrap();
        let ExprKind::For(id) = ast.get(only_value(&ast)).kind else {
            panic!("expected a for");
        };
        assert_eq!(ast.for_decl(id).guard, None);
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
                Span::new(12, 13)
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
            error("for x xs {}"),
            (
                "expected `in` after the `for` variable, found `xs`".to_owned(),
                Span::new(6, 8)
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
        let ast =
            parse_text("{ let a = 1; let mut b = 2; fn(): Int { let mut c = 3; c } }").unwrap();
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
}
