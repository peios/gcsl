//! Queries of a file whose braces do not match: answered from the workspace
//! indexed again with the file's braces balanced virtually, its results
//! mapped back to the text as it is (see the documentation of
//! [`crate::index`], "Unbalanced files").

use std::collections::HashMap;

use crate::ast::Syntax;
use crate::lexer::TokenKind;
use crate::parser;
use crate::source::{SourceFile, Span};

use super::complete::Tokens;
use super::{
    Completion, Highlight, Hover, Index, Indexed, InlayHint, Location, Prepared, Signature,
};

/// A change made to a file's text virtually: the bytes at a place removed,
/// and text put in their place.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Patch {
    /// Where, in the text as it is.
    at: u32,
    /// How many bytes are removed there: a stray `}`'s one, or none.
    removed: u32,
    /// What is put there: `}`s, or nothing.
    inserted: String,
}

impl Patch {
    /// How many bytes are put in.
    fn len(&self) -> u32 {
        u32::try_from(self.inserted.len()).unwrap_or(u32::MAX)
    }
}

/// How a file whose braces do not match is balanced virtually, for queries
/// at a place of it ([`Index::closing`]): the `}`s it lacks put in, and
/// each `}` that closes nothing taken out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closing {
    /// The file's input number.
    file: u32,
    /// The changes, in the order of their places, none overlapping.
    patches: Vec<Patch>,
}

/// The workspace indexed again with a file's braces balanced virtually
/// ([`Closing::index`]), answering queries of the file as it is: places
/// are mapped to the text as balanced, and what is found mapped back, what
/// touches the text put in dropped.
#[derive(Debug)]
pub struct Closed {
    /// How the file was balanced.
    closing: Closing,
    /// The index of the workspace with it balanced.
    index: Index,
    /// The file's input number in that index.
    file: u32,
    /// The input number in the first index of each input of this one, by
    /// its input number here, found by path.
    inputs: Vec<Option<u32>>,
}

/// The latest [`Closed`] index made from one index, kept so that the
/// queries after the first at its documents' versions are answered without
/// indexing again (see the documentation of [`crate::index`], "Unbalanced
/// files").
#[derive(Debug, Default)]
pub struct ClosedCache {
    /// The index kept.
    kept: Option<Closed>,
    /// How many times the workspace was indexed again.
    made: usize,
}

impl Index {
    /// Whether the file of input number `file` has braces that do not
    /// match ([`Syntax::Unbalanced`]): it is not checked, and has nothing
    /// indexed.
    #[must_use]
    pub fn unbalanced(&self, file: u32) -> bool {
        self.scan
            .files
            .get(file as usize)
            .is_some_and(|entry| entry.ast.syntax() == Syntax::Unbalanced)
    }

    /// Why nothing can be renamed in the file of input number `file`, if its
    /// braces do not match: "the file has an unclosed `{`", or, if it has
    /// none, "the file has a `}` that closes nothing".
    pub(super) fn unbalanced_refusal(&self, file: u32) -> Option<String> {
        if !self.unbalanced(file) {
            return None;
        }
        let unclosed = self
            .sources
            .get(file as usize)
            .and_then(Option::as_ref)
            .is_none_or(|source| Braces::of(&Tokens::new(source), 0).open_at_end > 0);
        let why = if unclosed {
            "the file has an unclosed `{`"
        } else {
            "the file has a `}` that closes nothing"
        };
        Some(why.to_owned())
    }

