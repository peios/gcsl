//! Parsing type expressions and `type` items: names, applied types, function
//! types and record types with their defaults.
//!
//! # Types
//!
//! A type is a name, such as `Int`, a qualified name, `t.PackageDef`, a
//! name applied to a type argument, such
//! as `Future<Int>` or `List<Int>`, a record type, a function type, a type
//! in parentheses, a list type written `T[]`, or a union of types. A
//! qualified name is an identifier directly followed by `.`, and then
//! identifiers separated by `.` ("expected a type's name after `.`"
//! otherwise), a [`TypeExprKind::Path`]; what it names is the workspace's
//! business (see [`crate::workspace`], "Types"). So `x : Int.y`, which was
//! the field access `(x : Int).y` and could only be an error, is now the
//! ascription of a qualified name. A
//! record type is `{` then `}`, or entries separated by `,`, each `NAME:
//! TYPE`, `NAME: TYPE = DEFAULT`, a spread, `...TYPE`, whose fields join
//! the record type, or a rest-field, `*: TYPE`, the type of every field the
//! record type does not name, with an optional trailing `,`, then `}`. A
//! rest-field has no default: an `=` after its type is "a rest-field cannot
//! have a default"; how many a record type has is the checker's business.
//! A field's
//! default, [`FieldDefault::Expr`], is any expression, after the `=`, as a
//! parameter's default is (`Parser::parse_field_default`), wherever the
//! record type is written: in an annotation, a `type`, a parameter's or a
//! return type, a list or function type, nested in another record type or
//! in a union; what a default may be is the checker's business (see its
//! "Field defaults"). A spread has none: an `=` after one is "expected
//! `,` or `}`", as before. The lexer does not know it is
//! in a type, so it emits the newlines inside the braces, and the parser
//! skips them all, around a default's `=` too: a record type may span lines
//! freely. Repeated field names
//! are left to the checker, and the merging of spreads to the expansion of
//! the file's types (see [`crate::ast`], "Types"). A function type is
//! `fn`, `(`, parameter types
//! separated by `,` with an optional trailing `,`, `)`, the marker `:`,
//! `->>` or `->`, and the return type; a parameter type is `NAME: TYPE`,
//! recognised
//! by an identifier followed by `:`, or just `TYPE`. A parameter type has no
//! default: an `=` after one is "a default is written only in a function
//! expression's parameters", at the `=` (see [Functions and
//! calls](crate::parser::functions#functions-and-calls)).
//!
//! A function type may have a *meta* (see the checker's "Metas"): after its
//! whole return type, a union included, the identifier `META` directly
//! followed by `:`, `->>` or `->`, its marker, then the meta's type, newlines
//! skipped after the marker, as in `fn(v: Version): PackageDef META->
//! PackageMeta` ([`MetaTy`], `Parser::meta_marker`). `META` is no keyword:
//! anywhere else it is a name, and followed by anything else after a return
//! type, it is what it always was; a newline outside parentheses ends the
//! type first, as it always did, so a meta begins on the line its return type
//! ends on. Since the innermost return type takes it, the meta of a function
//! type that returns a function type is written with that return type in
//! parentheses, `fn(): (fn(): Int) META: {a: Int}`. Only a function type has
//! a meta: `META` and a marker after the return type of a function
//! expression (a `fn` or a `def` included), or after the parameters of a
//! definition made with a def-type, are "a meta is written only in a
//! function type", at `META` and its marker. No program that parsed had
//! `META` and a marker there, so every one parses as it did.
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
//! A named type may be applied to type arguments, `NAME<TYPE, ...>` or
//! `a.NAME<TYPE, ...>` (`Parser::parse_type_args`): one argument or more,
//! separated by `,`, newlines allowed after the `<` and each `,`, and the
//! `>` directly after the last ("expected `>` to close the type
//! arguments" otherwise), whatever the name; how many each type takes is
//! the checker's and the workspace's business.
//!
//! A `type` item, `type NAME = TYPE` ([`ItemKind::Type`]), declares a type
//! (`Parser::parse_type_item`), its name a single identifier, newlines
//! skipped after its `=`. `type` is no keyword of the lexer: it is
//! contextual, a `type` item's start only where an item starts and an
//! identifier follows it on its line (`Parser::starts_type_item`), which no
//! item could before, so every program that parsed parses as it did, and a
//! variable or field may still be named `type` (`type = 1`, `let type =
//! 2`, `type.x = 3`, `type: Int = 4`). A `type` item is an item of a file's
//! outermost body only: anywhere else, a block's, a function's or a
//! `lazy`'s, it is "`type` is declared only at the top level of a file", at
//! the `type`. A `type` may be generic: type parameters after its name,
//! `type Pair<A, B> = {first: A, second: B}`, as a function item's are
//! (`Parser::parse_type_params`), in scope in its definition
//! ([`Ast::type_param`](super::Ast::type_param)). Anything else after the name is "expected `=`
//! after the type's name". `#[shadowable]` before one is the error it is
//! before anything but a field, a `def` or an import. Whether a name is
//! some built-in type's, or declared twice, is the checker's business.

