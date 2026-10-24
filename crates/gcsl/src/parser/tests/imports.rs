//! Tests of the parser: named-argument shorthand and imports.

use super::*;
use crate::ast::{DefTypeRole, ImportGlob};

#[test]
fn a_name_alone_after_a_named_argument_is_shorthand() {
    let kinds = |text: &str| -> Vec<String> {
        let ast = parse_text(text).unwrap();
        let ExprKind::Call { args, .. } = ast.get(only_value(&ast)).kind else {
            panic!("expected a call");
        };
        ast.args(args)
            .iter()
            .map(|arg| match arg.kind {
                ArgKind::Positional => "positional".to_owned(),
                ArgKind::Spread { .. } => "spread".to_owned(),
                ArgKind::Named {
                    name, shorthand, ..
                } => {
                    let name = ast.names().get(name);
                    if shorthand {
                        format!("shorthand {name}")
                    } else {
                        format!("named {name}")
                    }
                }
                ArgKind::Trailing => "trailing".to_owned(),
            })
            .collect()
    };
    // After a named argument, at the end of the list or before a `,`.
    assert_eq!(
        kinds("f(x, out = 1, compiler)"),
        ["positional", "named out", "shorthand compiler"]
    );
    assert_eq!(
        kinds("f(out = 1, a, b,)"),
        ["named out", "shorthand a", "shorthand b"]
    );
    // Before any named argument, a name alone is positional.
    assert_eq!(kinds("f(a, b = 1)"), ["positional", "named b"]);
    // After one, anything else is positional still.
    assert_eq!(
        kinds("f(a = 1, b.c, 2, g(b))"),
        ["named a", "positional", "positional", "positional"]
    );
    // Its value is the read of its name, at its name.
    let ast = parse_text("f(a = 1, compiler)").unwrap();
    let ExprKind::Call { args, .. } = ast.get(only_value(&ast)).kind else {
        panic!("expected a call");
    };
    let arg = ast.args(args)[1];
    assert_eq!(ast.get(arg.value).span, Span::new(9, 17));
    assert!(matches!(ast.get(arg.value).kind, ExprKind::Name(_)));
    assert_eq!(arg.span(&ast), Span::new(9, 17));
    assert_eq!(shape("f(a = 1, compiler)"), "f(a = 1, compiler)");
}

#[test]
fn every_import_form_prints_as_written() {
    for text in [
        "import lib",
        "import tools/build",
        "import tools/build as builder",
        "import com from lib",
        "import com as libs from lib",
        "import a, b as c, * from lib/sub",
        "import * from lib",
        "import *, a from lib",
        "import lib; x = 1; import lib/y as y",
        "import pkgs/net.busybox.busybox as busybox",
        "import pkgs/dev.peios.live-root as live",
        "import org/gnu/gmp-c++/v6.3/x_1 as gmp",
        "import kernel from pkgs/dev.peios.kernel",
        "import 2fa as two",
    ] {
        assert_eq!(shape(text), text, "{text}");
    }
    // `from` is a name everywhere outside an import.
    assert_eq!(shape("from = 1; x = from"), "from = 1; x = from");
}

#[test]
fn an_import_item_spans_its_keyword_through_its_path_or_alias() {
    let ast = parse_text("import a/b as c").unwrap();
    let &[id] = ast.items(ast.root()) else {
        panic!("expected one item");
    };
    assert_eq!(ast.item(id).span, Span::new(0, 15));
    let ast = parse_text("import x, * from a/b").unwrap();
    let &[id] = ast.items(ast.root()) else {
        panic!("expected one item");
    };
    assert_eq!(ast.item(id).span, Span::new(0, 20));
    let ItemKind::Import(import) = ast.item(id).kind else {
        panic!("expected an import");
    };
    assert_eq!(import.index(), 0);
}

#[test]
fn import_is_a_keyword() {
    assert_eq!(
        message("import = 1"),
        "expected a module path after `import`, found `=`"
    );
    assert_eq!(message("x = import"), "expected expression, found `import`");
    assert_eq!(message("let import = 1"), "expected a name after `let`");
}