    /// How to balance the file of input number `file`, whose braces do not
    /// match, for a query at its byte `offset` (see the module
    /// documentation, "Unbalanced files"): each `}` that closes nothing
    /// taken out, and the `}`s it lacks put in after the last token of the
    /// place's line, those of the blocks open there that nothing closes
    /// later, the others at the end of the file; or, if that does not
    /// parse with its braces matching and fewer errors than the file as it
    /// is, every one at the end of the file, if that parses better. `None`
    /// for a file whose braces match, and for one neither balances.
    #[must_use]
    pub fn closing(&self, file: u32, offset: u32) -> Option<Closing> {
        if !self.unbalanced(file) {
            return None;
        }
        let source = self.sources.get(file as usize)?.as_ref()?;
        let text = source.text();
        let end = u32::try_from(text.len()).ok()?;
        let tokens = Tokens::new(source);
        let point = close_point(&tokens, text, offset.min(end));
        if !text.is_char_boundary(point as usize) {
            return None;
        }
        let braces = Braces::of(&tokens, point);
        let near = braces.patches(point, end, true);
        let far = braces.patches(point, end, false);
        if far.is_empty() {
            return None;
        }
        // The errors of the text balanced so, if its braces match.
        let errors = |patches: &[Patch]| {
            let balanced = SourceFile::new(source.name(), apply(text, patches)).ok()?;
            let (ast, diagnostics) = parser::parse_source(&balanced);
            (ast.syntax() != Syntax::Unbalanced).then_some(diagnostics.len())
        };
        let first = errors(&near);
        let chosen = if near == far {
            first.map(|_| near)
        } else if first.is_some_and(|first| first < parser::parse_source(source).1.len()) {
            Some(near)
        } else {
            match (first, errors(&far)) {
                (Some(first), Some(second)) if second < first => Some(far),
                (Some(_), _) => Some(near),
                (None, Some(_)) => Some(far),
                (None, None) => None,
            }
        };
        Some(Closing {
            file,
            patches: chosen?,
        })
    }
}

/// The place the `}`s a file lacks are put at for a query at `offset` of
/// it, whose text is `text` and tokens `tokens`, so that the place stays in
/// the blocks open there: the end of the last token of its line, if that
/// is not before it (a trailing comment is passed over, and a name being
/// typed is not split); else the place itself, if it is in no comment or
/// string; else the end of the last token before it.
fn close_point(tokens: &Tokens<'_>, text: &str, offset: u32) -> u32 {
    let line_end = text
        .get(offset as usize..)
        .and_then(|rest| rest.find('\n'))
        .and_then(|at| u32::try_from(at).ok())
        .map_or_else(
            || u32::try_from(text.len()).unwrap_or(u32::MAX),
            |at| offset + at,
        );
    let mut last = tokens.starting_before(line_end);
    while let Some(before) = last.checked_sub(1)
        && tokens.kind(before) == TokenKind::Newline
    {
        last = before;
    }
    let last = last.checked_sub(1).map(|at| tokens.span(at).end);
    match last {
        Some(end) if end >= offset => end,
        _ if !tokens.in_text(offset) => offset,
        _ => {
            let before = tokens.starting_before(offset).checked_sub(1);
            before.map_or(0, |at| tokens.span(at).end)
        }
    }
}

/// A file's braces, as its tokens have them, counted for a place of it.
struct Braces {
    /// Each `}` that closes nothing.
    strays: Vec<Span>,
    /// The blocks open at the place that nothing closes later.
    unclosed_at: usize,
    /// The blocks open at the end of the file.
    open_at_end: usize,
}

impl Braces {
    /// The braces of the file whose tokens are `tokens`, counted for the
    /// place `point`, a token's end or a place between tokens.
    fn of(tokens: &Tokens<'_>, point: u32) -> Self {
        let mut strays = Vec::new();
        let mut depth = 0usize;
        // The fewest blocks open at the place or after it.
        let mut lowest = None;
        for at in 0..tokens.len() {
            let span = tokens.span(at);
            if lowest.is_none() && span.start >= point {
                lowest = Some(depth);
            }
            match tokens.kind(at) {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => match depth.checked_sub(1) {
                    Some(fewer) => depth = fewer,
                    None => strays.push(span),
                },
                _ => {}
            }
            if let Some(lowest) = lowest.as_mut() {
                *lowest = (*lowest).min(depth);
            }
        }
        Self {
            strays,
            unclosed_at: lowest.unwrap_or(depth),
            open_at_end: depth,
        }
    }

