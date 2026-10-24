//! Parsing `assert` and `recover`.
//!
//! # Failure
//!
//! ```text
//! ASSERT  = "assert" EXPR ("," EXPR)?
//! RECOVER = "recover" BLOCK "else" ("(" IDENT ")")? BLOCK
//! ```
//!
//! `assert`, which is no keyword, starts a statement when it is the first
//! word of a body's item and is followed on its line by what starts an
//! expression, `(` among them, but not `[`, which starts an indexed path
//! (`Parser::starts_assert`): `assert x == 1`, `assert ok, "it failed"`,
//! an expression item of [`ExprKind::Assert`], whose message, if none is
//! written, is the condition as it is written, in backticks. A call of a function named
//! `assert` there is written `(assert)(x)`. `recover`, no keyword either,
//! directly followed by `{`, starts a `recover` expression
//! ([`ExprKind::Recover`]): its body, then `else`, which may start a later
//! line, as an `if`'s may, the name its fallback binds the failure to, in
//! parentheses, if any, and the fallback. Anything but `else` after the
//! body is "expected `else` after the body of `recover`".

use crate::ast::{ExprId, ExprKind, Item, ItemId, ItemKind, RecoverDecl};
use crate::lexer::{Token, TokenKind};
use crate::names::Name;
use crate::source::Span;

use super::{ASSERT, LEAF_HEIGHT, ParseResult, Parser, RECOVER, error_span, one_line};

impl Parser<'_> {
    /// Whether the tokens here start an `assert` statement: the word
    /// `assert`, then on its line what starts an expression, but `[` (see
    /// [Failure](#failure)).
    pub(super) fn starts_assert(&self) -> bool {
        let keyword = self.peek();
        if !matches!(keyword.kind, TokenKind::Ident(name) if self.names.get(name) == ASSERT) {
            return false;
        }
        let next = self.peek_at(1);
        let on_line = !self
            .source
            .slice(Span::new(keyword.span.end, next.span.start))
            .contains('\n');
        on_line
            && matches!(
                next.kind,
                TokenKind::Ident(_)
                    | TokenKind::Int(_)
                    | TokenKind::Float(_)
                    | TokenKind::Duration(_)
                    | TokenKind::Size(_)
                    | TokenKind::Str(_)
                    | TokenKind::StrStart(_)
                    | TokenKind::True
                    | TokenKind::False
                    | TokenKind::LParen
                    | TokenKind::LBrace
                    | TokenKind::Neg
                    | TokenKind::Not
                    | TokenKind::Await
                    | TokenKind::Caret
                    | TokenKind::If
                    | TokenKind::Do
                    | TokenKind::For
                    | TokenKind::Fn
                    | TokenKind::Lazy
            )
    }

    /// Parses an `assert` statement, at its `assert`, `keyword`, which
    /// [`Parser::starts_assert`] has found (see [Failure](#failure)): the
    /// condition and, after `,`, the message; an expression item of type
    /// `{}`.
    pub(super) fn parse_assert(&mut self, keyword: Token) -> ParseResult<ItemId> {
        self.advance();
        let cond = self.parse_expr(0)?;
        let message = if self.peek().kind == TokenKind::Comma {
            self.advance();
            self.skip_newlines();
            self.parse_expr(0)?
        } else {
            // The condition as it is written, on one line, in backticks.
            let at = self.span_of(cond);
            let written = format!("`{}`", one_line(self.source.slice(at)));
            let text = self.strings.push(&written);
            self.alloc(ExprKind::Str(text), at, LEAF_HEIGHT)?
        };
        let span = Span::join(keyword.span, self.span_of(message));
        let height = self.height_of(cond).max(self.height_of(message)) + 1;
        let value = self.alloc(ExprKind::Assert { cond, message }, span, height)?;
        Ok(self.builder.alloc_item(Item {
            kind: ItemKind::Expr(value),
            span,
        }))
    }

    /// Whether the current token starts a `recover` expression: the word
    /// `recover`, directly followed by `{` (see [Failure](#failure)).
    pub(super) fn starts_recover(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(name) if self.names.get(name) == RECOVER)
            && self.peek_at(1).kind == TokenKind::LBrace
    }

    /// Parses a `recover` expression, at its `recover`, which
    /// [`Parser::starts_recover`] has found (see [Failure](#failure)): the
    /// body, `else`, the name of the failure in parentheses, if any, and
    /// the fallback.
    pub(super) fn parse_recover(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let body = self.parse_block()?;
        // An `else` may start a later line, as an `if`'s may.
        self.skip_newlines();
        let token = self.peek();
        if token.kind != TokenKind::Else {
            let found = self.describe(token);
            return Err(self.error(
                format!("expected `else` after the body of `recover`, found {found}"),
                error_span(token),
            ));
        }
        self.advance();
        let error = if self.peek().kind == TokenKind::LParen {
            Some(self.parse_failure_name()?)
        } else {
            None
        };
        let brace = self.peek();
        if brace.kind != TokenKind::LBrace {
            let found = self.describe(brace);
            return Err(self.error(
                format!("expected `{{` to start the fallback of `recover`, found {found}"),
                error_span(brace),
            ));
        }
        let fallback = self.parse_block()?;
        let span = Span::join(keyword.span, self.span_of(fallback));
        let height = self.height_of(body).max(self.height_of(fallback)) + 1;
        let id = self.builder.push_recover(RecoverDecl {
            body,
            error,
            fallback,
        });
        self.alloc(ExprKind::Recover(id), span, height)
    }

    /// Parses the name a `recover`'s fallback binds its failure to, `(e)`,
    /// at its `(` (see [Failure](#failure)).
    pub(super) fn parse_failure_name(&mut self) -> ParseResult<(Name, Span)> {
        self.advance();
        // The failure's field, which the checker and lowering name it by.
        self.names.intern(crate::check::FAILURE_MESSAGE);
        let name = self.peek();
        let TokenKind::Ident(text) = name.kind else {
            let found = self.describe(name);
            return Err(self.error(
                format!("expected the failure's name after `(`, found {found}"),
                error_span(name),
            ));
        };
        self.advance();
        let close = self.peek();
        if close.kind != TokenKind::RParen {
            let found = self.describe(close);
            return Err(self.error(
                format!("expected `)` after the failure's name, found {found}"),
                error_span(close),
            ));
        }
        self.advance();
        Ok((text, name.span))
    }
}
