//! "Did you mean" suggestions: the name most like a misspelled one among
//! those usable where it is written.
//!
//! A stage that reports a name that names nothing (an undefined variable, a
//! missing field, an unknown type, a named argument no parameter has, an
//! import of no module or member) gathers what could be written there
//! instead, each with how near its scope is, and asks [`best`] for the one to
//! suggest, which the diagnostic carries ([`crate::diagnostic::Suggestion`]).
//! The search runs only when such an error is reported, so it costs nothing
//! on valid input.
//!
//! THE RULE. Names are compared by their Damerau-Levenshtein distance, the
//! optimal string alignment form, counted in characters: an insertion, a
//! deletion, a substitution or a swap of two neighbouring characters is one
//! edit. A candidate qualifies if it is not the name itself and:
//!
//! - it differs from the name only in case (`int` and `Int`), which always
//!   qualifies, as distance 0; or
//! - the name has 1 or 2 characters, the candidate at least 3, and they are
//!   one edit apart (`id` and `idx`, never `x` and `y`); or
//! - the name has 3 or more characters, and they are at most `max(1, len /
//!   3)` edits apart, `len` being the name's length; and, at 2 or more
//!   edits, they begin with the same character, ignoring case, so that a
//!   name is not suggested for one it merely resembles in its middle.
//!
//! A candidate the name begins with, then a `-`, never qualifies: `count`
//! for `count-1`, a subtraction written without spaces, which reads as one
//! name and which the error's note explains.
//!
//! Of those that qualify, the best is the nearest by distance, then by
//! scope (the smaller rank, the nearer), then in alphabetical order; one
//! name is suggested at most.

/// The Damerau-Levenshtein distance (optimal string alignment) between `a`
/// and `b`, counted in characters, if it is at most `limit`; `None` if it is
/// more.
#[must_use]
pub(crate) fn distance(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    let width = b.len() + 1;
    // Three rows of the table: two back, the last, and the one being made.
    let mut before: Vec<usize> = vec![0; width];
    let mut last: Vec<usize> = (0..width).collect();
    let mut row: Vec<usize> = vec![0; width];
    for (i, &x) in a.iter().enumerate() {
        row[0] = i + 1;
        let mut smallest = row[0];
        for (j, &y) in b.iter().enumerate() {
            let cost = usize::from(x != y);
            let mut best = (last[j] + cost).min(last[j + 1] + 1).min(row[j] + 1);
            if i > 0 && j > 0 && Some(&x) == b.get(j - 1) && a.get(i - 1) == Some(&y) {
                best = best.min(before[j - 1] + 1);
            }
            row[j + 1] = best;
            smallest = smallest.min(best);
        }
        if smallest > limit {
            return None;
        }
        std::mem::swap(&mut before, &mut last);
        std::mem::swap(&mut last, &mut row);
    }
    let found = last.get(b.len()).copied()?;
    (found <= limit).then_some(found)
}

/// How far `candidate` is from `written` by the rule (see the module
/// documentation), if it qualifies.
fn qualifies(written: &str, candidate: &str) -> Option<usize> {
    if candidate == written {
        return None;
    }
    // `count-1` is most likely `count - 1`, which the error's note says.
    if written
        .strip_prefix(candidate)
        .is_some_and(|rest| rest.starts_with('-'))
    {
        return None;
    }
    if candidate.to_lowercase() == written.to_lowercase() {
        return Some(0);
    }
    let length = written.chars().count();
    if length <= 2 {
        if candidate.chars().count() <= 2 {
            return None;
        }
        return distance(written, candidate, 1);
    }
    let found = distance(written, candidate, (length / 3).max(1))?;
    let first = |text: &str| text.chars().next().map(|c| c.to_lowercase().to_string());
    if found >= 2 && first(written) != first(candidate) {
        return None;
    }
    Some(found)
}

/// The candidate to suggest for `written`, a name that names nothing, among
/// `candidates`, each a name usable where it is written with the rank of
/// its scope (0 the nearest): the best that qualifies by the rule (see the
/// module documentation), if any does.
#[must_use]
pub(crate) fn best<'a>(
    written: &str,
    candidates: impl IntoIterator<Item = (&'a str, u8)>,
) -> Option<&'a str> {
    candidates
        .into_iter()
        .filter_map(|(text, rank)| Some((qualifies(written, text)?, rank, text)))
        .min()
        .map(|(_, _, text)| text)
}