    /// The changes that balance the file, which ends at `end`: each stray
    /// `}` taken out; if `near`, a `}` at `point` for each block open there
    /// that nothing closes later; and the rest put at the end, on a line of
    /// their own, after any comment.
    fn patches(&self, point: u32, end: u32, near: bool) -> Vec<Patch> {
        let mut patches: Vec<Patch> = self
            .strays
            .iter()
            .map(|span| Patch {
                at: span.start,
                removed: span.end - span.start,
                inserted: String::new(),
            })
            .collect();
        let here = if near { self.unclosed_at } else { 0 };
        let rest = self.open_at_end.saturating_sub(here);
        if here > 0 {
            patches.push(Patch {
                at: point,
                removed: 0,
                inserted: "}".repeat(here),
            });
        }
        if rest > 0 {
            patches.push(Patch {
                at: end,
                removed: 0,
                inserted: format!("\n{}", "}".repeat(rest)),
            });
        }
        patches.sort_by_key(|patch| patch.at);
        patches
    }
}

/// `text` with `patches` made.
fn apply(text: &str, patches: &[Patch]) -> String {
    let mut made = String::with_capacity(text.len() + 16);
    let mut from = 0usize;
    for patch in patches {
        let at = patch.at as usize;
        made.push_str(text.get(from..at).unwrap_or_default());
        made.push_str(&patch.inserted);
        from = at + patch.removed as usize;
    }
    made.push_str(text.get(from..).unwrap_or_default());
    made
}

impl Closing {
    /// The input number of the file it balances.
    #[must_use]
    pub fn file(&self) -> u32 {
        self.file
    }

    /// The workspace of `original` indexed again by `reindex` (given the
    /// files after the standard ones, as [`Index::rename`] gives them),
    /// with the file balanced so: `None` if a file's source is not known,
    /// or `reindex` makes no index.
    pub fn index(
        self,
        original: &Index,
        reindex: impl FnOnce(&[SourceFile]) -> Indexed,
    ) -> Option<Closed> {
        let standard = original.standard() as usize;
        let mut sources = Vec::new();
        for (at, source) in original.sources.iter().enumerate().skip(standard) {
            let source = source.as_ref()?;
            let source = if at == self.file as usize {
                SourceFile::new(source.name(), apply(source.text(), &self.patches)).ok()?
            } else {
                source.clone()
            };
            sources.push(source);
        }
        let index = reindex(&sources).index?;
        let file = index.file(original.path(self.file)?)?;
        let by_path: HashMap<&str, u32> = (0u32..)
            .zip(&original.scan.files)
            .map(|(at, entry)| (entry.path.as_str(), at))
            .collect();
        let inputs = index
            .scan
            .files
            .iter()
            .map(|entry| by_path.get(entry.path.as_str()).copied())
            .collect();
        Some(Closed {
            closing: self,
            index,
            file,
            inputs,
        })
    }

    /// The place in the text as balanced of byte `offset` of the text as
    /// it is: a place where `}`s are put in is before them, and one in a
    /// `}` taken out where it was.
    fn forward(&self, offset: u32) -> u32 {
        let (mut added, mut removed) = (0u32, 0u32);
        for patch in &self.patches {
            if offset <= patch.at {
                break;
            }
            if offset < patch.at + patch.removed {
                return (patch.at + added).saturating_sub(removed);
            }
            added += patch.len();
            removed += patch.removed;
        }
        (offset + added).saturating_sub(removed)
    }

    /// The place in the text as it is of byte `offset` of the text as
    /// balanced, the end of a span if `end`: `None` in the text put in.
    fn back(&self, offset: u32, end: bool) -> Option<u32> {
        let (mut added, mut removed) = (0u32, 0u32);
        for patch in &self.patches {
            let start = (patch.at + added).saturating_sub(removed);
            let after = start + patch.len();
            if offset < start {
                break;
            }
            if offset == start && (end || patch.len() > 0) {
                return Some(patch.at);
            }
            if offset < after {
                return None;
            }
            if offset == after {
                return Some(patch.at + patch.removed);
            }
            added += patch.len();
            removed += patch.removed;
        }
        Some((offset + removed).saturating_sub(added))
    }

