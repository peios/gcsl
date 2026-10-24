//! Parsing attributes: outer and inner attributes, their arguments, and what
//! they attach to.
//!
//! # Attributes
//!
//! An *attribute* is a token of its own (see the lexer's
//! [Attributes](crate::lexer#attributes)): `#[NAME]` an outer one, which
//! applies to the item after it, and `#![NAME]` an inner one, which applies
//! to the whole file. Four names are the parser's own, which take no
//! arguments, "`#[shadowable]` takes no arguments" otherwise; three are
//! these, and `#[attribute]` is below:
//!
//! - `#[shadowable]` marks a top-level field (a dotted one included), a
//!   `def`, a definition made with a def-type or an `import` of any form, as
//!   one a later declaration of its name may shadow (see
//!   [`crate::workspace`], "Shadowable declarations"), recorded in the
//!   tree's side table ([`Ast::is_shadowable`](super::Ast::is_shadowable)). It applies to the next
//!   item, on its line or a later one, comments and blank lines between
//!   being skipped as any separators are.
//! - `#[def_type]` marks a top-level `def`, of one name or a path, as a
//!   def-type (see [Def-types](crate::parser::deftypes#def-types)), recorded in a side table of its
//!   own ([`Ast::is_def_type`](super::Ast::is_def_type)). It applies to the next item as
//!   `#[shadowable]` does, and both may mark one item, in either order;
//!   before anything but a top-level `def` (a definition made with a
//!   def-type, a field, a `fn`, an item of an inner block) it is
//!   "`#[def_type]` applies only to a file's top-level `def`s", at the
//!   attribute, and its other errors are `#[shadowable]`'s, with its name.
//! - `#![no_prelude]` marks the file as one without the implicit `import *
//!   from prelude` ([`Ast::no_prelude`](super::Ast::no_prelude)). It must come before the file's
//!   first item, comments and blank lines alone before it.
//!
//! The errors, each at the attribute: an unknown inner name is "unknown
//! attribute `NAME`"; `#![shadowable]` is "`shadowable` applies to a declaration:
//! write `#[shadowable]`" and `#[no_prelude]` "`no_prelude` applies to the
//! whole file: write `#![no_prelude]`"; one written twice on one item is
//! "`#[shadowable]` is written twice", and twice on the file "`#![no_prelude]`
//! is written twice", each at the second; `#![no_prelude]` anywhere but
//! before the file's first item (after an item or an outer attribute, or in
//! a block) is "`#![no_prelude]` must come before anything else in the
//! file"; `#[shadowable]` before anything but a top-level field, `def` or
//! `import` (a `let`, `let mut`, `fn`, `:=`, a `type`, a spread, a result,
//! an expression, a `break`, `continue` or `return`, or any item of an
//! inner block or of a `for` header) is "`#[shadowable]` applies only to a
//! file's
//! top-level fields, `def`s and imports", with the note "a `let` or `fn`
//! already shadows" before a `let`, a `let mut` or a `fn`; and one with no
//! item after it, at the end of its body, is "`#[shadowable]` is not
//! followed by anything". An attribute where an expression or a type
//! starts, or after an item on its line, is reported as any token is there.
//! Whether a session's input may be marked `#![no_prelude]` is the
//! workspace's to say: the parser does not know what it parses.
//!
//! `#[attribute]` marks a top-level `type` as an attribute's declaration
//! ([`Ast::is_attribute`](super::Ast::is_attribute)), applying to the next item as `#[shadowable]`
//! does; before anything else it is "`#[attribute]` applies only to a
//! file's top-level `type`s".
//!
//! Any other outer name, `#[NAME]` or `#[NAME(ARGS)]`, is a *use* of an
//! attribute, which the checker resolves (see the checker's
//! [Attributes](crate::check::attributes#attributes)): its arguments are parsed as a
//! call's are, positional or named, and its `]` must follow its `)`,
//! "expected `]` after the attribute's arguments" otherwise. A use applies
//! to the item after it, which must be a function item, a named `fn` at any
//! level, a `def` or a definition made with a def-type: before anything
//! else it is "attributes apply to functions: a `fn`, a `def` or a
//! definition". One name used twice on an item is "`#[retry]` is written
//! twice". The uses are kept by item, in source order
//! ([`Ast::attribute_uses`](super::Ast::attribute_uses)), each with its name read as a type's, a
//! [`TypeExprKind::Named`] at the name, so that the workspace resolves and
//! expands it as it does any type's. An attribute with arguments is consumed
//! through its `]` whatever goes wrong in it; an inner one with arguments is
//! "an inner attribute takes no arguments".

