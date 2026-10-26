//! Parsing imports: module paths, members and aliases.
//!
//! # Imports
//!
//! An import is an item of a file's outermost body only:
//!
//! ```text
//! IMPORT  = "import" PATH ("as" NAME)?
//!         | "import" MEMBER ("," MEMBER)* "from" PATH
//!         | "import" DT ("::" NAME)? ("*" | DEFPATH) "from" (PATH ("/" GLOB)? | GLOB)
//!         | "import" DT "::" "*" "from" (PATH ("/" GLOB)? | GLOB)
//! MEMBER  = NAME ("as" NAME)? | "*"
//! DT      = NAME ("." NAME)*
//! DEFPATH = a definition's path, as a `def` writes it
//! GLOB    = "*" | "**"                       (its stars glued to the path)
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
//!
//! **Definitions of a def-type.** `import package * from pkgs/*` imports
//! every definition of the def-type `package` in what its path names
//! (`Parser::parse_def_type_import`): its one member, a use of the
//! def-type of the role [`DefTypeRole::Import`], its path empty, written
//! `DT *`. A path may end in a glob, `PATH/*`, every directory directly in
//! it, or `PATH/**`, every one at any depth below it, its `*`s glued to
//! the `/`, or be a glob alone, `*` or `**`, the root's
//! ([`ImportGlob`], `Parser::parse_from_path`); the path and its glob are
//! where the import's path is written ([`ImportDecl::path_at`]), which the
//! formatter keeps as written. A def-type whose definitions are made with a
//! variant, `def Toolchain::C gcc()`, has its definitions imported by
//! variant: `import Toolchain::C * from PATH` those made with `C`, the
//! use's variant, and `import Toolchain::* from PATH` those made with any,
//! [`ImportMember::DefType`]'s `every` (`Parser::parse_import_variant`).
//! A definition's path in place of the `*`, `import build hello from app`
//! or `import Toolchain::C gcc from lib`, imports that one definition: the
//! use's path, read as a `def`'s is (`Parser::parse_def_type_path`). A name
//! after an import's first path, `from` aside, starts one.
//! After `::`, anything but a name or `*` is "expected a variant's name or
//! `*` after `::`", and after the variant's name anything but `*` or a
//! definition's path "expected `*` or a definition's name after the
//! variant". A `DT` of anything but names joined by `.`
//! is "a def-type is a name, or names joined by `.`"; another member with
//! it, "a def-type's `*` is imported alone, as in `import package * from
//! pkgs/*`"; a glob in any other import, "a glob imports the definitions of
//! one def-type, as in ..."; and three `*`s or more, "a glob is `*` or
//! `**`".