    /// The span of the text as it is of `span`, of the text as balanced:
    /// `None` if it touches the text put in (shares a byte with it, or,
    /// empty, is strictly inside it).
    fn span_back(&self, span: Span) -> Option<Span> {
        let (mut added, mut removed) = (0u32, 0u32);
        for patch in &self.patches {
            let start = (patch.at + added).saturating_sub(removed);
            let after = start + patch.len();
            // An empty span touches it only strictly inside it, and a `}`
            // taken out is no text put in.
            let touches = if patch.len() == 0 {
                false
            } else if span.start < span.end {
                span.start < after && start < span.end
            } else {
                start < span.start && span.start < after
            };
            if touches {
                return None;
            }
            added += patch.len();
            removed += patch.removed;
        }
        Some(Span::new(
            self.back(span.start, false)?,
            self.back(span.end, true)?,
        ))
    }
}

impl Closed {
    /// How the file was balanced.
    #[must_use]
    pub fn closing(&self) -> &Closing {
        &self.closing
    }

    /// Whether it answers a query at byte `offset` of the file of input
    /// number `file` as one made for the place would: it balances that
    /// file, and puts no `}` in before the place, which is in the blocks
    /// open there.
    #[must_use]
    pub fn serves(&self, file: u32, offset: u32) -> bool {
        self.closing.file == file
            && self
                .closing
                .patches
                .iter()
                .all(|patch| patch.inserted.is_empty() || patch.at >= offset)
    }

    /// [`Index::completion`] at byte `offset` of the file.
    #[must_use]
    pub fn completion(&self, offset: u32) -> Vec<Completion> {
        self.index
            .completion(self.file, self.closing.forward(offset))
    }

    /// [`Index::signature`] at byte `offset` of the file.
    #[must_use]
    pub fn signature(&self, offset: u32) -> Option<Signature> {
        self.index
            .signature(self.file, self.closing.forward(offset))
    }

    /// [`Index::hover`] at byte `offset` of the file.
    pub fn hover(&mut self, offset: u32) -> Option<Hover> {
        let mut found = self.index.hover(self.file, self.closing.forward(offset))?;
        found.span = self.closing.span_back(found.span)?;
        Some(found)
    }

    /// [`Index::definition`] at byte `offset` of the file.
    #[must_use]
    pub fn definition(&self, offset: u32) -> Vec<Location> {
        let found = self
            .index
            .definition(self.file, self.closing.forward(offset));
        self.locations(found)
    }

    /// [`Index::references`] at byte `offset` of the file, its declarations
    /// among them if `declarations`.
    #[must_use]
    pub fn references(&self, offset: u32, declarations: bool) -> Vec<Location> {
        let at = self.closing.forward(offset);
        self.locations(self.index.references(self.file, at, declarations))
    }

    /// [`Index::highlights`] at byte `offset` of the file.
    #[must_use]
    pub fn highlights(&self, offset: u32) -> Vec<Highlight> {
        self.index
            .highlights(self.file, self.closing.forward(offset))
            .into_iter()
            .filter_map(|found| {
                Some(Highlight {
                    span: self.closing.span_back(found.span)?,
                    write: found.write,
                })
            })
            .collect()
    }

    /// [`Index::prepare_rename`] at byte `offset` of the file.
    ///
    /// # Errors
    ///
    /// What [`Index::prepare_rename`] refuses, and a name in the text put
    /// in.
    pub fn prepare_rename(&self, offset: u32) -> Result<Prepared, String> {
        let prepared = self
            .index
            .prepare_rename(self.file, self.closing.forward(offset))?;
        let span = self
            .closing
            .span_back(prepared.span)
            .ok_or_else(|| "there is no name here to rename".to_owned())?;
        Ok(Prepared { span, ..prepared })
    }

