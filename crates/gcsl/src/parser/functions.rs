//! Parsing functions and calls: named functions, methods, function expressions,
//! parameters, defaults, `lazy`, and the arguments of a call.
//!
//! # Functions and calls
//!
//! A named function item, `fn NAME` or `def NAME`, may be generic: type
//! parameters, `<T, U>`, directly after its name, names separated by `,`,
//! each once, "type parameter `T` is declared twice" otherwise
//! (`Parser::parse_type_params`). They are recorded with the function
//! ([`Ast::generics`](super::Ast::generics)), with the type expressions its signature and body
//! are made of, where they are in scope ([`Ast::type_param`](super::Ast::type_param)): a name
//! written as a type there that is one of them is the parameter, whatever
//! type of its name is declared.
//!
//! A parameter written `...NAME: TYPE` is the *rest parameter*
//! ([`Param::rest`](crate::ast::Param::rest)), which the positional arguments
//! from its position on fill; it has no default, "a rest parameter has no
//! default", noted that given no arguments it is the empty list. A function
//! type's parameter may be one too, `fn(...words: String): Int`. At a call, an
//! argument `...EXPR` is a spread argument
//! ([`ArgKind::Spread`](crate::ast::ArgKind::Spread)), a list whose elements
//! fill the rest parameter. That a function has one rest parameter at most, and
//! what a call gives it, is the checker's.
//!
//! A function expression is `fn`, `(`, parameters `NAME: TYPE` separated by
//! `,` with an optional trailing `,`, `)`, the return marker, the return
//! type, and the body, a block whose `{` must follow the return type
//! directly, as an `if`'s block follows its condition. Every parameter is
//! annotated. A parameter may have a default, `NAME: TYPE = EXPR`, any
//! expression after the `=` as an argument is (`Parser::parse_default`),
//! in any position, in a `fn` expression, a named `fn` item, a `def` and a
//! def-type's `def` alike, since each is parsed as one ([`Param::default`]).
//! Once the parameters and the return marker are parsed, each default that
//! is no constant ([`crate::ast::is_constant`]) is given its *default
//! function* ([`Param::default_fn`], `Parser::with_default_fns`): a
//! [`FnDecl`] with no expression of its own, whose parameters are copies of
//! the ones before it, as written, without their defaults, whose return
//! type is the parameter's, whose effect is the function's (a memoised
//! one's is impure), whose marker is the function's, and whose body is the
//! default, as a `lazy`'s body is an expression. It adds no syntax: the
//! default is still where it is written, and the formatter and the index see
//! it there. How it is checked, and run, is the checker's business (see
//! [`crate::check`], "Calls"). Not in a function type,
//! nor in the parameters of a definition made with a def-type, which are
//! names alone ("expected `,` or `)`" at an `=` there, as before). The
//! marker is one of three, each an [`Effect`] of the
//! [`FnDecl`] and of a function type alike: `:` for a pure function, `->>`
//! for an impure one that is memoised, run at most once per distinct
//! arguments, and `->` for an impure one run on every call.
//!
//! A call is a postfix operator: after an operand, `(` starts an argument
//! list, whose arguments are separated by `,` with an optional trailing
//! `,`, each an expression or `NAME = EXPR` (recognised by an identifier
//! followed by `=`). After a named argument, an argument that is a name
//! alone, an identifier directly followed by `,` or `)`, is shorthand for
//! `NAME = NAME`, as in `gcc.compile(src, output = "a.o", compiler)`: a
//! named argument ([`ArgKind::Named`], marked `shorthand`) whose value is
//! the read of the variable of its name, at the same span. Before any named
//! argument a name alone is positional, as any expression is, and after one
//! any other expression is positional still (`f(a = 1, 2)`), so only a
//! program that passed a variable positionally, by its name alone, after a
//! named argument means something else: now the argument of the parameter
//! of the variable's name, an error if there is none. After the `)`, or
//! instead of the list, a `{` on the same line starts a trailing block: an
//! ordinary block, passed as the call's last argument. The parser accepts
//! arguments in any order; binding them to parameters is the checker's
//! business.
//!
//! # Methods
//!
//! A method, `fn (RECV: TYPE) NAME(PARAMS) MARKER RET { BODY }` or the
//! same with `def` (`Parser::parse_method`), is a function called on a value
//! of its receiver's type, `xs.unique()`, rather than by a name: `fn` or
//! `def`, the receiver in parentheses, one parameter written as any is but
//! with no default, the method's name, its type parameters if it is
//! generic, and then a function expression's parameters, marker, return
//! type and body. Its item is a `let` of a name no source can write
//! ([`crate::names::method_binding`]), its value the function, whose first
//! parameter is the receiver, and the tree records it as a method
//! ([`MethodDecl`], [`Ast::methods`](super::Ast::methods)); a type parameter is in scope in the
//! receiver too. `fn (` or `def (` starts a method when the token after
//! the matching `)` is a name, so `fn (x: Int): Int { x }` is still a
//! function expression. A method is an item of a file's outermost body only:
//! elsewhere it is "a method is declared only at the top level of a file",
//! at its keyword. A receiver list of no parameter is "expected the
//! method's receiver", one of more than one "a method has one receiver",
//! at the second, and a receiver with a default "a receiver takes no
//! default", at the `=`.
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

