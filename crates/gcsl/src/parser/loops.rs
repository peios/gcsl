//! Parsing loops and jumps: `for`, `while` and `do`, their headers, labels,
//! `break`, `continue` and `return`.
//!
//! # Loops
//!
//! Every loop is a prefix expression, with its details in the tree's `for`
//! arena (see [`crate::ast::ForDecl`]). After `for` and its label, if any
//! (see [Labels](#labels)), what follows decides the form
//! (`Parser::parse_for`):
//!
//! - A name directly followed by `in` begins the iteration, `for NAME in
//!   ITERABLE (if GUARD)? BLOCK`: the iterable, parsed at minimum 0 as an
//!   `if`'s condition is, optionally `if` and a guard, parsed the same way,
//!   and the body, a block that must follow on the same line. No operator
//!   starts with `if`, so an `if` after the complete iterable always begins
//!   the guard; an iterable that is itself an `if` expression starts with
//!   `if` and is parsed whole first.
//! - A `{` begins the body of `for BLOCK`, which has no condition.
//! - Anything else begins a header (`Parser::parse_header`): parts
//!   separated by `;`, after each of which newlines are skipped, and then
//!   the body. A part that begins `let` is a `let` item, parsed as one is
//!   in a block, and one that begins as a path item does
//!   (`Parser::starts_path_item`), with `:=`, is a rebind; any other part
//!   is an expression, the condition. Every expression in it, the values of
//!   its items included, is parsed as an `if`'s condition is, with no
//!   trailing block, and a `{` where one would start is the body, and so
//!   an error (see [Trailing blocks](crate::parser::expressions#trailing-blocks)). Exactly one part
//!   is the condition: a second is "a `for` header has one condition", at
//!   it, labelled "the condition is here" at the first, and none is "a
//!   `for` header needs a condition", at the header, with a note to write
//!   `true`. A `let` after the condition is "a `let` after the condition
//!   would be new on every iteration and seen by nothing", with a note to
//!   declare it before the condition and rebind it with `:=`; a field item
//!   is "a `for` header cannot define a field". Where neither `;` nor `{`
//!   follows a part, the error is the one the iteration's would be when
//!   the header is a single part that looks like one: "expected a name
//!   after `for`" for a part that starts with no name and is followed by
//!   `in` (a variable's name for a string literal), and "expected `in`
//!   after the `for` variable" at what follows the name of a part that
//!   starts with one; and "expected `;` or `{` in a `for` header"
//!   otherwise.
//!
//! `do BLOCK for COND` (`Parser::parse_do`) is `do`, its label, if any,
//! the body, and `for`, which must follow the body's `}` on the same line,
//! "expected `for` and a condition after the body of a `do`" otherwise,
//! and then the condition, an ordinary expression at minimum 0, which ends
//! as an expression statement ends: a `{` after it on its line starts a
//! trailing block, as anywhere else, not a body. Only a condition may
//! follow the `for`: a label, a `let` or a name followed by `in` there is
//! "only a condition follows the `for` of a `do`".
//!
//! `break`, `continue` and `return` are items (`Parser::parse_jump`),
//! allowed where any item of a block is, the keyword followed by its label,
//! if any, and its value, if one begins on the same line: a newline, a
//! `;`, a `}` or the end of input after the keyword and the label means
//! there is none. Where an expression starts, they are "expected
//! expression", as `let` is, so `x or return 1` is an error. Where a
//! `return` may go, and what it leaves, is the checker's business.
//!
//! # Labels
//!
//! A label, [`TokenKind::Label`], is written directly after the `for`,
//! `do`, `break`, `continue` or `return` it belongs to, or directly after
//! the `{` of a block, which it names (`Parser::parse_block`): any block,
//! a block expression, a function's body, a branch of an `if`, a loop's
//! body, a `lazy`'s block or a trailing block. With anything between, it
//! is "a label is written directly after `for`, with no space" (or `do`,
//! `break`, `continue`, `return`, `{`), at the label. A block with a field
//! or a spread item, a record block, cannot be labelled: "a record block
//! cannot be labelled", at the label, once the block is parsed. Anywhere
//! else, where an item or an expression starts or ends, or the name of a
//! `for`'s variable or of a `let` is expected, an interpolation's braces
//! included, it is "a label names a loop or a block: it follows `for`,
//! `do`, `break`, `continue`, `return` or `{`", at the label.

use crate::MAX_DEPTH;
use crate::ast::{ExprId, ExprKind, ForDecl, ForHead, Item, ItemId, ItemKind, Jump, Label};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::Name;
use crate::source::Span;

use super::{
    Braces, DO_CONDITION_ONLY, HEADER_BRACE, HEADER_BRACE_NOTE, MISPLACED_LABEL, MISSING_DO_FOR,
    PLAIN_VARIABLE_NAME, ParseResult, Parser, Recovery, Reported, error_span,
};

