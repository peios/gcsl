//! Interned names.
//!
//! Every identifier in a program is interned once, by the lexer, into a
//! [`Names`] table, and from then on is a [`Name`]: a `u32` that compares,
//! hashes and copies as cheaply as an integer. Later stages never compare
//! identifier text; they compare `Name`s. The text is only looked up again
//! to show a name to the user, in a diagnostic or a dump.
//!
//! The table is owned by whichever stage is using it: the lexer while
//! lexing, then the [`crate::ast::Ast`]. A [`crate::session::Session`] keeps
//! one table for its whole lifetime and lends it to each input, so equal
//! names in different inputs are equal `Name`s.
//!
//! # Namespace keys
//!
//! The definitions made with a def-type (`def package x() { ... }`) are kept
//! under a root of the tree of top-level fields that no program can write, a
//! *namespace key*, one per def-type (see [`crate::workspace`],
//! "Def-types"): a name whose text is `<`, the def-type's path as its
//! declaration writes it, `#`, the number of the file or session input that
//! declares it, `.`, the number of its declaring item, and `>`, as in
//! `<package#3.17>` ([`namespace_key`]). An identifier never holds `<`, so
//! no variable or field written as one can be a key. A path of names whose
//! first is a key is shown as its def-type and the rest of the path, as the
//! definition is written, `package dev.peios.testpkg` ([`show_path`]).

use std::collections::HashMap;
use std::fmt::Write as _;

/// An interned identifier: an index into the [`Names`] table that produced
/// it.
///
/// Names are only meaningful for the table that produced them; using one
/// with a different table is a logic error.
///
/// Names are ordered by when they were first interned, not by their text.
/// The order exists so that record types can keep their fields sorted (see
/// [`crate::types::RecordTy`]); it means nothing to a reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(u32);

impl Name {
    /// The name as an index into its table.
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The interning table: each distinct text once, with its [`Name`].
#[derive(Clone, Debug, Default)]
pub struct Names {
    /// The text of each name, indexed by [`Name`].
    texts: Vec<String>,
    /// The name of each text, the inverse of `texts`.
    lookup: HashMap<String, Name>,
}

impl Names {
    /// Creates an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the name of `text`, adding it to the table if it is new.
    ///
    /// A table holds at most `u32::MAX` names. Every identifier consumes at
    /// least one byte of source and sources are limited to `u32::MAX` bytes,
    /// so a lexer can never reach the limit; past it, names would saturate.
    pub fn intern(&mut self, text: &str) -> Name {
        if let Some(&name) = self.lookup.get(text) {
            return name;
        }
        let name = Name(u32::try_from(self.texts.len()).unwrap_or(u32::MAX));
        self.texts.push(text.to_owned());
        self.lookup.insert(text.to_owned(), name);
        name
    }

    /// The name of `text` if it has been interned, without adding it.
    #[must_use]
    pub fn lookup(&self, text: &str) -> Option<Name> {
        self.lookup.get(text).copied()
    }

    /// The text of `name`.
    ///
    /// # Panics
    ///
    /// Panics if `name` was not produced by this table.
    #[must_use]
    pub fn get(&self, name: Name) -> &str {
        &self.texts[name.index()]
    }

    /// The number of distinct names interned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.texts.len()
    }

    /// Whether no name has been interned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// Forgets every name interned after the first `len`, so the table is
    /// as it was when it held `len` names. Used by
    /// [`crate::session::Session`] to undo a failed input.
    pub(crate) fn truncate(&mut self, len: usize) {
        for text in self.texts.drain(len.min(self.texts.len())..) {
            self.lookup.remove(&text);
        }
    }
}

/// The text of the namespace key of the def-type declared by item number
/// `item` of file or session input number `number`, whose path, as the
/// declaration writes it, is `path` (see [Namespace keys](#namespace-keys)).
#[must_use]
pub fn namespace_key(path: &str, number: u32, item: u32) -> String {
    format!("<{path}#{number}.{item}>")
}

/// The path of the def-type whose namespace key is `text`, as its
/// declaration writes it, if `text` is a namespace key: `package` for
/// `<package#3.17>` (see [Namespace keys](#namespace-keys)).
#[must_use]
pub fn key_def_type(text: &str) -> Option<&str> {
    let inner = text.strip_prefix('<')?.strip_suffix('>')?;
    let (path, numbers) = inner.rsplit_once('#')?;
    let (number, item) = numbers.split_once('.')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    (digits(number) && digits(item)).then_some(path)
}

/// The path of names `path`, as a diagnostic shows it: joined by `.`, but a
/// first name that is a namespace key shown as its def-type, then a space and
/// the rest, `package dev.peios.testpkg` (see [Namespace
/// keys](#namespace-keys)).
#[must_use]
pub fn show_path(names: &Names, path: &[Name]) -> String {
    let mut shown = String::new();
    let mut separator = "";
    for (index, &name) in path.iter().enumerate() {
        let text = names.get(name);
        match key_def_type(text).filter(|_| index == 0) {
            Some(def_type) => {
                shown.push_str(def_type);
                separator = " ";
                continue;
            }
            None => {
                let _ = write!(shown, "{separator}{text}");
            }
        }
        separator = ".";
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_namespace_key_shows_as_its_def_type() {
        let text = namespace_key("t.package", 3, 17);
        assert_eq!(text, "<t.package#3.17>");
        assert_eq!(key_def_type(&text), Some("t.package"));
        for plain in ["package", "<package>", "<package#3>", "<a#b.c>", "#3.4"] {
            assert_eq!(key_def_type(plain), None, "{plain}");
        }
        let mut names = Names::new();
        let path = [names.intern(&text), names.intern("dev"), names.intern("x")];
        assert_eq!(show_path(&names, &path), "t.package dev.x");
        assert_eq!(show_path(&names, &path[..1]), "t.package");
        assert_eq!(show_path(&names, &path[1..]), "dev.x");
    }

    #[test]
    fn equal_texts_intern_to_equal_names() {
        let mut names = Names::new();
        let a = names.intern("hello-world");
        let b = names.intern("x");
        assert_eq!(names.intern("hello-world"), a);
        assert_ne!(a, b);
        assert_eq!(names.get(a), "hello-world");
        assert_eq!(names.get(b), "x");
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn lookup_does_not_intern() {
        let mut names = Names::new();
        assert_eq!(names.lookup("Int"), None);
        assert!(names.is_empty());
        let int = names.intern("Int");
        assert_eq!(names.lookup("Int"), Some(int));
    }

    #[test]
    fn truncate_forgets_later_names() {
        let mut names = Names::new();
        let a = names.intern("a");
        names.intern("b");
        names.intern("c");
        names.truncate(1);
        assert_eq!(names.len(), 1);
        assert_eq!(names.lookup("b"), None);
        assert_eq!(names.lookup("a"), Some(a));
        // A re-interned text gets the next free name again.
        let b = names.intern("b");
        assert_eq!(names.get(b), "b");
        assert_eq!(names.len(), 2);
        names.truncate(10);
        assert_eq!(names.len(), 2);
    }
}
