//! Parsing context: `context` items, reads of a context, and `with`.
//!
//! # Context
//!
//! A context (see the checker's "Context") is declared by a top-level item,
//! `context NAME: TYPE = DEFAULT` or `context NAME: TYPE`
//! (`Parser::parse_context_item`); `context` is no keyword, so the item
//! starts with the name `context`, then a name and `:`. With a default, the
//! item is the `let` of the default's function, pure and of no parameters,
//! returning TYPE, under a binding no source can write
//! ([`crate::names::context_binding`]), recorded as the file's
//! ([`ContextDecl`]); without one, it is [`ItemKind::Context`]. Anywhere
//! but at a file's top level it is "`context` is declared only at the top
//! level of a file".
//!
//! ```text
//! CONTEXT = "context" IDENT ":" TYPE ("=" EXPR)?
//! READ    = "^" IDENT
//! WITH    = POSTFIX "with" BLOCK | "with" BLOCK BLOCK
//! PARAM   = "^" IDENT (":" TYPE)? | ...
//! ```
//!
//! `^NAME` reads one ([`ExprKind::ContextRead`]). `with` is no keyword
//! either: the name `with` directly followed by `{` on the line where an
//! expression ends binds that expression, as a field access does, so `a +
//! f() with { arch = "x" }` binds `f()`; at an expression's start, `with
//! { ... } { BODY }` binds a block ([`ExprKind::With`]). The bindings are a
//! block of fields, parsed as a record block is, and checked as a context's
//! bindings. A parameter `^NAME`, with a type or none, is the parameter
//! `NAME` whose default is `^NAME`, of the context's type if none is
//! written ([`TypeExprKind::Context`]); a default written after it is "a
//! context parameter's default is its context".

