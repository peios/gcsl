//! Renaming a symbol everywhere it is written, checked by indexing the
//! workspace again as renamed (see the documentation of [`crate::index`],
//! "Rename").

use std::collections::HashMap;

use crate::ast::{ArgKind, ExprKind};
use crate::check::Builtin;
use crate::diagnostic::Severity;
use crate::lexer;
use crate::names::{self, Name};
use crate::source::{SourceFile, Span};
use crate::workspace::Target;

use super::{Index, Indexed, Key, Symbol};

/// What [`Index::prepare_rename`] answers: the name to rename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    /// Where the name is written.
    pub span: Span,
    /// The name, as written.
    pub placeholder: String,
}

/// One change a rename makes: the text at a place of a file replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// The file's input number.
    pub file: u32,
    /// What is replaced, in the file as it is.
    pub span: Span,
    /// What replaces it.
    pub text: String,
}

/// What a rename of a symbol changes, before its new name is known.
#[derive(Debug)]
struct Plan {
    /// The name.
    name: Name,
    /// Where it is written under the cursor.
    at: Span,
    /// Every place it is written, by file.
    places: Vec<(u32, Span)>,
    /// For a definition's path's segment, whether it is the first: a new
    /// name for it is a segment's, not an identifier's.
    segment: Option<bool>,
}

/// Why a rename cannot be made, to the user.
type Refusal = String;

impl Index {
    /// The name at byte `offset` of the file of input number `file`, if it
    /// can be renamed (see the module documentation, "Rename"); otherwise
    /// why not.
    ///
    /// # Errors
    ///
    /// Nothing there can be renamed: no name, or the name of something the
    /// standard library, a built-in, a directory or a file declares, an
    /// import's alias, or a name written as a string somewhere.
    pub fn prepare_rename(&self, file: u32, offset: u32) -> Result<Prepared, Refusal> {
        let plan = self.plan(file, offset)?;
        let placeholder = self
            .text(file)
            .get(plan.at.start as usize..plan.at.end as usize)
            .unwrap_or_default()
            .to_owned();
        Ok(Prepared {
            span: plan.at,
            placeholder,
        })
    }

    /// The edits that rename what is at byte `offset` of the file of input
    /// number `file` to `new`, in every file of the workspace, in input then
    /// source order, once checked: the workspace's files with them made are
    /// indexed again by `reindex` (given the files after the standard ones,
    /// as the index was made of them), and the rename is refused if that
    /// finds an error the index did not, or any name written anywhere
    /// refers to something else than it did (see the module
    /// documentation, "Rename").
    ///
    /// # Errors
    ///
    /// The file's braces do not match ("the file has an unclosed `{`"),
    /// as its check needs the text as it is, not as [`Index::closing`]
    /// balances it; what [`Index::prepare_rename`] refuses; `new` is not a
    /// name of the right kind; or the check of the renamed workspace found
    /// a problem, the first one named.
    pub fn rename(
        &self,
        (file, offset): (u32, u32),
        new: &str,
        reindex: impl FnOnce(&[SourceFile]) -> Indexed,
    ) -> Result<Vec<Edit>, Refusal> {
        if let Some(why) = self.unbalanced_refusal(file) {
            return Err(why);
        }
        let plan = self.plan(file, offset)?;
        let old = self.scan.names.get(plan.name).to_owned();
        let fits = match plan.segment {
            Some(first) => is_segment(new, first),
            None => lexer::is_identifier(new),
        };
        if !fits {
            return Err(match plan.segment {
                Some(_) => format!("`{new}` cannot be a definition path's segment"),
                None => format!("`{new}` is not a name: it must be one identifier"),
            });
        }
        if new == old {
            return Ok(Vec::new());
        }
        // A shorthand argument, `f(x = 1, old)`, is a read of the variable
        // and the name of its parameter at once: renamed, it keeps naming the
        // parameter, `old = new`.
        let edits: Vec<Edit> = plan
            .places
            .iter()
            .map(|&(file, span)| Edit {
                file,
                span,
                text: if self.is_shorthand(file, span) {
                    format!("{old} = {new}")
                } else {
                    new.to_owned()
                },
            })
            .collect();
        let shift = Shift::of(&edits);
        let standard = self.standard() as usize;
        let mut sources = Vec::new();
        for (at, source) in self.sources.iter().enumerate().skip(standard) {
            let Some(source) = source else {
                return Err("the workspace's files are not all known".to_owned());
            };
            let file = u32::try_from(at).unwrap_or(u32::MAX);
            let text = shift.apply(file, source.text());
            let renamed = SourceFile::new(source.name(), text).map_err(|e| e.to_string())?;
            sources.push(renamed);
        }
        let renamed = reindex(&sources);
        self.verify(&renamed, &shift, (&old, new))?;
        Ok(edits)
    }