#[test]
fn import_only_at_the_top_level_of_a_file() {
    let text = "x = { import a }";
    assert_eq!(error(text), (MISPLACED_IMPORT.to_owned(), Span::new(6, 12)));
    assert_eq!(message("fn f(): Int { import a }"), MISPLACED_IMPORT);
    assert_eq!(message("def f(): Int { import a }"), MISPLACED_IMPORT);
    assert_eq!(message("x = lazy { import a }"), MISPLACED_IMPORT);
    assert_eq!(message("for i in [1] { import a }"), MISPLACED_IMPORT);
}

#[test]
fn import_path_errors() {
    // A string names a library, but with an interpolation in it, or with
    // a glob, it names nothing.
    assert_eq!(
        error("import \"github.com/{x}\""),
        (QUOTED_MODULE_PATH.to_owned(), Span::new(7, 20))
    );
    assert_eq!(
        message("import package * from \"lib\"/*"),
        QUOTED_MODULE_PATH
    );
    assert!(message("import \"a//b\"").starts_with("a library's path is its segments"));
    assert_eq!(
        message("import from as f"),
        "expected a path after `from`, found `as`"
    );
    assert_eq!(
        message("import x from"),
        "expected a path after `from`, found end of input"
    );
    assert_eq!(
        message("import x from from"),
        "expected a path after `from`, found `from`"
    );
    assert_eq!(
        message("import a as"),
        "expected a name after `as`, found end of input"
    );
    // An empty segment, at its `/`: one that ends the path or follows
    // another.
    assert_eq!(
        error("import lib/ as l"),
        (EMPTY_MODULE_SEGMENT.to_owned(), Span::new(10, 11))
    );
    assert_eq!(
        error("import lib//x"),
        (EMPTY_MODULE_SEGMENT.to_owned(), Span::new(11, 12))
    );
    // A segment that starts or ends with what only goes inside one, or has
    // `..` in it.
    assert_eq!(
        error("import lib/x. as y"),
        (
            format!("`x.` cannot name a module: {MODULE_SEGMENT}"),
            Span::new(11, 13)
        )
    );
    assert_eq!(
        message("import a..b as c"),
        format!("`a..b` cannot name a module: {MODULE_SEGMENT}")
    );
    assert_eq!(
        message("import a/../b"),
        format!("`..` cannot name a module: {MODULE_SEGMENT}")
    );
}

#[test]
fn a_module_path_is_the_run_of_glued_tokens() {
    // Each segment is named by its text, whatever its tokens: dots, `-`,
    // `+` and digits included.
    let ast = parse_text("import pkgs/net.busybox.busybox/x-1+2 as b").unwrap();
    let &[id] = ast.items(ast.root()) else {
        panic!("expected one item");
    };
    let ItemKind::Import(import) = ast.item(id).kind else {
        panic!("expected an import");
    };
    let segments = ast.path(ast.import_decl(import).path);
    let texts: Vec<&str> = segments
        .iter()
        .map(|segment| ast.names().get(segment.name().unwrap()))
        .collect();
    assert_eq!(texts, ["pkgs", "net.busybox.busybox", "x-1+2"]);
    let spans: Vec<Span> = segments.iter().map(|segment| segment.span).collect();
    assert_eq!(
        spans,
        [Span::new(7, 11), Span::new(12, 31), Span::new(32, 37)]
    );
    // A space ends the path, so what follows is the next token: `as`, or
    // an error.
    assert_eq!(message("import lib /x"), "expected end of item, found `/`");
}

