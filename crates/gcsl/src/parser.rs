//! The parser: tokens to an [`Ast`].
//!
//! This is a Pratt parser, also called precedence climbing. Instead of one
//! grammar function per precedence level, a single function,
//! [`Parser::parse_expr`], parses any expression whose operators bind at
//! least as tightly as a given *minimum binding power*. It first parses a
//! *prefix* expression (a literal, a parenthesised expression, or a unary
//! operator applied to an operand), which becomes the left-hand side. Then it
//! loops: if the next token is a binary operator whose left binding power is
//! at least the minimum, it consumes the operator, parses the right-hand side
//! by calling itself with the operator's right binding power as the new
//! minimum, and folds the result into a new left-hand side. Otherwise the
//! operator belongs to an enclosing call, and the loop stops.
//!
//! Precedence and associativity both fall out of the binding-power pairs. A
//! tighter operator has higher powers, so in `1 + 2 * 3` the recursive call
//! for the right-hand side of `+` (minimum 2) still accepts `*` (left power
//! 3). Left associativity comes from making the right power one higher than
//! the left: in `8 - 3 - 2`, the right-hand side of the first `-` is parsed
//! with minimum 2, which rejects the second `-` (left power 1), so the first
//! subtraction completes before the second begins. Unary minus parses its
//! operand with a minimum above every binary operator, so `-2 * 3` is
//! `(-2) * 3`.
//!
//! Errors are values. Parsing functions return `Result`, and a failure is
//! recorded in the parser's diagnostic sink before the `Err` propagates. The
//! parser stops at the first error for now; recovery can be added later by
//! catching an `Err` at a synchronisation point and continuing, with the sink
//! then holding several diagnostics.
//!
//! The parser enforces [`MAX_DEPTH`] in two places, because nesting arises in
//! two ways. Parentheses and prefix operators nest by recursion, so
//! [`Parser::parse_expr`] counts its own depth and stops before the recursion
//! itself could overflow. Left-associative chains such as `1 + 1 + 1` nest
//! without recursion, since the loop folds each operator into the left-hand
//! side, so the parser also tracks the height of every node it allocates and
//! rejects a node that would exceed the limit. Together these guarantee that
//! no tree handed to a later stage is deeper than `MAX_DEPTH`.

use crate::MAX_DEPTH;
use crate::ast::{Ast, AstBuilder, BinaryOp, Expr, ExprId, ExprKind, UnaryOp};
use crate::diagnostic::Diagnostic;
use crate::lexer::{Token, TokenKind};
use crate::source::{SourceFile, Span};

/// Binding power of the operand of prefix `-`: tighter than every binary
/// operator.
const PREFIX_BP: u8 = 5;

/// Height of a leaf node.
const LEAF_HEIGHT: u32 = 1;

/// Parses `tokens`, lexed from `source`, as one expression.
///
/// All tokens must be consumed. `tokens` should end with
/// [`TokenKind::Eof`], as [`crate::lexer::tokenize`] guarantees; if it does
/// not, running out of tokens is treated as end of input.
///
/// # Errors
///
/// Returns the first syntax error.
pub fn parse(tokens: &[Token], source: &SourceFile) -> Result<Ast, Diagnostic> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        source,
        // Every node consumes at least one token, so these never reallocate.
        builder: AstBuilder::with_capacity(tokens.len()),
        heights: Vec::with_capacity(tokens.len()),
        depth: 0,
        diagnostics: Vec::new(),
    };
    match parser.parse_root() {
        Ok(root) => Ok(parser.builder.finish(root)),
        Err(Reported) => Err(parser.diagnostics.into_iter().next().unwrap_or_else(|| {
            Diagnostic::error(
                "internal error: parser failed without reporting a diagnostic",
                Span::new(0, 0),
            )
        })),
    }
}

/// Proof that a diagnostic has been pushed to the parser's sink. Only
/// [`Parser::error`] creates one.
#[derive(Debug)]
struct Reported;

type ParseResult<T> = Result<T, Reported>;

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    source: &'a SourceFile,
    builder: AstBuilder,
    /// The height of each allocated node, indexed like the builder's arena:
    /// 1 for a leaf, one more than the tallest child otherwise.
    heights: Vec<u32>,
    /// Current recursion depth of [`Parser::parse_expr`].
    depth: usize,
    /// Errors reported so far. Only one is ever pushed until error recovery
    /// exists.
    diagnostics: Vec<Diagnostic>,
}