impl Parser<'_> {
    /// Parses a `break`, `continue` or `return` item, starting at its
    /// keyword, `keyword`: the keyword, its label, if one is written
    /// directly after it, and its value, if one begins on the same line,
    /// since a newline, a `;` or a `}` after the keyword and the label means
    /// there is none. Returns it and its height: its value's, or 0 without
    /// one.
    pub(super) fn parse_jump(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let label = self.parse_label(keyword)?;
        let value = match self.peek().kind {
            TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof => None,
            _ => Some(self.parse_expr(0)?),
        };
        let end = match (value, label) {
            (Some(value), _) => self.span_of(value),
            (None, Some(label)) => label.span,
            (None, None) => keyword.span,
        };
        let span = Span::join(keyword.span, end);
        let height = value.map_or(0, |value| self.height_of(value));
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let jump = Jump { label, value };
        let kind = match keyword.kind {
            TokenKind::Break => ItemKind::Break(jump),
            TokenKind::Return => ItemKind::Return(jump),
            _ => ItemKind::Continue(jump),
        };
        let item = self.builder.alloc_item(Item { kind, span });
        Ok((item, height))
    }

    /// Parses the label of the `for`, `do`, `break`, `continue` or `return`
    /// `keyword`, or of the block whose `{` it is, just consumed, if one
    /// follows it: one written directly after it, with no space, or the
    /// error "a label is written directly after `for`, with no space" (or
    /// the keyword's, or `{`) at a label with space before it.
    pub(super) fn parse_label(&mut self, keyword: Token) -> ParseResult<Option<Label>> {
        let token = self.peek();
        let TokenKind::Label(name) = token.kind else {
            return Ok(None);
        };
        if token.span.start != keyword.span.end {
            let written = keyword.kind.symbol().unwrap_or("its keyword");
            return Err(self.error(
                format!("a label is written directly after `{written}`, with no space"),
                token.span,
            ));
        }
        self.advance();
        Ok(Some(Label {
            name,
            span: token.span,
        }))
    }

    /// Reports `token`, a label where none is allowed: anywhere but directly
    /// after `for`, `do`, `break`, `continue`, `return` or a block's `{`.
    pub(super) fn misplaced_label(&mut self, token: Token) -> Reported {
        self.error(MISPLACED_LABEL, token.span)
    }

    /// Reports `token`, a `{` where an expression starts in a `for` header,
    /// which begins the body (see [Trailing blocks](crate::parser::expressions#trailing-blocks)).
    pub(super) fn header_brace(&mut self, token: Token) -> Reported {
        self.report(Diagnostic::error(HEADER_BRACE, token.span).with_note(HEADER_BRACE_NOTE))
    }

    /// Parses a `for` expression, starting at its `for`, and its label, if
    /// it has one: then, as what follows says (see [Loops](#loops)), an
    /// iteration, `for BLOCK`, or a header and a body. The expression spans
    /// `for` through the body.
    pub(super) fn parse_for(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let label = self.parse_label(keyword)?;
        let next = self.peek();
        match (next.kind, self.peek_at(1).kind) {
            (TokenKind::Ident(name), TokenKind::In) => {
                self.advance();
                self.advance();
                self.parse_each(keyword, label, (name, next.span))
            }
            (TokenKind::LBrace, _) => {
                let body = self.parse_block()?;
                let empty = self.builder.push_item_list(&[]);
                let head = ForHead::While {
                    before: empty,
                    cond: None,
                    after: empty,
                };
                let height = self.height_of(body) + 1;
                self.finish_loop(keyword, (label, head, body), self.span_of(body), height)
            }
            _ => self.parse_header(keyword, label),
        }
    }

    /// Parses the rest of an iteration, `for NAME in ITERABLE (if GUARD)?
    /// BLOCK`, whose keyword, label and variable's name with its span, `for
    /// NAME in` having been consumed, are `keyword`, `label` and `name`: the
    /// iterable, an optional guard, `if` and an expression, and the body, a
    /// block. The iterable and the guard are parsed as an `if`'s condition
    /// is, at minimum binding power 0 with no trailing block, so a `{` after
    /// either is the body; an `if` after the complete iterable, which no
    /// expression continues with, begins the guard.
    pub(super) fn parse_each(
        &mut self,
        keyword: Token,
        label: Option<Label>,
        (name, name_span): (Name, Span),
    ) -> ParseResult<ExprId> {
        // A `{` after the iterable or the guard is the body, not a trailing
        // block of a call.
        let outer = std::mem::replace(&mut self.braces, Braces::NoTrailing);
        let parts = self.parse_for_head();
        self.braces = outer;
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
        let head = ForHead::Each {
            name,
            name_span,
            iterable,
            guard,
        };
        self.finish_loop(keyword, (label, head, body), self.span_of(body), height)
    }

    /// Allocates the loop whose keyword is `keyword`, with `label`, `head`
    /// and `body`, spanning `keyword` through `end`, of `height`.
    pub(super) fn finish_loop(
        &mut self,
        keyword: Token,
        (label, head, body): (Option<Label>, ForHead, ExprId),
        end: Span,
        height: u32,
    ) -> ParseResult<ExprId> {
        let span = Span::join(keyword.span, end);
        let id = self.builder.push_for(ForDecl { label, head, body });
        self.alloc(ExprKind::For(id), span, height)
    }

    /// Parses the header and the body of a condition loop whose keyword
    /// and label are `keyword` and `label`, starting at the header's first
    /// part (see [Loops](#loops)). The header's expressions are parsed with
    /// [`Braces::Header`], so that a `{` anywhere in it, but inside
    /// brackets, is the body.
    pub(super) fn parse_header(
        &mut self,
        keyword: Token,
        label: Option<Label>,
    ) -> ParseResult<ExprId> {
        let outer = std::mem::replace(&mut self.braces, Braces::Header);
        // An error in a part is the header's (see [Errors and
        // recovery](crate::parser::recovery#errors-and-recovery)).
        let recovery = std::mem::replace(&mut self.recovery, Recovery::None);
        let header = self.parse_header_parts();
        self.recovery = recovery;
        self.braces = outer;
        let (head, tallest) = header?;
        // The header ends only at a `{`.
        let body = self.parse_block()?;
        let height = tallest.max(self.height_of(body)) + 1;
        self.finish_loop(keyword, (label, head, body), self.span_of(body), height)
    }

    /// Parses the parts of a `for` header, up to the `{` of the body, not
    /// consumed: items and one condition, separated by `;`, after each of
    /// which newlines are skipped. Returns the loop's head and the height of
    /// its tallest part.
    pub(super) fn parse_header_parts(&mut self) -> ParseResult<(ForHead, u32)> {
        let base = self.item_stack.len();
        let first = self.peek();
        let mut cond: Option<ExprId> = None;
        // Where the items after the condition start on the item stack.
        let mut split = base;
        let mut tallest = 0;
        // The span of the last part parsed.
        let mut last;
        let mut parts = 0;
        loop {
            let start = self.pos;
            let token = self.peek();
            // An attribute applies to no part of a header.
            if matches!(
                token.kind,
                TokenKind::Attribute(_)
                    | TokenKind::InnerAttribute(_)
                    | TokenKind::AttributeArgs(_)
                    | TokenKind::InnerAttributeArgs(_)
            ) {
                return Err(self.misplaced_attribute(token));
            }
            if token.kind == TokenKind::Let || self.starts_path_item() {
                let (item, height) = self.parse_header_item(token, cond.is_some())?;
                self.item_stack.push(item);
                tallest = tallest.max(height);
                last = self.builder.item(item).span;
            } else {
                let expr = self.parse_expr(0)?;
                if let Some(first) = cond {
                    return Err(self.report(
                        Diagnostic::error("a `for` header has one condition", self.span_of(expr))
                            .with_context(self.span_of(first), "the condition is here"),
                    ));
                }
                cond = Some(expr);
                split = self.item_stack.len();
                tallest = tallest.max(self.height_of(expr));
                last = self.span_of(expr);
            }
            parts += 1;
            match self.peek().kind {
                TokenKind::Semi => {
                    self.advance();
                    self.skip_newlines();
                }
                TokenKind::LBrace => break,
                _ => return Err(self.header_error(start, parts == 1 && cond.is_some())),
            }
        }
        let Some(cond) = cond else {
            return Err(self.report(
                Diagnostic::error(
                    "a `for` header needs a condition",
                    Span::join(first.span, last),
                )
                .with_note("write `true` to loop until a `break`"),
            ));
        };
        let before = self.builder.push_item_list(&self.item_stack[base..split]);
        let after = self.builder.push_item_list(&self.item_stack[split..]);
        self.item_stack.truncate(base);
        let head = ForHead::While {
            before,
            cond: Some(cond),
            after,
        };
        Ok((head, tallest))
    }

    /// Parses an item of a `for` header, starting at `token`, a `let` or
    /// the first segment of a path, after the header's condition if
    /// `after_cond`: a `let`, only before the condition, or a `:=`. Returns
    /// it and its height.
    pub(super) fn parse_header_item(
        &mut self,
        token: Token,
        after_cond: bool,
    ) -> ParseResult<(ItemId, u32)> {
        let (item, height) = if token.kind == TokenKind::Let {
            let (kind, value, ty_height, shape) = self.parse_let(token)?;
            self.finish_item(token, (kind, value), ty_height, shape)?
        } else {
            let (kind, value, shape, ty_height) = self.parse_path_item()?;
            self.finish_item(token, (kind, value), ty_height, shape)?
        };
        let Item { kind, span } = *self.builder.item(item);
        match kind {
            ItemKind::Let { .. } if after_cond => Err(self.report(
                Diagnostic::error(
                    "a `let` after the condition would be new on every iteration and seen by \
                     nothing",
                    span,
                )
                .with_note("declare it before the condition and rebind it here with `:=`"),
            )),
            ItemKind::Field { .. } => Err(self.report(
                Diagnostic::error("a `for` header cannot define a field", span)
                    .with_note("declare a variable with `let`, or rebind one with `:=`"),
            )),
            _ => Ok((item, height)),
        }
    }

    /// The error for the token after a part of a `for` header that is
    /// neither `;` nor the body's `{`, the part having started at token
    /// number `start`. When the part is the header's only part so far and
    /// an expression (`alone`), it may be meant as the variable of an
    /// iteration: followed by `in`, it is "expected a name after `for`" at
    /// the part, unless it starts with a name, when it is "expected `in`
    /// after the `for` variable" at the token after the name, or a string
    /// literal, when it is the error for a variable's name that is not
    /// plain; and a lone name followed by anything on its line is
    /// "expected `in` after the `for` variable" at what follows. Anything
    /// else is "expected `;` or `{` in a `for` header".
    pub(super) fn header_error(&mut self, start: usize, alone: bool) -> Reported {
        let token = self.peek();
        let found = self.describe(token);
        let (first, second) = (self.token_at(start), self.token_at(start + 1));
        let after_name = |token: Token| {
            format!(
                "expected `in` after the `for` variable, found {}",
                self.describe(token)
            )
        };
        if alone && token.kind == TokenKind::In {
            let part = Span::join(first.span, self.token_at(self.pos - 1).span);
            return match first.kind {
                TokenKind::Ident(_) => self.error(after_name(second), error_span(second)),
                TokenKind::Str(_) | TokenKind::StrStart(_) => self.error(PLAIN_VARIABLE_NAME, part),
                _ => self.error("expected a name after `for`", part),
            };
        }
        let lone_name = matches!(first.kind, TokenKind::Ident(_)) && self.pos == start + 1;
        if alone && lone_name && !matches!(token.kind, TokenKind::Newline | TokenKind::Eof) {
            return self.error(after_name(token), error_span(token));
        }
        self.error(
            format!("expected `;` or `{{` in a `for` header, found {found}"),
            error_span(token),
        )
    }

    /// Parses a `do` loop, starting at its `do`: its label, if it has one,
    /// its body, a block, and `for`, which must follow the body's `}` on
    /// the same line, and the condition, an ordinary expression at minimum
    /// binding power 0, which ends as an expression statement does. Only a
    /// condition may follow the `for`: a label, a `let` or a name followed
    /// by `in` there is an error. The expression spans `do` through the
    /// condition.
    pub(super) fn parse_do(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let label = self.parse_label(keyword)?;
        let open = self.peek();
        if open.kind != TokenKind::LBrace {
            let found = self.describe(open);
            return Err(self.error(
                format!("expected `{{` after `do`, found {found}"),
                error_span(open),
            ));
        }
        let body = self.parse_block()?;
        let for_token = self.peek();
        if for_token.kind != TokenKind::For {
            return Err(self.error(MISSING_DO_FOR, error_span(for_token)));
        }
        self.advance();
        let next = self.peek();
        let misplaced = match next.kind {
            TokenKind::Label(_) | TokenKind::Let => Some(next.span),
            TokenKind::Ident(_) if self.peek_at(1).kind == TokenKind::In => {
                Some(Span::join(next.span, self.peek_at(1).span))
            }
            _ => None,
        };
        if let Some(span) = misplaced {
            return Err(
                self.report(Diagnostic::error(DO_CONDITION_ONLY, span).with_note(
                    "a `do` loop runs its body, then its condition, and again while it is true",
                )),
            );
        }
        let cond = self.parse_expr(0)?;
        let height = self.height_of(body).max(self.height_of(cond)) + 1;
        let head = ForHead::Do { cond };
        self.finish_loop(keyword, (label, head, body), self.span_of(cond), height)
    }

    /// Parses the iterable of a `for` and its guard, if an `if` follows the
    /// iterable, each at minimum binding power 0.
    pub(super) fn parse_for_head(&mut self) -> ParseResult<(ExprId, Option<ExprId>)> {
        let iterable = self.parse_expr(0)?;
        if self.peek().kind != TokenKind::If {
            return Ok((iterable, None));
        }
        self.advance();
        let guard = self.parse_expr(0)?;
        Ok((iterable, Some(guard)))
    }
}
