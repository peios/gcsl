//! Parsing expressions: operators and their precedence, prefixes, blocks, `if`,
//! interpolation, and the depth of nesting.
//!
//! # Trailing blocks
//!
//! A trailing block would make `if x { ... }` a call of `x`, so while the
//! parser is in the condition of an `if` (or `else if`), the iterable or
//! the guard of a `for`, or the header of a condition loop, a `{` after an
//! expression never starts one: the parser keeps a flag,
//! `Parser::braces`, that `Parser::parse_if` sets around its condition and
//! `Parser::parse_for` around its iterable and guard, or its header. The
//! flag applies to the whole condition at every operator, so `if a and b {
//! }` is an `if` with condition `a and b`. It is lifted, and restored
//! afterwards, inside parentheses, argument lists, list literals, indexes,
//! blocks and interpolations, so `if (f { }) { }` calls `f`. A newline
//! before a `{` ends the item as always, so it never starts a trailing
//! block either.
//!
//! In a `for` header the flag says more: a `{` is the body even where an
//! expression starts, so a block, a record block included, written as a
//! header's value or condition is an error, "a `{` in a `for` header begins
//! its body", whose note says to put the block in parentheses, as in `for
//! let r = ({ a = 1 }); r.a < 3; ... { }`. No `;` is looked for ahead to
//! decide what a `{` is.
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
//! A literal is a number, a duration or a size, a string, `true` or `false`,
//! each one token. A number with a name glued to it, `30sec`, which the
//! lexer leaves a number before any name that is no unit's (see the lexer),
//! is "unknown unit `sec`" where an expression is expected, noted with the
//! units there are (`Parser::unitless`); a module path or a definition's
//! path takes such a run of glued tokens as its segment.
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
//! | postfix `is` TYPE           | 5       | (none: non-associative)    |
//! | `..` `..=`                  | 6       | 7 (non-associative)        |
//! | `+` `-`                     | 7       | 8                          |
//! | `*` `/` `//` `%`            | 9       | 10                         |
//! | postfix `:` TYPE, `as` TYPE | 11      | (none: parses a type)      |
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
//! `EXPR as TYPE`, a checked narrowing, is a postfix operator binding
//! exactly as the ascription does, chaining with it left to right: `-x as
//! Int` is `(-x) as Int`, and `a + b as Int` is `a + (b as Int)`. `EXPR is
//! TYPE`, a type test, is a postfix operator binding as a comparison does:
//! its left power is comparison's, so `x + 1 is Int` is `(x + 1) is Int`,
//! `not x is Int` is `not (x is Int)` and `x is Int and y` is `(x is Int)
//! and y`; and it does not chain with a comparison or another `is`, either
//! way round, so `a == b is Int` and `x is Int == y` are "comparison
//! operators cannot be chained" (`Parser::not_chained`). Each parses a
//! whole type after its keyword, a union included, so `x is Int | String`
//! tests for the union. `is` and `as` are keywords (see [`crate::lexer`]).
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
//! A `for` is a prefix expression too, usable wherever an `if` is, and so
//! is a `do`: see [Loops](crate::parser::loops#loops).
//!
//! An interpolated string arrives from the lexer as a
//! [`TokenKind::StrStart`], each expression's tokens, a
//! [`TokenKind::StrMid`] between each two expressions, and a
//! [`TokenKind::StrEnd`]. The parser parses each expression at minimum 0
//! and collects the parts on a stack shared by all the interpolations being
//! parsed, as it does for items, so nested interpolations need no
//! allocation of their own.
//!
//! # Depth
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
//! and types (a `break`'s, `continue`'s or `return`'s value among them), an `if` or an
//! interpolation one above its tallest child, a
//! `for` one above the tallest of its iterable, its guard and its body, a
//! condition loop one above the tallest of its header's items, its
//! condition and its body, a `do` loop one above the taller of its body and
//! its condition, a
//! list literal one above its tallest element (a spread's being its
//! expression's), an index one above the taller of its list and its
//! position, a
//! field access one above its record, an ascription, an `is` or an `as`
//! one above the taller of its expression and its type (a record type
//! being one above its tallest
//! field type or field default, a function type one above the tallest of its parameter and
//! return types, an applied type one above its argument, a `T[]` one above
//! `T`, a parenthesised
//! type one above the type inside, though it leaves no node of its own, and
//! a union one above the taller of its two sides), a function expression
//! one above the tallest of its parameter types, its return type and its
//! body, a `lazy`
//! one above its operand, and a call one above the tallest of its callee and
//! arguments, and rejects a node that would exceed the limit. An item's
//! height is the taller of its value's and its path's index positions',
//! plus two levels for each segment of its path after the first, or its
//! annotation's if that is taller; an item taller than the limit is
//! rejected too, at the item. A position is parsed by
//! `Parser::parse_expr`, so it counts its own depth as any expression
//! does. Together these guarantee that no tree handed to a later stage is
//! deeper than `MAX_DEPTH`.