    /// What renaming what is at byte `offset` of file `file` changes, or
    /// why it cannot be renamed.
    fn plan(&self, file: u32, offset: u32) -> Result<Plan, Refusal> {
        let nothing = || "there is no name here to rename".to_owned();
        let found = self.at(file, offset).ok_or_else(nothing)?;
        if file < self.standard() {
            return Err(nothing());
        }
        let symbol = found.symbol.ok_or_else(|| {
            let name = found.name.map_or("", |name| self.scan.names.get(name));
            format!(
                "`{name}` is a field of a value whose record is not known: it cannot be renamed"
            )
        })?;
        let keys = self.closure(symbol);
        for &key in &keys {
            self.renamable(key)?;
        }
        let name = self.declared_name(symbol).ok_or_else(nothing)?;
        let text = self.scan.names.get(name);
        let at = match found.name {
            Some(written) if written != name => {
                let alias = self.scan.names.get(written);
                return Err(format!(
                    "`{alias}` is an import's alias of `{text}`: rename `{text}`, or the alias by hand"
                ));
            }
            Some(_) => found.span,
            None => match symbol {
                Symbol::Type(_) => self.tail(file, found.span, text).ok_or_else(nothing)?,
                _ => {
                    return Err(
                        "a meta's path is its definition's: rename the definition".to_owned()
                    );
                }
            },
        };
        let mut places = Vec::new();
        for (in_file, occurrence) in self.occurrences_of(&keys) {
            match occurrence.name {
                Some(written) if written == name => places.push((in_file, occurrence.span)),
                None if matches!(occurrence.symbol, Some(Symbol::Type(_))) => {
                    if let Some(span) = self.tail(in_file, occurrence.span, text) {
                        places.push((in_file, span));
                    }
                }
                _ => {}
            }
        }
        let mut segment = None;
        for &key in &keys {
            if let Key::Node(node) = key
                && let Some(depth) = self.definition_depth(node)
            {
                segment = Some(depth == 1);
                self.meta_places(node, depth, &mut places);
            }
        }
        places.sort_unstable_by_key(|&(file, span)| (file, span.start, span.end));
        places.dedup();
        for &(in_file, span) in &places {
            let written = self
                .text(in_file)
                .get(span.start as usize..span.end as usize)
                .unwrap_or_default();
            if written.starts_with('"') || written.starts_with("r\"") {
                return Err(format!(
                    "`{text}` is written as a string at {}: rename it by hand",
                    self.place(in_file, span.start)
                ));
            }
        }
        Ok(Plan {
            name,
            at,
            places,
            segment,
        })
    }