impl Parser<'_> {
    fn parse_root(&mut self) -> ParseResult<ExprId> {
        let root = self.parse_expr(0)?;
        let token = self.peek();
        if token.kind == TokenKind::Eof {
            Ok(root)
        } else {
            let found = self.describe(token);
            Err(self.error(format!("expected end of input, found {found}"), token.span))
        }
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
    /// [`MAX_DEPTH`]. Every successful call is paired with a decrement in
    /// [`Parser::parse_expr`].
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
        while let Some(op) = binary_op(self.peek().kind) {
            let (left_bp, right_bp) = binding_power(op);
            if left_bp < min_bp {
                break;
            }
            self.advance();
            let rhs = self.parse_expr(right_bp)?;
            let span = Span::join(self.span_of(lhs), self.span_of(rhs));
            let height = self.height_of(lhs).max(self.height_of(rhs)) + 1;
            lhs = self.alloc(ExprKind::Binary { op, lhs, rhs }, span, height)?;
        }
        Ok(lhs)
    }

    /// Parses a literal, a parenthesised expression or a prefix operator
    /// application.
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
            TokenKind::Minus => {
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
                    return Err(self.error("expected `)`", close.span));
                }
                self.advance();
                let height = self.height_of(inner) + 1;
                return self.alloc(
                    ExprKind::Paren(inner),
                    Span::join(token.span, close.span),
                    height,
                );
            }
            TokenKind::Plus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::SlashSlash
            | TokenKind::Percent
            | TokenKind::RParen
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

    /// The current token. Past the end of a token slice that lacks a final
    /// `Eof`, this synthesises one at the end of the source.
    fn peek(&self) -> Token {
        self.tokens.get(self.pos).copied().unwrap_or(Token {
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

    /// Describes a token for "found ..." messages: literals by their source
    /// text, operators by their symbol, both in backticks.
    fn describe(&self, token: Token) -> String {
        match token.kind.symbol() {
            Some(symbol) => format!("`{symbol}`"),
            None if token.kind == TokenKind::Eof => "end of input".to_owned(),
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

/// The binary operator a token denotes, if any.
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
        | TokenKind::LParen
        | TokenKind::RParen
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
    use crate::lexer::tokenize;

    fn parse_text(text: &str) -> Result<Ast, Diagnostic> {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let tokens = tokenize(&source).unwrap();
        parse(&tokens, &source)
    }

    /// Renders the tree as a compact fully-parenthesised string, making
    /// grouping explicit.
    fn shape(text: &str) -> String {
        fn go(ast: &Ast, id: ExprId, out: &mut String) {
            use std::fmt::Write as _;
            match ast.get(id).kind {
                ExprKind::Int(value) => write!(out, "{value}").unwrap(),
                ExprKind::Float(value) => write!(out, "{value:?}").unwrap(),
                ExprKind::Unary { op, operand } => {
                    out.push('(');
                    out.push_str(op.symbol());
                    go(ast, operand, out);
                    out.push(')');
                }
                ExprKind::Binary { op, lhs, rhs } => {
                    out.push('(');
                    go(ast, lhs, out);
                    out.push(' ');
                    out.push_str(op.symbol());
                    out.push(' ');
                    go(ast, rhs, out);
                    out.push(')');
                }
                ExprKind::Paren(inner) => {
                    out.push('[');
                    go(ast, inner, out);
                    out.push(']');
                }
            }
        }
        let ast = parse_text(text).unwrap();
        let mut out = String::new();
        go(&ast, ast.root(), &mut out);
        out
    }

    fn error(text: &str) -> (String, Span) {
        let diagnostic = parse_text(text).unwrap_err();
        (diagnostic.message, diagnostic.span)
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
        assert_eq!(shape("- 2 * 3"), "((-2) * 3)");
        assert_eq!(shape("-2 + 3"), "((-2) + 3)");
    }

    #[test]
    fn nested_unary_minus() {
        assert_eq!(shape("--2"), "(-(-2))");
        assert_eq!(shape("- - -2"), "(-(-(-2)))");
    }

    #[test]
    fn unary_minus_inside_binary() {
        assert_eq!(shape("2 * -3"), "(2 * (-3))");
        assert_eq!(shape("2 - -3"), "(2 - (-3))");
        assert_eq!(shape("2--3"), "(2 - (-3))");
    }

    #[test]
    fn parentheses_group() {
        assert_eq!(shape("(1 + 2) * 3"), "([(1 + 2)] * 3)");
        assert_eq!(shape("((1))"), "[[1]]");
        assert_eq!(shape("2 * (3 - (4 + 5))"), "(2 * [(3 - [(4 + 5)])])");
        assert_eq!(shape("-(1 + 2)"), "(-[(1 + 2)])");
    }

    #[test]
    fn spans() {
        let ast = parse_text("-(1 + 2) * 30").unwrap();
        let root = ast.get(ast.root());
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
            error(""),
            (
                "expected expression, found end of input".to_owned(),
                Span::new(0, 0)
            )
        );
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
    }

    #[test]
    fn expected_close_paren() {
        assert_eq!(
            error("(1 + 2"),
            ("expected `)`".to_owned(), Span::new(6, 6))
        );
        assert_eq!(error("(1 2)"), ("expected `)`".to_owned(), Span::new(3, 4)));
    }

    #[test]
    fn expected_end_of_input() {
        assert_eq!(
            error("1 2"),
            (
                "expected end of input, found `2`".to_owned(),
                Span::new(2, 3)
            )
        );
        assert_eq!(
            error("1 1_000.5e3"),
            (
                "expected end of input, found `1_000.5e3`".to_owned(),
                Span::new(2, 11)
            )
        );
        assert_eq!(
            error("(1))"),
            (
                "expected end of input, found `)`".to_owned(),
                Span::new(3, 4)
            )
        );
        assert_eq!(
            error("2 (3)"),
            (
                "expected end of input, found `(`".to_owned(),
                Span::new(2, 3)
            )
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
        let negations = format!("{}1", "-".repeat(MAX_DEPTH));
        too_deep(negations);
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
    }

    #[test]
    fn tokens_without_eof_are_treated_as_ending() {
        let source = SourceFile::new("test.gcsl", "1 +").unwrap();
        let tokens = tokenize(&source).unwrap();
        let without_eof = &tokens[..tokens.len() - 1];
        let diagnostic = parse(without_eof, &source).unwrap_err();
        assert_eq!(
            diagnostic.message,
            "expected expression, found end of input"
        );
        assert_eq!(diagnostic.span, Span::new(3, 3));
    }
}
