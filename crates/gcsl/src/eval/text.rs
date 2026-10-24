//! The built-in functions of strings and versions.

use std::cmp::Ordering;
use std::rc::Rc;

use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{List, Value};

use super::{Flow, mistyped};

/// The pieces of `s` between occurrences of `sep`, in order, as
/// [`str::split`] gives them, for the call `node` of `split` (see [The
/// built-in functions](crate::eval::calls#the-built-in-functions)): an empty `s` is one empty
/// piece, and a `sep` at either end or twice in a row an empty piece there.
/// Fails with "`split` needs a separator that is not empty" for an empty
/// `sep`, which `str::split` would match between every two characters.
pub(super) fn split(s: &str, sep: &str, node: &IrNode) -> Flow<Value> {
    if sep.is_empty() {
        return Err(
            Diagnostic::error("`split` needs a separator that is not empty", node.span).into(),
        );
    }
    let pieces = s.split(sep).map(|piece| Value::Str(Rc::from(piece)));
    Ok(Value::List(List::from_vec(pieces.collect())))
}

/// The strings `parts` with `sep` between each pair, for the call `node` of
/// `join` (see [The built-in functions](crate::eval::calls#the-built-in-functions)): `""` for
/// no parts. An element that is not a string, which the checker rules out,
/// is an internal error.
pub(super) fn join(parts: &List, sep: &str, node: &IrNode) -> Flow<Value> {
    let mut joined = String::new();
    for (at, part) in parts.values().iter().enumerate() {
        let Value::Str(part) = part else {
            return mistyped(node, "a part of `join` that is not a string");
        };
        if at > 0 {
            joined.push_str(sep);
        }
        joined.push_str(part);
    }
    Ok(Value::Str(Rc::from(joined)))
}

/// `s` without its leading and trailing whitespace, as [`str::trim`] finds
/// it (Unicode's `White_Space`): `s` itself, shared, when it has none.
pub(super) fn trim(s: Rc<str>) -> Rc<str> {
    let trimmed = s.trim();
    if trimmed.len() == s.len() {
        s
    } else {
        Rc::from(trimmed)
    }
}

/// `s` without the bytes of `prefix` if it begins with them, as
/// [`str::strip_prefix`] finds them, else `s` itself, shared: so an empty
/// `prefix`, or one that ends inside a character of `s`, gives `s`.
pub(super) fn strip_prefix(s: Rc<str>, prefix: &str) -> Rc<str> {
    match s.strip_prefix(prefix) {
        Some(stripped) if stripped.len() < s.len() => Rc::from(stripped),
        _ => s,
    }
}

/// `s` without the bytes of `suffix` if it ends with them, as
/// [`str::strip_suffix`] finds them, else `s` itself, shared: so an empty
/// `suffix`, or one that begins inside a character of `s`, gives `s`.
pub(super) fn strip_suffix(s: Rc<str>, suffix: &str) -> Rc<str> {
    match s.strip_suffix(suffix) {
        Some(stripped) if stripped.len() < s.len() => Rc::from(stripped),
        _ => s,
    }
}

/// The list of the strings `pieces`, in order: what `lines` and `fields`
/// give (see [The built-in functions](crate::eval::calls#the-built-in-functions)).
pub(super) fn string_list<'a>(pieces: impl Iterator<Item = &'a str>) -> Value {
    let pieces = pieces.map(|piece| Value::Str(Rc::from(piece)));
    Value::List(List::from_vec(pieces.collect()))
}

/// How `a` sorts against `b` in version order, the order of GNU `sort -V`'s
/// core, dpkg's `verrevcmp` applied to the whole strings (see [The built-in
/// functions](crate::eval::calls#the-built-in-functions)): alternately a run of non-digits and
/// a run of ASCII digits of each, starting with non-digits, either run
/// perhaps empty; non-digit runs byte by byte by [`version_weight`], the end
/// of a run weighing as `0`; digit runs as numbers, without their leading
/// zeros, the longer one the greater, else the first digit that differs,
/// never parsed, so of any length. Strings whose runs are all equal but
/// which differ (`1.01` and `1.1`) are ordered by their bytes, so only equal
/// strings are `Equal`. Public, so that a host orders versions as the
/// built-in does (`pekit` chooses `@latest` by it).
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (mut a_rest, mut b_rest) = (a.as_bytes(), b.as_bytes());
    while !a_rest.is_empty() || !b_rest.is_empty() {
        let (a_run, a_after) = version_run(a_rest, false);
        let (b_run, b_after) = version_run(b_rest, false);
        for at in 0..a_run.len().max(b_run.len()) {
            let order =
                version_weight(a_run.get(at).copied()).cmp(&version_weight(b_run.get(at).copied()));
            if order.is_ne() {
                return order;
            }
        }
        let (a_digits, a_after) = version_run(a_after, true);
        let (b_digits, b_after) = version_run(b_after, true);
        let (a_number, b_number) = (
            without_leading_zeros(a_digits),
            without_leading_zeros(b_digits),
        );
        let order = a_number
            .len()
            .cmp(&b_number.len())
            .then_with(|| a_number.cmp(b_number));
        if order.is_ne() {
            return order;
        }
        (a_rest, b_rest) = (a_after, b_after);
    }
    a.as_bytes().cmp(b.as_bytes())
}

/// The longest run at the start of `bytes` of ASCII digits if `digits`,
/// else of other bytes, and what follows it: a run of [`compare_versions`].
pub(super) fn version_run(bytes: &[u8], digits: bool) -> (&[u8], &[u8]) {
    let end = bytes
        .iter()
        .position(|byte| byte.is_ascii_digit() != digits)
        .unwrap_or(bytes.len());
    bytes.split_at(end)
}

/// The digits `digits` without their leading zeros: the number they write,
/// for [`compare_versions`], which compares two such by their length first.
pub(super) fn without_leading_zeros(digits: &[u8]) -> &[u8] {
    let start = digits
        .iter()
        .position(|&digit| digit != b'0')
        .unwrap_or(digits.len());
    digits.split_at(start).1
}

/// The weight of a byte of a non-digit run in [`compare_versions`], dpkg's
/// `order`: `~` lightest, before even the end of the run (`None`, `0`), then
/// the ASCII letters by their byte, then every other byte by its byte plus
/// 256.
pub(super) fn version_weight(byte: Option<u8>) -> i32 {
    match byte {
        None => 0,
        Some(b'~') => -1,
        Some(letter) if letter.is_ascii_alphabetic() => i32::from(letter),
        Some(other) => i32::from(other) + 256,
    }
}

/// The strings of `versions`, for the call `node` of `sort_versions`,
/// ascending in version order, as [`compare_versions`] orders two, by a
/// stable sort: since only equal strings compare equal, a string that is
/// there twice is there twice, side by side (see [The built-in
/// functions](crate::eval::calls#the-built-in-functions)). An element that is not a string
/// is an internal error, which the checker rules out.
pub(super) fn sort_versions(versions: &List, node: &IrNode) -> Flow<Value> {
    if versions
        .values()
        .iter()
        .any(|version| !matches!(version, Value::Str(_)))
    {
        return mistyped(node, "a version to sort that is not a string");
    }
    let mut sorted = versions.values().to_vec();
    sorted.sort_by(|a, b| match (a, b) {
        (Value::Str(a), Value::Str(b)) => compare_versions(a, b),
        _ => Ordering::Equal,
    });
    Ok(Value::List(List::from_vec(sorted)))
}
