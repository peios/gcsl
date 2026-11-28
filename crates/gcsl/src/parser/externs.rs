//! Parsing `extern` items: the handle types and the operations a host of
//! the language gives a program.
//!
//! # Host operations
//!
//! `extern`, which is no keyword, starts an item where an item of a file's
//! outermost body starts and it is directly followed on its line by `fn`, by
//! `def`, or by the word `type` and a name (`Parser::starts_extern_item`).
//! Each of those was a syntax error before there were `extern` items, but
//! one: `extern type with { ... }`, the reference to the definition `type`
//! made with a def-type named `extern`, under bindings of contexts, which is
//! still that. Everywhere else `extern` is the identifier it always was: a
//! variable, a field, a def-type. In a block it starts no item, and is
//! parsed as that identifier, as before.
//!
//! ```text
//! HANDLE    = "extern" "type" IDENT ("=" STRING)?
//! OPERATION = "extern" ("fn" | "def") RECEIVER? NAME PARAMS "->" TYPE ("=" STRING)?
//! ```
//!
//! `extern type NAME` declares a *handle type* ([`ExternType`]): a nominal
//! type with no fields whose values only the host makes, an
//! [`ItemKind::Enum`] with no bound written. `= "KIND"` gives the kind's
//! name as the host knows it; left out, the kind is `NAME`. Type parameters
//! are "a handle type has no type parameters", at their `<`.
//!
//! `extern fn` or `extern def` declares an *operation* ([`ExternFn`]): a
//! function with a signature and no body, whose calls the host answers.
//! Its signature is any function item's: named `fn NAME`, the method `fn
//! (RECV: TYPE) NAME`, or public with `def`, by a name or a dotted path,
//! its parameters with their defaults, a rest parameter, context
//! parameters. `= "NAME"` after the return type, on its line, gives the
//! operation's name as the host knows it; left out, it is the function's
//! own name, the last of a dotted path's. The item spans `extern` through
//! the return type, or the name's string.
//!
//! The parser gives an operation its body (`Parser::operation_body`): a
//! block whose one item is the call of an [`ExprKind::Operation`], written
//! nowhere, with two arguments, the operation's name as a string literal
//! and a list literal of a read of each parameter, in order, a method's
//! receiver first. So the function is a function item like any other to
//! every later stage: called by name with its defaults, passed, stored.
//! What the call means is the checker's (see
//! [Host operations](crate::check::operations#host-operations)).
//!
//! What an operation cannot be is an error where it is written:
//!
//! - a body, "an operation has no body", at its `{`, labelled at the
//!   `extern`;
//! - a marker other than `->`, "an operation is declared with `->`", at
//!   the marker: an operation is an effect, so it is impure, and it is
//!   never memoised;
//! - type parameters, "an operation has no type parameters", at their `<`;
//! - a block parameter, "an operation takes no block", at its type;
//! - a definition made with a def-type, `extern def package x()`, "a
//!   definition made with a def-type is not `extern`", at the `extern`;
//! - `#[def_type]` on one, "`#[def_type]` does not apply to an operation",
//!   at the attribute;
//! - a name, of a kind or of an operation, that is empty, "a handle type's
//!   kind is not empty" and "an operation's name is not empty", or that is
//!   an interpolated string, "a handle type's kind is plain text" and "an
//!   operation's name is plain text", at the string; and anything but a
//!   string after the `=`, "expected the operation's name, a string, after
//!   `=`".