use super::{ATTRIBUTE, DEF_TYPE, SHADOWABLE};
use crate::ast::{ArgRange, AttributeUse, ItemId, ItemKind, TypeExpr, TypeExprKind};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::Name;
use crate::source::Span;

use super::{
    Braces, LET_SHADOWS_NOTE, MISPLACED_ATTRIBUTE, MISPLACED_DEF_TYPE, MISPLACED_NO_PRELUDE,
    MISPLACED_SHADOWABLE, MISPLACED_USE, NO_PRELUDE, ParseResult, Parser, Pending, Reported,
};

impl Parser<'_> {
    /// Takes `token`, at the start of an item of a body, if it is an
    /// attribute: an outer one as an attribute of the item after it,
    /// `pending`, and an inner one as the file's, if `first`, before the
    /// file's first item and any outer attribute (see
    /// [Attributes](#attributes)). Returns whether it failed, which makes
    /// the item after it an error item; `None` if it is no attribute.
    pub(super) fn body_attribute(
        &mut self,
        token: Token,
        pending: &mut Pending,
        first: bool,
    ) -> Option<bool> {
        let failed = match token.kind {
            TokenKind::Attribute(name) => {
                let failed = self.outer_attribute(token, name, pending).is_err();
                if failed {
                    self.advance();
                }
                failed
            }
            TokenKind::InnerAttribute(name) => {
                let failed = self
                    .inner_attribute(token, name, first && pending.is_empty())
                    .is_err();
                if failed {
                    self.advance();
                }
                failed
            }
            TokenKind::AttributeArgs(name) => {
                self.attribute_with_args(token, name, pending).is_err()
            }
            TokenKind::InnerAttributeArgs(_) => {
                self.error("an inner attribute takes no arguments", token.span);
                self.skip_attribute();
                true
            }
            _ => return None,
        };
        Some(failed)
    }

    /// Takes the outer attribute `token`, named `name`, at the start of an
    /// item of a body, as an attribute of the item after it, `pending` (see
    /// [Attributes](#attributes)): `#[shadowable]`, `#[def_type]` and
    /// `#[attribute]` are the parser's, each once; any other name is a use,
    /// once on an item, of an attribute the checker resolves.
    pub(super) fn outer_attribute(
        &mut self,
        token: Token,
        name: Name,
        pending: &mut Pending,
    ) -> ParseResult<()> {
        let text = self.names.get(name).to_owned();
        let slot = match text.as_str() {
            SHADOWABLE => &mut pending.shadowable,
            DEF_TYPE => &mut pending.def_type,
            ATTRIBUTE => &mut pending.attribute,
            NO_PRELUDE => return Err(self.misplaced_attribute(token)),
            _ => {
                let used = self.attribute_use(token, (name, &text), token.span, None);
                self.pend_use(used, pending)?;
                self.advance();
                return Ok(());
            }
        };
        if slot.is_some() {
            return Err(self.error(format!("`#[{text}]` is written twice"), token.span));
        }
        *slot = Some(token.span);
        self.advance();
        Ok(())
    }

    /// The use of the attribute `token`, named `name`, spelt `text`, written
    /// at `span` with the arguments `args`, for the item after it to take:
    /// its name read as a type's, at the name, which is written after its
    /// `#[` and any spaces or tabs.
    pub(super) fn attribute_use(
        &mut self,
        token: Token,
        (name, text): (Name, &str),
        span: Span,
        args: Option<ArgRange>,
    ) -> AttributeUse {
        let written = self.source.slice(token.span);
        let offset = written.find(text).unwrap_or_default();
        let start = token.span.start + u32::try_from(offset).unwrap_or_default();
        let name_span = Span::new(start, start + u32::try_from(text.len()).unwrap_or_default());
        let ty = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Named(name),
            span: name_span,
        });
        AttributeUse {
            item: ItemId::PENDING,
            name,
            span,
            name_span,
            args,
            ty,
        }
    }

    /// Takes the use `used` of an attribute that is none of the parser's
    /// as one of `pending`: "`#[retry]` is written twice" if one of its name
    /// is waiting already.
    pub(super) fn pend_use(
        &mut self,
        used: AttributeUse,
        pending: &mut Pending,
    ) -> ParseResult<()> {
        if pending.uses.iter().any(|earlier| earlier.name == used.name) {
            let text = self.names.get(used.name).to_owned();
            return Err(self.error(format!("`#[{text}]` is written twice"), used.span));
        }
        pending.uses.push(used);
        Ok(())
    }

    /// Takes the outer attribute with arguments `token`, `#[NAME(ARGS)]`,
    /// named `name`, as [`Parser::outer_attribute`] takes one without: its
    /// arguments parsed as a call's are, positional or named, and its `]`
    /// after them. None of the parser's own takes arguments: "`#[shadowable]`
    /// takes no arguments". The attribute is consumed through its `]`
    /// whatever goes wrong, so the item after it is parsed as ever.
    pub(super) fn attribute_with_args(
        &mut self,
        token: Token,
        name: Name,
        pending: &mut Pending,
    ) -> ParseResult<()> {
        let text = self.names.get(name).to_owned();
        if matches!(
            text.as_str(),
            SHADOWABLE | DEF_TYPE | ATTRIBUTE | NO_PRELUDE
        ) {
            let reported = self.error(format!("`#[{text}]` takes no arguments"), token.span);
            self.skip_attribute();
            return Err(reported);
        }
        self.advance();
        let open = self.peek();
        if open.kind != TokenKind::LParen {
            let reported = self.error("expected `(`", open.span);
            self.skip_attribute_rest();
            return Err(reported);
        }
        self.advance();
        let base = self.arg_stack.len();
        let mut tallest = 0;
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let closed = self.parse_arguments(open, &mut tallest);
        self.braces = outer;
        let args = self.builder.push_args(&self.arg_stack[base..]);
        self.arg_stack.truncate(base);
        let bracket = self.peek();
        match closed {
            Ok(Some(_)) if bracket.kind == TokenKind::RBracket => {
                self.advance();
                let span = Span::join(token.span, bracket.span);
                let used = self.attribute_use(token, (name, &text), span, Some(args));
                self.pend_use(used, pending)
            }
            Ok(Some(_)) => {
                let found = self.describe(bracket);
                let reported = self.error(
                    format!("expected `]` after the attribute's arguments, found {found}"),
                    bracket.span,
                );
                self.skip_attribute_rest();
                Err(reported)
            }
            Ok(None) | Err(_) => {
                self.skip_attribute_rest();
                Err(closed.err().unwrap_or(Reported))
            }
        }
    }

    /// Skips the attribute with arguments at the current token, through the
    /// `]` closing it.
    pub(super) fn skip_attribute(&mut self) {
        self.advance();
        self.skip_attribute_rest();
    }

    /// Skips the rest of an attribute with arguments whose `#[` is behind,
    /// through the `]` closing it: brackets nested in its arguments are
    /// matched, and the end of the input stops it.
    pub(super) fn skip_attribute_rest(&mut self) {
        let mut depth = 0u32;
        loop {
            match self.peek().kind {
                TokenKind::Eof => return,
                TokenKind::LBracket | TokenKind::AttributeArgs(_) => depth += 1,
                TokenKind::RBracket if depth == 0 => {
                    self.advance();
                    return;
                }
                TokenKind::RBracket => depth -= 1,
                _ => {}
            }
            self.advance();
        }
    }

    /// Marks `item`, an item of a body that is the file's if `top`, with the
    /// `#[attribute]` written at `attribute` before it, if it is a top-level
    /// `type`; else the error that it is not one (see
    /// [Attributes](#attributes)).
    pub(super) fn attach_attribute(
        &mut self,
        attribute: Span,
        item: ItemId,
        top: bool,
    ) -> ParseResult<()> {
        if top && matches!(self.builder.item(item).kind, ItemKind::Type { .. }) {
            self.builder.mark_attribute(item);
            return Ok(());
        }
        Err(self.error(MISPLACED_ATTRIBUTE, attribute))
    }

    /// Gives `item` the uses `uses` of attributes that are none of the
    /// parser's, if it is a function item: a named `fn`, a `def` or a
    /// definition made with a def-type, at any level; else the error, at the
    /// first use, that attributes apply to functions (see
    /// [Attributes](#attributes)).
    pub(super) fn attach_uses(&mut self, uses: Vec<AttributeUse>, item: ItemId) -> ParseResult<()> {
        let function = matches!(
            self.builder.item(item).kind,
            ItemKind::Let { function: true, .. }
                | ItemKind::Field { function: true, .. }
                | ItemKind::DefTypeDef { .. }
        );
        if !function {
            let at = uses
                .first()
                .map_or(self.builder.item(item).span, |used| used.span);
            return Err(self.error(MISPLACED_USE, at));
        }
        for used in uses {
            self.builder
                .push_attribute_use(AttributeUse { item, ..used });
        }
        Ok(())
    }

    /// Takes the inner attribute `token`, named `name`, at the start of an
    /// item of a body, as the file's, if it is `first`: before the file's
    /// first item and any outer attribute (see [Attributes](#attributes)).
    pub(super) fn inner_attribute(
        &mut self,
        token: Token,
        name: Name,
        first: bool,
    ) -> ParseResult<()> {
        if self.names.get(name) != NO_PRELUDE || !first {
            return Err(self.misplaced_attribute(token));
        }
        if self.builder.no_prelude().is_some() {
            return Err(self.error("`#![no_prelude]` is written twice", token.span));
        }
        self.builder.set_no_prelude(token.span);
        self.advance();
        Ok(())
    }

    /// Reports the attribute `token` where it cannot be: by its name, an
    /// unknown one, one of the other kind, or, where it is, one that is not
    /// allowed there (see [Attributes](#attributes)).
    pub(super) fn misplaced_attribute(&mut self, token: Token) -> Reported {
        let (inner, name) = match token.kind {
            TokenKind::Attribute(name) | TokenKind::AttributeArgs(name) => (false, name),
            TokenKind::InnerAttribute(name) | TokenKind::InnerAttributeArgs(name) => (true, name),
            _ => return self.error("expected an attribute", token.span),
        };
        let text = self.names.get(name).to_owned();
        let message = match (text.as_str(), inner) {
            (SHADOWABLE | DEF_TYPE | ATTRIBUTE, true) => {
                format!("`{text}` applies to a declaration: write `#[{text}]`")
            }
            (NO_PRELUDE, false) => {
                "`no_prelude` applies to the whole file: write `#![no_prelude]`".to_owned()
            }
            (SHADOWABLE, false) => MISPLACED_SHADOWABLE.to_owned(),
            (DEF_TYPE, false) => MISPLACED_DEF_TYPE.to_owned(),
            (ATTRIBUTE, false) => MISPLACED_ATTRIBUTE.to_owned(),
            (NO_PRELUDE, true) => MISPLACED_NO_PRELUDE.to_owned(),
            // Any other outer one is a use, which applies to functions.
            (_, false) => MISPLACED_USE.to_owned(),
            (_, true) => format!("unknown attribute `{text}`"),
        };
        self.error(message, token.span)
    }

    /// Marks `item`, an item of a body that is the file's if `top`, with the
    /// `#[def_type]` written at `attribute` before it, if it is a top-level
    /// `def` (not a definition made with a def-type); else the error that it
    /// is not one (see [Attributes](#attributes)).
    pub(super) fn attach_def_type(
        &mut self,
        attribute: Span,
        item: ItemId,
        top: bool,
    ) -> ParseResult<()> {
        let kind = self.builder.item(item).kind;
        if top && matches!(kind, ItemKind::Field { function: true, .. }) {
            self.builder.mark_def_type(item);
            return Ok(());
        }
        Err(self.error(MISPLACED_DEF_TYPE, attribute))
    }

    /// Marks `item`, an item of a body that is the file's if `top`, with the
    /// `#[shadowable]` written at `attribute` before it, if it is a
    /// top-level field, `def` or `import`; else the error that it is not one
    /// (see [Attributes](#attributes)).
    pub(super) fn attach_shadowable(
        &mut self,
        attribute: Span,
        item: ItemId,
        top: bool,
    ) -> ParseResult<()> {
        let kind = self.builder.item(item).kind;
        if top
            && matches!(
                kind,
                ItemKind::Field { .. } | ItemKind::Import(_) | ItemKind::DefTypeDef { .. }
            )
        {
            self.builder.mark_shadowable(item);
            return Ok(());
        }
        // A meta is its definition's: a definition that shadows another
        // brings its own.
        if matches!(kind, ItemKind::DefMeta { .. }) {
            return Err(self.error("`#[shadowable]` does not apply to a `defmeta`", attribute));
        }
        let mut error = Diagnostic::error(MISPLACED_SHADOWABLE, attribute);
        if matches!(kind, ItemKind::Let { .. }) {
            error = error.with_note(LET_SHADOWS_NOTE);
        }
        Err(self.report(error))
    }
}
