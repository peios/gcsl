//! Formatting: `textDocument/formatting` and `textDocument/rangeFormatting`
//! (see [`gcsl::fmt`]).
//!
//! A document is formatted whole, from the text the server keeps of it, and
//! answered with the edits that make its text the formatted one: the lines
//! that differ, as a line diff finds them ([`hunks`]), each one a
//! [`TextEdit`] replacing whole lines, so that an editor keeps its marks
//! and its cursor on the lines formatting leaves alone: a line of its own
//! where formatting replaces a run of lines by as many, else the run, which
//! adds or removes lines. A range is answered with those of the edits that
//! touch its lines alone, so lines either side of it are left as they are
//! unless a run that adds or removes lines spans them: each edit
//! leaves a document that parses, so any of them may be made without the
//! others. A document with a lexical or syntax error is not formatted:
//! the request fails, saying so and where its first error is, and why is
//! logged.
//!
//! The diff is Myers's, on the lines left once those the two texts start
//! and end with are set aside, and gives up past [`MAX_DIFFERENCES`]
//! differences, a document every line of which formatting changes, for one
//! edit of everything between those lines.

use std::ops::{Range, RangeInclusive};

use lsp_types::TextEdit;

use gcsl::SourceFile;
use crate::position::{Encoding, LineIndex};

/// The most lines added and removed that the diff looks for before it
/// gives up, making the whole middle of the document one edit. It bounds
/// the diff's memory to a few hundred thousand numbers.
pub const MAX_DIFFERENCES: usize = 256;

/// The edits that format `text`, a document's, named `name` for what is
/// logged, positions counted in `encoding`; with `lines`, only those that
/// touch those lines, counted from 0.
///
/// # Errors
///
/// Why it is not formatted, logged too: it has a lexical or syntax error,
/// `not formatted: it has a syntax error (LINE:COL)`, at the first one's
/// place, its line and its column in characters counted from 1; or it is
/// too long to be a source.
pub fn edits(
    name: &str,
    text: &str,
    encoding: Encoding,
    lines: Option<&RangeInclusive<usize>>,
) -> Result<Vec<TextEdit>, String> {
    let source = SourceFile::new(name, text).map_err(|error| format!("not formatted: {error}"))?;
    let formatted = match gcsl::fmt::format(&source) {
        Ok(formatted) => formatted,
        Err(diagnostics) => {
            let first = diagnostics
                .iter()
                .min_by_key(|diagnostic| diagnostic.span.start);
            let at = first.map_or((1, 1), |first| source.line_col(first.span.start));
            let detail = first.map_or_else(String::new, |first| {
                format!(", the first at {}:{}: {}", at.0, at.1, first.message)
            });
            let count = match diagnostics.len() {
                1 => "1 error".to_owned(),
                count => format!("{count} errors"),
            };
            crate::log(format_args!("not formatting {name}: {count}{detail}"));
            return Err(format!(
                "not formatted: it has a syntax error ({}:{})",
                at.0, at.1
            ));
        }
    };
    let old: Vec<&str> = text.split_inclusive('\n').collect();
    let new: Vec<&str> = formatted.split_inclusive('\n').collect();
    let starts: Vec<usize> = old
        .iter()
        .scan(0, |at, line| {
            let start = *at;
            *at += line.len();
            Some(start)
        })
        .chain([text.len()])
        .collect();
    let index = LineIndex::new(text);
    let edits = hunks(&old, &new)
        .into_iter()
        .flat_map(|hunk| per_line(hunk, &old, &new))
        .filter(|(from, _)| {
            // An edit touches the lines if it replaces one of them, or,
            // replacing none, inserts before one or just after the last.
            lines.is_none_or(|lines| {
                let last = if from.is_empty() {
                    from.start
                } else {
                    from.end - 1
                };
                from.start <= *lines.end() && *lines.start() <= last
            })
        })
        .map(|(from, to)| {
            let start = starts.get(from.start).copied().unwrap_or(text.len());
            let end = starts.get(from.end).copied().unwrap_or(text.len());
            TextEdit {
                range: index.range(span(start, end), encoding),
                new_text: new.get(to).unwrap_or_default().concat(),
            }
        })
        .collect();
    Ok(edits)
}

/// `hunk`, a run of lines of `old` replaced by a run of lines of `new`, as
/// edits a range can be answered with: one a line if it replaces each line
/// by one (formatting changed their indents or spacing), leaving out the
/// lines it leaves as they are, else whole (it adds or removes lines).
fn per_line(
    (from, to): (Range<usize>, Range<usize>),
    old: &[&str],
    new: &[&str],
) -> Vec<(Range<usize>, Range<usize>)> {
    if from.len() != to.len() {
        return vec![(from, to)];
    }
    from.zip(to)
        .filter(|&(before, after)| old.get(before) != new.get(after))
        .map(|(before, after)| (before..before + 1, after..after + 1))
        .collect()
}