use crate::ast::{
    DefTypeRole, DefTypeUse, ImportDecl, ImportForm, ImportGlob, ImportMember, ImportName, Item,
    ItemId, ItemKind, PathRange, PathSegment, SegmentKind,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{self, Token, TokenKind};
use crate::names::Name;
use crate::source::Span;

use super::{
    EMPTY_MODULE_SEGMENT, LIBRARY_PATH, MODULE_SEGMENT, ParseResult, Parser, QUOTED_MODULE_PATH,
    Reported, SINGLE_MEMBER,
};

/// The error for a glob in an import of anything but a def-type's
/// definitions.
const GLOB_NEEDS_DEF_TYPE: &str =
    "a glob imports the definitions of one def-type, as in `import package * from pkgs/*`";

/// What follows a def-type import's `DT` (`Parser::parse_import_variant`).
struct ImportVariant {
    /// The variant `V` of `DT::V *` or `DT::V NAME`, with where it is
    /// written.
    variant: Option<(Name, Span)>,
    /// Whether it is `DT::*`, every variant.
    every: bool,
    /// The definition's path of `DT NAME` or `DT::V NAME`, the one
    /// definition imported; empty for every one, `*`.
    path: Vec<PathSegment>,
    /// Where its `*` or its definition's path ends.
    end: Span,
}

/// The error for a def-type of an import that is no name or names joined
/// by `.`.
const DEF_TYPE_NAME: &str = "a def-type is a name, or names joined by `.`";

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
            let named = matches!(next.kind, TokenKind::Ident(_)) && !self.is_from(next);
            if alias.is_none()
                && (named || matches!(next.kind, TokenKind::Star | TokenKind::ColonColon))
            {
                return self.parse_def_type_import(keyword, base, path_span);
            }
            if next.kind != TokenKind::Comma && !self.is_from(next) {
                if alias.is_none() {
                    self.unnamed_import(base)?;
                }
                let path = self.builder.push_path(&self.path[base..]);
                self.path.truncate(base);
                let decl = ImportDecl {
                    path,
                    form: ImportForm::Whole(alias),
                    glob: ImportGlob::Exact,
                    path_at: path_span,
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
        let (path, glob, path_span) = self.parse_from_path()?;
        if glob != ImportGlob::Exact {
            return Err(self.error(GLOB_NEEDS_DEF_TYPE, path_span));
        }
        let range = self.builder.push_import_members(&members);
        let decl = ImportDecl {
            path,
            form: ImportForm::Members(range),
            glob,
            path_at: path_span,
        };
        Ok(self.finish_import(decl, keyword.span.join(path_span)))
    }

    /// Reads the path after an import's `from`, the next token, and the glob
    /// ending it, if any (see [Imports](#imports)): a path, `PATH/*` or
    /// `PATH/**`, or `*` or `**` alone, the root's; the path, the glob and
    /// the span of both.
    pub(super) fn parse_from_path(&mut self) -> ParseResult<(PathRange, ImportGlob, Span)> {
        let path_start = self.peek();
        if path_start.kind == TokenKind::Star {
            let (glob, span) = self.parse_glob(path_start.span.start)?;
            return Ok((self.builder.push_path(&[]), glob, span));
        }
        let names_a_path = matches!(
            path_start.kind,
            TokenKind::Ident(_) | TokenKind::Str(_) | TokenKind::StrStart(_)
        );
        if !names_a_path || self.is_from(path_start) {
            return Err(self.expected_path_after_from());
        }
        let base = self.path.len();
        // A path ending in `/` that a `*` follows, glued, ends in a glob.
        let (glued, end) = self.module_run();
        let globbed = self
            .source
            .slice(Span::new(path_start.span.start, end))
            .ends_with('/')
            && self.peek_at(glued).kind == TokenKind::Star
            && self.peek_at(glued).span.start == end;
        let path_span = self.parse_path_run(path_start, "after `from`", globbed)?;
        let path = self.builder.push_path(&self.path[base..]);
        self.path.truncate(base);
        if !globbed {
            return Ok((path, ImportGlob::Exact, path_span));
        }
        let (glob, glob_span) = self.parse_glob(path_span.start)?;
        Ok((path, glob, path_span.join(glob_span)))
    }

    /// Reads a glob, `*` or `**`, its stars glued to each other, the next
    /// token its first, the path it ends starting at `start`; what it is
    /// and the span from `start` through it. More stars are "a glob is `*`
    /// or `**`".
    pub(super) fn parse_glob(&mut self, start: u32) -> ParseResult<(ImportGlob, Span)> {
        let first = self.peek();
        self.advance();
        let mut end = first.span;
        let mut glued = 1;
        loop {
            let next = self.peek();
            if next.kind != TokenKind::Star || next.span.start != end.end {
                break;
            }
            self.advance();
            end = next.span;
            glued += 1;
        }
        let span = Span::new(start, end.end);
        match glued {
            1 => Ok((ImportGlob::Children, span)),
            2 => Ok((ImportGlob::Descendants, span)),
            _ => Err(self.error("a glob is `*` or `**`", first.span.join(end))),
        }
    }

    /// Reads `import DT * from PATH`, the `DT` read onto the path stack from
    /// `base`, written at `dt_span`, of the item starting at `keyword`, the
    /// next token the `*` or the `::` of `DT::V *` and `DT::*`: an import of
    /// every definition of the def-type `DT`, of its variant `V`, or of
    /// every variant of it (see [Imports](#imports)). `DT` is a name or
    /// names joined by `.`; the member stands alone, "a def-type's `*` is
    /// imported alone" otherwise.
    pub(super) fn parse_def_type_import(
        &mut self,
        keyword: Token,
        base: usize,
        dt_span: Span,
    ) -> ParseResult<ItemId> {
        let ImportVariant {
            variant,
            every,
            path: definition,
            end,
        } = self.parse_import_variant()?;
        let written: Vec<PathSegment> = self.path[base..].to_vec();
        self.path.truncate(base);
        let mut segments = Vec::new();
        for segment in written {
            let Some(name) = segment.name() else {
                return Err(self.error(DEF_TYPE_NAME, segment.span));
            };
            let text = self.names.get(name).to_owned();
            let mut start = segment.span.start;
            for part in text.split('.') {
                if !lexer::is_identifier(part) {
                    return Err(self.error(DEF_TYPE_NAME, segment.span));
                }
                let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
                segments.push(PathSegment {
                    kind: SegmentKind::Name(self.names.intern(part)),
                    span: Span::new(start, start + len),
                });
                start += len + 1;
            }
        }
        if segments.len() != 1 && self.path_has_slash(dt_span) {
            return Err(self.error(DEF_TYPE_NAME, dt_span));
        }
        let next = self.peek();
        if next.kind == TokenKind::Comma {
            return Err(self.error(
                "a def-type's `*` is imported alone, as in `import package * from pkgs/*`",
                next.span,
            ));
        }
        if !self.is_from(next) {
            let found = self.describe(next);
            return Err(self.error(format!("expected `from`, found {found}"), next.span));
        }
        self.advance();
        let dt = self.path_expr(&segments)?;
        let definition = self.builder.push_path(&definition);
        let span = dt_span.join(end);
        let def = self.builder.push_def_type(DefTypeUse {
            role: DefTypeRole::Import,
            dt,
            variant,
            path: definition,
            read: None,
            key: None,
            meta: None,
            meta_read: None,
            body: None,
            span,
        });
        let (path, glob, path_span) = self.parse_from_path()?;
        let range = self
            .builder
            .push_import_members(&[ImportMember::DefType { def, span, every }]);
        let decl = ImportDecl {
            path,
            form: ImportForm::Members(range),
            glob,
            path_at: path_span,
        };
        Ok(self.finish_import(decl, keyword.span.join(path_span)))
    }

    /// Reads what follows a def-type import's `DT`, the next token: `*`,
    /// a definition's path, `::V` and either, or `::*` (see
    /// [Imports](#imports)). Anything else after `::` is "expected a
    /// variant's name or `*` after `::`", and after `V` "expected `*` or a
    /// definition's name after the variant".
    fn parse_import_variant(&mut self) -> ParseResult<ImportVariant> {
        let mut variant = None;
        if self.peek().kind == TokenKind::ColonColon {
            self.advance();
            let after = self.peek();
            match after.kind {
                TokenKind::Star => {
                    self.advance();
                    return Ok(ImportVariant {
                        variant: None,
                        every: true,
                        path: Vec::new(),
                        end: after.span,
                    });
                }
                TokenKind::Ident(name) => {
                    self.advance();
                    variant = Some((name, after.span));
                }
                _ => {
                    return Err(
                        self.error("expected a variant's name or `*` after `::`", after.span)
                    );
                }
            }
        }
        let next = self.peek();
        match next.kind {
            TokenKind::Star => {
                self.advance();
                Ok(ImportVariant {
                    variant,
                    every: false,
                    path: Vec::new(),
                    end: next.span,
                })
            }
            TokenKind::Ident(_) if !self.is_from(next) => {
                let path = self.parse_def_type_path(false)?;
                let end = path.last().map_or(next.span, |segment| segment.span);
                Ok(ImportVariant {
                    variant,
                    every: false,
                    path,
                    end,
                })
            }
            _ => {
                let found = self.describe(next);
                Err(self.error(
                    format!("expected `*` or a definition's name after the variant, found {found}"),
                    next.span,
                ))
            }
        }
    }

    /// Whether the source written at `span` holds a `/`.
    fn path_has_slash(&self, span: Span) -> bool {
        self.source.slice(span).contains('/')
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
                    ImportMember::Named { .. } | ImportMember::DefType { .. } => None,
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
        self.parse_path_run(first, after, false)
    }

    /// Reads a module path as [`Parser::parse_module_path`] does, but, if
    /// `globbed`, its run ends in the `/` before a glob, which ends no
    /// segment: the span covers the path without that `/`.
    pub(super) fn parse_path_run(
        &mut self,
        first: Token,
        after: &str,
        globbed: bool,
    ) -> ParseResult<Span> {
        match first.kind {
            TokenKind::Str(_) if !globbed => return self.parse_library_path(first),
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
        let end = if globbed { end - 1 } else { end };
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

    /// Reads the library path `first`, a string with no interpolation, as the
    /// module path `<lib>/SEGMENTS` (see [Imports](#imports)): its segments
    /// those of a module path, separated by `/`, each one at its place in the
    /// string, after a first, [`LIBRARIES`](crate::workspace::LIBRARIES), at
    /// the opening quote, which no other path can name.
    fn parse_library_path(&mut self, first: Token) -> ParseResult<Span> {
        let span = first.span;
        let written = self.source.slice(span).to_owned();
        let text = written
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or("");
        if text.is_empty() || text.contains('\\') {
            return Err(self.error(LIBRARY_PATH, span));
        }
        self.path.push(PathSegment {
            kind: SegmentKind::Name(self.names.intern(crate::workspace::LIBRARIES)),
            span: Span::new(span.start, span.start.saturating_add(1)),
        });
        let mut start = span.start.saturating_add(1);
        for part in text.split('/') {
            let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
            let segment = Span::new(start, start.saturating_add(len));
            if part.is_empty() {
                return Err(self.error(LIBRARY_PATH, span));
            }
            if !lexer::is_module_segment(part) {
                return Err(self.error(
                    format!("`{part}` cannot name a library: {MODULE_SEGMENT}"),
                    segment,
                ));
            }
            self.path.push(PathSegment {
                kind: SegmentKind::Name(self.names.intern(part)),
                span: segment,
            });
            start = segment.end.saturating_add(1);
        }
        self.pos += 1;
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