    /// [`Index::inlay_hints`] of `range` of the file, its ends included.
    pub fn inlay_hints(&mut self, range: Span) -> Vec<InlayHint> {
        let start = self.closing.forward(range.start);
        let end = self.closing.forward(range.end);
        let found = self.index.inlay_hints(self.file, Span::new(start, end));
        found
            .into_iter()
            .filter_map(|hint| {
                let offset = self.closing.back(hint.offset, false)?;
                Some(InlayHint { offset, ..hint })
            })
            .collect()
    }

    /// `found`, places of this index, as places of the first: in the file,
    /// mapped back, and dropped if they touch the text put in; in another
    /// file, renumbered.
    fn locations(&self, found: Vec<Location>) -> Vec<Location> {
        found
            .into_iter()
            .filter_map(|location| {
                let file = self.inputs.get(location.file as usize).copied().flatten()?;
                let span = if location.file == self.file {
                    self.closing.span_back(location.span)?
                } else {
                    location.span
                };
                Some(Location { file, span })
            })
            .collect()
    }
}

impl ClosedCache {
    /// The [`Closed`] index of `index` for a query at byte `offset` of the
    /// file of input number `file`, whose braces do not match: the one kept,
    /// if it serves the place ([`Closed::serves`]) or balances the file as
    /// the place needs ([`Index::closing`]); else one made by `reindex`
    /// (see [`Closing::index`]), kept in its place. `None` if the file is
    /// not balanced so, or no index is made.
    pub fn at(
        &mut self,
        index: &Index,
        (file, offset): (u32, u32),
        reindex: impl FnOnce(&[SourceFile]) -> Indexed,
    ) -> Option<&mut Closed> {
        if !index.unbalanced(file) {
            return None;
        }
        let serves = self
            .kept
            .as_ref()
            .is_some_and(|kept| kept.serves(file, offset));
        if !serves {
            let closing = index.closing(file, offset)?;
            if self
                .kept
                .as_ref()
                .is_none_or(|kept| kept.closing != closing)
            {
                self.made += 1;
                self.kept = closing.index(index, reindex);
            }
        }
        self.kept.as_mut()
    }

    /// The [`Closed`] index of `index` for the inlay hints of `range` of the
    /// file of input number `file`, whose braces do not match: the one kept,
    /// if it balances the file, wherever it puts its `}`s; else as
    /// [`ClosedCache::at`] makes one for the end of the range, which keeps
    /// the range in the blocks open in it.
    pub fn over(
        &mut self,
        index: &Index,
        (file, range): (u32, Span),
        reindex: impl FnOnce(&[SourceFile]) -> Indexed,
    ) -> Option<&mut Closed> {
        if !index.unbalanced(file) {
            return None;
        }
        if self
            .kept
            .as_ref()
            .is_some_and(|kept| kept.closing.file == file)
        {
            return self.kept.as_mut();
        }
        self.at(index, (file, range.end), reindex)
    }

