//! Checking files and directories: the types `File` and `Dir`, and their
//! one field.
//!
//! # Files
//!
//! `File` ([`TyId::FILE`]) is the type of a file: a value of its own kind
//! that holds its path and the content identity a host gives it (see
//! [Files](crate::eval::files#files)). It is nominal: no record fits it, and it fits
//! no record type, so that a host finds the files among a step's arguments
//! and result by their type, and a path passed as a plain `String` is never
//! taken for one. Two files are equal if their paths and identities are;
//! they have no order, "cannot order File". The prelude makes one,
//! `file(path)`, and gives it methods: `f.read()`, `f.exec(...words)` and,
//! on a `List<File>`, `files.file(name)`.
//!
//! `Dir` ([`TyId::DIR`]) is the type of a directory, a tree of files, of a
//! value of its own kind as a file's, and as nominal: an unpacked archive,
//! a toolchain's root, what a step leaves in a directory. The prelude makes
//! one, `dir(path)`, and gives it `d.file(path)`, `d.dir(path)`,
//! `d.files()` and `d.exec(...words)`; the `fs` module composes one,
//! `fs.compose_dir(entries)`.
//!
//! **Its field.** `f.path`, a file's or a directory's one field, is its
//! path, a `String`,
//! recorded for lowering ([`Resolution::file_path`](super::Resolution::file_path))
//! to read as [`IrKind::FilePath`](crate::ir::IrKind::FilePath). A file that
//! may be a future is awaited at the top first, as a record is. Any other
//! field is "no field `x` in File", noted "a file's one field is `path`",
//! and a directory's as it.

use crate::ast::ExprId;
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::strings::FieldName;
use crate::types::TyId;

use super::Checker;

impl Checker<'_> {
    /// Types the field access `id`, reading the field `name` of a file or a
    /// directory, of type `ty`, [`TyId::FILE`] or [`TyId::DIR`] (see
    /// [Files](#files)): `path`, a `String`, recorded for lowering, and an
    /// error for any other.
    pub(super) fn file_field(&mut self, id: ExprId, name: Name, ty: TyId) -> TyId {
        let text = self.ast.names().get(name);
        if text == "path" {
            self.lowering.file_paths.insert(id);
            return TyId::STRING;
        }
        let expr = self.ast.get(id);
        let member = expr.member_span(self.ast).unwrap_or(expr.span);
        let (shown, what) = if ty == TyId::DIR {
            ("Dir", "a directory's")
        } else {
            ("File", "a file's")
        };
        self.errors.push(
            Diagnostic::error(format!("no field `{}` in {shown}", FieldName(text)), member)
                .with_note(format!("{what} one field is `path`")),
        );
        TyId::ERROR
    }
}
