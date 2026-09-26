//! The parser: tokens to an [`Ast`].
//!
//! # Items and bodies
//!
//! A file, like the inside of a block `{ ... }`, is a *body*: a sequence of
//! items separated by newlines or `;` (`Parser::parse_body`). Runs of
//! separators, and so blank and comment-only lines, are skipped. An item is
//! `let NAME (: TYPE)? = EXPR`, `NAME := EXPR`, `= EXPR` or an expression.
//! After an item the next token must be a separator or the end of the body;
//! anything else is "expected end of item".
//!
//! A newline ends an item except where the item cannot be complete: after
//! a binary operator, after `:` (in an annotation or an ascription), and
//! after `=`, `:=` and `let`. There the parser skips newlines before going
//! on. Inside parentheses the lexer emits no newlines at all, so an
//! expression in parentheses may span lines freely.
//!
//! # Expressions
//!
//! Expressions are parsed by a Pratt parser, also called precedence
//! climbing. Instead of one grammar function per precedence level, a single
//! function, [`Parser::parse_expr`], parses any expression whose operators
//! bind at least as tightly as a given *minimum binding power*. It first
//! parses a *prefix* expression (a literal, a name, a parenthesised
//! expression, a block, or a unary operator applied to an operand), which
//! becomes the left-hand side. Then it loops: if the next token is a binary
//! operator whose left binding power is at least the minimum, it consumes
//! the operator, parses the right-hand side by calling itself with the
//! operator's right binding power as the new minimum, and folds the result
//! into a new left-hand side. Otherwise the operator belongs to an enclosing
//! call, and the loop stops.
//!
//! Precedence and associativity both fall out of the binding-power pairs:
//!
//! | operator            | left bp | right bp / operand min bp |
//! |---------------------|---------|---------------------------|
//! | `+` `-`             | 1       | 2                         |
//! | `*` `/` `//` `%`    | 3       | 4                         |
//! | postfix `:` TYPE    | 5       | (none: parses a type)     |
//! | prefix `-`          | (none)  | operand parsed with min 7 |
//!
//! A tighter operator has higher powers, so in `1 + 2 * 3` the recursive
//! call for the right-hand side of `+` (minimum 2) still accepts `*` (left
//! power 3). Left associativity comes from making the right power one higher
//! than the left: in `8 - 3 - 2`, the right-hand side of the first `-` is
//! parsed with minimum 2, which rejects the second `-` (left power 1), so the
//! first subtraction completes before the second begins.
//!
//! Type ascription `EXPR : TYPE` is a postfix operator in the same loop: when
//! the next token is `:` and its left power 5 is at least the minimum, the
//! parser consumes it, parses a type, and wraps the left-hand side. It binds
//! tighter than every binary operator, so `1 + 2 : Float` is
//! `1 + (2 : Float)`, and looser than negation, whose operand is parsed with
//! minimum 7, so `-5 : Float` is `(-5) : Float`. It chains left to right.
//!
//! Negation is its own token, [`TokenKind::Neg`], distinct from subtraction's
//! [`TokenKind::Minus`]; the lexer tells them apart by spacing. `Neg` is only
//! accepted in prefix position and `Minus` only as a binary operator.
//!
//! # Errors and depth
//!
//! Errors are values. Parsing functions return `Result`, and a failure is
//! recorded in the parser's diagnostic sink before the `Err` propagates. The
//! parser stops at the first error for now; recovery can be added later by
//! catching an `Err` at a synchronisation point (such as the next item
//! separator) and continuing, with the sink then holding several
//! diagnostics.
//!
//! The parser enforces [`MAX_DEPTH`] in two places, because nesting arises in
//! two ways. Parentheses, prefix operators and blocks nest by recursion, so
//! [`Parser::parse_expr`] and the body of every nested block count their own
//! depth and stop before the recursion itself could overflow. Left-associative
//! chains such as `1 + 1 + 1` nest without recursion, since the loop folds
//! each operator into the left-hand side, so the parser also tracks the
//! height of every node it allocates, a block being one above the tallest of
//! its items' expressions, and rejects a node that would exceed the limit.
//! Together these guarantee that no tree handed to a later stage is deeper
//! than `MAX_DEPTH`.

use crate::MAX_DEPTH;
use crate::ast::{
    Ast, AstBuilder, BinaryOp, Expr, ExprId, ExprKind, Item, ItemId, ItemKind, ItemRange, TypeExpr,
    TypeExprId, TypeExprKind, UnaryOp,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::names::Names;
use crate::source::{SourceFile, Span};

/// Minimum binding power of the operand of prefix `-`: tighter than every
/// binary operator and than ascription.
const PREFIX_BP: u8 = 7;

/// Left binding power of postfix `:` (type ascription): tighter than every
/// binary operator, looser than prefix `-`.
const ASCRIBE_BP: u8 = 5;

/// Height of a leaf node.
const LEAF_HEIGHT: u32 = 1;

/// Parses `tokens`, lexed from `source` with its names interned into
/// `names`, as a file: a body of items. The returned [`Ast`] owns `names`.
///
/// All tokens must be consumed. `tokens` should end with
/// [`TokenKind::Eof`], as [`crate::lexer::tokenize`] guarantees; if it does
/// not, running out of tokens is treated as end of input.
///
/// # Errors
///
/// Returns the first syntax error.
pub fn parse(tokens: &[Token], names: Names, source: &SourceFile) -> Result<Ast, Diagnostic> {
    let (builder, root) = parse_parts(tokens, source)?;
    Ok(builder.finish(root, names))
}

/// Like [`parse`], returning the unfinished arenas and the file's body
/// instead of an [`Ast`], so that a caller lending its [`Names`] only hands
/// them over once parsing has succeeded.
pub(crate) fn parse_parts(
    tokens: &[Token],
    source: &SourceFile,
) -> Result<(AstBuilder, ItemRange), Diagnostic> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        source,
        // Every node consumes at least one token, so these never reallocate.
        builder: AstBuilder::with_capacity(tokens.len()),
        heights: Vec::with_capacity(tokens.len()),
        depth: 0,
        item_stack: Vec::new(),
        diagnostics: Vec::new(),
    };
    match parser.parse_body(None) {
        Ok((root, _)) => Ok((parser.builder, root)),
        Err(Reported) => Err(parser.diagnostics.into_iter().next().unwrap_or_else(|| {
            Diagnostic::error(
                "internal error: parser failed without reporting a diagnostic",
                Span::new(0, 0),
            )
        })),
    }
}