    /// The keys of `symbol`, and of every symbol an occurrence filed under
    /// one of them has, and so on: what is renamed with it, a view's node
    /// with every node it merges.
    fn closure(&self, symbol: Symbol) -> Vec<Key> {
        let mut keys = self.keys(symbol);
        let mut seen = 0;
        while seen < keys.len() {
            let more: Vec<Key> = self
                .occurrences_of(&keys[seen..])
                .into_iter()
                .filter_map(|(_, occurrence)| occurrence.symbol)
                .flat_map(|other| self.keys(other))
                .collect();
            seen = keys.len();
            for key in more {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        keys
    }

    /// Whether what `key` is can be renamed: not if a built-in, the
    /// standard library, a directory or a file declares it.
    fn renamable(&self, key: Key) -> Result<(), Refusal> {
        let standard = self.standard();
        let library = |name: &str| format!("`{name}` is declared by the standard library");
        match key {
            Key::Local(file, _) if file < standard => Err(library("this")),
            Key::Local(..) => Ok(()),
            Key::Node(node) => {
                let Some(entry) = self.scan.nodes.get(node as usize) else {
                    return Ok(());
                };
                let name = self.scan.names.get(entry.name);
                if (node as usize) < Builtin::ALL.len() {
                    return Err(format!("`{name}` is built in"));
                }
                let in_library = (0..standard).any(|file| {
                    self.scan
                        .files
                        .get(file as usize)
                        .is_some_and(|f| f.dir == entry.dir)
                });
                if in_library {
                    return Err(library(name));
                }
                Ok(())
            }
            Key::Module(Target::Dir(dir)) => {
                let path = self
                    .scan
                    .dirs
                    .get(dir as usize)
                    .map_or("", |d| d.path.as_str());
                if (0..standard).any(|file| {
                    self.scan
                        .files
                        .get(file as usize)
                        .is_some_and(|f| f.dir == dir)
                }) || dir == self.scan.builtins_dir
                {
                    return Err("this module is built in, or the standard library's".to_owned());
                }
                Err(format!(
                    "this is the directory `{path}/`: a rename does not rename directories or files"
                ))
            }
            Key::Module(Target::File(file)) => {
                if file < standard {
                    return Err(library("this module"));
                }
                let path = self.path(file).unwrap_or_default();
                Err(format!(
                    "this is the file `{path}`: a rename does not rename directories or files"
                ))
            }
            Key::Module(Target::Fileroot(_)) => {
                Err("`fileroot` and its members are built in".to_owned())
            }
            Key::Module(_) => Err("this is a module: it cannot be renamed".to_owned()),
            Key::Type(decl) => match self.scan.types.get(decl as usize) {
                Some(declared) if declared.file < standard => {
                    Err(library(self.scan.names.get(declared.name)))
                }
                _ => Ok(()),
            },
            Key::BuiltinType(name) => Err(format!(
                "`{}` is a built-in type",
                self.scan.names.get(name)
            )),
        }
    }

    /// The name `symbol` is declared with.
    fn declared_name(&self, symbol: Symbol) -> Option<Name> {
        match symbol {
            Symbol::Local { file, name } => {
                let local = self.files.get(file as usize)?.locals.get(&name)?;
                Some(local.name)
            }
            Symbol::Node(node) => Some(self.scan.nodes.get(node as usize)?.name),
            Symbol::View(view) => Some(self.scan.views.get(view as usize)?.name),
            Symbol::Type(decl) => Some(self.scan.types.get(decl as usize)?.name),
            Symbol::Module(_) | Symbol::BuiltinType(_) => None,
        }
    }

    /// The span of `name` at the end of `span` of file `file`, if it ends
    /// there: a qualified type's name, `T` of `lib.T`.
    fn tail(&self, file: u32, span: Span, name: &str) -> Option<Span> {
        let written = self
            .text(file)
            .get(span.start as usize..span.end as usize)?;
        if !written.ends_with(name) {
            return None;
        }
        let length = u32::try_from(name.len()).ok()?;
        Some(Span::new(span.end.checked_sub(length)?, span.end))
    }

    /// How deep the node `node` is in a def-type's namespace, its key's
    /// child 1, if it is a definition's or a prefix of one's.
    pub(super) fn definition_depth(&self, node: u32) -> Option<usize> {
        let mut depth = 0;
        let mut at = node;
        while let Some(parent) = self.scan.nodes.get(at as usize)?.parent {
            depth += 1;
            at = parent;
        }
        let root = self.scan.names.get(self.scan.nodes.get(at as usize)?.name);
        (depth >= 1 && names::key_def_type(root).is_some()).then_some(depth)
    }

    /// Adds to `places` the segment at `depth` of the path of each meta of
    /// a definition at or under the node `node` of a def-type's namespace:
    /// a meta is paired with its definition by its path.
    fn meta_places(&self, node: u32, depth: usize, places: &mut Vec<(u32, Span)>) {
        let mut path = Vec::new();
        let mut at = Some(node);
        while let Some(index) = at
            && let Some(entry) = self.scan.nodes.get(index as usize)
        {
            path.push(entry.name);
            at = entry.parent;
        }
        path.reverse();
        let Some((&key, wanted)) = path.split_first() else {
            return;
        };
        for (file, entry) in (0u32..)
            .zip(&self.scan.files)
            .skip(self.standard() as usize)
        {
            let ast = &entry.ast;
            for (_, used) in ast.def_types() {
                if used.meta.is_none() || used.meta_read.is_some() || used.key != Some(key) {
                    continue;
                }
                let segments = ast.path(used.path);
                let matches = segments.len() >= depth
                    && segments
                        .iter()
                        .zip(wanted)
                        .all(|(segment, &name)| segment.name() == Some(name));
                if matches && let Some(segment) = segments.get(depth - 1) {
                    places.push((file, segment.span));
                }
            }
        }
    }

    /// Where byte `offset` of the file of input number `file` is, as
    /// `path:line:column`.
    fn place(&self, file: u32, offset: u32) -> String {
        let path = self.path(file).unwrap_or_default();
        match self.sources.get(file as usize).and_then(Option::as_ref) {
            Some(source) => {
                let (line, column) = source.line_col(offset);
                format!("{path}:{line}:{column}")
            }
            None => path.to_owned(),
        }
    }

    /// Checks `renamed`, the index of the workspace with the edits of
    /// `shift` made, renaming `old` to `new`: it must have no error this
    /// index does not have, and every occurrence of each file must refer
    /// to what it did, its symbol's declarations being where they were.
    fn verify(
        &self,
        renamed: &Indexed,
        shift: &Shift,
        (old, new): (&str, &str),
    ) -> Result<(), Refusal> {
        let why = |problem: String| format!("renaming `{old}` to `{new}` would {problem}");
        let mut errors: HashMap<(Option<u32>, Span, String), usize> = HashMap::new();
        for error in &self.errors {
            let key = (error.input, error.span, error.message.clone());
            *errors.entry(key).or_default() += 1;
        }
        let lone = self.standard();
        for error in renamed
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
        {
            let file = error.input.unwrap_or(lone);
            let span = shift.back_span(file, error.span);
            let key = (error.input, span, error.message.replace(new, old));
            match errors.get_mut(&key) {
                Some(count) if *count > 0 => *count -= 1,
                _ => {
                    return Err(why(format!(
                        "be an error: {} (at {})",
                        error.message,
                        self.place(file, span.start)
                    )));
                }
            }
        }
        let Some(index) = &renamed.index else {
            return Err(why("leave the workspace unchecked".to_owned()));
        };
        let mut ours = Identities::default();
        let mut theirs = Identities::default();
        for file in self.standard()..u32::try_from(self.files.len()).unwrap_or(u32::MAX) {
            let mut before: HashMap<Span, (Symbol, Option<Name>)> = self
                .occurrences(file)
                .iter()
                .filter_map(|o| Some((o.span, (o.symbol?, o.name))))
                .collect();
            for occurrence in index.occurrences(file) {
                let Some(symbol) = occurrence.symbol else {
                    continue;
                };
                let span = shift.back_span(file, occurrence.span);
                let written = self.text(file).get(span.start as usize..span.end as usize);
                let written = written.unwrap_or_default();
                let Some((was, _)) = before.remove(&span) else {
                    return Err(why(format!(
                        "make `{written}` at {} refer to something it did not",
                        self.place(file, span.start)
                    )));
                };
                let then = ours.of(self, was, None);
                let now = theirs.of(index, symbol, Some(shift));
                if then != now {
                    return Err(why(format!(
                        "make `{written}` at {} refer to something else",
                        self.place(file, span.start)
                    )));
                }
            }
            if let Some((span, _)) = before.into_iter().min_by_key(|(span, _)| span.start) {
                let written = self.text(file).get(span.start as usize..span.end as usize);
                return Err(why(format!(
                    "leave `{}` at {} referring to nothing",
                    written.unwrap_or_default(),
                    self.place(file, span.start)
                )));
            }
        }
        Ok(())
    }
}

/// What a symbol is, told apart across two indexes of one workspace
/// whatever their numbering: where it is declared, or, for what has no
/// declaration in a file, its name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Identity {
    /// Its declarations, by file and span, in the files before the rename.
    Sites(Vec<(u32, u32, u32)>),
    /// What it is called.
    Named(String),
}

/// The identities of an index's symbols, each found once.
#[derive(Default)]
struct Identities(HashMap<Symbol, Identity>);

impl Identities {
    /// The identity of `symbol` of `index`, its declarations mapped back
    /// through `shift` if `index` is of the files renamed.
    fn of(&mut self, index: &Index, symbol: Symbol, shift: Option<&Shift>) -> Identity {
        if let Some(found) = self.0.get(&symbol) {
            return found.clone();
        }
        let named = match symbol {
            Symbol::Module(Target::Dir(dir)) => Some(format!(
                "directory {}",
                index
                    .scan
                    .dirs
                    .get(dir as usize)
                    .map_or("", |d| d.path.as_str())
            )),
            Symbol::Module(Target::File(file)) => {
                Some(format!("file {}", index.path(file).unwrap_or_default()))
            }
            Symbol::Module(Target::Fileroot(member)) => Some(format!("fileroot {member:?}")),
            Symbol::BuiltinType(name) => Some(format!("type {}", index.scan.names.get(name))),
            _ => None,
        };
        let identity = named.map_or_else(
            || {
                let sites = match symbol {
                    Symbol::Local { file, name } => vec![super::Location { file, span: name }],
                    Symbol::Node(node) => index.node_sites(node),
                    Symbol::View(view) => index.view_sites(view),
                    Symbol::Module(target) => index.module_sites(target),
                    Symbol::Type(decl) => index
                        .scan
                        .types
                        .get(decl as usize)
                        .map(|declared| super::Location {
                            file: declared.file,
                            span: declared.span,
                        })
                        .into_iter()
                        .collect(),
                    Symbol::BuiltinType(_) => Vec::new(),
                };
                let mut sites: Vec<(u32, u32, u32)> = sites
                    .into_iter()
                    .map(|site| {
                        let span = shift.map_or(site.span, |s| s.back_span(site.file, site.span));
                        (site.file, span.start, span.end)
                    })
                    .collect();
                sites.sort_unstable();
                sites.dedup();
                if sites.is_empty() {
                    let shown = match symbol {
                        Symbol::Node(node) => index.node_path(node),
                        Symbol::View(view) => index.view_path(view),
                        _ => String::new(),
                    };
                    Identity::Named(format!("{shown} {symbol:?}"))
                } else {
                    Identity::Sites(sites)
                }
            },
            Identity::Named,
        );
        self.0.insert(symbol, identity.clone());
        identity
    }
}

/// The edits of a rename, by file, to map places between the files as
/// they are and as renamed.
#[derive(Debug, Default)]
struct Shift {
    /// Each file's edits, in source order: what each replaces, and its new
    /// text.
    files: HashMap<u32, Vec<(Span, String)>>,
}

impl Shift {
    /// The shift of `edits`, which do not overlap.
    fn of(edits: &[Edit]) -> Self {
        let mut files: HashMap<u32, Vec<(Span, String)>> = HashMap::new();
        for edit in edits {
            files
                .entry(edit.file)
                .or_default()
                .push((edit.span, edit.text.clone()));
        }
        for list in files.values_mut() {
            list.sort_by_key(|(span, _)| span.start);
        }
        Self { files }
    }