use crate::MAX_DEPTH;
use crate::ast::{
    Effect, ExprId, ExprKind, FieldDefault, FieldTy, FieldTyKind, GenericOwner, Generics, Item,
    ItemId, ItemKind, MetaTy, ParamTy, PathSegment, SegmentKind, TypeArgRange, TypeExpr,
    TypeExprId, TypeExprKind, path_span,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::Name;
use crate::source::Span;

use super::{
    Braces, LEAF_HEIGHT, META, MISPLACED_DEFAULT, MISSING_RETURN_TYPE, Nesting, Open, ParseResult,
    Parser, RETURN_TYPE_NOTE, Reported, TYPE, error_span, nest,
};

impl Parser<'_> {
    /// Whether the current token begins a `type` item: the identifier
    /// `type`, the contextual keyword, directly followed on its line by an
    /// identifier (see [Types](#types)).
    pub(super) fn starts_type_item(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(name) if self.names.get(name) == TYPE)
            && matches!(self.peek_at(1).kind, TokenKind::Ident(_))
    }

    /// Parses a `type` item of the file's body, `type NAME = TYPE`,
    /// starting at its `type`, `keyword`, which [`Parser::starts_type_item`]
    /// has found to be followed by an identifier. Returns it and its height,
    /// its type's. Type parameters, `type Box<T> = ...`, are an error at
    /// their `<`.
    pub(super) fn parse_type_item(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let name_token = self.peek();
        let TokenKind::Ident(name) = name_token.kind else {
            return Err(self.error("expected the type's name after `type`", name_token.span));
        };
        self.advance();
        let params = match self.peek().kind {
            TokenKind::Lt => Some(self.parse_type_params()?),
            _ => None,
        };
        let eq = self.peek();
        if eq.kind != TokenKind::Eq {
            let found = self.describe(eq);
            return Err(self.error(
                format!("expected `=` after the type's name, found {found}"),
                error_span(eq),
            ));
        }
        self.advance();
        self.skip_newlines();
        let first = self.builder.type_count();
        let (ty, height) = self.parse_type()?;
        let span = Span::join(keyword.span, self.type_span(ty));
        let item = self.builder.alloc_item(Item {
            kind: ItemKind::Type {
                name,
                name_span: name_token.span,
                ty,
            },
            span,
        });
        if let Some(params) = params {
            self.builder.push_generics(Generics {
                owner: GenericOwner::Type(item),
                params,
                types: (first, self.builder.type_count()),
            });
        }
        Ok((item, height))
    }

    /// Parses a type expression: a name, an applied type, a record type, a
    /// function type or a parenthesised type, or a union of such types.
    /// Returns it and its height: 1 for a name, one more than its argument
    /// for an applied type, one more than its tallest field's type for a
    /// record type, one more than the tallest of its parameter and return
    /// types for a function type, one more than the type inside for a
    /// parenthesised type, and one more than the taller of its two sides
    /// for a union.
    pub(super) fn parse_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
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
    pub(super) fn parse_type_member(&mut self) -> ParseResult<(TypeExprId, u32)> {
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
    pub(super) fn parse_type_atom(&mut self) -> ParseResult<(TypeExprId, u32)> {
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
            TokenKind::Ident(_) if self.peek_at(1).kind == TokenKind::Dot => self.parse_type_path(),
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

    /// Parses a qualified type name, `a.b` or `a.b.c`, starting at its first
    /// name, an identifier directly followed by `.`: identifiers separated by
    /// `.`, each `.` followed by an identifier ("expected a type's name after
    /// `.`" otherwise), and, after a `<`, its type arguments
    /// ([`Parser::parse_type_args`]). Returns it, a leaf one level tall
    /// without arguments.
    pub(super) fn parse_type_path(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let base = self.path.len();
        let mut token = self.peek();
        loop {
            let TokenKind::Ident(name) = token.kind else {
                self.path.truncate(base);
                return Err(self.error("expected a type's name after `.`", error_span(token)));
            };
            self.advance();
            self.path.push(PathSegment {
                kind: SegmentKind::Name(name),
                span: token.span,
            });
            if self.peek().kind != TokenKind::Dot {
                break;
            }
            self.advance();
            token = self.peek();
        }
        let span = path_span(self.path.get(base..).unwrap_or_default());
        let (path, _) = self.finish_path(base);
        if self.peek().kind == TokenKind::Lt {
            // Applied types nest by recursion, so they count their depth.
            self.enter()?;
            let applied = self.parse_type_args(span);
            self.depth -= 1;
            let (args, close, height) = applied?;
            let id = self.builder.alloc_type(TypeExpr {
                kind: TypeExprKind::PathApplied { path, args },
                span: Span::join(span, close),
            });
            return Ok((id, height));
        }
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Path(path),
            span,
        });
        Ok((id, LEAF_HEIGHT))
    }

    /// Parses a parenthesised type, starting at its `(`: the `(`, newlines,
    /// the type, and the `)`. Returns the type inside itself, with a height
    /// one more than its own, since the parentheses leave no node of their
    /// own.
    pub(super) fn parse_paren_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
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
    /// current token and is followed by `<`: the name, then its type
    /// arguments ([`Parser::parse_type_args`]).
    pub(super) fn parse_applied_type(
        &mut self,
        name: Name,
        name_span: Span,
    ) -> ParseResult<(TypeExprId, u32)> {
        self.advance();
        let (args, close, height) = self.parse_type_args(name_span)?;
        let span = Span::join(name_span, close);
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Applied {
                name,
                span: name_span,
                args,
            },
            span,
        });
        Ok((id, height))
    }

    /// Parses the type arguments of an applied type, at their `<`, whose
    /// name starts at `start`: the `<`, newlines, one type argument or more
    /// separated by `,`, newlines allowed after each `,`, and the `>`, which
    /// must follow the last argument directly. Returns the arguments, where
    /// the `>` is, and the applied type's height, one more than its tallest
    /// argument's.
    pub(super) fn parse_type_args(
        &mut self,
        start: Span,
    ) -> ParseResult<(TypeArgRange, Span, u32)> {
        let open = self.peek();
        self.advance();
        self.skip_newlines();
        let mut args = Vec::new();
        let mut tallest = 0;
        let close = loop {
            let (arg, arg_height) = self.parse_type()?;
            args.push(arg);
            tallest = tallest.max(arg_height);
            let next = self.peek();
            match next.kind {
                TokenKind::Comma => {
                    self.advance();
                    self.skip_newlines();
                }
                TokenKind::Gt => break next,
                _ => {
                    return Err(self.report(
                        Diagnostic::error(
                            "expected `>` to close the type arguments",
                            error_span(next),
                        )
                        .with_context(open.span, "unclosed `<`"),
                    ));
                }
            }
        };
        self.advance();
        let height = tallest.saturating_add(1);
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(Span::join(start, close.span)));
        }
        Ok((self.builder.push_type_args(&args), close.span, height))
    }

    /// Parses a function type, starting at its `fn`: `(`, parameter types
    /// separated by `,` with an optional trailing `,`, each `NAME: TYPE` or
    /// `TYPE`, `)`, the marker `:` or `->`, and the return type.
    pub(super) fn parse_fn_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let keyword = self.peek();
        self.advance();
        let open = self.expect_fn_paren()?;
        let base = self.param_ty_stack.len();
        let mut tallest = 0;
        // A parameter that fails is skipped to its `,`, and the list goes
        // on, to fail at its end (see [Errors and
        // recovery](crate::parser::recovery#errors-and-recovery)).
        let mut failed = None;
        loop {
            let start = self.pos;
            if self.peek().kind == TokenKind::RParen {
                self.advance();
                break;
            }
            match self.parse_param_ty(open, &mut tallest) {
                Ok(true) => break,
                Ok(false) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RParen) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        }
        if let Some(reported) = failed {
            return Err(reported);
        }
        let params = self.builder.push_param_tys(&self.param_ty_stack[base..]);
        self.param_ty_stack.truncate(base);
        let (effect, _) = self.return_marker()?;
        let (ret, ret_height) = self.parse_type()?;
        // The return type, a union included, and then its meta, if `META`
        // and a marker follow it.
        let (meta, end, meta_height) = match self.meta_marker() {
            Some(meta_effect) => {
                self.advance();
                self.advance();
                self.skip_newlines();
                let (ty, height) = self.parse_type()?;
                let meta = MetaTy {
                    effect: meta_effect,
                    ty,
                };
                (Some(meta), ty, height)
            }
            None => (None, ret, 0),
        };
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Fn {
                params,
                ret,
                effect,
                meta,
            },
            span: Span::join(keyword.span, self.type_span(end)),
        });
        Ok((id, tallest.max(ret_height).max(meta_height) + 1))
    }

    /// Parses a parameter of a function type whose parameter list `open`
    /// opens, `NAME: TYPE` or `TYPE`, and the `,` or the `)` after it,
    /// raising `tallest` to the height of its type. Returns whether the `)`
    /// ends the list.
    pub(super) fn parse_param_ty(&mut self, open: Token, tallest: &mut u32) -> ParseResult<bool> {
        let token = self.peek();
        let name = match (token.kind, self.peek_at(1).kind) {
            (TokenKind::Ident(name), TokenKind::Colon) => {
                self.advance();
                self.advance();
                Some(name)
            }
            _ => None,
        };
        let (ty, height) = self.parse_type()?;
        *tallest = (*tallest).max(height);
        // Only a function expression's parameter has a default.
        let eq = self.peek();
        if eq.kind == TokenKind::Eq {
            return Err(self.error(MISPLACED_DEFAULT, eq.span));
        }
        self.param_ty_stack.push(ParamTy { name, ty });
        self.list_separator(open)
    }

    /// The effect of the meta whose marker the current token begins, if it
    /// does: `META` followed by `:`, `->>` or `->` (see [Types](#types)).
    /// `META` followed by anything else is a name.
    pub(super) fn meta_marker(&self) -> Option<Effect> {
        let TokenKind::Ident(name) = self.peek().kind else {
            return None;
        };
        if self.names.get(name) != META {
            return None;
        }
        match self.peek_at(1).kind {
            TokenKind::Colon => Some(Effect::Pure),
            TokenKind::MemoArrow => Some(Effect::Memo),
            TokenKind::Arrow => Some(Effect::Impure),
            _ => None,
        }
    }

    /// Consumes the `(` after `fn`, returning it, or reports what is there
    /// instead.
    pub(super) fn expect_fn_paren(&mut self) -> ParseResult<Token> {
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
    pub(super) fn list_separator(&mut self, open: Token) -> ParseResult<bool> {
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
                Err(self.report_close(
                    Diagnostic::error(
                        format!("expected `,` or `)`, found {found}"),
                        error_span(separator),
                    )
                    .with_context(open.span, "unclosed `(`"),
                ))
            }
        }
    }

    /// The field access of `record` whose `.`, `dot`, is followed by no
    /// name but by `next`: "expected a field name after `.`", and, as an
    /// editor leaves one half-typed, if `next` may end it there
    /// ([`Parser::ends_unfinished`]), a [`ExprKind::NoField`] of the record,
    /// spanning it through the `.`; else the error.
    pub(super) fn no_field(
        &mut self,
        record: ExprId,
        dot: Token,
        next: Token,
    ) -> ParseResult<ExprId> {
        let reported = self.error("expected a field name after `.`", error_span(next));
        if !self.ends_unfinished(next.kind) {
            return Err(reported);
        }
        let span = Span::join(self.span_of(record), dot.span);
        let height = self.height_of(record) + 1;
        self.alloc(ExprKind::NoField(record), span, height)
    }

    /// Whether a field access whose `.` is followed by the token of `kind`,
    /// no name, ends there as an editor leaves one half-typed, `a.` (see
    /// [Errors and recovery](crate::parser::recovery#errors-and-recovery)): what may end an
    /// expression where it is, in the innermost bracket open, the lexer's
    /// way ([`nest`]): the end of a line or of the input, a `;` or the `}`
    /// of a block (which closes a `(` or `[` open in it too), the `)` or `]`
    /// of the bracket open, or a `,` in one. Costs a walk of the tokens
    /// before it, on an error only.
    pub(super) fn ends_unfinished(&self, kind: TokenKind) -> bool {
        let mut nesting = Nesting::default();
        for index in 0..self.pos {
            nest(&mut nesting, self.token_at(index).kind);
        }
        let open = &nesting.open;
        let inner = open.last().copied();
        match kind {
            TokenKind::Eof => true,
            TokenKind::Newline | TokenKind::Semi => matches!(inner, None | Some(Open::Brace)),
            TokenKind::RBrace => {
                inner != Some(Open::Str)
                    && open
                        .get(nesting.strings.last().map_or(0, |&at| at + 1)..)
                        .is_some_and(|inside| inside.contains(&Open::Brace))
            }
            TokenKind::RParen => inner == Some(Open::Paren),
            TokenKind::RBracket => inner == Some(Open::Square),
            TokenKind::Comma => matches!(inner, Some(Open::Paren | Open::Square)),
            _ => false,
        }
    }

    /// Reports `diagnostic`, of a list's separator or a closing bracket
    /// missing at a token, unless an error is reported at that token
    /// already: a field access's with no name before it, `f(a.` at the end
    /// of the input or before the `}` around it, whose error is there, and
    /// the bracket is reported unclosed by it.
    pub(super) fn report_close(&mut self, diagnostic: Diagnostic) -> Reported {
        if self
            .diagnostics
            .last()
            .is_some_and(|last| last.span == diagnostic.span)
        {
            return Reported;
        }
        self.report(diagnostic)
    }

    /// Consumes the return marker of a function or function type, `:` for a
    /// pure one, `->>` for a memoised one and `->` for an impure one, and
    /// the newlines after it. Returns the effect it declares, and the
    /// marker's span.
    pub(super) fn return_marker(&mut self) -> ParseResult<(Effect, Span)> {
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
    pub(super) fn parse_record_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let open = self.peek();
        self.advance();
        let base = self.field_ty_stack.len();
        let mut tallest = 0;
        // A field that fails is skipped to its `,`, and the type goes on, to
        // fail at its end (see [Errors and recovery](crate::parser::recovery#errors-and-recovery)).
        let mut failed = None;
        let close = loop {
            self.skip_newlines();
            let start = self.pos;
            let token = self.peek();
            if token.kind == TokenKind::RBrace {
                self.advance();
                break token;
            }
            match self.parse_field_ty(&mut tallest) {
                Ok(Some(close)) => break close,
                Ok(None) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RBrace) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        };
        if let Some(reported) = failed {
            return Err(reported);
        }
        let fields = self.builder.push_field_tys(&self.field_ty_stack[base..]);
        self.field_ty_stack.truncate(base);
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Record(fields),
            span: Span::join(open.span, close.span),
        });
        Ok((id, tallest + 1))
    }

    /// Parses an entry of a record type, a field, `NAME: TYPE` with its
    /// default if it has one, a spread, `...TYPE`, or a rest-field, `*:
    /// TYPE`, and the `,` or the `}` after it, newlines skipped, raising
    /// `tallest` to the height of its type and its default. Returns the `}`
    /// if it ends the type. A rest-field has no default: an `=` after its
    /// type is "a rest-field cannot have a default".
    pub(super) fn parse_field_ty(&mut self, tallest: &mut u32) -> ParseResult<Option<Token>> {
        let token = self.peek();
        let kind = match token.kind {
            // A spread, `...TYPE`.
            TokenKind::DotDotDot => {
                self.advance();
                self.skip_newlines();
                FieldTyKind::Spread
            }
            // A rest-field, `*: TYPE`.
            TokenKind::Star => {
                self.advance();
                self.skip_newlines();
                self.expect_field_colon("expected `:` after `*`")?;
                FieldTyKind::Rest
            }
            _ => {
                let Some(name) = self.field_name()? else {
                    return Err(
                        self.error("expected a field name in record type", error_span(token))
                    );
                };
                self.skip_newlines();
                self.expect_field_colon("expected `:` after field name")?;
                FieldTyKind::Field(name)
            }
        };
        let (ty, height) = self.parse_type()?;
        *tallest = (*tallest).max(height);
        self.skip_newlines();
        // A field's default; a spread has none, nor may a rest-field.
        let default = match kind {
            FieldTyKind::Field(_) => self.parse_field_default()?,
            FieldTyKind::Rest => {
                let eq = self.peek();
                if eq.kind == TokenKind::Eq {
                    return Err(self.error("a rest-field cannot have a default", error_span(eq)));
                }
                None
            }
            FieldTyKind::Spread => None,
        };
        if let Some(default) = default {
            *tallest = (*tallest).max(self.height_of(default));
        }
        self.field_ty_stack.push(FieldTy {
            kind,
            span: token.span,
            ty,
            default: default.map(FieldDefault::Expr),
        });
        self.skip_newlines();
        let separator = self.peek();
        match separator.kind {
            // A `}` after it ends the type at the top of the loop.
            TokenKind::Comma => {
                self.advance();
                Ok(None)
            }
            TokenKind::RBrace => {
                self.advance();
                Ok(Some(separator))
            }
            _ => Err(self.error("expected `,` or `}`", error_span(separator))),
        }
    }

    /// Consumes the `:` after a record type's field name or `*`, and the
    /// newlines after it; `message` at the current token if it is not one.
    pub(super) fn expect_field_colon(&mut self, message: &str) -> ParseResult<()> {
        let colon = self.peek();
        if colon.kind != TokenKind::Colon {
            return Err(self.error(message, error_span(colon)));
        }
        self.advance();
        self.skip_newlines();
        Ok(())
    }

    /// Parses a field's default in a record type, `= EXPR`, if the current
    /// token, after the field's type and any newlines, is its `=`: the
    /// newlines after the `=`, and the expression, any expression, parsed
    /// at minimum binding power 0 as a parameter's default is, inside the
    /// type's braces, where a `{` after an expression on its line starts a
    /// trailing block; `None` if no `=` follows.
    pub(super) fn parse_field_default(&mut self) -> ParseResult<Option<ExprId>> {
        if self.peek().kind != TokenKind::Eq {
            return Ok(None);
        }
        self.advance();
        self.skip_newlines();
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let default = self.parse_expr(0);
        self.braces = outer;
        default.map(Some)
    }
}