#[test]
fn a_whole_import_needs_a_name_its_path_does_not_give() {
    let only = parse_text("import pkgs/net.busybox.busybox").unwrap_err();
    assert_eq!(
        only.message,
        "`net.busybox.busybox` is not a name, so the import needs one"
    );
    assert_eq!(only.span, Span::new(12, 31));
    // A name of identifiers joined by `.` may be the old form of a path.
    assert!(
        only.notes
            .iter()
            .any(|note| note.contains("write `net/busybox/busybox`"))
    );
    // A last segment that is a keyword names nothing either.
    assert_eq!(
        message("import lib/for"),
        "`for` is not a name, so the import needs one"
    );
    assert_eq!(
        message("import 2fa"),
        "`2fa` is not a name, so the import needs one"
    );
    // With `as`, any segment will do.
    assert!(parse_text("import pkgs/net.busybox.busybox as b").is_ok());
}

#[test]
fn import_member_errors() {
    assert_eq!(
        error("import a/b, c from p"),
        (SINGLE_MEMBER.to_owned(), Span::new(7, 10))
    );
    assert_eq!(
        error("import c, a.b from p"),
        (SINGLE_MEMBER.to_owned(), Span::new(10, 13))
    );
    assert_eq!(message("import a/b from p"), SINGLE_MEMBER);
    assert_eq!(message("import a/b from p"), SINGLE_MEMBER);
    assert_eq!(message("import c, a/b from p"), SINGLE_MEMBER);
    assert_eq!(
        error("import *"),
        (
            "`import *` needs `from` and a path".to_owned(),
            Span::new(7, 8)
        )
    );
    assert_eq!(message("import a, *"), "`import *` needs `from` and a path");
    assert_eq!(
        error("import *, a, * from p"),
        (
            "`*` is imported more than once".to_owned(),
            Span::new(13, 14)
        )
    );
    assert_eq!(
        message("import a, b c"),
        "expected `,` or `from`, found `c`"
    );
}

#[test]
fn an_import_of_a_def_type_s_definitions_prints_as_written() {
    for text in [
        "import package * from pkgs",
        "import package * from pkgs/*",
        "import package * from pkgs/**",
        "import package * from *",
        "import package * from **",
        "import pekit.package * from pkgs/dev.peios/*",
    ] {
        assert_eq!(shape(text), text, "{text}");
    }
    // The member is a use of the def-type, of its own role, its path
    // empty; the glob is the import's.
    let ast = parse_text("import package * from pkgs/**").unwrap();
    let ItemKind::Import(id) = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an import");
    };
    let decl = ast.import_decl(id);
    assert_eq!(decl.glob, ImportGlob::Descendants);
    assert_eq!(decl.path_at, Span::new(22, 29));
    let ImportForm::Members(range) = decl.form else {
        panic!("expected members");
    };
    let [ImportMember::DefType { def, span }] = ast.import_members(range) else {
        panic!("expected a def-type member");
    };
    assert_eq!(*span, Span::new(7, 16));
    assert_eq!(ast.def_type(*def).role, DefTypeRole::Import);
    assert!(ast.path(ast.def_type(*def).path).is_empty());
}

#[test]
fn an_import_of_a_def_type_s_definitions_has_one_member_and_globs_have_it() {
    assert_eq!(
        error("import package *, x from pkgs"),
        (
            "a def-type's `*` is imported alone, as in `import package * from pkgs/*`".to_owned(),
            Span::new(16, 17)
        )
    );
    let glob = "a glob imports the definitions of one def-type, as in `import package * from \
                pkgs/*`";
    assert_eq!(message("import x from pkgs/*"), glob);
    assert_eq!(message("import * from pkgs/**"), glob);
    assert_eq!(
        error("import package * from pkgs/***"),
        ("a glob is `*` or `**`".to_owned(), Span::new(27, 30))
    );
    assert_eq!(
        message("import a/b * from pkgs"),
        "a def-type is a name, or names joined by `.`"
    );
    assert_eq!(
        message("import package * pkgs"),
        "expected `from`, found `pkgs`"
    );
    // A path ending in `/` with no glob after it is the usual error.
    assert_eq!(
        message("import package * from pkgs/"),
        "expected a module name before and after each `/`"
    );
}