    /// `text`, the text of file `file`, with its edits made.
    fn apply(&self, file: u32, text: &str) -> String {
        let Some(edits) = self.files.get(&file) else {
            return text.to_owned();
        };
        let mut out = String::with_capacity(text.len());
        let mut from = 0;
        for (span, new) in edits {
            out.push_str(text.get(from..span.start as usize).unwrap_or_default());
            out.push_str(new);
            from = span.end as usize;
        }
        out.push_str(text.get(from..).unwrap_or_default());
        out
    }

    /// The place of the file `file` as it is that `offset` of it renamed
    /// is at: an edit's new text's start and end are those of what it
    /// replaced, and a place inside it is its start.
    fn back(&self, file: u32, offset: u32) -> u32 {
        let Some(edits) = self.files.get(&file) else {
            return offset;
        };
        let mut delta: i64 = 0;
        let offset = i64::from(offset);
        for (span, new) in edits {
            let start = i64::from(span.start) + delta;
            let length = i64::try_from(new.len()).unwrap_or(0);
            let end = start + length;
            if offset < start {
                break;
            }
            if offset <= end {
                let old = if offset == end && offset != start {
                    span.end
                } else {
                    span.start
                };
                return old;
            }
            delta += length - i64::from(span.end - span.start);
        }
        u32::try_from(offset - delta).unwrap_or(u32::MAX)
    }

