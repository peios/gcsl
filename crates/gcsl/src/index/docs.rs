//! Doc comments: the run of `#` lines directly above a declaration, but a
//! file's header, a run from its first line (a shebang's included), which
//! describes the file (see the documentation of [`crate::index`],
//! "Documentation").

use crate::ast::{ItemKind, PathSegment};
use crate::lexer::{self, Comments};
use crate::names::Names;
use crate::source::Span;

use super::{Index, Symbol, path_span};

impl Index {
    /// The documentation of what is at byte `offset` of the file of input
    /// number `file`, if it is declared with a doc comment: the comment's
    /// lines, each without its `#` and one space after it (see the module
    /// documentation, "Documentation").
    #[must_use]
    pub fn documentation(&self, file: u32, offset: u32) -> Option<String> {
        let symbol = self.at(file, offset)?.symbol?;
        self.documentation_of(symbol)
    }

    /// The documentation of `symbol`: the doc comment of the first item
    /// that declares it, in input then source order, that has one; for a
    /// definition with none, its meta's.
    pub(super) fn documentation_of(&self, symbol: Symbol) -> Option<String> {
        let own = self
            .declaring_items(symbol)
            .into_iter()
            .find_map(|(file, start)| self.doc_comment(file, start));
        own.or_else(|| match symbol {
            Symbol::Node(node) => self
                .meta_items(node)
                .into_iter()
                .find_map(|(file, start)| self.doc_comment(file, start)),
            _ => None,
        })
    }

    /// Where each meta of the definition whose node is `node` starts, by
    /// file, `defmeta package x.y` of `def package x.y`, paired by path.
    fn meta_items(&self, node: u32) -> Vec<(u32, u32)> {
        let Some(depth) = self.definition_depth(node) else {
            return Vec::new();
        };
        let mut path = Vec::with_capacity(depth + 1);
        let mut at = Some(node);
        while let Some(index) = at
            && let Some(entry) = self.scan.nodes.get(index as usize)
        {
            path.push(entry.name);
            at = entry.parent;
        }
        path.reverse();
        let Some((&key, wanted)) = path.split_first() else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for (file, entry) in (0u32..).zip(&self.scan.files) {
            let ast = &entry.ast;
            for &item in ast.items(ast.root()) {
                let ItemKind::DefMeta { def, .. } = ast.item(item).kind else {
                    continue;
                };
                let used = ast.def_type(def);
                let names: Vec<_> = ast.path(used.path).iter().map(PathSegment::name).collect();
                let same = names.len() == wanted.len()
                    && names.iter().zip(wanted).all(|(name, &w)| *name == Some(w));
                if used.key == Some(key) && same {
                    found.push((file, ast.item(item).span.start));
                }
            }
        }
        found
    }

    /// Where each item that declares `symbol` starts, by file, in input
    /// then source order: a local's `let`, `fn` or field item; each item
    /// whose path ends at a node; a `type` item. Interior nodes, written by
    /// no item of their own, modules, parameters and built-ins have none.
    fn declaring_items(&self, symbol: Symbol) -> Vec<(u32, u32)> {
        let sites = match symbol {
            Symbol::Local { file, name } => {
                let item = self
                    .files
                    .get(file as usize)
                    .and_then(|f| f.locals.get(&name))
                    .and_then(|local| local.item);
                let ast = self.scan.files.get(file as usize).map(|f| &f.ast);
                return match (item, ast) {
                    (Some(item), Some(ast)) => vec![(file, ast.item(item).span.start)],
                    _ => Vec::new(),
                };
            }
            Symbol::Type(decl) => {
                let Some(declared) = self.scan.types.get(decl as usize) else {
                    return Vec::new();
                };
                let Some(file) = self.scan.files.get(declared.file as usize) else {
                    return Vec::new();
                };
                return vec![(declared.file, file.ast.item(declared.item).span.start)];
            }
            Symbol::Node(node) => self.node_sites(node),
            Symbol::View(view) => self.view_sites(view),
            Symbol::Module(_) | Symbol::BuiltinType(_) => return Vec::new(),
        };
        let mut found: Vec<(u32, u32)> = sites
            .into_iter()
            .filter_map(|site| Some((site.file, self.item_ending_at(site.file, site.span)?)))
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The start of the top-level item of file `file` whose path ends at
    /// `span`: a field or `def` whose last segment it is, a definition's
    /// last segment, a meta's path, a `type` item's name.
    fn item_ending_at(&self, file: u32, span: Span) -> Option<u32> {
        let ast = &self.scan.files.get(file as usize)?.ast;
        ast.items(ast.root()).iter().find_map(|&item| {
            let ends = match ast.item(item).kind {
                ItemKind::Field { path, .. } => ast.path(path).last().map(|s| s.span),
                ItemKind::DefTypeDef { def, .. } => {
                    ast.path(ast.def_type(def).path).last().map(|s| s.span)
                }
                ItemKind::DefMeta { def, .. } => path_span(ast.path(ast.def_type(def).path)),
                ItemKind::Type { name_span, .. } => Some(name_span),
                _ => None,
            };
            (ends == Some(span)).then(|| ast.item(item).span.start)
        })
    }

    /// The comments of the file of input number `file`, found once.
    fn comments(&self, file: u32) -> Option<&Comments> {
        let cell = self.comments.get(file as usize)?;
        Some(cell.get_or_init(
            || match self.sources.get(file as usize).and_then(Option::as_ref) {
                Some(source) => lexer::lex_with_comments(source, &mut Names::new()).1,
                None => Comments::default(),
            },
        ))
    }

    /// The doc comment of the item that starts at byte `start` of the file
    /// of input number `file`: the run of lines directly above it, its
    /// attributes' lines apart, each of which is a comment alone; `None` if
    /// there is none, if the item does not start its line, or if the run
    /// starts on the file's first line, a shebang's included, which makes it
    /// the file's header (see the module documentation of [`crate::index`],
    /// "Documentation").
    pub(super) fn doc_comment(&self, file: u32, start: u32) -> Option<String> {
        let text = self.text(file);
        let comments = self.comments(file)?;
        let start = start as usize;
        let mut below = text.get(..start)?.rfind('\n').map_or(0, |at| at + 1);
        if !text.get(below..start)?.trim().is_empty() {
            return None;
        }
        let mut lines: Vec<&str> = Vec::new();
        // Where the run's first line starts.
        let mut top = None;
        while below > 0 {
            let end = below - 1;
            let above = text.get(..end)?.rfind('\n').map_or(0, |at| at + 1);
            let line = text.get(above..end)?;
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                break;
            }
            let first = u32::try_from(above + (line.len() - trimmed.len())).ok()?;
            match comments.at(first) {
                Some(comment) if comment.start == first => {
                    let body = trimmed.strip_prefix('#').unwrap_or(trimmed);
                    let body = body.strip_prefix(' ').unwrap_or(body);
                    lines.push(body.trim_end());
                    top = Some(above);
                }
                // An attribute of the item, between it and its comment.
                None if lines.is_empty()
                    && (trimmed.starts_with("#[") || trimmed.starts_with("#![")) => {}
                _ => break,
            }
            below = above;
        }
        // A run from the file's first line, or from a shebang on it, is the
        // file's header, which describes the file, not the item below it.
        if top == Some(0) {
            return None;
        }
        // Blank lines at either end are no part of it.
        while lines.first().is_some_and(|line| line.is_empty()) {
            lines.remove(0);
        }
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        if lines.is_empty() {
            return None;
        }
        lines.reverse();
        Some(lines.join("\n"))
    }
}