/// Proof that a diagnostic has been pushed to the parser's sink. Only
/// [`Parser::report`] creates one.
#[derive(Debug)]
struct Reported;

type ParseResult<T> = Result<T, Reported>;

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    source: &'a SourceFile,
    builder: AstBuilder,
    /// The height of each allocated expression node, indexed like the
    /// builder's arena: 1 for a leaf, one more than the tallest child
    /// otherwise. A block's children are its items' expressions.
    heights: Vec<u32>,
    /// Current recursion depth: nested [`Parser::parse_expr`] calls and
    /// nested block bodies.
    depth: usize,
    /// The items of the bodies being parsed, innermost last. Each body
    /// pushes its items here as it parses them and moves them to the
    /// builder's item-list pool when it ends, so nested bodies need no
    /// allocation of their own.
    item_stack: Vec<ItemId>,
    /// Errors reported so far. Only one is ever pushed until error recovery
    /// exists.
    diagnostics: Vec<Diagnostic>,
}

impl Parser<'_> {
    /// Parses a body up to its end: the `}` matching `open` (not consumed),
    /// or the end of input for the file's body, when `open` is `None`.
    /// Returns the body and the height of its tallest item expression (0 if
    /// it has no items).
    fn parse_body(&mut self, open: Option<Span>) -> ParseResult<(ItemRange, u32)> {
        let base = self.item_stack.len();
        let mut height = 0;
        loop {
            self.skip_separators();
            let token = self.peek();
            match (token.kind, open) {
                (TokenKind::Eof, None) | (TokenKind::RBrace, Some(_)) => break,
                (TokenKind::Eof, Some(open)) => {
                    let found = self.describe(token);
                    return Err(self.report(
                        Diagnostic::error(format!("expected `}}`, found {found}"), token.span)
                            .with_context(open, "unclosed `{`"),
                    ));
                }
                (TokenKind::RBrace, None) => {
                    return Err(self.error("unexpected `}`", token.span));
                }
                _ => {}
            }
            let (item, item_height) = self.parse_item()?;
            self.item_stack.push(item);
            height = height.max(item_height);
            self.expect_end_of_item()?;
        }
        let range = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok((range, height))
    }

    /// Parses one item. Returns it and the height of its expression.
    fn parse_item(&mut self) -> ParseResult<(ItemId, u32)> {
        let token = self.peek();
        let (kind, value) = match token.kind {
            TokenKind::Let => {
                self.advance();
                self.skip_newlines();
                let name_token = self.peek();
                let TokenKind::Ident(name) = name_token.kind else {
                    return Err(self.error("expected a name after `let`", name_token.span));
                };
                self.advance();
                let ty = if self.peek().kind == TokenKind::Colon {
                    self.advance();
                    self.skip_newlines();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let eq = self.peek();
                if eq.kind != TokenKind::Eq {
                    return Err(self.error("expected `=` after variable name", eq.span));
                }
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                let kind = ItemKind::Let {
                    name,
                    name_span: name_token.span,
                    ty,
                    value,
                };
                (kind, value)
            }
            TokenKind::Ident(name) if self.peek_at(1).kind == TokenKind::ColonEq => {
                self.advance();
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                let kind = ItemKind::Rebind {
                    name,
                    name_span: token.span,
                    value,
                };
                (kind, value)
            }
            TokenKind::Eq => {
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                (ItemKind::Result(value), value)
            }
            _ => {
                let value = self.parse_expr(0)?;
                (ItemKind::Expr(value), value)
            }
        };
        let span = Span::join(token.span, self.span_of(value));
        let item = self.builder.alloc_item(Item { kind, span });
        Ok((item, self.height_of(value)))
    }

    /// Checks that the item just parsed is followed by a separator or the
    /// end of its body.
    fn expect_end_of_item(&mut self) -> ParseResult<()> {
        let token = self.peek();
        match token.kind {
            TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof => Ok(()),
            // `a -b`: the user most likely meant a subtraction. The lexer
            // only makes a `Neg` when an operand follows directly, so show
            // the `-` together with that operand's first token.
            TokenKind::Neg => {
                let span = Span::join(token.span, self.peek_at(1).span);
                let found = self.source.slice(span);
                Err(self.report(
                    Diagnostic::error(format!("expected end of item, found `{found}`"), span)
                        .with_note("write `a - b` with spaces for subtraction"),
                ))
            }
            _ => {
                let found = self.describe(token);
                Err(self.error(format!("expected end of item, found {found}"), token.span))
            }
        }
    }

    /// Parses a type expression: a name.
    fn parse_type(&mut self) -> ParseResult<TypeExprId> {
        let token = self.peek();
        let TokenKind::Ident(name) = token.kind else {
            return Err(self.error("expected a type", token.span));
        };
        self.advance();
        Ok(self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Named(name),
            span: token.span,
        }))
    }

    /// Parses an expression whose binary operators all have a left binding
    /// power of at least `min_bp`.
    fn parse_expr(&mut self, min_bp: u8) -> ParseResult<ExprId> {
        self.enter()?;
        let result = self.parse_expr_inner(min_bp);
        self.depth -= 1;
        result
    }

    /// Records one more level of recursion, failing if it would exceed
    /// [`MAX_DEPTH`]. Every successful call is paired with a decrement by
    /// its caller.
    fn enter(&mut self) -> ParseResult<()> {
        if self.depth >= MAX_DEPTH {
            let span = self.peek().span;
            return Err(self.too_deep(span));
        }
        self.depth += 1;
        Ok(())
    }

    fn parse_expr_inner(&mut self, min_bp: u8) -> ParseResult<ExprId> {
        let mut lhs = self.parse_prefix()?;
        loop {
            let token = self.peek();
            if token.kind == TokenKind::Colon {
                if ASCRIBE_BP < min_bp {
                    break;
                }
                self.advance();
                self.skip_newlines();
                let ty = self.parse_type()?;
                let span = Span::join(self.span_of(lhs), self.type_span(ty));
                let height = self.height_of(lhs) + 1;
                lhs = self.alloc(ExprKind::Ascribe { expr: lhs, ty }, span, height)?;
                continue;
            }
            let Some(op) = binary_op(token.kind) else {
                break;
            };
            let (left_bp, right_bp) = binding_power(op);
            if left_bp < min_bp {
                break;
            }
            let op_start = token.span.start;
            self.advance();
            self.skip_newlines();
            let rhs = self.parse_expr(right_bp)?;
            let span = Span::join(self.span_of(lhs), self.span_of(rhs));
            let height = self.height_of(lhs).max(self.height_of(rhs)) + 1;
            lhs = self.alloc(
                ExprKind::Binary {
                    op,
                    op_start,
                    lhs,
                    rhs,
                },
                span,
                height,
            )?;
        }
        Ok(lhs)
    }

    /// Parses a literal, a name, a parenthesised expression, a block or a
    /// prefix operator application.
    fn parse_prefix(&mut self) -> ParseResult<ExprId> {
        let token = self.peek();
        let kind = match token.kind {
            TokenKind::Int(value) => {
                self.advance();
                ExprKind::Int(value)
            }
            TokenKind::Float(value) => {
                self.advance();
                ExprKind::Float(value)
            }
            TokenKind::Ident(name) => {
                self.advance();
                ExprKind::Name(name)
            }
            TokenKind::Neg => {
                self.advance();
                let operand = self.parse_expr(PREFIX_BP)?;
                let span = Span::join(token.span, self.span_of(operand));
                let height = self.height_of(operand) + 1;
                return self.alloc(
                    ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand,
                    },
                    span,
                    height,
                );
            }
            TokenKind::LParen => {
                self.advance();
                let inner = self.parse_expr(0)?;
                let close = self.peek();
                if close.kind != TokenKind::RParen {
                    let found = self.describe(close);
                    return Err(self.report(
                        Diagnostic::error(format!("expected `)`, found {found}"), close.span)
                            .with_context(token.span, "unclosed `(`"),
                    ));
                }
                self.advance();
                let height = self.height_of(inner) + 1;
                return self.alloc(
                    ExprKind::Paren(inner),
                    Span::join(token.span, close.span),
                    height,
                );
            }
            TokenKind::LBrace => {
                self.advance();
                self.enter()?;
                let body = self.parse_body(Some(token.span));
                self.depth -= 1;
                let (items, tallest) = body?;
                // `parse_body` only returns successfully at the matching `}`.
                let close = self.peek();
                self.advance();
                return self.alloc(
                    ExprKind::Block(items),
                    Span::join(token.span, close.span),
                    tallest + 1,
                );
            }
            TokenKind::Minus
            | TokenKind::Plus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::SlashSlash
            | TokenKind::Percent
            | TokenKind::Let
            | TokenKind::Colon
            | TokenKind::ColonEq
            | TokenKind::Eq
            | TokenKind::RParen
            | TokenKind::RBrace
            | TokenKind::Semi
            | TokenKind::Newline
            | TokenKind::Eof => {
                let found = self.describe(token);
                return Err(self.error(format!("expected expression, found {found}"), token.span));
            }
        };
        self.alloc(kind, token.span, LEAF_HEIGHT)
    }

    /// Allocates a node of the given `height`, failing if that height
    /// exceeds [`MAX_DEPTH`].
    fn alloc(&mut self, kind: ExprKind, span: Span, height: u32) -> ParseResult<ExprId> {
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let id = self.builder.alloc(Expr { kind, span });
        self.heights.push(height);
        Ok(id)
    }

    fn height_of(&self, id: ExprId) -> u32 {
        // Every id came from `alloc`, which pushes a height for each node.
        self.heights.get(id.index()).copied().unwrap_or(LEAF_HEIGHT)
    }

    fn too_deep(&mut self, span: Span) -> Reported {
        self.report(
            Diagnostic::error("expression nested too deeply", span)
                .with_note(format!("the limit is {MAX_DEPTH} levels")),
        )
    }

    /// Skips item separators: newlines and `;`.
    fn skip_separators(&mut self) {
        while matches!(self.peek().kind, TokenKind::Newline | TokenKind::Semi) {
            self.advance();
        }
    }

    /// Skips newlines where an item continues on the next line.
    fn skip_newlines(&mut self) {
        while self.peek().kind == TokenKind::Newline {
            self.advance();
        }
    }

    /// The current token. Past the end of a token slice that lacks a final
    /// `Eof`, this synthesises one at the end of the source.
    fn peek(&self) -> Token {
        self.peek_at(0)
    }

    /// The token `ahead` places after the current one, or a synthesised
    /// `Eof` past the end.
    fn peek_at(&self, ahead: usize) -> Token {
        self.tokens.get(self.pos + ahead).copied().unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.source.len(), self.source.len()),
        })
    }

    /// Moves past the current token. Never moves past `Eof`, which callers
    /// never consume.
    fn advance(&mut self) {
        self.pos += 1;
    }

    fn span_of(&self, id: ExprId) -> Span {
        self.builder.get(id).span
    }

    fn type_span(&self, id: TypeExprId) -> Span {
        self.builder.type_expr(id).span
    }

    /// Describes a token for "found ..." messages: literals and names by
    /// their source text, operators and keywords by their symbol, both in
    /// backticks.
    fn describe(&self, token: Token) -> String {
        match token.kind.symbol() {
            Some(symbol) => format!("`{symbol}`"),
            None if token.kind == TokenKind::Eof => "end of input".to_owned(),
            None if token.kind == TokenKind::Newline => "newline".to_owned(),
            None => format!("`{}`", self.source.slice(token.span)),
        }
    }

    /// Records an error in the sink and returns the proof that it was
    /// recorded.
    fn error(&mut self, message: impl Into<String>, span: Span) -> Reported {
        self.report(Diagnostic::error(message, span))
    }

    /// Records a diagnostic in the sink and returns the proof that it was
    /// recorded.
    fn report(&mut self, diagnostic: Diagnostic) -> Reported {
        self.diagnostics.push(diagnostic);
        Reported
    }
}

