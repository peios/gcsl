//! Reading the imports of a prelude a host gives a file (see [Given
//! preludes](crate::workspace#given-preludes)).
//!
//! A host gives a file its prelude as import items, each as it is written
//! in a file ([`SourceFile::with_prelude`]): `import * from "provium"`,
//! `import "provium/vm"`, `import step, fixture as fx from "provium"`.
//! Each is parsed on its own ([`prelude_imports`]) and must be one import
//! item of a library, whole, `*` or by its members, so that a prelude, like
//! a library, means the same in every workspace: an import of a workspace's
//! module or of a standard module, of a def-type's definitions, or with a
//! glob, is none, and neither is anything else. What is not is the error of
//! the file given it, at its start, "the prelude given for this file
//! imports only libraries, whole, `*` or by name: `import lib/x`"; its
//! other imports are given still.

use crate::ast::{ImportForm, ImportGlob, ImportMember, ItemKind};
use crate::parser;
use crate::source::SourceFile;

use super::LIBRARIES;

/// One import of a prelude a host gave a file, as [`prelude_imports`] read
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreludeImport {
    /// The library's path, as an import string names it: `provium/vm`.
    pub(crate) library: String,
    /// What it binds.
    pub(crate) form: PreludeForm,
}

/// What an import of a prelude binds, by the names' text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PreludeForm {
    /// The library whole, `import "provium/vm"`, named as written with
    /// `as`, if it is.
    Whole(Option<String>),
    /// Every root of it, `import * from "provium"`.
    All,
    /// Its members, each with the name written with `as`, if one is.
    Members(Vec<(String, Option<String>)>),
}

/// The error of a prelude's import `text` that is not one a prelude takes
/// (see the module documentation).
#[must_use]
pub(crate) fn not_a_prelude_import(text: &str) -> String {
    format!(
        "the prelude given for this file imports only libraries, whole, `*` or by name: \
         `{}`",
        text.trim()
    )
}

/// Each of `imports`, a prelude's, read, in order: the import it is, or
/// the error that it is none a prelude takes (see the module
/// documentation).
#[must_use]
pub(crate) fn prelude_imports(imports: &[String]) -> Vec<Result<PreludeImport, String>> {
    imports.iter().map(|text| prelude_import(text)).collect()
}

/// The import `text` is, as [`prelude_imports`] reads each.
fn prelude_import(text: &str) -> Result<PreludeImport, String> {
    let error = || not_a_prelude_import(text);
    let source = SourceFile::new("<prelude>", text).map_err(|_| error())?;
    let (ast, errors) = parser::parse_source(&source);
    let items = ast.items(ast.root());
    let ([item], true) = (items, errors.is_empty()) else {
        return Err(error());
    };
    let ItemKind::Import(id) = ast.item(*item).kind else {
        return Err(error());
    };
    let decl = ast.import_decl(id);
    let names = ast.names();
    let segments: Vec<&str> = ast
        .path(decl.path)
        .iter()
        .map(|segment| segment.name().map(|name| names.get(name)))
        .collect::<Option<_>>()
        .ok_or_else(error)?;
    let Some((&LIBRARIES, library)) = segments.split_first() else {
        return Err(error());
    };
    if decl.glob != ImportGlob::Exact || library.is_empty() {
        return Err(error());
    }
    let text_of = |name| names.get(name).to_owned();
    let form = match decl.form {
        ImportForm::Whole(alias) => PreludeForm::Whole(alias.map(|alias| text_of(alias.name))),
        ImportForm::Members(members) => match ast.import_members(members) {
            [ImportMember::All(_)] => PreludeForm::All,
            members => PreludeForm::Members(
                members
                    .iter()
                    .map(|member| match member {
                        ImportMember::Named { name, alias } => {
                            Some((text_of(name.name), alias.map(|alias| text_of(alias.name))))
                        }
                        ImportMember::All(_) | ImportMember::DefType { .. } => None,
                    })
                    .collect::<Option<_>>()
                    .ok_or_else(error)?,
            ),
        },
    };
    Ok(PreludeImport {
        library: library.join("/"),
        form,
    })
}

/// The segments of `import`'s module path, [`LIBRARIES`] first.
pub(crate) fn library_path(import: &PreludeImport) -> impl Iterator<Item = &str> {
    std::iter::once(LIBRARIES).chain(import.library.split('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<PreludeImport, String> {
        prelude_import(text)
    }

    #[test]
    fn a_prelude_imports_libraries_whole_by_star_or_by_name() {
        assert_eq!(
            read("import * from \"provium\""),
            Ok(PreludeImport {
                library: "provium".to_owned(),
                form: PreludeForm::All
            })
        );
        assert_eq!(
            read("import \"provium/vm\""),
            Ok(PreludeImport {
                library: "provium/vm".to_owned(),
                form: PreludeForm::Whole(None)
            })
        );
        assert_eq!(
            read("import \"provium/vm\" as machine"),
            Ok(PreludeImport {
                library: "provium/vm".to_owned(),
                form: PreludeForm::Whole(Some("machine".to_owned()))
            })
        );
        assert_eq!(
            read("import step, fixture as fx from \"provium\""),
            Ok(PreludeImport {
                library: "provium".to_owned(),
                form: PreludeForm::Members(vec![
                    ("step".to_owned(), None),
                    ("fixture".to_owned(), Some("fx".to_owned()))
                ])
            })
        );
        let import = read("import \"a/b\"").unwrap();
        let library: Vec<&str> = library_path(&import).collect();
        assert_eq!(library, ["<lib>", "a", "b"]);
    }

    #[test]
    fn anything_else_is_no_import_of_a_prelude() {
        for text in [
            "import lib/x",
            "import path",
            "import * from lib/*",
            "import test * from \"provium\"",
            "x = 1",
            "import * from \"provium\"\nimport \"other\"",
            "import * from",
            "",
        ] {
            assert_eq!(read(text), Err(not_a_prelude_import(text)), "{text}");
        }
        assert_eq!(
            not_a_prelude_import("import lib/x"),
            "the prelude given for this file imports only libraries, whole, `*` or by name: \
             `import lib/x`"
        );
    }
}
