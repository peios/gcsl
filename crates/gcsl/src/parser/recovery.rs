//! Recovering from errors: marks and rewinding, skipping to the next item, and
//! the values parsed after an error.
//!
//! # Errors and recovery
//!
//! Errors are values. Parsing functions return `Result`, and a failure is
//! recorded in the parser's diagnostic sink before the `Err` propagates, up
//! to the nearest *recovery point*, which puts back what the failed parse
//! made (the parser's stacks and every arena of the tree, so that nothing
//! of it is left in the tree, `Parser::restore`), skips the source to a
//! *synchronisation point* and goes on. [`parse_all`](super::parse_all) returns the tree so
//! recovered, with every error; [`parse`](super::parse) returns the first error, as it
//! always did, or the tree if there is none.
//!
//! The recovery points, innermost first:
//!
//! - **An entry of a list**: an argument of a call, a parameter of a
//!   function expression, of a function type or of a definition made with
//!   a def-type, an element of a list literal, an entry of a record type.
//!   The list skips to the `,` after the entry and goes on with the next,
//!   or to its closing bracket, and ends. Each later entry is parsed as any
//!   is, its errors reported. A list with an error fails once it has ended,
//!   so its errors are reported and its construct (the call, the function,
//!   the list, the type) is no part of the tree.
//! - **The expression of an interpolation**: the string skips to the piece
//!   after it and goes on, to fail at its end, as a list does.
//! - **The value of an item of a body**, after the `=` of a `let`, a field
//!   or a result, the `:=` of a rebind or the `...` of a spread, the end of
//!   the item checked with it (`Parser::parse_value`): the value is an
//!   error node, [`ExprKind::Error`], spanning what is skipped to the end
//!   of the item, so the item still declares what it declares. Not in a
//!   `for` header, whose parts' errors are the header's.
//! - **An item of a body**, the file's or a block's: it is an error item,
//!   [`ItemKind::Error`], spanning what is skipped to the end of the item
//!   (`Parser::error_item`). So is the item after an attribute that is an
//!   error, which may have meant something else for it, and attributes
//!   waiting for an item that fails are dropped.
//!
//! Two constructs an editor leaves half-typed are kept, for a language
//! server, which completes what is typed after them, rather than failing
//! what they are in: their error is reported as it was, and the parse goes
//! on. A field access with no name after its `.`, `a.`, where what follows
//! may end an expression in the innermost bracket open
//! (`Parser::ends_unfinished`: the end of a line or of the input, a `;`, a
//! `}` of a block, the `)` or `]` of the bracket open, or a `,` in one) is a
//! [`ExprKind::NoField`] of its record, which is checked as any expression
//! is; where anything else follows, it fails as before. A call whose
//! argument list ends where no `)` is, at the end of the input or at the
//! `}` of the block around it, which closes the `(` too, is an
//! [`ExprKind::Unclosed`] of its callee and of the arguments it parsed (an
//! argument that failed leaves nothing of it), from its callee through the
//! last token it went on to; since the lexer emits no newline inside a
//! `(`, that is the rest of the file, or of the block, each line of it
//! parsed as an argument (`q = 2` a named one). A list that ends at its
//! `)` with an error fails as before. The bracket of a list or of
//! parentheses whose `,`, `)` or `]` is missing at the token a `.` with no
//! name was reported at, the end of the input or a `}`, is not reported
//! again (`Parser::report_close`).
//!
//! An item, or a value, that fails having gone on past a newline, as it
//! does after an operator, `=`, `:` or `let`, to a line that is an item of
//! its own (`Parser::item_line`: a `let`, `fn NAME`, `def`, `import`,
//! `type`, `defmeta`, `return`, `break`, `continue`, an attribute, or a
//! path followed by `=`, `:=` or an annotation and `=`), with no bracket
//! opened in it left open, is forgotten, its errors with it, and parsed
//! again with that newline its end (`Parser::rewind`), so that the next
//! line is the next item, as an editor's half-typed `x = ` above it leaves
//! it: its error is then the one at that newline, "expected expression,
//! found newline". A line that may go on the line before, an operand, is
//! not one, and stays the item's. Nor is a line inside a bracket left
//! open, a `(` or `[`: the lexer emits no newline there, and the rest of
//! the source is in the bracket.
//!
//! A skip (`Parser::skip`) follows the brackets the lexer follows (`nest`):
//! it goes over everything between brackets opened since what failed
//! started, newlines inside braces included, and stops, without consuming
//! it, at the synchronisation point with nothing opened since then left
//! open: an item's newline or `;`; a `}` that closes a block's body; an
//! entry's `,`, or the list's closing bracket; the string's piece after an
//! interpolation; or the end of the input. A closing bracket of something
//! opened before what failed that is not the point is something around a
//! list (a `}`, or a string's piece, which closes the list too), where the
//! list ends, or one the lexer ignores, as a `)` or `]` of nothing is,
//! skipped. In the file's body a `}` closes nothing, and is skipped. An
//! item starts at none of the points, so it, and every loop, always moves
//! on; a value may be empty, `x = ;`. Nesting too deep is recovered from by the item or its value
//! alone, not by every list it is in, which would skip what is nested in
//! each of them again.
//!
//! What is reported: each error, once, where it is found. Nothing skipped
//! is reported: it may be part of the error. A list that has failed reports
//! what is wrong in its later entries, and the end of the input where its
//! closing bracket should be. Nothing is reported at a lexical error's
//! token, [`TokenKind::Error`], which the lexer has reported: what it is in
//! fails silently, as an expression, a type, a separator; nor after a
//! lexical error whose token runs to the end of the input (an unterminated
//! string literal), after which nothing was lexed. A block whose body meets
//! the end of the input ends there, "expected `}`", reported once, for the
//! innermost block; a `}` in the file's body that closes nothing is
//! "unexpected `}`", and skipped. The errors of one file, the lexer's and
//! the parser's together, are capped at [`MAX_ERRORS`](super::MAX_ERRORS): after them the
//! parser skips to the end of the input, and the next is replaced by "too
//! many errors" ([`capped`](super::capped)).
//!
//! What the tree says: [`Ast::syntax`](super::Ast::syntax) is [`Syntax::Clean`] for a tree with
//! no error, [`Syntax::Recovered`] for one with errors, each in an error
//! node or left out of the tree (a failed list's construct, a failed
//! interpolated string), and [`Syntax::Unbalanced`] for one whose braces
//! did not match, a `}` that closes nothing or a block that the end of the
//! input closed, where items may be in the wrong block: the workspace does
//! not check that one. A recovered tree is checked, an error node being of
//! type `Error`, silently, and a block with an error item erroneous, as is
//! a name that names nothing in it (see the workspace's [Syntax
//! errors](crate::workspace::parse#syntax-errors)); it is never lowered.

