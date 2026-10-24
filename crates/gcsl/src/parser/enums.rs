//! Parsing enums: `enum` items, variants, and references to them.
//!
//! # Enums
//!
//! `enum`, which is no keyword, starts an item when a name follows it
//! (`Parser::parse_enum_item`): `enum NAME extensible`, the word
//! `extensible` (no keyword either) and optionally the bound in angle
//! brackets, `<TYPE>`, declares an enum ([`ItemKind::Enum`]); `enum
//! ENUM::NAME`, the enum a name or a qualified one, then `::` and a name,
//! and optionally the payload, `<TYPE>`, a variant ([`ItemKind::Variant`]).
//! Anything but `extensible` after an enum's name is "expected `extensible`
//! after the enum's name", noting that only extensible enums are
//! declared. Like a `type`, an `enum` is an item of a file's outermost
//! body only: "`enum` is declared only at the top level of a file". In an
//! expression, names joined by `.` and then `::` and a name are a variant
//! as a value ([`ExprKind::Variant`]), its enum a type expression.

use crate::ast::{
    ExprId, ExprKind, Item, ItemId, ItemKind, PathSegment, SegmentKind, TypeExpr, TypeExprId,
    TypeExprKind,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::source::Span;

use super::{ENUM, EXTENSIBLE, LEAF_HEIGHT, ParseResult, Parser, error_span};

impl Parser<'_> {
    /// Whether the tokens here start an `enum` item: the word `enum`, then
    /// a name (see [Enums](#enums)).
    pub(super) fn starts_enum_item(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(name) if self.names.get(name) == ENUM)
            && matches!(self.peek_at(1).kind, TokenKind::Ident(_))
    }

    /// Parses an `enum` item of the file's body, starting at its `enum`,
    /// `keyword` (see [Enums](#enums)): an enum, `enum NAME extensible`
    /// and its bound, or a variant, `enum ENUM::NAME` and its payload.
    pub(super) fn parse_enum_item(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let name_token = self.peek();
        let TokenKind::Ident(name) = name_token.kind else {
            return Err(self.error("expected the enum's name after `enum`", name_token.span));
        };
        let qualified = self.peek_at(1).kind == TokenKind::Dot;
        if qualified || self.peek_at(1).kind == TokenKind::ColonColon {
            let (of, of_height) = if qualified {
                self.parse_type_path()?
            } else {
                self.advance();
                let of = self.builder.alloc_type(TypeExpr {
                    kind: TypeExprKind::Named(name),
                    span: name_token.span,
                });
                (of, LEAF_HEIGHT)
            };
            let colons = self.peek();
            if colons.kind != TokenKind::ColonColon {
                let found = self.describe(colons);
                return Err(self.error(
                    format!("expected `::` and a variant's name after the enum, found {found}"),
                    error_span(colons),
                ));
            }
            self.advance();
            let variant = self.peek();
            let TokenKind::Ident(variant_name) = variant.kind else {
                let found = self.describe(variant);
                return Err(self.error(
                    format!("expected the variant's name after `::`, found {found}"),
                    error_span(variant),
                ));
            };
            self.advance();
            let (payload, height) = self.parse_enum_type_arg(of_height)?;
            let end = payload.map_or(variant.span, |payload| self.type_span(payload));
            let item = self.builder.alloc_item(Item {
                kind: ItemKind::Variant {
                    of,
                    name: variant_name,
                    name_span: variant.span,
                    payload,
                },
                span: Span::join(keyword.span, end),
            });
            return Ok((item, height));
        }
        self.advance();
        let word = self.peek();
        let extensible =
            matches!(word.kind, TokenKind::Ident(text) if self.names.get(text) == EXTENSIBLE);
        if !extensible {
            let found = self.describe(word);
            return Err(self.report(
                Diagnostic::error(
                    format!("expected `extensible` after the enum's name, found {found}"),
                    error_span(word),
                )
                .with_note("only an extensible enum is declared: `enum NAME extensible<BOUND>`"),
            ));
        }
        self.advance();
        let (bound, height) = self.parse_enum_type_arg(LEAF_HEIGHT)?;
        let end = bound.map_or(word.span, |bound| self.type_span(bound));
        let item = self.builder.alloc_item(Item {
            kind: ItemKind::Enum {
                name,
                name_span: name_token.span,
                bound,
            },
            span: Span::join(keyword.span, end),
        });
        Ok((item, height))
    }

    /// Parses an enum's bound or a variant's payload, `<TYPE>`, if a `<` is
    /// next: one type, newlines allowed inside, and the `>` directly after
    /// it. Returns it, if there is one, and the item's height, one more than
    /// the type's, or `at_least`.
    pub(super) fn parse_enum_type_arg(
        &mut self,
        at_least: u32,
    ) -> ParseResult<(Option<TypeExprId>, u32)> {
        let open = self.peek();
        if open.kind != TokenKind::Lt {
            return Ok((None, at_least));
        }
        self.advance();
        self.skip_newlines();
        let (ty, ty_height) = self.parse_type()?;
        self.skip_newlines();
        let close = self.peek();
        if close.kind != TokenKind::Gt {
            return Err(self.report(
                Diagnostic::error("expected `>` to close the type", error_span(close))
                    .with_context(open.span, "unclosed `<`"),
            ));
        }
        self.advance();
        Ok((Some(ty), ty_height.saturating_add(1).max(at_least)))
    }

    /// Whether the tokens here start a variant as a value, `ENUM::NAME` or
    /// `t.ENUM::NAME`: names joined by `.`, then `::` (see [Enums](#enums)).
    pub(super) fn starts_variant(&self) -> bool {
        let mut at = 0;
        loop {
            if !matches!(self.peek_at(at).kind, TokenKind::Ident(_)) {
                return false;
            }
            match self.peek_at(at + 1).kind {
                TokenKind::ColonColon => return true,
                TokenKind::Dot => at += 2,
                _ => return false,
            }
        }
    }

    /// Whether the tokens here, which [`Parser::starts_variant`] has found
    /// to start `DT::V`, go on with a name on the variant's line: a
    /// reference to a definition made with the variant, `Toolchain::C gcc`
    /// (see [Def-types](crate::parser::deftypes#def-types)).
    pub(super) fn starts_variant_ref(&self) -> bool {
        let mut at = 0;
        while self.peek_at(at + 1).kind == TokenKind::Dot {
            at += 2;
        }
        let variant = self.peek_at(at + 2);
        matches!(variant.kind, TokenKind::Ident(_))
            && self.is_name_after(variant.span, self.peek_at(at + 3))
    }

    /// Parses a reference to a definition made with a variant, `DT::V
    /// PATH`, which [`Parser::starts_variant_ref`] has found here (see
    /// [Def-types](crate::parser::deftypes#def-types)): its def-type, a name or a dotted path read
    /// as a value, then the variant and the reference.
    pub(super) fn parse_variant_ref(&mut self) -> ParseResult<ExprId> {
        let mut dt = Vec::new();
        while let token = self.peek()
            && let TokenKind::Ident(name) = token.kind
        {
            dt.push(PathSegment {
                kind: SegmentKind::Name(name),
                span: token.span,
            });
            self.advance();
            if self.peek().kind != TokenKind::Dot {
                break;
            }
            self.advance();
        }
        let dt_expr = self.path_expr(&dt)?;
        let variant = self.parse_def_type_variant()?;
        self.parse_def_type_ref(dt_expr, variant)
    }

    /// Parses a variant as a value, `ENUM::NAME`, which
    /// [`Parser::starts_variant`] has found here (see [Enums](#enums)), or a
    /// reference made with one, `ENUM::NAME PATH`.
    pub(super) fn parse_variant_expr(&mut self) -> ParseResult<ExprId> {
        if self.starts_variant_ref() {
            return self.parse_variant_ref();
        }
        let first = self.peek();
        let of = if self.peek_at(1).kind == TokenKind::Dot {
            self.parse_type_path()?.0
        } else {
            let TokenKind::Ident(name) = first.kind else {
                return Err(self.error("expected an enum's name", first.span));
            };
            self.advance();
            self.builder.alloc_type(TypeExpr {
                kind: TypeExprKind::Named(name),
                span: first.span,
            })
        };
        // `::`, which `starts_variant` found.
        self.advance();
        let variant = self.peek();
        let TokenKind::Ident(name) = variant.kind else {
            let found = self.describe(variant);
            return Err(self.error(
                format!("expected the variant's name after `::`, found {found}"),
                error_span(variant),
            ));
        };
        self.advance();
        let span = Span::join(first.span, variant.span);
        self.alloc(ExprKind::Variant { of, name }, span, 1)
    }
}