    /// [`Shift::back`] of both ends of `span`.
    fn back_span(&self, file: u32, span: Span) -> Span {
        let start = self.back(file, span.start);
        let end = self.back(file, span.end).max(start);
        Span::new(start, end)
    }
}

/// Whether `text` can be a definition path's segment, the first if
/// `first`, as the parser takes one: ASCII letters, digits, `_`, `-` and
/// `+`, starting with a letter or `_`, or after the first segment a
/// digit, each `-` followed by a letter, a digit, `_` or `+`; the first
/// no keyword.
fn is_segment(text: &str, first: bool) -> bool {
    let bytes = text.as_bytes();
    let Some(&start) = bytes.first() else {
        return false;
    };
    let starts = start.is_ascii_alphabetic() || start == b'_' || (!first && start.is_ascii_digit());
    let allowed = bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+'));
    let dashes = bytes
        .iter()
        .enumerate()
        .filter(|&(_, &b)| b == b'-')
        .all(|(at, _)| {
            bytes
                .get(at + 1)
                .is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+'))
        });
    // A plain name that is no identifier is a keyword.
    let plain = bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'_');
    let keyword = plain && start.is_ascii_alphabetic() && !lexer::is_identifier(text);
    starts && allowed && dashes && !keyword
}

impl Index {
    /// Whether `span` of file `file` is the value of a shorthand argument,
    /// `name` alone after a named argument, which reads the variable `name`
    /// and names the parameter `name` at once (see the parser's [Functions
    /// and calls](crate::parser::functions#functions-and-calls)).
    fn is_shorthand(&self, file: u32, span: Span) -> bool {
        let Some(ast) = self.scan.files.get(file as usize).map(|f| &f.ast) else {
            return false;
        };
        ast.expr_arena().iter().any(|expr| match expr.kind {
            ExprKind::Call { args, .. } | ExprKind::Unclosed { args, .. } => {
                ast.args(args).iter().any(|arg| {
                    matches!(
                        arg.kind,
                        ArgKind::Named {
                            shorthand: true,
                            ..
                        }
                    ) && ast.get(arg.value).span == span
                })
            }
            _ => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_is_as_the_parser_takes_one() {
        for (text, first, fits) in [
            ("go", true, true),
            ("gmp-c++", true, true),
            ("1x", false, true),
            ("1x", true, false),
            ("24", false, true),
            ("x-", false, false),
            ("x--y", false, false),
            ("a.b", false, false),
            ("if", true, false),
            ("", false, false),
        ] {
            assert_eq!(is_segment(text, first), fits, "{text} {first}");
        }
    }

    #[test]
    fn places_map_back_through_the_edits() {
        let edit = |start, end, text: &str| Edit {
            file: 0,
            span: Span::new(start, end),
            text: text.to_owned(),
        };
        // `ab cd ef` with `cd` renamed `wxyz`: `ab wxyz ef`.
        let shift = Shift::of(&[edit(3, 5, "wxyz")]);
        assert_eq!(shift.apply(0, "ab cd ef"), "ab wxyz ef");
        assert_eq!(shift.back(0, 1), 1);
        assert_eq!(shift.back_span(0, Span::new(3, 7)), Span::new(3, 5));
        assert_eq!(shift.back_span(0, Span::new(8, 10)), Span::new(6, 8));
        assert_eq!(shift.back_span(0, Span::new(0, 7)), Span::new(0, 5));
        // Another file is as it was.
        assert_eq!(shift.back(1, 9), 9);
    }
}
