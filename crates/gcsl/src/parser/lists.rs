//! Parsing lists: list literals, their elements, and indexes.
//!
//! # Lists
//!
//! A list literal is `[`, elements separated by `,` with an optional
//! trailing `,`, and `]`; `[]` is the empty list. An element is an
//! expression, or `...` and an expression, a spread, parsed at minimum
//! binding power 0 as any element is, so `...a + b` spreads `a + b`. A
//! `...` anywhere else that an expression starts, but at the start of an
//! item (see [Items and bodies](crate::parser::bodies#items-and-bodies)), is an error, "`...` is
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

use crate::ast::{Element, ExprId, ExprKind};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::source::Span;

use super::{Braces, ParseResult, Parser, error_span};

impl Parser<'_> {
    /// Parses a list literal, starting at its `[`: its elements, each an
    /// expression or `...` and one, separated by `,` with an optional
    /// trailing `,`, through its `]`.
    pub(super) fn parse_list(&mut self) -> ParseResult<ExprId> {
        let open = self.peek();
        self.advance();
        let base = self.element_stack.len();
        let mut tallest = 0;
        // Elements are inside brackets.
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let close = self.parse_elements(open, &mut tallest);
        self.braces = outer;
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
    /// expression's. An element that fails is skipped to its `,`, and the
    /// list goes on, to fail at its end (see [Errors and
    /// recovery](crate::parser::recovery#errors-and-recovery)).
    pub(super) fn parse_elements(&mut self, open: Token, tallest: &mut u32) -> ParseResult<Span> {
        let mut failed = None;
        loop {
            let start = self.pos;
            let token = self.peek();
            if token.kind == TokenKind::RBracket {
                self.advance();
                return failed.map_or(Ok(token.span), Err);
            }
            match self.parse_element(open, tallest) {
                Ok(Some(close)) => return failed.map_or(Ok(close), Err),
                Ok(None) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RBracket) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        }
    }

    /// Parses an element of a list literal that `open` opens, and the `,`
    /// or the `]` after it, raising `tallest` to its height. Returns the
    /// span of the `]` if it ends the list.
    pub(super) fn parse_element(
        &mut self,
        open: Token,
        tallest: &mut u32,
    ) -> ParseResult<Option<Span>> {
        let token = self.peek();
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
            TokenKind::Comma => {
                self.advance();
                Ok(None)
            }
            TokenKind::RBracket => {
                self.advance();
                Ok(Some(separator.span))
            }
            _ => {
                let found = self.describe(separator);
                Err(self.report_close(
                    Diagnostic::error(
                        format!("expected `,` or `]`, found {found}"),
                        error_span(separator),
                    )
                    .with_context(open.span, "unclosed `[`"),
                ))
            }
        }
    }

    /// Parses the index of `list`, starting at its `[` (see
    /// [`Parser::parse_position`]). The index spans `list` through the `]`.
    pub(super) fn parse_index(&mut self, list: ExprId) -> ParseResult<ExprId> {
        let (index, brackets) = self.parse_position()?;
        let height = self.height_of(list).max(self.height_of(index)) + 1;
        let span = Span::join(self.span_of(list), brackets);
        self.alloc(ExprKind::Index { expr: list, index }, span, height)
    }

    /// Parses the brackets of an index, of an expression or of a path,
    /// starting at the `[`: the position, parsed at minimum binding power
    /// 0, and the `]`. Returns the position and the span of the brackets.
    pub(super) fn parse_position(&mut self) -> ParseResult<(ExprId, Span)> {
        let open = self.peek();
        self.advance();
        // The position is inside brackets.
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let index = self.parse_expr(0);
        self.braces = outer;
        let index = index?;
        let close = self.peek();
        if close.kind != TokenKind::RBracket {
            let found = self.describe(close);
            return Err(self.report_close(
                Diagnostic::error(format!("expected `]`, found {found}"), error_span(close))
                    .with_context(open.span, "unclosed `[`"),
            ));
        }
        self.advance();
        Ok((index, Span::join(open.span, close.span)))
    }
}
