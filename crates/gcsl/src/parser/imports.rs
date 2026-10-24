//! Parsing imports: module paths, members and aliases.
//!
//! # Imports
//!
//! An import is an item of a file's outermost body only:
//!
//! ```text
//! IMPORT  = "import" PATH ("as" NAME)?
//!         | "import" MEMBER ("," MEMBER)* "from" PATH
//! MEMBER  = NAME ("as" NAME)? | "*"
//! PATH    = SEGMENT ("/" SEGMENT)*          (one run of glued tokens, below)
//! SEGMENT = a run of ASCII letters, digits, `_`, `-`, `+` and `.`
//! ```
//!
//! (see [`ItemKind::Import`] and [`ImportDecl`]). `import` is a keyword. `as`
//! was already one, for narrowing (`x as Int`), and is the same token here.
//! `from` is contextual: it is the keyword wherever it stands in an import
//! item, its first token after `import` included, and a name everywhere
//! else, so a module or a member called `from` cannot be imported (`import
//! from as f` is "expected a path after `from`"). The two forms are told
//! apart after the first path or member and its `as`: a `,` or `from` next
//! makes it a member list.
//!
//! A path names directories and files from the workspace's root, as the
//! file system does: its segments are separated by `/`, and a segment is
//! named as its directory or file is, dots and all, so
//! `pkgs/net.busybox.busybox` is the two segments `pkgs` and
//! `net.busybox.busybox` (`Parser::parse_module_path`). The lexer, unchanged,
//! makes tokens of the path's parts, `pkgs`, `/`, `net`, `.` and so on, and
//! the path is the run of the token it starts with, an identifier or a
//! number, and every token after it that directly follows the one before,
//! nothing between them, whose text is made only of ASCII letters, digits,
//! `_`, `-`, `+`, `.` and `/` (`Parser::module_run`); the run's text is split
//! at each `/`, and each part, interned, names a segment
//! ([`PathSegment`], a name of the source's text, spanning its part). So
//! a space ends the path: `import lib /x` is the path `lib` and "expected
//! end of item, found `/`". A segment starts with an ASCII letter, a digit
//! or `_`, ends with one or `+` (`gmp-c++`), and has no `..` in it
//! ([`lexer::is_module_segment`]): any other is "`x.` cannot name a module:
//! ...", at the segment, and a `/` that ends the path, or follows another,
//! "expected a module name before and after each `/`", at it.
//!
//! A whole import with no `as` binds its path's last segment, which must
//! then be a name: an identifier, not a keyword. `import
//! pkgs/net.busybox.busybox` is "`net.busybox.busybox` is not a name, so
//! the import needs one", at the segment, with a note to write `as NAME`,
//! and, when the segment is identifiers joined by `.`, a second saying that
//! a path separates its segments with `/`: dots once separated them, and
//! `import lib.x`, which was `lib/x.gcsl`, is now the one segment `lib.x`
//! (the workspace says the same of such a path that names nothing, and
//! suggests the path with slashes, see [`crate::workspace`], "Imports").
//! With `as`, any segment will do.
//!
//! An `import` that starts an item of any other body, a block's, a
//! function's or a `lazy`'s, is "`import` is only allowed at the top level
//! of a file"; anywhere else it is reported as any keyword is. A path
//! written as a string (`import "github.com/x"`) is "a module path is
//! written without quotes, its segments separated by `/`"; any other token
//! where a path starts is "expected a module path after `import`" or
//! "expected a path after `from`". A member that is not one name, a path
//! (`import a/b, c from p`) or a name with dots (`import a.b from p`), is
//! "a name imported with `from` is a single name". `*` without `from` is
//! "`import *` needs `from` and a path", and a second `*` in one list "`*`
//! is imported more than once". Whether the path names anything is the
//! workspace's business, and outside a workspace the checker's.