use crate::ast::{
    Arg, ArgKind, Element, ExprId, ExprKind, ExternFn, ExternType, Item, ItemId, ItemKind, Param,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::Name;
use crate::source::Span;
use crate::strings::StrId;

use super::{EXTERN, LEAF_HEIGHT, ParseResult, Parser, TYPE, WITH, error_span};

/// The error for an operation with a body, at the body's `{`.
const OPERATION_BODY: &str = "an operation has no body";

/// The note of [`OPERATION_BODY`].
const OPERATION_BODY_NOTE: &str =
    "the host runs an operation: declare it by its signature alone, `extern fn NAME(...) -> T`";

/// The error for an operation declared with `:` or `->>`, at the marker.
const OPERATION_MARKER: &str = "an operation is declared with `->`";

/// The note of [`OPERATION_MARKER`].
const OPERATION_MARKER_NOTE: &str = "an operation is an effect of the host's, run on every call: \
                                     it is not pure, and it is never memoised; a memoised \
                                     function may call one";

/// The error for an operation with type parameters, at their `<`.
const OPERATION_GENERIC: &str = "an operation has no type parameters";

/// The note of [`OPERATION_GENERIC`].
const OPERATION_GENERIC_NOTE: &str =
    "the host is given plain data and handles, whose types the declaration says";

/// The error for an operation with a block parameter, at its type.
const OPERATION_BLOCK: &str = "an operation takes no block";

/// The note of [`OPERATION_BLOCK`].
const OPERATION_BLOCK_NOTE: &str = "the host is given plain data and handles; write a function that takes the block and calls \
     the operation";

/// The error for `extern` on a definition made with a def-type, at the
/// `extern`.
const EXTERN_DEFINITION: &str = "a definition made with a def-type is not `extern`";

/// The note of [`EXTERN_DEFINITION`].
const EXTERN_DEFINITION_NOTE: &str = "its def-type says what a definition is; declare the operation with `extern def NAME(...)` \
     and call it from the definition";

/// The error for `#[def_type]` on an operation, at the attribute.
pub(super) const EXTERN_DEF_TYPE: &str = "`#[def_type]` does not apply to an operation";

/// The note of [`EXTERN_DEF_TYPE`].
pub(super) const EXTERN_DEF_TYPE_NOTE: &str =
    "a def-type is called with each definition made with it, which the host knows nothing of";

/// An operation being parsed: what [`Parser::parse_extern_item`] leaves for
/// the function item's parser, which takes it up at the function's `(`.
#[derive(Clone, Copy, Debug)]
pub(super) struct External {
    /// Where the `extern` is written.
    pub(super) word: Span,
    /// The function's own name and where it is written, once it is read.
    pub(super) name: Option<(Name, Span)>,
}

/// An operation whose own name is read: what [`Parser::operation_body`]
/// takes of an [`External`].
#[derive(Clone, Copy, Debug)]
pub(super) struct Named {
    /// Where the `extern` is written.
    word: Span,
    /// The function's own name.
    name: Name,
    /// Where the function's own name is written.
    declared: Span,
}

/// What is said of a name after an `=`, a kind's or an operation's.
#[derive(Clone, Copy)]
struct Naming {
    /// "a handle type's kind" or "an operation's name".
    what: &'static str,
    /// "the kind's name" or "the operation's name".
    expected: &'static str,
}

/// How a handle type's kind is named.
const KIND: Naming = Naming {
    what: "a handle type's kind",
    expected: "the kind's name",
};

/// How an operation is named.
const OPERATION: Naming = Naming {
    what: "an operation's name",
    expected: "the operation's name",
};

impl Parser<'_> {
    /// Whether the tokens here start an `extern` item (see [Host
    /// operations](#host-operations)): the word `extern`, directly followed
    /// on its line by `fn`, by `def`, or by the word `type` and a name, but
    /// `extern type with {`, which binds contexts around a reference made
    /// with a def-type named `extern`.
    pub(super) fn starts_extern_item(&self) -> bool {
        let is = |token: Token, word: &str| matches!(token.kind, TokenKind::Ident(name) if self.names.get(name) == word);
        if !is(self.peek(), EXTERN) {
            return false;
        }
        let next = self.peek_at(1);
        if matches!(next.kind, TokenKind::Fn | TokenKind::Def) {
            return true;
        }
        let name = self.peek_at(2);
        is(next, TYPE)
            && matches!(name.kind, TokenKind::Ident(_))
            && !(is(name, WITH) && self.peek_at(3).kind == TokenKind::LBrace)
    }

    /// Parses an `extern` item of the file's body, starting at its
    /// `extern`, `word`, which [`Parser::starts_extern_item`] has found
    /// (see [Host operations](#host-operations)): a handle type, or an
    /// operation, parsed as the function item it is, a named function, a
    /// method or a `def`, whose keyword is taken to start at the `extern`,
    /// so that the item and its function span it.
    pub(super) fn parse_extern_item(&mut self, word: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let keyword = self.peek();
        if !matches!(keyword.kind, TokenKind::Fn | TokenKind::Def) {
            return self.parse_extern_type(word);
        }
        let wide = Token {
            kind: keyword.kind,
            span: Span::join(word.span, keyword.span),
        };
        self.external = Some(External {
            word: word.span,
            name: None,
        });
        let name = self.peek_at(1);
        let parsed = if self.starts_method(self.pos) {
            self.parse_method(wide)
        } else if keyword.kind == TokenKind::Def {
            self.parse_def(wide)
        } else if let TokenKind::Ident(text) = name.kind {
            self.parse_named_fn(wide, (text, name.span))
        } else {
            let found = self.describe(name);
            Err(self.error(
                format!("expected the operation's name after `extern fn`, found {found}"),
                error_span(name),
            ))
        };
        self.external = None;
        parsed
    }

    /// Parses a handle type, `extern type NAME` and its kind, if `=`
    /// follows, the `extern`, `word`, consumed (see [Host
    /// operations](#host-operations)).
    fn parse_extern_type(&mut self, word: Token) -> ParseResult<(ItemId, u32)> {
        // `type`, which `starts_extern_item` found.
        self.advance();
        let name_token = self.peek();
        let TokenKind::Ident(name) = name_token.kind else {
            return Err(self.error("expected the type's name after `type`", name_token.span));
        };
        self.advance();
        if self.peek().kind == TokenKind::Lt {
            return Err(self.report(
                Diagnostic::error("a handle type has no type parameters", self.peek().span)
                    .with_note("a handle stands for one thing of the host's, of one kind"),
            ));
        }
        let (kind, kind_span) = if let Some((kind, span)) = self.external_name(KIND)? {
            (kind, Some(span))
        } else {
            let text = self.names.get(name).to_owned();
            (self.strings.push(&text), None)
        };
        let item = self.builder.alloc_item(Item {
            kind: ItemKind::Enum {
                name,
                name_span: name_token.span,
                bound: None,
            },
            span: Span::join(word.span, kind_span.unwrap_or(name_token.span)),
        });
        self.builder.push_extern_type(ExternType {
            item,
            keyword: word.span,
            kind,
            kind_span,
        });
        Ok((item, LEAF_HEIGHT))
    }

    /// Parses the name a host knows a handle type's kind or an operation
    /// by, `= "NAME"`, if the current token is its `=` (see [Host
    /// operations](#host-operations)): its text and where the string is
    /// written; `None` if no `=` follows. A name that is empty, an
    /// interpolated string, or no string, is an error, worded as `naming`
    /// says.
    fn external_name(&mut self, naming: Naming) -> ParseResult<Option<(StrId, Span)>> {
        if self.peek().kind != TokenKind::Eq {
            return Ok(None);
        }
        self.advance();
        let token = self.peek();
        let Naming { what, expected } = naming;
        match token.kind {
            TokenKind::Str(text) if self.strings.get(text).is_empty() => Err(self.report(
                Diagnostic::error(format!("{what} is not empty"), token.span)
                    .with_note("leave the `=` and the string out for the declaration's own name"),
            )),
            TokenKind::Str(text) => {
                self.advance();
                Ok(Some((text, token.span)))
            }
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                Err(self.report(
                    Diagnostic::error(format!("{what} is plain text"), span).with_note(
                        "the host knows it by this text, which is the same wherever the \
                         declaration is read",
                    ),
                ))
            }
            _ => {
                let found = self.describe(token);
                Err(self.error(
                    format!("expected {expected}, a string, after `=`, found {found}"),
                    error_span(token),
                ))
            }
        }
    }

    /// Reports the type parameters at the current token, a `<`, of the
    /// operation being parsed, if one is (see [Host
    /// operations](#host-operations)).
    pub(super) fn no_external_generics(&mut self) -> ParseResult<()> {
        if self.external.is_none() || self.peek().kind != TokenKind::Lt {
            return Ok(());
        }
        Err(self.report(
            Diagnostic::error(OPERATION_GENERIC, self.peek().span)
                .with_note(OPERATION_GENERIC_NOTE),
        ))
    }

    /// Reports the `extern` of the operation being parsed, if one is, as
    /// one on a definition made with a def-type (see [Host
    /// operations](#host-operations)).
    pub(super) fn no_external_definition(&mut self) -> ParseResult<()> {
        let Some(external) = self.external else {
            return Ok(());
        };
        Err(self.report(
            Diagnostic::error(EXTERN_DEFINITION, external.word).with_note(EXTERN_DEFINITION_NOTE),
        ))
    }

    /// Records `name`, written at `span`, as the own name of the operation
    /// being parsed, if one is.
    pub(super) fn external_named(&mut self, name: Name, span: Span) {
        if let Some(external) = &mut self.external {
            external.name = Some((name, span));
        }
    }

    /// Takes the operation being parsed, if one is and its own name is
    /// read: what the function parsed next is, which is its function.
    pub(super) fn take_external(&mut self) -> Option<Named> {
        let external = self.external.take()?;
        let (name, declared) = external.name?;
        Some(Named {
            word: external.word,
            name,
            declared,
        })
    }

    /// Checks the parameters `own` and the marker `marker`, of an effect
    /// that is impure if `impure`, of an operation (see [Host
    /// operations](#host-operations)): it takes no block and is declared
    /// with `->`.
    pub(super) fn operation_signature(
        &mut self,
        own: &[Param],
        (impure, marker): (bool, Span),
    ) -> ParseResult<()> {
        if let Some(block) = own.last().filter(|param| self.is_block_type(param.ty)) {
            let span = self.type_span(block.ty);
            return Err(self
                .report(Diagnostic::error(OPERATION_BLOCK, span).with_note(OPERATION_BLOCK_NOTE)));
        }
        if !impure {
            return Err(self.report(
                Diagnostic::error(OPERATION_MARKER, marker).with_note(OPERATION_MARKER_NOTE),
            ));
        }
        Ok(())
    }

    /// Parses what follows the return type of the operation `external`,
    /// whose function's keyword is `keyword` and whose parameters are
    /// `reads`, each by its name and span, the return type ending at `end`:
    /// its name, if `=` follows, and no body. Returns the body the parser
    /// gives it (see [Host operations](#host-operations)), a block of the
    /// call of an [`ExprKind::Operation`] with the name and a list of a read
    /// of each parameter, and what is recorded of the operation but its
    /// function: where its `extern` and its own name are written, its name
    /// and where that is written.
    pub(super) fn operation_body(
        &mut self,
        keyword: Token,
        external: Named,
        reads: &[(Name, Span)],
        end: Span,
    ) -> ParseResult<(ExprId, OperationNames)> {
        let brace = self.peek();
        if brace.kind == TokenKind::LBrace {
            return Err(self.report(
                Diagnostic::error(OPERATION_BODY, brace.span)
                    .with_context(external.word, "declared `extern` here")
                    .with_note(OPERATION_BODY_NOTE),
            ));
        }
        let written = self.external_name(OPERATION)?;
        let (name, name_span) = if let Some((name, span)) = written {
            (name, Some(span))
        } else {
            let text = self.names.get(external.name).to_owned();
            (self.strings.push(&text), None)
        };
        let span = Span::join(keyword.span, name_span.unwrap_or(end));
        // Children first, the callee the first of them, as a call's is.
        let callee = self.alloc(ExprKind::Operation, span, LEAF_HEIGHT)?;
        let text = self.alloc(ExprKind::Str(name), span, LEAF_HEIGHT)?;
        let mut elements = Vec::with_capacity(reads.len());
        for &(param, at) in reads {
            elements.push(Element {
                value: self.alloc(ExprKind::Name(param), at, LEAF_HEIGHT)?,
                spread: None,
            });
        }
        let elements = self.builder.push_elements(&elements);
        let list = self.alloc(ExprKind::List(elements), span, LEAF_HEIGHT + 1)?;
        let args = self.builder.push_args(&[
            Arg {
                kind: ArgKind::Positional,
                value: text,
            },
            Arg {
                kind: ArgKind::Positional,
                value: list,
            },
        ]);
        let call = self.alloc(ExprKind::Call { callee, args }, span, LEAF_HEIGHT + 2)?;
        let item = self.builder.alloc_item(Item {
            kind: ItemKind::Expr(call),
            span,
        });
        let items = self.builder.push_item_list(&[item]);
        let body = self.alloc(
            ExprKind::Block { items, label: None },
            span,
            LEAF_HEIGHT + 3,
        )?;
        let names = OperationNames {
            keyword: external.word,
            declared: external.declared,
            name,
            name_span,
        };
        Ok((body, names))
    }

    /// Records the operation whose function is the function expression
    /// `value`, with `names`, what [`Parser::operation_body`] found of it.
    pub(super) fn record_operation(&mut self, value: ExprId, names: OperationNames) {
        if let ExprKind::Fn(function) = self.builder.get(value).kind {
            self.builder.push_extern_fn(ExternFn {
                function,
                keyword: names.keyword,
                declared: names.declared,
                name: names.name,
                name_span: names.name_span,
            });
        }
    }
}

/// What [`Parser::operation_body`] found of an operation but its function.
#[derive(Clone, Copy, Debug)]
pub(super) struct OperationNames {
    /// Where its `extern` is written.
    keyword: Span,
    /// Where the function's own name is written.
    declared: Span,
    /// The operation's name.
    name: StrId,
    /// Where the operation's name is written, if it is.
    name_span: Option<Span>,
}