use crate::ast::{
    ContextDecl, Effect, ExprId, ExprKind, FnDecl, Item, ItemId, ItemKind, Param, PathSegment,
    SegmentKind, TypeExpr, TypeExprKind, WithBinding, WithRange,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::source::Span;

use super::{Braces, CONTEXT, LEAF_HEIGHT, ParseResult, Parser, WITH, error_span};

impl Parser<'_> {
    /// Whether the tokens here start a `context` item: the word `context`,
    /// then a name and `:` (see [Context](#context)).
    pub(super) fn starts_context_item(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(name) if self.names.get(name) == CONTEXT)
            && matches!(self.peek_at(1).kind, TokenKind::Ident(_))
            && self.peek_at(2).kind == TokenKind::Colon
    }

    /// Parses a `context` item of the file's body, starting at its
    /// `context`, `keyword`, which [`Parser::starts_context_item`] has found
    /// (see [Context](#context)): its name, its type and its default, if
    /// `=` follows, which becomes the body of a pure function of no
    /// parameters returning the type, the value of the `let` of the
    /// context's binding.
    pub(super) fn parse_context_item(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let name_token = self.peek();
        let TokenKind::Ident(name) = name_token.kind else {
            return Err(self.error("expected the context's name", name_token.span));
        };
        self.advance();
        // `:`, which `starts_context_item` found.
        let colon = self.peek();
        self.advance();
        let (ty, ty_height) = self.parse_type()?;
        if self.peek().kind != TokenKind::Eq {
            let item = self.builder.alloc_item(Item {
                kind: ItemKind::Context {
                    name,
                    name_span: name_token.span,
                    ty,
                },
                span: Span::join(keyword.span, self.type_span(ty)),
            });
            self.builder.push_context(ContextDecl {
                item,
                name,
                name_span: name_token.span,
                ty,
                binding: None,
            });
            return Ok((item, ty_height));
        }
        let eq = self.peek();
        self.advance();
        self.skip_newlines();
        let value = self.parse_expr(0)?;
        let span = self.span_of(value);
        let height = self.height_of(value);
        let result = self.builder.alloc_item(Item {
            kind: ItemKind::Result(value),
            span: Span::join(eq.span, span),
        });
        let items = self.builder.push_item_list(&[result]);
        let body = self.alloc(
            ExprKind::Block { items, label: None },
            span,
            height.saturating_add(1),
        )?;
        let params = self.builder.push_params(&[]);
        let function = self.builder.push_fn(FnDecl {
            params,
            ret: Some(ty),
            effect: Effect::Pure,
            marker: colon.span,
            body,
        });
        let height = self.height_of(body).max(LEAF_HEIGHT) + 1;
        let value = self.alloc(
            ExprKind::Fn(function),
            Span::join(name_token.span, span),
            height,
        )?;
        // The first ordinal of the name no context's binding has yet, of
        // this input or of an earlier one sharing the table.
        let context = self.names.get(name).to_owned();
        let mut ordinal = 0;
        let text = loop {
            let text = crate::names::context_binding(&context, ordinal);
            if self.names.lookup(&text).is_none() {
                break text;
            }
            ordinal += 1;
        };
        let binding = self.names.intern(&text);
        let base = self.path.len();
        self.path.push(PathSegment {
            kind: SegmentKind::Name(binding),
            span: name_token.span,
        });
        let (path, shape) = self.finish_path(base);
        let kind = ItemKind::Let {
            path,
            ty: None,
            value,
            mutable: false,
            function: true,
        };
        let (item, height) = self.finish_item(keyword, (kind, value), ty_height, shape)?;
        self.builder.push_context(ContextDecl {
            item,
            name,
            name_span: name_token.span,
            ty,
            binding: Some(binding),
        });
        Ok((item, height))
    }

    /// Parses a read of a context, `^NAME`, at its `^` (see
    /// [Context](#context)). Anything but a name after the `^` is "expected
    /// a context's name after `^`".
    pub(super) fn parse_context_read(&mut self) -> ParseResult<ExprId> {
        let caret = self.peek();
        self.advance();
        let token = self.peek();
        let TokenKind::Ident(name) = token.kind else {
            let found = self.describe(token);
            return Err(self.error(
                format!("expected a context's name after `^`, found {found}"),
                error_span(token),
            ));
        };
        self.advance();
        self.alloc(
            ExprKind::ContextRead(name),
            Span::join(caret.span, token.span),
            1,
        )
    }

    /// Whether the current token, `token`, is a `with` binding `lhs`: the
    /// name `with` on the line where `lhs` ends, directly followed by `{`
    /// (see [Context](#context)).
    pub(super) fn starts_with_postfix(&self, lhs: ExprId, token: Token) -> bool {
        matches!(token.kind, TokenKind::Ident(name) if self.names.get(name) == WITH)
            && self.is_name_after(self.span_of(lhs), token)
            && self.peek_at(1).kind == TokenKind::LBrace
    }

    /// Whether the current token, `token`, is a name on the line where
    /// `lhs` ends that applies to it: `with { ... }` (see
    /// [Context](#context)), or, after a name or a dotted path, a def-type
    /// reference's path (see [Def-types](crate::parser::deftypes#def-types)).
    pub(super) fn starts_juxtaposed(&self, lhs: ExprId, token: Token) -> bool {
        self.starts_with_postfix(lhs, token)
            || (self.is_name_after(self.span_of(lhs), token) && self.is_path_expr(lhs))
    }

    /// Parses what [`Parser::starts_juxtaposed`] has found at `token` after
    /// `lhs`: its `with`, or its def-type reference.
    pub(super) fn parse_juxtaposed(&mut self, lhs: ExprId, token: Token) -> ParseResult<ExprId> {
        if self.starts_with_postfix(lhs, token) {
            self.parse_with(Some(lhs))
        } else {
            self.parse_def_type_ref(lhs, None)
        }
    }

    /// Whether the current token starts a `with` block, `with { ... } {
    /// BODY }`: the name `with`, directly followed by `{` (see
    /// [Context](#context)).
    pub(super) fn starts_with_block(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(name) if self.names.get(name) == WITH)
            && self.peek_at(1).kind == TokenKind::LBrace
    }

    /// Parses `with { ... }` after `body`, at its `with` (see
    /// [Context](#context)), or, with no `body`, the block form, `with {
    /// ... } { BODY }`: anything but `{` after the bindings there is
    /// "expected `{` to start the body of `with`".
    pub(super) fn parse_with(&mut self, body: Option<ExprId>) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let (bindings, tallest, close) = self.parse_with_bindings()?;
        // From the body through the bindings, or from `with` through the
        // body.
        let (body, span) = if let Some(body) = body {
            (body, Span::join(self.span_of(body), close))
        } else {
            let brace = self.peek();
            if brace.kind != TokenKind::LBrace {
                let found = self.describe(brace);
                return Err(self.error(
                    format!("expected `{{` to start the body of `with`, found {found}"),
                    error_span(brace),
                ));
            }
            let body = self.parse_block()?;
            (body, Span::join(keyword.span, self.span_of(body)))
        };
        let height = self.height_of(body).max(tallest) + 1;
        self.alloc(ExprKind::With { bindings, body }, span, height)
    }

    /// Parses the bindings of a `with`, at their `{` (see
    /// [Context](#context)): each `NAME = VALUE`, `NAME.PATH = VALUE`, or
    /// the shorthand `NAME`, the variable of the name, separated by `;` or
    /// line breaks, through the `}`. A dotted key reads the context's value
    /// it updates, `^NAME`, allocated before its value. Returns them, the
    /// tallest value's height and the `}`'s span. Anything but a name where
    /// a binding starts is "expected a context's name in `with`", and
    /// anything but `=` after a dotted key "expected `=` after
    /// `platform.arch`".
    pub(super) fn parse_with_bindings(&mut self) -> ParseResult<(WithRange, u32, Span)> {
        let open = self.peek();
        self.advance();
        let mut bindings = Vec::new();
        let mut tallest = LEAF_HEIGHT;
        loop {
            while matches!(self.peek().kind, TokenKind::Newline | TokenKind::Semi) {
                self.advance();
            }
            let token = self.peek();
            let name = match token.kind {
                TokenKind::RBrace => {
                    self.advance();
                    let range = self.builder.push_with_bindings(&bindings);
                    return Ok((range, tallest, token.span));
                }
                TokenKind::Ident(name) => name,
                TokenKind::Eof => {
                    return Err(self.report(
                        Diagnostic::error("expected `}` to close the bindings", token.span)
                            .with_context(open.span, "unclosed `{`"),
                    ));
                }
                _ => {
                    let found = self.describe(token);
                    return Err(self.error(
                        format!("expected a context's name in `with`, found {found}"),
                        error_span(token),
                    ));
                }
            };
            self.advance();
            let base = self.path.len();
            while self.peek().kind == TokenKind::Dot
                && let segment = self.peek_at(1)
                && let TokenKind::Ident(field) = segment.kind
            {
                self.advance();
                self.advance();
                self.path.push(PathSegment {
                    kind: SegmentKind::Name(field),
                    span: segment.span,
                });
            }
            let dotted = self.path.len() > base;
            // The value a dotted key updates, before the key's value.
            let current = if dotted {
                Some(self.alloc(ExprKind::ContextRead(name), token.span, 1)?)
            } else {
                None
            };
            let value = if self.peek().kind == TokenKind::Eq {
                self.advance();
                self.skip_newlines();
                let outer = std::mem::replace(&mut self.braces, Braces::Free);
                let value = self.parse_expr(0);
                self.braces = outer;
                value?
            } else if dotted {
                let written =
                    Span::join(token.span, self.path.last().map_or(token.span, |s| s.span));
                let shown = self.source.slice(written).to_owned();
                let found = self.describe(self.peek());
                return Err(self.error(
                    format!("expected `=` after `{shown}`, found {found}"),
                    error_span(self.peek()),
                ));
            } else {
                // The shorthand, `with { arch }`: the variable `arch`.
                self.alloc(ExprKind::Name(name), token.span, LEAF_HEIGHT)?
            };
            tallest = tallest.max(self.height_of(value));
            let (path, _) = self.finish_path(base);
            bindings.push(WithBinding {
                name,
                name_span: token.span,
                path,
                value,
                current,
            });
            let next = self.peek();
            if !matches!(
                next.kind,
                TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace
            ) {
                let found = self.describe(next);
                return Err(self.error(
                    format!("expected `;` or a line break after a binding, found {found}"),
                    error_span(next),
                ));
            }
        }
    }

    /// Parses a parameter `^NAME`, at its `^` (see [Context](#context)):
    /// the parameter `NAME`, of the type written after it or, with none, of
    /// the context's, whose default is `^NAME`. A default written after it
    /// is "a context parameter's default is its context".
    pub(super) fn parse_context_param(
        &mut self,
        open: Token,
        tallest: &mut u32,
    ) -> ParseResult<bool> {
        let caret = self.peek();
        let read = self.parse_context_read()?;
        let ExprKind::ContextRead(name) = self.builder.get(read).kind else {
            return Err(self.error("expected a context's name after `^`", caret.span));
        };
        let name_span = Span::new(caret.span.end, self.span_of(read).end);
        let ty = if self.peek().kind == TokenKind::Colon {
            self.advance();
            let (ty, height) = self.parse_type()?;
            *tallest = (*tallest).max(height);
            ty
        } else {
            self.builder.alloc_type(TypeExpr {
                kind: TypeExprKind::Context(name),
                span: name_span,
            })
        };
        let eq = self.peek();
        if eq.kind == TokenKind::Eq {
            let shown = self.names.get(name).to_owned();
            return Err(self.report(
                Diagnostic::error("a context parameter's default is its context", eq.span)
                    .with_note(format!("`^{shown}` stands for `{shown}: T = ^{shown}`")),
            ));
        }
        *tallest = (*tallest).max(self.height_of(read));
        self.param_stack.push(Param {
            name,
            span: Span::join(caret.span, self.type_span(ty)),
            ty,
            default: Some(read),
            default_fn: None,
        });
        self.list_separator(open)
    }
}