    /// How many times the workspace was indexed again for it.
    #[must_use]
    pub fn made(&self) -> usize {
        self.made
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::index_with_standard;

    /// Indexes `files` (paths and texts) again, as the tests index them.
    fn reindex(files: &[SourceFile]) -> Indexed {
        index_with_standard(files, &[], "/")
    }

    /// The index of a workspace of `app.gcsl`, `text` with its `|` taken
    /// out, and `others`, with the file's input number and the place of the
    /// `|`.
    fn indexed(text: &str, others: &[(&str, &str)]) -> (Index, u32, u32) {
        let at = u32::try_from(text.find('|').unwrap()).unwrap();
        let mut sources = vec![
            SourceFile::new("app.gcsl", text.replacen('|', "", 1)).unwrap(),
            SourceFile::new("workspace.gcsl", "").unwrap(),
        ];
        for &(path, other) in others {
            sources.push(SourceFile::new(path, other).unwrap());
        }
        sources.sort_by(|a, b| a.name().cmp(b.name()));
        let index = reindex(&sources).index.unwrap();
        let file = index.file("app.gcsl").unwrap();
        (index, file, at)
    }

    /// The workspace of `index` indexed again with the file `file` balanced
    /// for a query at `at`.
    fn balanced(index: &Index, file: u32, at: u32) -> Closed {
        let closing = index.closing(file, at).unwrap();
        closing.index(index, reindex).unwrap()
    }

    /// The labels of the completions at the `|` of `text`, the file's
    /// braces not matching.
    fn completed(text: &str) -> Vec<String> {
        let (index, file, at) = indexed(text, &[]);
        assert!(index.unbalanced(file), "{text}");
        assert_eq!(index.completion(file, at), Vec::new(), "{text}");
        let found = balanced(&index, file, at).completion(at);
        found.into_iter().map(|found| found.label).collect()
    }

    /// Asserts that `found` has each of `wanted`.
    fn offers(found: &[String], wanted: &[&str], text: &str) {
        for label in wanted {
            assert!(
                found.iter().any(|l| l == label),
                "{label} in {text}: {found:?}"
            );
        }
    }

    #[test]
    fn a_name_is_completed_in_a_block_left_open() {
        // The case clackatoo found: a record block left open.
        let text = "r = { k = 1; j = k|";
        offers(&completed(text), &["k"], text);
        let text = "r = { k = 1; j = k|\nlater = 2\n";
        offers(&completed(text), &["k"], text);
        // A `def`'s body.
        let text = "w = 0\ndef f(a: Int): Int {\n    let b = a\n    b + |\n";
        offers(&completed(text), &["a", "b", "f"], text);
        // An `if`'s block.
        let text = "def g(a: Int): Int {\n    if a > 0 {\n        let z = a\n        |\n";
        offers(&completed(text), &["a", "z"], text);
        // Blocks nested, each left open, with an item after the place.
        let text = "r = {\n    s = {\n        t = 1\n        u = t|\n        v = 2\n";
        offers(&completed(text), &["t"], text);
        // One closed later: the `}`s the file lacks are fewer than the
        // blocks open at the place.
        let text = "r = {\n    s = {\n        t = 1\n        u = |\n    }\n";
        offers(&completed(text), &["t"], text);
        // A name half-typed at the end of the file.
        let text = "r = { kappa = 1; j = ka|";
        offers(&completed(text), &["kappa"], text);
        // After a `.`.
        let text = "c = {port = 80}\nr = {\n    p = c.|";
        offers(&completed(text), &["port"], text);
    }

    #[test]
    fn hover_and_definition_in_a_block_left_open() {
        let text = "r = {\n    k = 1\n    j = |k\n";
        let (index, file, at) = indexed(text, &[]);
        let mut closed = balanced(&index, file, at);
        let hover = closed.hover(at).unwrap();
        assert!(hover.text.contains("k: Int"), "{}", hover.text);
        assert_eq!(hover.span, Span::new(at, at + 1));
        let declared = u32::try_from(text.find("k = 1").unwrap()).unwrap();
        let found = closed.definition(at);
        assert_eq!(
            found,
            [Location {
                file,
                span: Span::new(declared, declared + 1)
            }]
        );
        // In another file: its place as it is.
        let text = "import lib\nr = {\n    j = lib.|base\n";
        let (index, file, at) = indexed(text, &[("lib.gcsl", "w = 0\nbase = 1\n")]);
        let found = balanced(&index, file, at).definition(at);
        let lib = index.file("lib.gcsl").unwrap();
        assert_eq!(
            found,
            [Location {
                file: lib,
                span: Span::new(6, 10)
            }]
        );
    }

    #[test]
    fn what_is_found_after_the_place_is_mapped_back() {
        // `a` is read before the `}` put in, at the end of the place's
        // line, and after it, past an `é`, where the text as balanced is a
        // byte longer.
        let text = "a = 1\nr = {\n    k = |a\ns = \"é\"\nb = a\n";
        let (index, file, at) = indexed(text, &[]);
        let closed = balanced(&index, file, at);
        let plain = text.replacen('|', "", 1);
        let places: Vec<u32> = plain
            .match_indices('a')
            .filter(|&(byte, _)| {
                let next = plain.as_bytes().get(byte + 1);
                next.is_none_or(|c| !c.is_ascii_alphanumeric())
            })
            .map(|(byte, _)| u32::try_from(byte).unwrap())
            .collect();
        assert_eq!(places.len(), 3);
        let found: Vec<Span> = closed
            .references(at, true)
            .into_iter()
            .map(|location| location.span)
            .collect();
        let wanted: Vec<Span> = places.iter().map(|&at| Span::new(at, at + 1)).collect();
        assert_eq!(found, wanted);
        // The highlights too, and a hover after the place.
        let found: Vec<Span> = closed.highlights(at).into_iter().map(|h| h.span).collect();
        assert_eq!(found, wanted);
        let mut closed = closed;
        let last = places[2];
        assert_eq!(closed.hover(last).unwrap().span, Span::new(last, last + 1));
    }

    #[test]
    fn spans_in_the_text_put_in_are_dropped() {
        // Two `}`s put in at 5, a `}` taken out at 8, in "0123456789".
        let closing = Closing {
            file: 0,
            patches: vec![
                Patch {
                    at: 5,
                    removed: 0,
                    inserted: "}}".to_owned(),
                },
                Patch {
                    at: 8,
                    removed: 1,
                    inserted: String::new(),
                },
            ],
        };
        // As balanced: "01234}}567" then "9".
        for (offset, balanced) in [(0, 0), (5, 5), (6, 8), (7, 9), (8, 10), (9, 10)] {
            assert_eq!(closing.forward(offset), balanced, "{offset}");
        }
        for (span, back) in [
            ((0, 5), Some((0, 5))),
            ((5, 5), Some((5, 5))),
            ((4, 6), None),
            ((5, 7), None),
            ((6, 6), None),
            ((7, 7), Some((5, 5))),
            ((7, 9), Some((5, 7))),
            ((9, 11), Some((7, 10))),
            ((10, 11), Some((9, 10))),
        ] {
            let found = closing.span_back(Span::new(span.0, span.1));
            assert_eq!(found, back.map(|(s, e)| Span::new(s, e)), "{span:?}");
        }
    }

    #[test]
    fn a_stray_closing_brace_is_taken_out() {
        let text = "a = 1\n}\nb = |a + 1\n";
        let (index, file, at) = indexed(text, &[]);
        assert!(index.unbalanced(file));
        let mut closed = balanced(&index, file, at);
        let hover = closed.hover(at).unwrap();
        assert!(hover.text.contains("a: Int"), "{}", hover.text);
        assert_eq!(hover.span, Span::new(at, at + 1));
        assert_eq!(closed.definition(at)[0].span, Span::new(0, 1));
        // A stray `}` before, and a block left open after it.
        let text = "a = 1\n}\nr = {\n    k = a\n    j = |k\n";
        let (index, file, at) = indexed(text, &[]);
        let mut closed = balanced(&index, file, at);
        assert_eq!(closed.hover(at).unwrap().span, Span::new(at, at + 1));
        let k = u32::try_from(text.find("k = ").unwrap()).unwrap();
        assert_eq!(closed.definition(at)[0].span, Span::new(k, k + 1));
        let a = k + 4;
        assert_eq!(closed.definition(a)[0].span, Span::new(0, 1));
        let found: Vec<Span> = closed
            .references(at, true)
            .into_iter()
            .map(|location| location.span)
            .collect();
        assert_eq!(found, [Span::new(k, k + 1), Span::new(at, at + 1)]);
    }

    #[test]
    fn the_end_of_the_file_is_tried_when_the_line_does_not_balance() {
        // A `}` at the end of the place's line would end the call's
        // arguments early: the `)` on the next line closes them.
        let text =
            "def f(a: Int, b: Int): Int { a }\nr = {\n    k = 1\n    j = f(k|,\n        2)\n";
        let (index, file, at) = indexed(text, &[]);
        let closing = index.closing(file, at).unwrap();
        let end = u32::try_from(text.len() - 1).unwrap();
        assert_eq!(closing.patches.len(), 1);
        assert_eq!(closing.patches[0].at, end);
        let closed = closing.index(&index, reindex).unwrap();
        offers(&labels(closed.completion(at)), &["k"], text);
        assert_eq!(closed.signature(at).unwrap().active, Some(0));
    }

    /// The labels of `found`.
    fn labels(found: Vec<Completion>) -> Vec<String> {
        found.into_iter().map(|found| found.label).collect()
    }

    #[test]
    fn a_rename_is_refused_and_prepared() {
        let text = "r = {\n    k = 1\n    j = |k\n";
        let (index, file, at) = indexed(text, &[]);
        let refused = index.rename((file, at), "z", reindex);
        assert_eq!(refused, Err("the file has an unclosed `{`".to_owned()));
        let prepared = balanced(&index, file, at).prepare_rename(at).unwrap();
        assert_eq!(prepared.span, Span::new(at, at + 1));
        assert_eq!(prepared.placeholder, "k");
        let (index, file, at) = indexed("a = 1\n}\nb = |a\n", &[]);
        let refused = index.rename((file, at), "z", reindex);
        assert_eq!(
            refused,
            Err("the file has a `}` that closes nothing".to_owned())
        );
    }

    #[test]
    fn the_index_made_again_is_kept_for_the_places_it_serves() {
        let text = "r = {\n    k = 1\n    j = k|\n    m = 2\n";
        let (index, file, at) = indexed(text, &[]);
        let mut cache = ClosedCache::default();
        let first = cache
            .at(&index, (file, at), reindex)
            .unwrap()
            .completion(at);
        // Again at the place, and before it: kept.
        let again = cache
            .at(&index, (file, at), reindex)
            .unwrap()
            .completion(at);
        assert_eq!(labels(first), labels(again));
        let before = u32::try_from(text.find("k = 1").unwrap()).unwrap();
        assert!(cache.at(&index, (file, before), reindex).is_some());
        let whole = Span::new(0, u32::try_from(text.len()).unwrap());
        assert!(cache.over(&index, (file, whole), reindex).is_some());
        assert_eq!(cache.made(), 1);
        // After it, on a later line: made again.
        let after = u32::try_from(text.find("m = 2").unwrap()).unwrap();
        assert!(cache.at(&index, (file, after), reindex).is_some());
        assert_eq!(cache.made(), 2);
        // A file whose braces match has none.
        let (index, file, at) = indexed("r = {k = |1}\n", &[]);
        assert!(index.closing(file, at).is_none());
        assert!(cache.at(&index, (file, at), reindex).is_none());
    }

    #[test]
    fn inlay_hints_of_a_block_left_open() {
        // Asked for after a completion at the place, from its index: the
        // hint after the place, past an `é`, mapped back.
        let text = "r = {\n    let a = 1 + 1\n    x = a|\n# é\nlet b = 2 * 3\n";
        let (index, file, at) = indexed(text, &[]);
        let mut cache = ClosedCache::default();
        assert!(cache.at(&index, (file, at), reindex).is_some());
        let plain = text.replacen('|', "", 1);
        let whole = Span::new(0, u32::try_from(plain.len()).unwrap());
        let hints = cache
            .over(&index, (file, whole), reindex)
            .unwrap()
            .inlay_hints(whole);
        assert_eq!(cache.made(), 1);
        let after = |name: &str| u32::try_from(plain.find(name).unwrap() + name.len()).unwrap();
        let found: Vec<(u32, String)> = hints.into_iter().map(|h| (h.offset, h.label)).collect();
        let int = ": Int".to_owned();
        assert_eq!(
            found,
            [(after("let a"), int.clone()), (after("let b"), int)]
        );
        // Asked for first: the `}`s put at the end of the range.
        let mut cache = ClosedCache::default();
        let hints = cache
            .over(&index, (file, whole), reindex)
            .unwrap()
            .inlay_hints(whole);
        assert_eq!(hints.len(), 2);
    }
}