use super::Sync;
use crate::ast::{Expr, ExprId, ExprKind, Item, ItemId, ItemKind, Syntax};
use crate::lexer::TokenKind;
use crate::source::Span;

use super::{LEAF_HEIGHT, Marks, Nesting, ParseResult, Parser, Recovery, Stop, nest};

impl Parser<'_> {
    /// Where the parser's stacks and the builder's arenas are, with its
    /// depth and what a `{` means, for [`Parser::restore`].
    pub(super) fn marks(&self) -> Marks {
        Marks {
            builder: self.builder.mark(),
            heights: self.heights.len(),
            items: self.item_stack.len(),
            parts: self.part_stack.len(),
            field_tys: self.field_ty_stack.len(),
            params: self.param_stack.len(),
            param_tys: self.param_ty_stack.len(),
            args: self.arg_stack.len(),
            elements: self.element_stack.len(),
            path: self.path.len(),
            depth: self.depth,
            braces: self.braces,
            diagnostics: self.diagnostics.len(),
            syntax: self.builder.syntax(),
            unclosed: self.unclosed,
        }
    }

    /// Puts the parser's stacks and the builder's arenas back where
    /// `marks` says they were, with its depth and what a `{` means:
    /// forgets everything parsed since.
    pub(super) fn restore(&mut self, marks: Marks) {
        self.builder.truncate(marks.builder);
        self.heights.truncate(marks.heights);
        self.item_stack.truncate(marks.items);
        self.part_stack.truncate(marks.parts);
        self.field_ty_stack.truncate(marks.field_tys);
        self.param_stack.truncate(marks.params);
        self.param_ty_stack.truncate(marks.param_tys);
        self.arg_stack.truncate(marks.args);
        self.element_stack.truncate(marks.elements);
        self.path.truncate(marks.path);
        self.depth = marks.depth;
        self.braces = marks.braces;
    }

    /// After what started at token number `start` failed, `marks` saying
    /// where the parser was then: forgets what it parsed, skips to its end
    /// as `sync` says ([`Parser::skip`]), and returns the span of the
    /// source skipped, from `start`; an empty span at `start` if nothing
    /// was.
    pub(super) fn recover(&mut self, start: usize, marks: Marks, sync: Sync) -> Span {
        self.deep = false;
        self.restore(marks);
        self.skip(start, sync);
        let first = self.token_at(start).span;
        if self.pos > start {
            Span::join(first, self.token_at(self.pos - 1).span)
        } else {
            Span::new(first.start, first.start)
        }
    }

    /// The error item standing for the item of a body that started at
    /// token number `start` and failed, `marks` saying where the parser
    /// was then: spanning the item and what is skipped after it, as `sync`
    /// says (see [Errors and recovery](#errors-and-recovery)).
    pub(super) fn error_item(&mut self, start: usize, marks: Marks, sync: Sync) -> ItemId {
        let span = self.recover(start, marks, sync);
        self.builder.alloc_item(Item {
            kind: ItemKind::Error,
            span,
        })
    }

    /// Parses the value of an item of a body, after its `=`, `:=` or
    /// `...`: newlines, then any expression, at minimum binding power 0,
    /// which must be followed by the end of the item. Having gone on to a
    /// line that is an item of its own ([`Parser::item_line`]), it is
    /// parsed again up to that line. If it fails, or something else
    /// follows it, the value is an error node spanning it and what is
    /// skipped to the item's end (see [Errors and
    /// recovery](#errors-and-recovery)), so that the item still declares
    /// what it declares; in a `for` header, which does not recover, it
    /// fails, and the end of the item is not looked for.
    pub(super) fn parse_value(&mut self) -> ParseResult<ExprId> {
        let Recovery::Body { top } = self.recovery else {
            self.skip_newlines();
            return self.parse_expr(0);
        };
        let start = self.pos;
        let marks = self.marks();
        if let Some(value) = self.attempt_value() {
            return Ok(value);
        }
        // Gone on to a line that is an item of its own: parsed again to the
        // end of its own line, and the next line left for the next item.
        if let Some(line) = self.item_line(start) {
            self.rewind(start, marks);
            let outer = self.line_end.replace(line);
            let value = self.attempt_value();
            self.line_end = outer;
            if let Some(value) = value {
                return Ok(value);
            }
        }
        let span = self.recover(start, marks, Sync::Item { top });
        let id = self.builder.alloc(Expr {
            kind: ExprKind::Error,
            span,
        });
        self.heights.push(LEAF_HEIGHT);
        Ok(id)
    }

    /// Parses a value, as [`Parser::parse_value`] does, and the end of its
    /// item; `None` if either fails.
    pub(super) fn attempt_value(&mut self) -> Option<ExprId> {
        self.skip_newlines();
        let value = self.parse_expr(0).ok()?;
        self.expect_end_of_item().ok()?;
        Some(value)
    }

    /// After the entry of a list closed by `close` that started at token
    /// number `start` failed: skips to its end, and past the `,` after it,
    /// and returns whether the list has ended there, past `close` or before
    /// what closes something around it (see [Errors and
    /// recovery](#errors-and-recovery)).
    pub(super) fn recover_entry(&mut self, start: usize, close: TokenKind) -> bool {
        // Nesting too deep is no entry's error: the item's recovers from it,
        // once, rather than every list it is in, each skipping what is
        // nested in it again.
        if self.deep {
            return true;
        }
        match self.skip(start, Sync::Entry { close }) {
            Stop::Separator => {
                self.advance();
                false
            }
            Stop::Close => {
                self.advance();
                true
            }
            Stop::Outer => true,
        }
    }

    /// Skips the source of what started at token number `start` and failed,
    /// from the current token to where `sync` says it ends, following the
    /// brackets as the lexer does ([`nest`]) from `start` on: at a
    /// separator, or a closing token, with no bracket opened since `start`
    /// still open, or at the end of the input. The token it stops at is not
    /// consumed. A closing bracket of nothing opened since `start` that
    /// does not end it is skipped, as the lexer ignores it, but a `}` of
    /// the file's body, which closes no block, makes the tree
    /// [`Syntax::Unbalanced`].
    pub(super) fn skip(&mut self, start: usize, sync: Sync) -> Stop {
        let mut open = Nesting::default();
        for index in start..self.pos {
            nest(&mut open, self.token_at(index).kind);
        }
        loop {
            let kind = self.peek().kind;
            if kind == TokenKind::Eof {
                return Stop::Outer;
            }
            if open.open.is_empty() {
                match (sync, kind) {
                    (Sync::Item { .. }, TokenKind::Newline | TokenKind::Semi)
                    | (Sync::Entry { .. }, TokenKind::Comma) => return Stop::Separator,
                    _ => {}
                }
            }
            if nest(&mut open, kind) {
                match (sync, kind) {
                    (Sync::Entry { close }, _) if kind == close => return Stop::Close,
                    (
                        Sync::Entry { .. },
                        TokenKind::RBrace | TokenKind::StrMid(_) | TokenKind::StrEnd(_),
                    )
                    | (Sync::Interpolation, TokenKind::RBrace) => return Stop::Outer,
                    (Sync::Item { top: false }, TokenKind::RBrace)
                    | (Sync::Interpolation, TokenKind::StrMid(_) | TokenKind::StrEnd(_)) => {
                        return Stop::Close;
                    }
                    (Sync::Item { top: true }, TokenKind::RBrace) => {
                        self.builder.set_syntax(Syntax::Unbalanced);
                    }
                    _ => {}
                }
            }
            self.advance();
        }
    }

    /// After the item, or the value of an item, that started at token number
    /// `start` failed, the parser being where it failed: the first newline
    /// token it went on past, as it does after an operator, `=` or `:`, with
    /// no bracket opened since `start` left open, after which an item
    /// starts ([`Parser::starts_item`]): the end of the item, if it does not
    /// go on to the next line, so that the next line, which an editor's
    /// half-typed `x = ` above it leaves alone, is the next item. `None` if
    /// there is none. Costs nothing but on an error.
    pub(super) fn item_line(&mut self, start: usize) -> Option<usize> {
        let mut open = Nesting::default();
        for index in start..self.pos {
            let kind = self.token_at(index).kind;
            if kind == TokenKind::Newline && open.open.is_empty() && self.starts_item(index + 1) {
                return Some(index);
            }
            nest(&mut open, kind);
        }
        None
    }

    /// Whether the token number `index` starts what is surely an item, not
    /// the rest of an expression: a `let`, a `fn` and its name, a `def`, an
    /// `import`, a `type`, a `defmeta`, a `return`, `break` or `continue`,
    /// an attribute, or a field or a rebind, a path followed by `=`, `:=`
    /// or an annotation and `=`.
    pub(super) fn starts_item(&mut self, index: usize) -> bool {
        let token = self.token_at(index);
        match token.kind {
            TokenKind::Let
            | TokenKind::Def
            | TokenKind::Import
            | TokenKind::Return
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::Attribute(_)
            | TokenKind::InnerAttribute(_) => true,
            TokenKind::Fn => {
                matches!(self.token_at(index + 1).kind, TokenKind::Ident(_))
                    || self.starts_method(index)
            }
            _ => {
                let at = std::mem::replace(&mut self.pos, index);
                let item = self.starts_type_item()
                    || self.starts_enum_item()
                    || self.starts_context_item()
                    || self.starts_defmeta()
                    || self.starts_path_item();
                self.pos = at;
                item
            }
        }
    }

    /// Puts the parser back where `marks` says it was, at token number
    /// `start`, as if what it parsed since had never been: its errors
    /// forgotten too, and what they made of the tree. For parsing it again,
    /// once more than the first time is known of where it ends.
    pub(super) fn rewind(&mut self, start: usize, marks: Marks) {
        self.restore(marks);
        self.diagnostics.truncate(marks.diagnostics);
        self.builder.reset_syntax(marks.syntax);
        self.unclosed = marks.unclosed;
        self.deep = false;
        self.pos = start;
    }
}