/// The binary operator a token denotes, if any. `Neg` is not one: a `-`
/// directly followed by its operand is always a negation.
fn binary_op(kind: TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Plus => Some(BinaryOp::Add),
        TokenKind::Minus => Some(BinaryOp::Sub),
        TokenKind::Star => Some(BinaryOp::Mul),
        TokenKind::Slash => Some(BinaryOp::Div),
        TokenKind::SlashSlash => Some(BinaryOp::FloorDiv),
        TokenKind::Percent => Some(BinaryOp::Rem),
        TokenKind::Int(_)
        | TokenKind::Float(_)
        | TokenKind::Ident(_)
        | TokenKind::Let
        | TokenKind::Neg
        | TokenKind::Colon
        | TokenKind::ColonEq
        | TokenKind::Eq
        | TokenKind::LParen
        | TokenKind::RParen
        | TokenKind::LBrace
        | TokenKind::RBrace
        | TokenKind::Semi
        | TokenKind::Newline
        | TokenKind::Eof => None,
    }
}

/// `(left, right)` binding powers of a binary operator. Right is left + 1,
/// which makes every level left-associative.
fn binding_power(op: BinaryOp) -> (u8, u8) {
    match op {
        BinaryOp::Add | BinaryOp::Sub => (1, 2),
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::FloorDiv | BinaryOp::Rem => (3, 4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::ItemKind;
    use crate::lexer::tokenize;

    fn parse_text(text: &str) -> Result<Ast, Diagnostic> {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names) = tokenize(&source)?;
        parse(&tokens, names, &source)
    }

    /// Renders a file compactly, making grouping explicit: items are joined
    /// by `; `, binary operations and negations are parenthesised, parens
    /// show as `[ ]`, blocks as `{ }` and ascriptions as `(e : T)`.
    fn shape(text: &str) -> String {
        let ast = parse_text(text).unwrap();
        let mut out = String::new();
        body(&ast, ast.root(), &mut out);
        out
    }

    fn body(ast: &Ast, range: ItemRange, out: &mut String) {
        for (index, &id) in ast.items(range).iter().enumerate() {
            if index > 0 {
                out.push_str("; ");
            }
            match ast.item(id).kind {
                ItemKind::Let {
                    name, ty, value, ..
                } => {
                    out.push_str("let ");
                    out.push_str(ast.names().get(name));
                    if let Some(ty) = ty {
                        out.push_str(": ");
                        type_name(ast, ty, out);
                    }
                    out.push_str(" = ");
                    expr(ast, value, out);
                }
                ItemKind::Rebind { name, value, .. } => {
                    out.push_str(ast.names().get(name));
                    out.push_str(" := ");
                    expr(ast, value, out);
                }
                ItemKind::Result(value) => {
                    out.push_str("= ");
                    expr(ast, value, out);
                }
                ItemKind::Expr(value) => expr(ast, value, out),
            }
        }
    }

    fn type_name(ast: &Ast, id: TypeExprId, out: &mut String) {
        match ast.type_expr(id).kind {
            TypeExprKind::Named(name) => out.push_str(ast.names().get(name)),
        }
    }

    fn expr(ast: &Ast, id: ExprId, out: &mut String) {
        use std::fmt::Write as _;
        match ast.get(id).kind {
            ExprKind::Int(value) => write!(out, "{value}").unwrap(),
            ExprKind::Float(value) => write!(out, "{value:?}").unwrap(),
            ExprKind::Name(name) => out.push_str(ast.names().get(name)),
            ExprKind::Unary { op, operand } => {
                out.push('(');
                out.push_str(op.symbol());
                expr(ast, operand, out);
                out.push(')');
            }
            ExprKind::Binary { op, lhs, rhs, .. } => {
                out.push('(');
                expr(ast, lhs, out);
                out.push(' ');
                out.push_str(op.symbol());
                out.push(' ');
                expr(ast, rhs, out);
                out.push(')');
            }
            ExprKind::Paren(inner) => {
                out.push('[');
                expr(ast, inner, out);
                out.push(']');
            }
            ExprKind::Block(items) => {
                out.push('{');
                body(ast, items, out);
                out.push('}');
            }
            ExprKind::Ascribe { expr: inner, ty } => {
                out.push('(');
                expr(ast, inner, out);
                out.push_str(" : ");
                type_name(ast, ty, out);
                out.push(')');
            }
        }
    }

    fn error(text: &str) -> (String, Span) {
        let diagnostic = parse_text(text).unwrap_err();
        (diagnostic.message, diagnostic.span)
    }

    fn message(text: &str) -> String {
        parse_text(text).unwrap_err().message
    }

    #[test]
    fn literals() {
        assert_eq!(shape("42"), "42");
        assert_eq!(shape("2.5"), "2.5");
    }

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        assert_eq!(shape("1 + 2 * 3"), "(1 + (2 * 3))");
        assert_eq!(shape("1 * 2 + 3"), "((1 * 2) + 3)");
        assert_eq!(shape("1 - 6 / 3 // 2 % 5"), "(1 - (((6 / 3) // 2) % 5))");
    }

    #[test]
    fn binary_operators_are_left_associative() {
        assert_eq!(shape("8 - 3 - 2"), "((8 - 3) - 2)");
        assert_eq!(shape("1 + 2 - 3 + 4"), "(((1 + 2) - 3) + 4)");
        assert_eq!(shape("2 * 3 // 4"), "((2 * 3) // 4)");
        assert_eq!(shape("8 / 4 / 2"), "((8 / 4) / 2)");
        assert_eq!(shape("9 % 5 * 2"), "((9 % 5) * 2)");
    }

    #[test]
    fn unary_minus_binds_tighter_than_multiplication() {
        assert_eq!(shape("-2 * 3"), "((-2) * 3)");
        assert_eq!(shape("-2 + 3"), "((-2) + 3)");
        assert_eq!(shape("-x * y"), "((-x) * y)");
    }

    #[test]
    fn nested_unary_minus() {
        // Negating a negation needs parentheses: `--2` and `--x` are lexical
        // errors (reported through `parse_text`, which lexes first), and
        // `- - -2` starts with a subtraction `-`.
        assert_eq!(shape("-(-2)"), "(-[(-2)])");
        assert_eq!(shape("-(-x)"), "(-[(-x)])");
        assert_eq!(
            error("- - -2"),
            ("expected expression, found `-`".to_owned(), Span::new(0, 1))
        );
        for text in ["--2", "--x", "1 + --x"] {
            let diagnostic = parse_text(text).unwrap_err();
            assert_eq!(diagnostic.message, "unexpected character `-`", "{text}");
        }
        assert_eq!(error("--x").1, Span::new(0, 1));
    }

    #[test]
    fn spaced_minus_cannot_start_an_expression() {
        assert_eq!(
            error("- 2 * 3"),
            ("expected expression, found `-`".to_owned(), Span::new(0, 1))
        );
        assert_eq!(
            error("1 * - 2"),
            ("expected expression, found `-`".to_owned(), Span::new(4, 5))
        );
    }

    #[test]
    fn unary_minus_inside_binary() {
        assert_eq!(shape("2 * -3"), "(2 * (-3))");
        assert_eq!(shape("2 - -3"), "(2 - (-3))");
    }

    #[test]
    fn negation_directly_after_an_item_is_reported_as_a_likely_subtraction() {
        let diagnostic = parse_text("a -b").unwrap_err();
        assert_eq!(diagnostic.message, "expected end of item, found `-b`");
        assert_eq!(diagnostic.span, Span::new(2, 4));
        assert_eq!(
            diagnostic.notes,
            vec!["write `a - b` with spaces for subtraction"]
        );
        let diagnostic = parse_text("let x = 1 -2.5").unwrap_err();
        assert_eq!(diagnostic.message, "expected end of item, found `-2.5`");
        assert_eq!(diagnostic.span, Span::new(10, 14));
        let diagnostic = parse_text("{ x -hello-world }").unwrap_err();
        assert_eq!(
            diagnostic.message,
            "expected end of item, found `-hello-world`"
        );
    }

    #[test]
    fn parentheses_group() {
        assert_eq!(shape("(1 + 2) * 3"), "([(1 + 2)] * 3)");
        assert_eq!(shape("((1))"), "[[1]]");
        assert_eq!(shape("2 * (3 - (4 + 5))"), "(2 * [(3 - [(4 + 5)])])");
        assert_eq!(shape("-(1 + 2)"), "(-[(1 + 2)])");
    }

    #[test]
    fn names() {
        assert_eq!(shape("x"), "x");
        assert_eq!(shape("hello-world + x-1"), "(hello-world + x-1)");
        assert_eq!(shape("x - 1"), "(x - 1)");
    }

    #[test]
    fn items_are_separated_by_newlines_and_semicolons() {
        assert_eq!(shape("let x = 1\nx"), "let x = 1; x");
        assert_eq!(shape("let x = 1; x"), "let x = 1; x");
        assert_eq!(shape("let x = 1\nx := 2; = x"), "let x = 1; x := 2; = x");
        // Blank lines, comment-only lines and repeated separators.
        assert_eq!(
            shape("\n\n# comment\nlet x = 1;;\n\n  ;\n# another\n x ;\n"),
            "let x = 1; x"
        );
        assert_eq!(shape(";"), "");
        assert_eq!(shape(""), "");
        assert_eq!(shape("# only a comment"), "");
    }

    #[test]
    fn items_continue_after_a_trailing_operator() {
        for op in ["+", "-", "*", "/", "//", "%"] {
            let text = format!("1 {op}\n\n  2");
            assert_eq!(shape(&text), format!("(1 {op} 2)"), "{text}");
        }
        assert_eq!(shape("1 :\nFloat"), "(1 : Float)");
        // A comment after the operator does not end the item either.
        assert_eq!(shape("1 + # more\n2"), "(1 + 2)");
    }

    #[test]
    fn items_continue_after_eq_colon_eq_and_let() {
        assert_eq!(shape("let x =\n1"), "let x = 1");
        assert_eq!(shape("let\nx = 1"), "let x = 1");
        assert_eq!(shape("let x:\nInt = 1"), "let x: Int = 1");
        assert_eq!(shape("x :=\n\n1"), "x := 1");
        assert_eq!(shape("=\n1"), "= 1");
    }

    #[test]
    fn newlines_elsewhere_end_the_item() {
        // A leading operator on the next line does not continue the item.
        assert_eq!(
            error("1\n+ 2"),
            ("expected expression, found `+`".to_owned(), Span::new(2, 3))
        );
        assert_eq!(
            error("let x\n= 1"),
            (
                "expected `=` after variable name".to_owned(),
                Span::new(5, 6)
            )
        );
    }

    #[test]
    fn items_continue_inside_parentheses() {
        assert_eq!(shape("(1\n+\n2)\n3"), "[(1 + 2)]; 3");
        assert_eq!(shape("let x = (\n  1\n)"), "let x = [1]");
    }

    #[test]
    fn blocks() {
        assert_eq!(shape("{}"), "{}");
        assert_eq!(shape("{ 1 }"), "{1}");
        assert_eq!(
            shape("let x = {\n  let y = 1\n  y + 1\n}"),
            "let x = {let y = 1; (y + 1)}"
        );
        assert_eq!(shape("{ {} ; { = 1 } }"), "{{}; {= 1}}");
        assert_eq!(shape("{{{ 1 }}}"), "{{{1}}}");
        assert_eq!(shape("1 + { 2 } * 3"), "(1 + ({2} * 3))");
        let ast = parse_text("x + {\n1 }").unwrap();
        let root = ast.items(ast.root())[0];
        let ItemKind::Expr(sum) = ast.item(root).kind else {
            panic!("expected an expression item");
        };
        let ExprKind::Binary { rhs, .. } = ast.get(sum).kind else {
            panic!("expected a binary node");
        };
        // A block's span covers its braces.
        assert_eq!(ast.get(rhs).span, Span::new(4, 9));
    }

    #[test]
    fn result_items_parse_anywhere_in_a_body() {
        // Whether `= e` is the last item is checked by the checker, which
        // can point at the item after it.
        assert_eq!(shape("= 1; 2"), "= 1; 2");
        assert_eq!(shape("= 1; = 2"), "= 1; = 2");
    }

    #[test]
    fn eq_is_only_accepted_at_the_start_of_an_item() {
        assert_eq!(
            error("x = 1"),
            (
                "expected end of item, found `=`".to_owned(),
                Span::new(2, 3)
            )
        );
        assert_eq!(
            error("1 + = 2"),
            ("expected expression, found `=`".to_owned(), Span::new(4, 5))
        );
        assert_eq!(
            error("(= 1)"),
            ("expected expression, found `=`".to_owned(), Span::new(1, 2))
        );
        assert_eq!(
            error("let x = = 1"),
            ("expected expression, found `=`".to_owned(), Span::new(8, 9))
        );
    }

    #[test]
    fn rebind_needs_a_bare_name() {
        assert_eq!(
            error("(x) := 1"),
            (
                "expected end of item, found `:=`".to_owned(),
                Span::new(4, 6)
            )
        );
        assert_eq!(
            error("x + y := 1"),
            (
                "expected end of item, found `:=`".to_owned(),
                Span::new(6, 8)
            )
        );
        // `: =` is an ascription missing its type, not a rebind.
        assert_eq!(
            error("x : = 1"),
            ("expected a type".to_owned(), Span::new(4, 5))
        );
    }

    #[test]
    fn item_spans() {
        let ast = parse_text("let x: Int = 1 + 2\nx := 3\n= x\nx").unwrap();
        let spans: Vec<Span> = ast
            .items(ast.root())
            .iter()
            .map(|&id| ast.item(id).span)
            .collect();
        assert_eq!(
            spans,
            vec![
                Span::new(0, 18),
                Span::new(19, 25),
                Span::new(26, 29),
                Span::new(30, 31)
            ]
        );
        let ItemKind::Let { name_span, .. } = ast.item(ast.items(ast.root())[0]).kind else {
            panic!("expected a let");
        };
        assert_eq!(name_span, Span::new(4, 5));
    }

    #[test]
    fn ascription_binds_between_multiplication_and_negation() {
        assert_eq!(shape("1 + 2 : Float"), "(1 + (2 : Float))");
        assert_eq!(shape("-5 : Float"), "((-5) : Float)");
        assert_eq!(shape("2 * 3 : Float"), "(2 * (3 : Float))");
        assert_eq!(shape("(1 + 2) : Float"), "([(1 + 2)] : Float)");
        assert_eq!(shape("x : Int : Int"), "((x : Int) : Int)");
        assert_eq!(shape("x : Int + 1"), "((x : Int) + 1)");
    }

    #[test]
    fn ascription_shapes_are_exact() {
        // `1 + 2 : Float` is Add(1, Ascribe(2)).
        let ast = parse_text("1 + 2 : Float").unwrap();
        let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
            panic!("expected an expression item");
        };
        let ExprKind::Binary {
            op: BinaryOp::Add,
            lhs,
            rhs,
            ..
        } = ast.get(root).kind
        else {
            panic!("expected an addition, got {:?}", ast.get(root));
        };
        assert_eq!(ast.get(lhs).kind, ExprKind::Int(1));
        let ExprKind::Ascribe { expr, ty } = ast.get(rhs).kind else {
            panic!("expected an ascription, got {:?}", ast.get(rhs));
        };
        assert_eq!(ast.get(expr).kind, ExprKind::Int(2));
        assert_eq!(ast.get(rhs).span, Span::new(4, 13));
        assert_eq!(ast.type_expr(ty).span, Span::new(8, 13));

        // `-5 : Float` is Ascribe(Neg(5)).
        let ast = parse_text("-5 : Float").unwrap();
        let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
            panic!("expected an expression item");
        };
        let ExprKind::Ascribe { expr, .. } = ast.get(root).kind else {
            panic!("expected an ascription, got {:?}", ast.get(root));
        };
        let ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } = ast.get(expr).kind
        else {
            panic!("expected a negation, got {:?}", ast.get(expr));
        };
        assert_eq!(ast.get(operand).kind, ExprKind::Int(5));
    }

    #[test]
    fn spans() {
        let ast = parse_text("-(1 + 2) * 30").unwrap();
        let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
            panic!("expected an expression item");
        };
        let root = ast.get(root);
        assert_eq!(root.span, Span::new(0, 13));
        let ExprKind::Binary { lhs, rhs, .. } = root.kind else {
            panic!("expected binary, got {root:?}");
        };
        let neg = ast.get(lhs);
        assert_eq!(neg.span, Span::new(0, 8));
        let ExprKind::Unary { operand, .. } = neg.kind else {
            panic!("expected unary, got {neg:?}");
        };
        let paren = ast.get(operand);
        assert_eq!(paren.span, Span::new(1, 8));
        let ExprKind::Paren(sum) = paren.kind else {
            panic!("expected paren, got {paren:?}");
        };
        assert_eq!(ast.get(sum).span, Span::new(2, 7));
        assert_eq!(ast.get(rhs).span, Span::new(11, 13));
    }

    #[test]
    fn expected_expression() {
        assert_eq!(
            error("1 +"),
            (
                "expected expression, found end of input".to_owned(),
                Span::new(3, 3)
            )
        );
        assert_eq!(
            error("* 2"),
            ("expected expression, found `*`".to_owned(), Span::new(0, 1))
        );
        assert_eq!(
            error("()"),
            ("expected expression, found `)`".to_owned(), Span::new(1, 2))
        );
        assert_eq!(
            error("1 + // 2"),
            (
                "expected expression, found `//`".to_owned(),
                Span::new(4, 6)
            )
        );
        assert_eq!(
            error("1 + let"),
            (
                "expected expression, found `let`".to_owned(),
                Span::new(4, 7)
            )
        );
        assert_eq!(
            error("{ 1 + }"),
            ("expected expression, found `}`".to_owned(), Span::new(6, 7))
        );
        assert_eq!(
            error("x := ;"),
            ("expected expression, found `;`".to_owned(), Span::new(5, 6))
        );
    }

    #[test]
    fn expected_close_paren() {
        assert_eq!(
            error("(1 + 2"),
            (
                "expected `)`, found end of input".to_owned(),
                Span::new(6, 6)
            )
        );
        let diagnostic = parse_text("(1 2)").unwrap_err();
        assert_eq!(diagnostic.message, "expected `)`, found `2`");
        assert_eq!(diagnostic.span, Span::new(3, 4));
        assert_eq!(diagnostic.labels.len(), 1);
        assert_eq!(diagnostic.labels[0].span, Span::new(0, 1));
        assert_eq!(diagnostic.labels[0].text, "unclosed `(`");
        assert_eq!(message("(1; 2)"), "expected `)`, found `;`");
    }

    #[test]
    fn unterminated_block() {
        let diagnostic = parse_text("let x = {\n  1\n").unwrap_err();
        assert_eq!(diagnostic.message, "expected `}`, found end of input");
        assert_eq!(diagnostic.span, Span::new(13, 13));
        assert_eq!(diagnostic.labels.len(), 1);
        assert_eq!(diagnostic.labels[0].span, Span::new(8, 9));
        assert_eq!(diagnostic.labels[0].text, "unclosed `{`");
        // The innermost unclosed brace is the one reported.
        let diagnostic = parse_text("{ { }").unwrap_err();
        assert_eq!(diagnostic.labels[0].span, Span::new(0, 1));
        let diagnostic = parse_text("{ {").unwrap_err();
        assert_eq!(diagnostic.labels[0].span, Span::new(2, 3));
    }

    #[test]
    fn stray_close_brace_at_top_level() {
        assert_eq!(error("}"), ("unexpected `}`".to_owned(), Span::new(0, 1)));
        assert_eq!(error("1 }"), ("unexpected `}`".to_owned(), Span::new(2, 3)));
        assert_eq!(
            error("{ 1 }\n}"),
            ("unexpected `}`".to_owned(), Span::new(6, 7))
        );
    }

    #[test]
    fn let_errors() {
        assert_eq!(
            error("let x 5"),
            (
                "expected `=` after variable name".to_owned(),
                Span::new(6, 7)
            )
        );
        assert_eq!(
            error("let x: Int 5"),
            (
                "expected `=` after variable name".to_owned(),
                Span::new(11, 12)
            )
        );
        assert_eq!(
            error("let x"),
            (
                "expected `=` after variable name".to_owned(),
                Span::new(5, 5)
            )
        );
        assert_eq!(
            error("let 5 = 1"),
            ("expected a name after `let`".to_owned(), Span::new(4, 5))
        );
        assert_eq!(
            error("let let = 1"),
            ("expected a name after `let`".to_owned(), Span::new(4, 7))
        );
        assert_eq!(
            error("let = 1"),
            ("expected a name after `let`".to_owned(), Span::new(4, 5))
        );
        assert_eq!(
            error("let"),
            ("expected a name after `let`".to_owned(), Span::new(3, 3))
        );
    }

    #[test]
    fn expected_a_type() {
        assert_eq!(
            error("let x: = 1"),
            ("expected a type".to_owned(), Span::new(7, 8))
        );
        assert_eq!(
            error("1 : 2"),
            ("expected a type".to_owned(), Span::new(4, 5))
        );
        assert_eq!(
            error("1 :"),
            ("expected a type".to_owned(), Span::new(3, 3))
        );
        assert_eq!(
            error("1 : (Int)"),
            ("expected a type".to_owned(), Span::new(4, 5))
        );
    }

    #[test]
    fn binary_nodes_record_their_operator() {
        let ast = parse_text("1 + 2 // 3").unwrap();
        let ItemKind::Expr(root) = ast.item(ast.items(ast.root())[0]).kind else {
            panic!("expected an expression item");
        };
        let root = ast.get(root);
        assert_eq!(root.operator_span(), Some(Span::new(2, 3)));
        let ExprKind::Binary { rhs, .. } = root.kind else {
            panic!("expected binary, got {root:?}");
        };
        assert_eq!(ast.get(rhs).operator_span(), Some(Span::new(6, 8)));
    }

    #[test]
    fn expected_end_of_item() {
        assert_eq!(
            error("1 2"),
            (
                "expected end of item, found `2`".to_owned(),
                Span::new(2, 3)
            )
        );
        assert_eq!(
            error("1 1_000.5e3"),
            (
                "expected end of item, found `1_000.5e3`".to_owned(),
                Span::new(2, 11)
            )
        );
        assert_eq!(
            error("(1))"),
            (
                "expected end of item, found `)`".to_owned(),
                Span::new(3, 4)
            )
        );
        assert_eq!(
            error("2 (3)"),
            (
                "expected end of item, found `(`".to_owned(),
                Span::new(2, 3)
            )
        );
        assert_eq!(
            error("{ x y }"),
            (
                "expected end of item, found `y`".to_owned(),
                Span::new(4, 5)
            )
        );
        assert_eq!(
            error("let x = 1 let y = 2"),
            (
                "expected end of item, found `let`".to_owned(),
                Span::new(10, 13)
            )
        );
    }

    #[test]
    fn pretty_output_for_a_small_program() {
        let ast = parse_text("let x: Float = 1\ny := { x : Float }\n= -x").unwrap();
        assert_eq!(
            ast.pretty().to_string(),
            "(file\n\
             \x20 (let x : Float @0..16\n\
             \x20   (int 1 @15..16))\n\
             \x20 (rebind y @17..35\n\
             \x20   (block @22..35\n\
             \x20     (expr @24..33\n\
             \x20       (: Float @24..33\n\
             \x20         (name x @24..25)))))\n\
             \x20 (result @36..40\n\
             \x20   (neg @38..40\n\
             \x20     (name x @39..40))))"
        );
    }

    /// Runs `f` on a thread with [`crate::STACK_SIZE`] of stack, as the
    /// pipeline is meant to be run. Test threads are far smaller by default.
    fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(crate::STACK_SIZE)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    fn too_deep(text: String) -> Diagnostic {
        on_big_stack(move || parse_text(&text).unwrap_err())
    }

    #[test]
    fn nesting_by_recursion_is_limited() {
        // MAX_DEPTH parentheses around a literal is MAX_DEPTH + 1 levels.
        let just_over = format!("{}1{}", "(".repeat(MAX_DEPTH), ")".repeat(MAX_DEPTH));
        let diagnostic = too_deep(just_over);
        assert_eq!(diagnostic.message, "expression nested too deeply");
        assert_eq!(
            diagnostic.notes,
            vec![format!("the limit is {MAX_DEPTH} levels")]
        );
        let deepest = format!(
            "{}1{}",
            "(".repeat(MAX_DEPTH - 1),
            ")".repeat(MAX_DEPTH - 1)
        );
        on_big_stack(move || parse_text(&deepest).unwrap());
        // Negations as `-(-(...))`: each pair is two levels.
        let pairs = MAX_DEPTH / 2;
        let negations = format!("{}1{}", "-(".repeat(pairs), ")".repeat(pairs));
        too_deep(negations);
    }

    #[test]
    fn nesting_blocks_is_limited() {
        // Each block is two levels of recursion: the block's body and the
        // expression inside it. With the outermost expression, N blocks are
        // 2N + 1 levels.
        let deepest_blocks = (MAX_DEPTH - 1) / 2;
        let deepest = format!(
            "{}1{}",
            "{".repeat(deepest_blocks),
            "}".repeat(deepest_blocks)
        );
        on_big_stack(move || parse_text(&deepest).unwrap());
        let just_over = format!(
            "{}1{}",
            "{".repeat(deepest_blocks + 1),
            "}".repeat(deepest_blocks + 1)
        );
        let diagnostic = too_deep(just_over);
        assert_eq!(diagnostic.message, "expression nested too deeply");
        // Items inside the blocks do not change the count.
        let with_items = format!(
            "{}1{}",
            "{ let x = 1\n".repeat(deepest_blocks + 1),
            "}".repeat(deepest_blocks + 1)
        );
        too_deep(with_items);
    }

    #[test]
    fn blocks_count_towards_height() {
        // A chain one short of the limit, inside one block, is at the limit;
        // inside two, it is over.
        let chain = format!("1{}", " + 1".repeat(MAX_DEPTH - 2));
        let one = format!("{{ {chain} }}");
        on_big_stack(move || parse_text(&one).unwrap());
        let two = format!("{{ {{ {chain} }} }}");
        let diagnostic = too_deep(two);
        assert_eq!(diagnostic.message, "expression nested too deeply");
    }

    #[test]
    fn nesting_by_left_chaining_is_limited() {
        // Each `+` is one level above its left operand, so N terms are N
        // levels deep. This nests without recursion in the parser.
        let just_over = format!("1{}", " + 1".repeat(MAX_DEPTH));
        let diagnostic = too_deep(just_over);
        assert_eq!(diagnostic.message, "expression nested too deeply");
        let deepest = format!("1{}", " + 1".repeat(MAX_DEPTH - 1));
        on_big_stack(move || parse_text(&deepest).unwrap());
        // Ascriptions chain the same way.
        let ascriptions = format!("1{}", " : Int".repeat(MAX_DEPTH));
        too_deep(ascriptions);
    }

    #[test]
    fn tokens_without_eof_are_treated_as_ending() {
        let source = SourceFile::new("test.gcsl", "1 +").unwrap();
        let (tokens, names) = tokenize(&source).unwrap();
        let without_eof = &tokens[..tokens.len() - 1];
        let diagnostic = parse(without_eof, names, &source).unwrap_err();
        assert_eq!(
            diagnostic.message,
            "expected expression, found end of input"
        );
        assert_eq!(diagnostic.span, Span::new(3, 3));
    }
}