use super::Sync;
use crate::ast::{
    Arg, ArgKind, Effect, ExprId, ExprKind, FnDecl, GenericOwner, Generics, ItemId, ItemKind,
    MethodDecl, Param, PathSegment, SegmentKind,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::Name;
use crate::source::Span;

use super::{
    Braces, Entry, MISPLACED_META, MUT_PARAM_NOTE, PLAIN_VARIABLE_NAME, ParseResult, Parser, Stop,
    error_span,
};

impl Parser<'_> {
    /// Parses a named function item, `fn NAME(...) ...` or `def NAME(...)
    /// ...`, starting at its keyword, `keyword`, followed by `name` and its
    /// span. After `fn`, a `let` of the name, and after `def`, a field of
    /// the name, whose value is the function expression from `(` on, both
    /// spanning the keyword through the body's `}`; either marked as
    /// written as a named function, not as `let NAME = fn` or `NAME = fn`.
    pub(super) fn parse_named_fn(
        &mut self,
        keyword: Token,
        (name, name_span): (Name, Span),
    ) -> ParseResult<(ItemId, u32)> {
        self.advance();
        self.advance();
        let params = if self.peek().kind == TokenKind::Lt {
            Some(self.parse_type_params()?)
        } else {
            None
        };
        let types = self.builder.type_count();
        let value = self.parse_fn(keyword)?;
        if let Some(params) = params
            && let ExprKind::Fn(function) = self.builder.get(value).kind
        {
            self.builder.push_generics(Generics {
                owner: GenericOwner::Fn(function),
                params,
                types: (types, self.builder.type_count()),
            });
        }
        let base = self.path.len();
        self.path.push(PathSegment {
            kind: SegmentKind::Name(name),
            span: name_span,
        });
        let (path, shape) = self.finish_path(base);
        let kind = if keyword.kind == TokenKind::Def {
            ItemKind::Field {
                path,
                ty: None,
                value,
                function: true,
            }
        } else {
            ItemKind::Let {
                path,
                ty: None,
                value,
                mutable: false,
                function: true,
            }
        };
        self.finish_item(keyword, (kind, value), 0, shape)
    }

    /// Whether the tokens from `index` start a method (see
    /// [Methods](#methods)): `fn` or `def`, `(`, and, after the `)` that
    /// closes it, a name.
    pub(super) fn starts_method(&self, index: usize) -> bool {
        if !matches!(self.token_at(index).kind, TokenKind::Fn | TokenKind::Def)
            || self.token_at(index + 1).kind != TokenKind::LParen
        {
            return false;
        }
        let mut depth = 0usize;
        let mut at = index + 1;
        loop {
            match self.token_at(at).kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.token_at(at + 1).kind, TokenKind::Ident(_));
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }
            at += 1;
        }
    }

    /// Parses a method, starting at its keyword, `keyword`, `fn` or `def`
    /// (see [Methods](#methods)): its receiver, its name, its type
    /// parameters and the rest of its function.
    pub(super) fn parse_method(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        // The type parameters are in scope from the receiver on.
        let types = self.builder.type_count();
        let receiver = self.parse_receiver()?;
        let name_token = self.peek();
        let TokenKind::Ident(name) = name_token.kind else {
            let found = self.describe(name_token);
            return Err(self.error(
                format!("expected the method's name after its receiver, found {found}"),
                error_span(name_token),
            ));
        };
        self.advance();
        let params = if self.peek().kind == TokenKind::Lt {
            Some(self.parse_type_params()?)
        } else {
            None
        };
        let value = self.parse_fn_with(keyword, Some(receiver))?;
        if let Some(params) = params
            && let ExprKind::Fn(function) = self.builder.get(value).kind
        {
            self.builder.push_generics(Generics {
                owner: GenericOwner::Fn(function),
                params,
                types: (types, self.builder.type_count()),
            });
        }
        // The first ordinal of the name no method has yet, of this input or
        // of an earlier one sharing the table.
        let method = self.names.get(name).to_owned();
        let mut ordinal = 0;
        let text = loop {
            let text = crate::names::method_binding(&method, ordinal);
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
        let (item, height) = self.finish_item(keyword, (kind, value), 0, shape)?;
        self.builder.push_method(MethodDecl {
            item,
            name,
            name_span: name_token.span,
            binding,
            public: keyword.kind == TokenKind::Def,
        });
        Ok((item, height))
    }

    /// Parses a method's receiver, at its `(`: one parameter, with no
    /// default, and the `)` (see [Methods](#methods)).
    pub(super) fn parse_receiver(&mut self) -> ParseResult<Param> {
        let open = self.peek();
        self.advance();
        if self.peek().kind == TokenKind::RParen {
            return Err(self.error("expected the method's receiver", self.peek().span));
        }
        let base = self.param_stack.len();
        let mut tallest = 0;
        let closed = self.parse_param(open, &mut tallest)?;
        let receiver = self.param_stack.drain(base..).next();
        let Some(receiver) = receiver else {
            return Err(self.error("expected the method's receiver", open.span));
        };
        if !closed {
            let next = self.peek();
            return Err(self.error("a method has one receiver", error_span(next)));
        }
        if let Some(default) = receiver.default {
            return Err(self.error("a receiver takes no default", self.span_of(default)));
        }
        Ok(receiver)
    }

    /// Parses the type parameters of a generic function item, `<T, U>`, at
    /// its `<`: one name or more, separated by `,`, as type arguments are,
    /// each once, "type parameter `T` is declared twice" otherwise.
    pub(super) fn parse_type_params(&mut self) -> ParseResult<Box<[(Name, Span)]>> {
        self.advance();
        let mut params: Vec<(Name, Span)> = Vec::new();
        loop {
            let token = self.peek();
            let TokenKind::Ident(name) = token.kind else {
                let found = self.describe(token);
                return Err(self.error(
                    format!("expected a type parameter's name, found {found}"),
                    token.span,
                ));
            };
            if params.iter().any(|&(param, _)| param == name) {
                let text = self.names.get(name).to_owned();
                return Err(self.error(
                    format!("type parameter `{text}` is declared twice"),
                    token.span,
                ));
            }
            self.advance();
            params.push((name, token.span));
            let next = self.peek();
            match next.kind {
                TokenKind::Comma => self.advance(),
                TokenKind::Gt => {
                    self.advance();
                    return Ok(params.into_boxed_slice());
                }
                _ => {
                    let found = self.describe(next);
                    return Err(self.error(
                        format!("expected `,` or `>` after a type parameter, found {found}"),
                        next.span,
                    ));
                }
            }
        }
    }

    /// Parses a function expression in expression position, starting at
    /// its `fn`. A name after the `fn` is an error here: a named function
    /// is an item.
    pub(super) fn parse_fn_expr(&mut self) -> ParseResult<ExprId> {
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
    pub(super) fn parse_fn(&mut self, keyword: Token) -> ParseResult<ExprId> {
        self.parse_fn_with(keyword, None)
    }

    /// As [`Parser::parse_fn`], with a method's `receiver`, if it is one's,
    /// its first parameter (see [Methods](#methods)).
    pub(super) fn parse_fn_with(
        &mut self,
        keyword: Token,
        receiver: Option<Param>,
    ) -> ParseResult<ExprId> {
        let open = self.expect_fn_paren()?;
        let base = self.param_stack.len();
        self.param_stack.extend(receiver);
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
            match self.parse_param(open, &mut tallest) {
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
        let own: Vec<Param> = self.param_stack.drain(base..).collect();
        let (effect, marker) = self.return_marker()?;
        let own = self.with_default_fns(own, (effect, marker));
        let params = self.builder.push_params(&own);
        let (ret, ret_height) = self.parse_type()?;
        // Only a function type has a meta.
        if self.meta_marker().is_some() {
            let span = Span::join(self.peek().span, self.peek_at(1).span);
            return Err(self.error(MISPLACED_META, span));
        }
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

    /// The parameters `params` of a function expression of effect `effect`,
    /// each whose default is no constant ([`crate::ast::is_constant`]) given
    /// its default function (see [Functions and calls](#functions-and-calls)):
    /// a function of the parameters before it, as they are written, with no
    /// defaults of their own, whose result type is the parameter's, whose
    /// effect is the function's, a memoised one's impure, whose marker is
    /// the function's, `marker`, so that an error of its effect points where
    /// the function declares it, and whose body is the default.
    pub(super) fn with_default_fns(
        &mut self,
        mut params: Vec<Param>,
        (effect, marker): (Effect, Span),
    ) -> Vec<Param> {
        let effect = match effect {
            Effect::Pure => Effect::Pure,
            Effect::Memo | Effect::Impure => Effect::Impure,
        };
        for at in 0..params.len() {
            let Some(default) = params.get(at).and_then(|param| param.default) else {
                continue;
            };
            if crate::ast::is_constant(&self.builder, default) {
                continue;
            }
            let earlier: Vec<Param> = params
                .iter()
                .take(at)
                .map(|param| Param {
                    default: None,
                    default_fn: None,
                    ..*param
                })
                .collect();
            let ty = params.get(at).map(|param| param.ty);
            let earlier = self.builder.push_params(&earlier);
            let function = self.builder.push_fn(FnDecl {
                params: earlier,
                ret: ty,
                effect,
                marker,
                body: default,
            });
            if let Some(param) = params.get_mut(at) {
                param.default_fn = Some(function);
            }
        }
        params
    }

    /// Parses a parameter of a function expression whose parameter list
    /// `open` opens, `NAME: TYPE` and its default, if it has one, and the
    /// `,` or the `)` after it, raising `tallest` to the height of its type
    /// and its default. Returns whether the `)` ends the list.
    pub(super) fn parse_param(&mut self, open: Token, tallest: &mut u32) -> ParseResult<bool> {
        let mut token = self.peek();
        if token.kind == TokenKind::Caret {
            return self.parse_context_param(open, tallest);
        }
        // A rest parameter, `...words: String`.
        let dots = (token.kind == TokenKind::DotDotDot).then_some(token.span);
        if dots.is_some() {
            self.advance();
            token = self.peek();
        }
        let name = match token.kind {
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
        *tallest = (*tallest).max(height);
        if dots.is_some() && self.peek().kind == TokenKind::Eq {
            return Err(self.report(
                Diagnostic::error("a rest parameter has no default", self.peek().span)
                    .with_note("given no arguments, it is the empty list"),
            ));
        }
        let default = self.parse_default()?;
        if let Some(default) = default {
            *tallest = (*tallest).max(self.height_of(default));
        }
        self.param_stack.push(Param {
            name,
            span: Span::join(dots.unwrap_or(token.span), self.type_span(ty)),
            ty,
            default,
            default_fn: None,
            rest: dots.is_some(),
        });
        self.list_separator(open)
    }

    /// Parses a parameter's default, `= EXPR`, if the current token, after
    /// the parameter's type, is its `=`: the expression, any expression,
    /// parsed at minimum binding power 0 as an argument is, inside the
    /// parameters' parentheses; `None` if no `=` follows.
    pub(super) fn parse_default(&mut self) -> ParseResult<Option<ExprId>> {
        if self.peek().kind != TokenKind::Eq {
            return Ok(None);
        }
        self.advance();
        // A default is inside the parameters' parentheses.
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let default = self.parse_expr(0);
        self.braces = outer;
        default.map(Some)
    }

    /// Parses `lazy EXPR`, starting at its `lazy` keyword: the operand,
    /// parsed at minimum binding power 0, becomes the body of a function
    /// with no parameters and no return type, impure, whose marker is the
    /// keyword. The expression spans the keyword through the operand.
    pub(super) fn parse_lazy(&mut self) -> ParseResult<ExprId> {
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
    pub(super) fn parse_call(&mut self, callee: ExprId) -> ParseResult<ExprId> {
        let base = self.arg_stack.len();
        let mut tallest = self.height_of(callee);
        let open = self.peek();
        let mut end = open.span;
        if open.kind == TokenKind::LParen {
            self.advance();
            // An argument list is inside parentheses.
            let outer = std::mem::replace(&mut self.braces, Braces::Free);
            let close = self.parse_arguments(open, &mut tallest);
            self.braces = outer;
            let Some(close) = close? else {
                // Never closed: what was parsed of it is kept, for an
                // editor, through the last token it went on to.
                let args = self.builder.push_args(&self.arg_stack[base..]);
                self.arg_stack.truncate(base);
                let last = self.token_at(self.pos.saturating_sub(1)).span;
                let last = if last.end > open.span.end {
                    last
                } else {
                    open.span
                };
                let span = Span::join(self.span_of(callee), last);
                return self.alloc(ExprKind::Unclosed { callee, args }, span, tallest + 1);
            };
            end = close;
        }
        if self.peek().kind == TokenKind::LBrace && self.braces == Braces::Free {
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
    /// by `=` names its argument. An argument that fails is skipped to its
    /// `,`, nothing of it left in the tree, and the list goes on, to fail at
    /// its end (see [Errors and recovery](crate::parser::recovery#errors-and-recovery)); but a list
    /// that ends where no `)` is, at the end of the input or at the `}`
    /// around it, which closes it, is the unclosed call's, `None`, the
    /// arguments parsed pushed.
    pub(super) fn parse_arguments(
        &mut self,
        open: Token,
        tallest: &mut u32,
    ) -> ParseResult<Option<Span>> {
        let mut failed = None;
        // Whether a named argument has been read: after one, a name alone
        // is the shorthand, and anything else positional an error.
        let mut named = false;
        loop {
            let start = self.pos;
            let token = self.peek();
            if token.kind == TokenKind::RParen {
                self.advance();
                return failed.map_or(Ok(Some(token.span)), Err);
            }
            let marks = self.marks();
            let (reported, kept) = match self.parse_argument(open, tallest, &mut named) {
                Ok(Some(close)) => return failed.map_or(Ok(Some(close)), Err),
                Ok(None) => continue,
                Err(Entry::Separator(reported)) => (reported, true),
                Err(Entry::Value(reported)) => (reported, false),
            };
            if !kept {
                self.restore(marks);
            }
            // As `recover_entry`, knowing whether the list ended at its `)`.
            if self.deep {
                return Err(reported);
            }
            match self.skip(
                start,
                Sync::Entry {
                    close: TokenKind::RParen,
                },
            ) {
                Stop::Separator => self.advance(),
                Stop::Close => {
                    self.advance();
                    return Err(reported);
                }
                Stop::Outer => return Ok(None),
            }
            failed = Some(reported);
        }
    }

    /// Parses an argument of a call whose argument list `open` opens, and
    /// the `,` or the `)` after it, raising `tallest` to its height, and
    /// `named` if it is named. Returns the span of the `)` if it ends the
    /// list; if what fails is the separator, the argument is pushed. After a
    /// named argument (`named`), a name alone is the shorthand for `NAME =
    /// NAME` (see [Functions and calls](#functions-and-calls)).
    pub(super) fn parse_argument(
        &mut self,
        open: Token,
        tallest: &mut u32,
        named: &mut bool,
    ) -> Result<Option<Span>, Entry> {
        let token = self.peek();
        let next = self.peek_at(1).kind;
        let kind = match (token.kind, next) {
            (TokenKind::DotDotDot, _) => {
                self.advance();
                ArgKind::Spread { span: token.span }
            }
            (TokenKind::Ident(name), TokenKind::Eq) => {
                self.advance();
                self.advance();
                ArgKind::Named {
                    name,
                    span: token.span,
                    shorthand: false,
                }
            }
            (TokenKind::Ident(name), TokenKind::Comma | TokenKind::RParen) if *named => {
                ArgKind::Named {
                    name,
                    span: token.span,
                    shorthand: true,
                }
            }
            _ => ArgKind::Positional,
        };
        *named |= matches!(kind, ArgKind::Named { .. });
        let value = self.parse_expr(0).map_err(Entry::Value)?;
        *tallest = (*tallest).max(self.height_of(value));
        self.arg_stack.push(Arg { kind, value });
        let close = self.peek();
        let closed = self.list_separator(open).map_err(Entry::Separator)?;
        Ok(closed.then_some(close.span))
    }
}