use crate::ast::{
    ImportDecl, ImportForm, ImportMember, ImportName, Item, ItemId, ItemKind, PathSegment,
    SegmentKind,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{self, Token, TokenKind};
use crate::source::Span;

use super::{
    EMPTY_MODULE_SEGMENT, MODULE_SEGMENT, ParseResult, Parser, QUOTED_MODULE_PATH, Reported,
    SINGLE_MEMBER,
};

impl Parser<'_> {
    /// Parses an import item of the file's body, starting at its keyword,
    /// `keyword` (see [Imports](#imports)).
    pub(super) fn parse_import(&mut self, keyword: Token) -> ParseResult<ItemId> {
        self.advance();
        let first = self.peek();
        let mut members = Vec::new();
        if first.kind == TokenKind::Star {
            self.advance();
            members.push(ImportMember::All(first.span));
        } else {
            let mut end = keyword.span;
            if self.is_from(first) {
                self.advance();
                return Err(self.expected_path_after_from());
            }
            let base = self.path.len();
            let path_span = self.parse_module_path(first, "after `import`")?;
            end = end.join(path_span);
            let alias = self.parse_import_alias(&mut end)?;
            let next = self.peek();
            if next.kind != TokenKind::Comma && !self.is_from(next) {
                if alias.is_none() {
                    self.unnamed_import(base)?;
                }
                let path = self.builder.push_path(&self.path[base..]);
                self.path.truncate(base);
                let decl = ImportDecl {
                    path,
                    form: ImportForm::Whole(alias),
                };
                return Ok(self.finish_import(decl, keyword.span.join(end)));
            }
            let segments = &self.path[base..];
            let first_segment = segments.first().copied();
            let single = segments.len() == 1
                && first_segment.is_some_and(|segment| {
                    segment
                        .name()
                        .is_some_and(|name| lexer::is_identifier(self.names.get(name)))
                });
            self.path.truncate(base);
            if !single {
                return Err(self.error(SINGLE_MEMBER, path_span));
            }
            if let Some(PathSegment {
                kind: SegmentKind::Name(name),
                span,
            }) = first_segment
            {
                members.push(ImportMember::Named {
                    name: ImportName { name, span },
                    alias,
                });
            }
        }
        self.parse_import_members(&mut members)?;
        let path_start = self.peek();
        let names_a_path = matches!(
            path_start.kind,
            TokenKind::Ident(_) | TokenKind::Str(_) | TokenKind::StrStart(_)
        );
        if !names_a_path || self.is_from(path_start) {
            return Err(self.expected_path_after_from());
        }
        let base = self.path.len();
        let path_span = self.parse_module_path(path_start, "after `from`")?;
        let path = self.builder.push_path(&self.path[base..]);
        self.path.truncate(base);
        let range = self.builder.push_import_members(&members);
        let decl = ImportDecl {
            path,
            form: ImportForm::Members(range),
        };
        Ok(self.finish_import(decl, keyword.span.join(path_span)))
    }

    /// Reads the rest of a `from` import's member list after its first
    /// member, which is in `members`, through its `from`, adding each.
    pub(super) fn parse_import_members(
        &mut self,
        members: &mut Vec<ImportMember>,
    ) -> ParseResult<()> {
        loop {
            let next = self.peek();
            if self.is_from(next) {
                self.advance();
                return Ok(());
            }
            if next.kind != TokenKind::Comma {
                if let Some(star) = members.iter().find_map(|member| match member {
                    ImportMember::All(span) => Some(*span),
                    ImportMember::Named { .. } => None,
                }) {
                    return Err(self.error("`import *` needs `from` and a path", star));
                }
                let found = self.describe(next);
                return Err(self.error(format!("expected `,` or `from`, found {found}"), next.span));
            }
            self.advance();
            let member = self.peek();
            if member.kind == TokenKind::Star {
                self.advance();
                if members.iter().any(|m| matches!(m, ImportMember::All(_))) {
                    return Err(self.error("`*` is imported more than once", member.span));
                }
                members.push(ImportMember::All(member.span));
                continue;
            }
            let TokenKind::Ident(name) = member.kind else {
                let found = self.describe(member);
                return Err(self.error(
                    format!("expected a name to import after `,`, found {found}"),
                    member.span,
                ));
            };
            if self.is_from(member) {
                let found = self.describe(member);
                return Err(self.error(
                    format!("expected a name to import after `,`, found {found}"),
                    member.span,
                ));
            }
            if self.module_run().0 > 1 {
                let base = self.path.len();
                let span = self.parse_module_path(member, "after `,`")?;
                self.path.truncate(base);
                return Err(self.error(SINGLE_MEMBER, span));
            }
            self.advance();
            let mut member_end = member.span;
            let alias = self.parse_import_alias(&mut member_end)?;
            members.push(ImportMember::Named {
                name: ImportName {
                    name,
                    span: member.span,
                },
                alias,
            });
        }
    }

    /// Reads a module path, segments separated by `/`, starting at `first`,
    /// which is the next token, onto the path stack (the caller moves or
    /// discards it), and returns the span it covers (see [Imports](#imports)).
    /// The path is the run of `first` and every token glued to it
    /// ([`Parser::module_run`]), split at each `/`; each segment is named by
    /// its source text, interned, so `pkgs/net.busybox.busybox` is the two
    /// segments `pkgs` and `net.busybox.busybox`. `after` says where the
    /// path is, for the error when `first` cannot start one.
    pub(super) fn parse_module_path(&mut self, first: Token, after: &str) -> ParseResult<Span> {
        match first.kind {
            TokenKind::Str(_) | TokenKind::StrStart(_) => {
                return Err(self.error(QUOTED_MODULE_PATH, first.span));
            }
            TokenKind::Ident(_) if !self.is_from(first) => {}
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::Duration(_)
            | TokenKind::Size(_) => {}
            _ => {
                let found = self.describe(first);
                return Err(self.error(
                    format!("expected a module path {after}, found {found}"),
                    first.span,
                ));
            }
        }
        let (glued, end) = self.module_run();
        let span = Span::new(first.span.start, end);
        let text = self.source.slice(span).to_owned();
        let mut start = span.start;
        let parts = text.split('/').count();
        for (index, part) in text.split('/').enumerate() {
            let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
            let segment = Span::new(start, start.saturating_add(len));
            if part.is_empty() {
                // The run starts with a name or a number, so an empty
                // segment follows a `/`: the `/` that ends the path, or the
                // second of two in a row.
                let slash = if index + 1 == parts {
                    Span::new(start.saturating_sub(1), start)
                } else {
                    Span::new(start, start + 1)
                };
                return Err(self.error(EMPTY_MODULE_SEGMENT, slash));
            }
            if !lexer::is_module_segment(part) {
                return Err(self.error(
                    format!("`{part}` cannot name a module: {MODULE_SEGMENT}"),
                    segment,
                ));
            }
            self.path.push(PathSegment {
                kind: SegmentKind::Name(self.names.intern(part)),
                span: segment,
            });
            start = segment.end.saturating_add(1);
        }
        self.pos += glued;
        Ok(span)
    }

    /// How many tokens, from the current one on, make up the run of a module
    /// path, and where the run ends (see [Imports](#imports)): the current
    /// token, and every token after it that directly follows the one before,
    /// nothing between them, and whose text is made only of ASCII letters,
    /// digits, `_`, `-`, `+`, `.` and `/`.
    pub(super) fn module_run(&self) -> (usize, u32) {
        let first = self.peek();
        let (mut count, mut end) = (1, first.span.end);
        loop {
            let token = self.peek_at(count);
            let text = self.source.slice(token.span);
            if token.span.start != end
                || text.is_empty()
                || !text.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'.' | b'/')
                })
            {
                return (count, end);
            }
            count += 1;
            end = token.span.end;
        }
    }

    /// Checks that a whole import with no `as`, whose path is on the path
    /// stack from `base`, can be bound as its path's last segment: that the
    /// segment is a name, as `helpers` is and `net.busybox.busybox` is not
    /// (see [Imports](#imports)).
    pub(super) fn unnamed_import(&mut self, base: usize) -> ParseResult<()> {
        let Some(&last) = self.path[base..].last() else {
            return Ok(());
        };
        let Some(name) = last.name() else {
            return Ok(());
        };
        let text = self.names.get(name);
        if lexer::is_identifier(text) {
            return Ok(());
        }
        let message = format!("`{text}` is not a name, so the import needs one");
        let note = format!("write `as NAME` after the path to name what `{text}` imports");
        let mut error = Diagnostic::error(message, last.span).with_note(note);
        // A path in the old form, its segments separated by `.`, is one
        // segment of dots now.
        if text.split('.').all(lexer::is_identifier) {
            let slashed = text.replace('.', "/");
            error = error.with_note(format!(
                "a module path separates its segments with `/`: if `{text}` is meant as \
                 directories, write `{slashed}`"
            ));
        }
        Err(self.report(error))
    }

    /// Reads `as NAME` after an import's path or member, if the next token
    /// is `as`, widening `end` to cover it.
    pub(super) fn parse_import_alias(&mut self, end: &mut Span) -> ParseResult<Option<ImportName>> {
        if self.peek().kind != TokenKind::As {
            return Ok(None);
        }
        self.advance();
        let token = self.peek();
        match token.kind {
            TokenKind::Ident(name) if !self.is_from(token) => {
                self.advance();
                *end = end.join(token.span);
                Ok(Some(ImportName {
                    name,
                    span: token.span,
                }))
            }
            _ => {
                let found = self.describe(token);
                Err(self.error(
                    format!("expected a name after `as`, found {found}"),
                    token.span,
                ))
            }
        }
    }

    /// Whether `token` is the identifier `from`, which is the keyword
    /// wherever it appears in an import item.
    pub(super) fn is_from(&self, token: Token) -> bool {
        matches!(token.kind, TokenKind::Ident(name) if self.names.get(name) == "from")
    }

    /// Reports "expected a path after `from`" at the next token.
    pub(super) fn expected_path_after_from(&mut self) -> Reported {
        let token = self.peek();
        let found = self.describe(token);
        self.error(
            format!("expected a path after `from`, found {found}"),
            token.span,
        )
    }

    /// Allocates the import item `decl`, spanning `span`.
    pub(super) fn finish_import(&mut self, decl: ImportDecl, span: Span) -> ItemId {
        let id = self.builder.push_import(decl);
        self.builder.alloc_item(Item {
            kind: ItemKind::Import(id),
            span,
        })
    }
}