/// The items of `candidates` that qualify as suggestions for `written`, by
/// the text and rank each comes with, best first (see [`best`]): for a
/// caller that must still check that the best is usable, and so checks only
/// those close enough.
#[must_use]
pub(crate) fn ranked<'a, T>(
    written: &str,
    candidates: impl IntoIterator<Item = (&'a str, u8, T)>,
) -> Vec<T> {
    let mut found: Vec<(usize, u8, &str, T)> = candidates
        .into_iter()
        .filter_map(|(text, rank, item)| Some((qualifies(written, text)?, rank, text, item)))
        .collect();
    found.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    found.into_iter().map(|(_, _, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_counts_edits_and_swaps() {
        assert_eq!(distance("version", "version", 3), Some(0));
        assert_eq!(distance("verison", "version", 3), Some(1));
        assert_eq!(distance("versio", "version", 3), Some(1));
        assert_eq!(distance("versionn", "version", 3), Some(1));
        assert_eq!(distance("varsion", "version", 3), Some(1));
        assert_eq!(distance("vresoin", "version", 3), Some(2));
        assert_eq!(distance("", "abc", 3), Some(3));
        assert_eq!(distance("abc", "", 3), Some(3));
        assert_eq!(distance("kitten", "sitting", 3), Some(3));
        // Counted in characters, not bytes.
        assert_eq!(distance("é", "e", 1), Some(1));
        // Beyond the limit: none, early.
        assert_eq!(distance("kitten", "sitting", 2), None);
        assert_eq!(distance("a", "abcd", 2), None);
    }

    #[test]
    fn a_name_differing_in_case_always_qualifies() {
        assert_eq!(best("int", [("Int", 3), ("Float", 3)]), Some("Int"));
        assert_eq!(best("X", [("x", 0)]), Some("x"));
        // Before a nearer name one edit away.
        assert_eq!(best("Foo", [("foo", 5), ("Fob", 0)]), Some("foo"));
    }

    #[test]
    fn the_name_itself_is_never_suggested() {
        assert_eq!(best("count", [("count", 0)]), None);
    }

    #[test]
    fn a_subtraction_written_without_spaces_is_not_a_misspelling() {
        assert_eq!(best("count-1", [("count", 0)]), None);
        assert_eq!(best("count-n", [("count-m", 0)]), Some("count-m"));
    }

    #[test]
    fn a_name_too_far_from_every_candidate_has_no_suggestion() {
        assert_eq!(best("version", [("revision", 0), ("vendor", 0)]), None);
        // Three letters: one edit at most.
        assert_eq!(best("abc", [("xyc", 0)]), None);
        assert_eq!(best("abc", [("abd", 0)]), Some("abd"));
    }

    #[test]
    fn short_names_match_only_longer_names_one_edit_away() {
        // One or two characters: never another short name.
        assert_eq!(best("x", [("y", 0), ("xs", 0)]), None);
        assert_eq!(best("ab", [("ac", 0)]), None);
        // A name of 3 or more, one edit away.
        assert_eq!(best("id", [("idx", 0)]), Some("idx"));
        assert_eq!(best("id", [("ide", 0), ("idxx", 0)]), Some("ide"));
        assert_eq!(best("x", [("xyz", 0)]), None);
    }

    #[test]
    fn two_edits_need_the_same_first_character() {
        // Six characters allow two edits.
        assert_eq!(best("pkgnme", [("pkgname", 0)]), Some("pkgname"));
        assert_eq!(best("qlatform", [("platform", 0)]), Some("platform"));
        assert_eq!(best("qlatfirm", [("platform", 0)]), None);
        assert_eq!(best("platfirn", [("platform", 0)]), Some("platform"));
    }

    #[test]
    fn ties_go_to_the_nearer_scope_then_alphabetical_order() {
        assert_eq!(best("cout", [("count", 2), ("court", 1)]), Some("court"));
        assert_eq!(best("cout", [("court", 1), ("count", 1)]), Some("count"));
        // A nearer distance wins over a nearer scope.
        assert_eq!(
            best("verson", [("versions", 0), ("version", 3)]),
            Some("version")
        );
    }
}