use super::Sync;
use crate::MAX_DEPTH;
use crate::ast::{BinaryOp, Expr, ExprId, ExprKind, ItemKind, ItemRange, StrPart, UnaryOp};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::source::Span;
use crate::strings::StrId;

use super::{
    ASCRIBE_BP, Braces, FIELD_BP, IS_BP, LABELLED_RECORD, LEAF_HEIGHT, MISPLACED_SPREAD, NOT_BP,
    PREFIX_BP, ParseResult, Parser, Reported, Stop, binary_op, binding_power, error_span,
};

impl Parser<'_> {
    /// Parses an expression whose binary operators all have a left binding
    /// power of at least `min_bp`.
    pub(super) fn parse_expr(&mut self, min_bp: u8) -> ParseResult<ExprId> {
        self.enter()?;
        let result = self.parse_expr_inner(min_bp);
        self.depth -= 1;
        result
    }

    /// Records one more level of recursion, failing if it would exceed
    /// [`MAX_DEPTH`]. Every successful call is paired with a decrement by
    /// its caller.
    pub(super) fn enter(&mut self) -> ParseResult<()> {
        if self.depth >= MAX_DEPTH {
            let span = self.peek().span;
            return Err(self.too_deep(span));
        }
        self.depth += 1;
        Ok(())
    }

    pub(super) fn parse_expr_inner(&mut self, min_bp: u8) -> ParseResult<ExprId> {
        let mut lhs = self.parse_prefix()?;
        loop {
            let token = self.peek();
            let starts_call = match token.kind {
                TokenKind::LParen => true,
                TokenKind::LBrace => self.braces == Braces::Free,
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
                    lhs = self.no_field(lhs, token, name_token)?;
                    continue;
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
            if matches!(token.kind, TokenKind::Is | TokenKind::As) {
                let is = token.kind == TokenKind::Is;
                if (if is { IS_BP } else { ASCRIBE_BP }) < min_bp {
                    break;
                }
                lhs = self.parse_type_operator(lhs, is)?;
                continue;
            }
            // `with { ... }` on the line, or a def-type reference: each binds
            // as a field access does.
            if self.starts_juxtaposed(lhs, token) {
                if FIELD_BP < min_bp {
                    break;
                }
                lhs = self.parse_juxtaposed(lhs, token)?;
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
                self.not_chained()?;
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

    /// Parses the postfix `is TYPE`, if `is`, or `as TYPE` otherwise, whose
    /// keyword is the next token, applied to `lhs`: the type is parsed as
    /// an ascription's is, newlines before it skipped. An `is` binds as a
    /// comparison and, like one, may not be followed by a comparison or
    /// another `is` (see [`Parser::not_chained`]).
    pub(super) fn parse_type_operator(&mut self, lhs: ExprId, is: bool) -> ParseResult<ExprId> {
        self.advance();
        self.skip_newlines();
        let (ty, ty_height) = self.parse_type()?;
        let span = Span::join(self.span_of(lhs), self.type_span(ty));
        let height = self.height_of(lhs).max(ty_height) + 1;
        let kind = if is {
            ExprKind::Is { expr: lhs, ty }
        } else {
            ExprKind::As { expr: lhs, ty }
        };
        let expr = self.alloc(kind, span, height)?;
        if is {
            self.not_chained()?;
        }
        Ok(expr)
    }

    /// After a comparison or an `is`, whose right side stops before any
    /// further comparison or `is`: reports one that follows directly,
    /// "comparison operators cannot be chained", since `a < b < c` would
    /// otherwise mean `(a < b) < c`.
    pub(super) fn not_chained(&mut self) -> ParseResult<()> {
        let next = self.peek();
        if next.kind == TokenKind::Is || binary_op(next.kind).is_some_and(BinaryOp::is_comparison) {
            return Err(self.report(
                Diagnostic::error("comparison operators cannot be chained", next.span)
                    .with_note("write `a < b and b < c`, or add parentheses"),
            ));
        }
        Ok(())
    }

    /// Parses a literal, a name, a string, a parenthesised expression, a
    /// block, a list literal, an `if`, a `for`, a function expression, a
    /// `lazy` or a prefix operator application. Postfix operators, `.`, calls and
    /// indexes included, are applied by the caller's loop. A `...` here is
    /// not the start of a list's element, and so an error.
    pub(super) fn parse_prefix(&mut self) -> ParseResult<ExprId> {
        let token = self.peek();
        let kind = match token.kind {
            TokenKind::Int(value) => self.unitless(token, ExprKind::Int(value))?,
            TokenKind::Float(value) => self.unitless(token, ExprKind::Float(value))?,
            TokenKind::Duration(value) => ExprKind::Duration(value),
            TokenKind::Size(value) => ExprKind::Size(value),
            TokenKind::True => ExprKind::Bool(true),
            TokenKind::False => ExprKind::Bool(false),
            TokenKind::Str(id) => ExprKind::Str(id),
            TokenKind::Ident(_) if self.starts_variant() => return self.parse_variant_expr(),
            TokenKind::Ident(_) if self.starts_with_block() => return self.parse_with(None),
            TokenKind::Ident(_) if self.starts_recover() => return self.parse_recover(),
            TokenKind::Caret => return self.parse_context_read(),
            TokenKind::Ident(name) => ExprKind::Name(name),
            TokenKind::StrStart(first) => return self.parse_interpolation(first),
            TokenKind::If => return self.parse_if(),
            TokenKind::For => return self.parse_for(),
            TokenKind::Do => return self.parse_do(),
            TokenKind::Label(_) => return Err(self.misplaced_label(token)),
            TokenKind::LBrace if self.braces == Braces::Header => {
                return Err(self.header_brace(token));
            }
            TokenKind::Fn => return self.parse_fn_expr(),
            TokenKind::Lazy => return self.parse_lazy(),
            TokenKind::Neg => return self.parse_unary(UnaryOp::Neg, PREFIX_BP),
            TokenKind::Await => return self.parse_unary(UnaryOp::Await, PREFIX_BP),
            TokenKind::Not => return self.parse_unary(UnaryOp::Not, NOT_BP),
            TokenKind::LParen => return self.parse_paren(token),
            TokenKind::LBrace => return self.parse_block(),
            TokenKind::LBracket => return self.parse_list(),
            TokenKind::DotDotDot => return Err(self.error(MISPLACED_SPREAD, token.span)),
            TokenKind::Mut => return Err(self.misplaced_mut(token, None)),
            // A lexical error, the lexer's to report: what it is part of
            // fails silently.
            TokenKind::Error => return Err(self.error("invalid token", token.span)),
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
            | TokenKind::Is
            | TokenKind::As
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::Return
            | TokenKind::DotDot
            | TokenKind::DotDotEq
            | TokenKind::Let
            | TokenKind::Def
            | TokenKind::Import
            | TokenKind::Colon
            | TokenKind::ColonEq
            | TokenKind::ColonColon
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
            | TokenKind::Attribute(_)
            | TokenKind::InnerAttribute(_)
            | TokenKind::AttributeArgs(_)
            | TokenKind::InnerAttributeArgs(_)
            | TokenKind::Newline
            | TokenKind::Eof => return Err(self.expected_expression(token)),
        };
        self.advance();
        self.alloc(kind, token.span, LEAF_HEIGHT)
    }

    /// Reports `token`, where an expression starts, as none: "expected
    /// expression, found `)`".
    pub(super) fn expected_expression(&mut self, token: Token) -> Reported {
        let found = self.describe(token);
        self.error(
            format!("expected expression, found {found}"),
            error_span(token),
        )
    }

    /// Parses a parenthesised expression, starting at its `(`, `open`, the
    /// current token.
    pub(super) fn parse_paren(&mut self, open: Token) -> ParseResult<ExprId> {
        self.advance();
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let inner = self.parse_expr(0);
        self.braces = outer;
        let inner = inner?;
        let close = self.peek();
        if close.kind != TokenKind::RParen {
            let found = self.describe(close);
            return Err(self.report_close(
                Diagnostic::error(format!("expected `)`, found {found}"), error_span(close))
                    .with_context(open.span, "unclosed `(`"),
            ));
        }
        self.advance();
        let height = self.height_of(inner) + 1;
        self.alloc(
            ExprKind::Paren(inner),
            Span::join(open.span, close.span),
            height,
        )
    }

    /// Parses a prefix operator `op`, the current token, and its operand,
    /// parsed with minimum binding power `operand_bp`.
    pub(super) fn parse_unary(&mut self, op: UnaryOp, operand_bp: u8) -> ParseResult<ExprId> {
        let token = self.peek();
        self.advance();
        let operand = self.parse_expr(operand_bp)?;
        let span = Span::join(token.span, self.span_of(operand));
        let height = self.height_of(operand) + 1;
        self.alloc(ExprKind::Unary { op, operand }, span, height)
    }

    /// Parses a block `{ ... }`, starting at its `{`, with its label, if one
    /// is written directly after the `{`. A record block, one with a field
    /// or a spread item, cannot be labelled: "a record block cannot be
    /// labelled", at the label.
    pub(super) fn parse_block(&mut self) -> ParseResult<ExprId> {
        let open = self.peek();
        self.advance();
        let label = self.parse_label(open)?;
        self.enter()?;
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let (items, tallest) = self.parse_body(Some(open.span));
        self.braces = outer;
        self.depth -= 1;
        let label = match label {
            Some(label) if self.is_record_body(items) => {
                return Err(self.error(LABELLED_RECORD, label.span));
            }
            Some(label) => Some(self.builder.push_label(label)),
            None => None,
        };
        // `parse_body` returns at the matching `}`, or at the end of the
        // input, which it has reported.
        let close = self.peek();
        if close.kind == TokenKind::RBrace {
            self.advance();
        }
        self.alloc(
            ExprKind::Block { items, label },
            Span::join(open.span, close.span),
            tallest + 1,
        )
    }

    /// Whether the body `items`, just parsed, has a field or a spread item,
    /// which makes it a record block.
    pub(super) fn is_record_body(&self, items: ItemRange) -> bool {
        self.builder.items(items).iter().any(|&id| {
            matches!(
                self.builder.item(id).kind,
                ItemKind::Field { .. } | ItemKind::Spread(_)
            )
        })
    }

    /// Parses an `if` expression, starting at its `if`.
    pub(super) fn parse_if(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        // A `{` after the condition is the `if`'s block, not a trailing
        // block of a call.
        let outer = std::mem::replace(&mut self.braces, Braces::NoTrailing);
        let cond = self.parse_expr(0);
        self.braces = outer;
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
            let block = ExprKind::Block {
                items: empty,
                label: None,
            };
            let else_ = self.alloc(block, Span::new(end, end), LEAF_HEIGHT)?;
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

    /// Parses an interpolated string, starting at its
    /// [`TokenKind::StrStart`], whose text is `first`.
    pub(super) fn parse_interpolation(&mut self, first: StrId) -> ParseResult<ExprId> {
        let start = self.peek();
        self.advance();
        let base = self.part_stack.len();
        self.part_stack.push(StrPart::Lit(first));
        let mut tallest = 0;
        // The `{` the current interpolation opened with: the last byte of
        // the token before it.
        let mut open = start.span;
        // An expression that fails is skipped to the piece after it, and the
        // string goes on, to fail at its end (see [Errors and
        // recovery](crate::parser::recovery#errors-and-recovery)).
        let mut failed = None;
        let end = loop {
            let part = self.pos;
            if let Err(reported) = self.parse_interpolated(open, &mut tallest) {
                if self.deep || self.skip(part, Sync::Interpolation) != Stop::Close {
                    return Err(reported);
                }
                failed = Some(reported);
            }
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
                // Neither follows an expression that parsed.
                _ => return Err(failed.unwrap_or_else(|| self.error("expected `}`", token.span))),
            }
        };
        if let Some(reported) = failed {
            return Err(reported);
        }
        let parts = self.builder.push_parts(&self.part_stack[base..]);
        self.part_stack.truncate(base);
        self.alloc(
            ExprKind::Interp(parts),
            Span::join(start.span, end),
            tallest + 1,
        )
    }

    /// Parses the expression of an interpolation of a string, opened by the
    /// last byte of `open`, at minimum binding power 0, which the string's
    /// piece after it must follow, raising `tallest` to its height.
    pub(super) fn parse_interpolated(&mut self, open: Span, tallest: &mut u32) -> ParseResult<()> {
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let expr = self.parse_expr(0);
        self.braces = outer;
        let expr = expr?;
        *tallest = (*tallest).max(self.height_of(expr));
        self.part_stack.push(StrPart::Expr(expr));
        let token = self.peek();
        if matches!(token.kind, TokenKind::StrMid(_) | TokenKind::StrEnd(_)) {
            return Ok(());
        }
        let found = self.describe(token);
        let brace = Span::new(open.end.saturating_sub(1), open.end);
        Err(self.report(
            Diagnostic::error(format!("expected `}}`, found {found}"), token.span)
                .with_context(brace, "unclosed `{`"),
        ))
    }

    /// Allocates a node of the given `height`, failing if that height
    /// exceeds [`MAX_DEPTH`].
    pub(super) fn alloc(&mut self, kind: ExprKind, span: Span, height: u32) -> ParseResult<ExprId> {
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let id = self.builder.alloc(Expr { kind, span });
        self.heights.push(height);
        Ok(id)
    }

    pub(super) fn height_of(&self, id: ExprId) -> u32 {
        // Every id came from `alloc`, which pushes a height for each node.
        self.heights.get(id.index()).copied().unwrap_or(LEAF_HEIGHT)
    }

    pub(super) fn too_deep(&mut self, span: Span) -> Reported {
        self.deep = true;
        self.report(
            Diagnostic::error("expression nested too deeply", span)
                .with_note(format!("the limit is {MAX_DEPTH} levels")),
        )
    }
}