/// The span from byte `start` to byte `end`, which a document's text, its
/// length checked by [`SourceFile::new`], always fits.
fn span(start: usize, end: usize) -> gcsl::Span {
    let at = |offset: usize| u32::try_from(offset).unwrap_or(u32::MAX);
    gcsl::Span::new(at(start), at(end))
}

/// Where `old` and `new` differ: each run of lines of `old` replaced by a
/// run of lines of `new`, by their ranges, in order, as few lines as a
/// longest common subsequence of lines leaves, or the lines between the
/// common start and end when there are more than [`MAX_DIFFERENCES`].
#[must_use]
pub fn hunks(old: &[&str], new: &[&str]) -> Vec<(Range<usize>, Range<usize>)> {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(before, after)| before == after)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(before, after)| before == after)
        .count();
    let old_middle = &old[prefix..old.len() - suffix];
    let new_middle = &new[prefix..new.len() - suffix];
    if old_middle.is_empty() && new_middle.is_empty() {
        return Vec::new();
    }
    let Some(common) = common_lines(old_middle, new_middle) else {
        return vec![(
            prefix..prefix + old_middle.len(),
            prefix..prefix + new_middle.len(),
        )];
    };
    // The runs between the common lines.
    let mut hunks = Vec::new();
    let (mut old_at, mut new_at) = (0, 0);
    let end = (old_middle.len(), new_middle.len());
    for (old_common, new_common) in common.into_iter().chain([end]) {
        if old_common > old_at || new_common > new_at {
            hunks.push((
                prefix + old_at..prefix + old_common,
                prefix + new_at..prefix + new_common,
            ));
        }
        (old_at, new_at) = (old_common + 1, new_common + 1);
    }
    hunks
}

