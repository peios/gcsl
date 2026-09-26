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

use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use super::*;

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