/// The lines `old` and `new` have in common, as pairs of their indices in
/// order, a longest common subsequence of them, by Myers's algorithm; `None`
/// if they differ by more than [`MAX_DIFFERENCES`] lines.
fn common_lines(old: &[&str], new: &[&str]) -> Option<Vec<(usize, usize)>> {
    // `furthest[slot(diagonal)]`: the furthest `x`, a line of `old`,
    // reached on a diagonal, `x - y` for `y` a line of `new`, which lies
    // between `-MAX_DIFFERENCES` and `MAX_DIFFERENCES`; one copy of it
    // before each round in `trace`, to walk the path back.
    const OFFSET: usize = MAX_DIFFERENCES + 1;
    let slot = |diagonal: isize| OFFSET.checked_add_signed(diagonal).unwrap_or(0);
    let mut furthest = vec![0_usize; 2 * OFFSET + 1];
    let mut trace: Vec<Vec<usize>> = Vec::new();
    // Whether the path to `diagonal` comes down from the one above it,
    // adding a line of `new`, rather than across, removing one of `old`.
    let down = |furthest: &[usize], diagonal: isize, round: isize| {
        diagonal == -round
            || (diagonal != round && furthest[slot(diagonal - 1)] < furthest[slot(diagonal + 1)])
    };
    let mut reached = None;
    'search: for round in 0..=MAX_DIFFERENCES.min(old.len() + new.len()) {
        trace.push(furthest.clone());
        let round = isize::try_from(round).ok()?;
        for diagonal in (-round..=round).step_by(2) {
            let mut x = if down(&furthest, diagonal, round) {
                furthest[slot(diagonal + 1)]
            } else {
                furthest[slot(diagonal - 1)] + 1
            };
            let mut y = x.checked_add_signed(-diagonal)?;
            while x < old.len() && y < new.len() && old[x] == new[y] {
                x += 1;
                y += 1;
            }
            furthest[slot(diagonal)] = x;
            if x >= old.len() && y >= new.len() {
                reached = Some(round);
                break 'search;
            }
        }
    }
    let rounds = reached?;
    // Back from the end, along the path found: each round's move, then
    // the common lines it slid along.
    let mut common = Vec::new();
    let (mut x, mut y) = (old.len(), new.len());
    for (round, furthest) in (0..=rounds).rev().zip(trace.iter().rev()) {
        let diagonal = isize::try_from(x).ok()? - isize::try_from(y).ok()?;
        let (previous_x, previous_y) = if round == 0 {
            (0, 0)
        } else {
            let previous = if down(furthest, diagonal, round) {
                diagonal + 1
            } else {
                diagonal - 1
            };
            let previous_x = furthest[slot(previous)];
            (previous_x, previous_x.checked_add_signed(-previous)?)
        };
        while x > previous_x && y > previous_y {
            x -= 1;
            y -= 1;
            common.push((x, y));
        }
        (x, y) = (previous_x, previous_y);
    }
    common.reverse();
    Some(common)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hunks of two texts, split into lines.
    fn diff(old: &str, new: &str) -> Vec<(Range<usize>, Range<usize>)> {
        let old: Vec<&str> = old.split_inclusive('\n').collect();
        let new: Vec<&str> = new.split_inclusive('\n').collect();
        hunks(&old, &new)
    }

    #[test]
    fn lines_that_differ_are_found() {
        assert_eq!(diff("a\nb\nc\n", "a\nb\nc\n"), []);
        assert_eq!(diff("a\nb\nc\n", "a\nB\nc\n"), [(1..2, 1..2)]);
        // A line removed, one added, and two apart.
        assert_eq!(diff("a\n\nb\n", "a\nb\n"), [(1..2, 1..1)]);
        assert_eq!(diff("a\nb\n", "a\nx\nb\n"), [(1..1, 1..2)]);
        assert_eq!(
            diff("x\na\nb\nc\nd\ny\n", "X\na\nb\nc\nd\nY\n"),
            [(0..1, 0..1), (5..6, 5..6)]
        );
        // Everything.
        assert_eq!(diff("a\n", "b\nc\n"), [(0..1, 0..2)]);
        assert_eq!(diff("", "a\n"), [(0..0, 0..1)]);
    }

    #[test]
    fn past_the_limit_the_middle_is_one_hunk() {
        let old: String = (0..400).map(|at| at.to_string() + "\n").collect();
        let new: String = (0..400)
            .map(|at| "x".to_owned() + &at.to_string() + "\n")
            .collect();
        assert_eq!(diff(&old, &new), [(0..400, 0..400)]);
    }

    /// `text` with `made` applied, as a client applies them: each range
    /// of the text as it was.
    fn apply(text: &str, made: &[TextEdit]) -> String {
        let index = LineIndex::new(text);
        let mut out = text.to_owned();
        for edit in made.iter().rev() {
            let start = index.offset(edit.range.start, Encoding::Utf16);
            let end = index.offset(edit.range.end, Encoding::Utf16);
            out.replace_range(start..end, &edit.new_text);
        }
        out
    }

    #[test]
    fn edits_replace_the_lines_that_change() {
        let text = "x = {\n  a=1\n\n\n  b = 2\n}\n";
        let formatted = "x = {\n    a = 1\n\n    b = 2\n}\n";
        let made = edits("t.gcsl", text, Encoding::Utf16, None).unwrap();
        assert_eq!(apply(text, &made), formatted);
        // Two runs of lines, the first and the last lines left alone.
        assert_eq!(made.len(), 2, "{made:?}");
        assert_eq!(made[0].range.start.line, 1);
        assert_eq!(made[1].range.end.line, 5);
        // Those on the lines of a range alone: the second.
        let ranged = edits("t.gcsl", text, Encoding::Utf16, Some(&(4..=4))).unwrap();
        assert_eq!(ranged, made[1..]);
        // None for a formatted text; for one with an error, why, at the
        // first error, in characters.
        assert_eq!(
            edits("t.gcsl", formatted, Encoding::Utf16, None),
            Ok(vec![])
        );
        assert_eq!(
            edits("t.gcsl", "x = 1\ny = \"é\" (", Encoding::Utf16, None),
            Err("not formatted: it has a syntax error (2:10)".to_owned())
        );
        // A text with `\r\n` line breaks has every line edited to `\n`.
        let crlf = "x = 1\r\ny = 2\r\n";
        let made = edits("t.gcsl", crlf, Encoding::Utf16, None).unwrap();
        assert_eq!(apply(crlf, &made), "x = 1\ny = 2\n");
    }

    #[test]
    fn a_range_leaves_the_lines_either_side_of_it_alone() {
        // Four lines formatting respaces, one run of the diff: a range of
        // the middle two has those two edited, and the others kept.
        let text = "b   =   1\nc   =   2\nd   =   3\ne   =   4\n";
        let ranged = edits("t.gcsl", text, Encoding::Utf16, Some(&(1..=2))).unwrap();
        assert_eq!(apply(text, &ranged), "b   =   1\nc = 2\nd = 3\ne   =   4\n");
        // A run that removes a line is one edit, made whole: the line
        // respaced with the blank one removed after it.
        let text = "x = {\n    a=1\n\n\n    b = 2\n}\n";
        let ranged = edits("t.gcsl", text, Encoding::Utf16, Some(&(2..=2))).unwrap();
        assert_eq!(ranged.len(), 1, "{ranged:?}");
        assert_eq!(apply(text, &ranged), "x = {\n    a = 1\n\n    b = 2\n}\n");
    }
}
