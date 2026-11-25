//! Tests of what of the standard files a step's code reaches (see
//! [Reach](crate::workspace::reach#reach)): an edit of a standard item the
//! code reaches changes its digest, and one of an item it does not, or a
//! comment, does not.

use super::strings::{checked_beside, prelude_entry};

/// The digest of the standard items the function `name` of
/// `app/build.gcsl`, of the text `build`, reaches, beside the prelude with
/// `extra` after it.
fn reach_of(extra: &str, build: &str, name: &str) -> [u8; 32] {
    let prelude = format!("{}\n{extra}\n", prelude_entry().1);
    let table = [("prelude", prelude.as_str())];
    let files = [("workspace.gcsl", ""), ("app/build.gcsl", build)];
    let (_, checked) = checked_beside(&table, &files);
    let workspace = checked.unwrap_or_else(|errors| panic!("{errors:?}"));
    let file = workspace.file("app/build.gcsl").unwrap();
    let at = build.find(&format!("fn {name}(")).unwrap();
    workspace
        .standard_reach(file, u32::try_from(at).unwrap())
        .unwrap()
}

/// The identity of the prelude function whose body begins after `marker`,
/// beside the prelude with `extra` after it.
fn function_of(extra: &str, marker: &str) -> [u8; 32] {
    let prelude = format!("{}\n{extra}\n", prelude_entry().1);
    let table = [("prelude", prelude.as_str())];
    let files = [("workspace.gcsl", "")];
    let (_, checked) = checked_beside(&table, &files);
    let workspace = checked.unwrap_or_else(|errors| panic!("{errors:?}"));
    let at = prelude.find(marker).unwrap() + marker.len();
    workspace
        .function_identity(0, u32::try_from(at).unwrap())
        .unwrap()
}

/// A step's function that calls the prelude's `zz_used`.
const USES: &str = "fn step(s: String) -> String { = zz_used(s) }\n";

#[test]
fn an_edit_of_a_standard_item_the_code_does_not_reach_changes_nothing() {
    let base = "def zz_used(s: String) -> String { = \"a{s}\" }\n\
                def zz_other() -> Int { = 1 }";
    let other = "def zz_used(s: String) -> String { = \"a{s}\" }\n\
                 def zz_other() -> Int { = 2 }";
    assert_eq!(reach_of(base, USES, "step"), reach_of(other, USES, "step"));
    // Nor does one added before it, which moves every slot after it.
    let before = "def zz_new() -> Int { = 3 }\n\
                  def zz_used(s: String) -> String { = \"a{s}\" }\n\
                  def zz_other() -> Int { = 1 }";
    assert_eq!(reach_of(base, USES, "step"), reach_of(before, USES, "step"));
    // Nor removing one.
    let removed = "def zz_used(s: String) -> String { = \"a{s}\" }";
    assert_eq!(
        reach_of(base, USES, "step"),
        reach_of(removed, USES, "step")
    );
}

#[test]
fn an_edit_of_a_standard_item_the_code_reaches_changes_its_digest() {
    let base = "def zz_used(s: String) -> String { = zz_inner(s) }\n\
                def zz_inner(s: String) -> String { = \"a{s}\" }";
    // The item itself.
    let itself = "def zz_used(s: String) -> String { = zz_inner(\"{s}!\") }\n\
                  def zz_inner(s: String) -> String { = \"a{s}\" }";
    assert_ne!(reach_of(base, USES, "step"), reach_of(itself, USES, "step"));
    // What it calls in turn.
    let inner = "def zz_used(s: String) -> String { = zz_inner(s) }\n\
                 def zz_inner(s: String) -> String { = \"b{s}\" }";
    assert_ne!(reach_of(base, USES, "step"), reach_of(inner, USES, "step"));
}

#[test]
fn a_comment_or_a_space_in_a_standard_item_changes_nothing() {
    let base = "def zz_used(s: String) -> String { = \"a{s}\" }";
    let commented = "# What it gives.\n\
                     def   zz_used(s: String)  ->  String { = \"a{s}\" }  # prefixed";
    assert_eq!(
        reach_of(base, USES, "step"),
        reach_of(commented, USES, "step")
    );
}

#[test]
fn a_method_of_the_name_a_field_call_spells_is_reached() {
    let build = "fn step(s: String) -> String { = s.zz_m() }\n";
    let base = "def (s: String) zz_m() -> String { = s }";
    let edited = "def (s: String) zz_m() -> String { = \"{s}!\" }";
    assert_ne!(
        reach_of(base, build, "step"),
        reach_of(edited, build, "step")
    );
    // One of that name for another type is not what the call chose.
    let other = "def (s: String) zz_m() -> String { = s }\n\
                 def (n: Int) zz_m() -> Int { = n }";
    assert_eq!(
        reach_of(base, build, "step"),
        reach_of(other, build, "step")
    );
    // One added that the receiver's type chooses over the method it chose
    // keys the step again, as its removal does.
    let build = "fn step(xs: List<String>) -> Int { = xs.zz_n() }\n";
    let generic = "def (xs: List<T>) zz_n<T>() -> Int { = len(xs) }";
    let specific = "def (xs: List<T>) zz_n<T>() -> Int { = len(xs) }\n\
                    def (xs: List<String>) zz_n() -> Int { = 0 }";
    assert_ne!(
        reach_of(generic, build, "step"),
        reach_of(specific, build, "step")
    );
}

#[test]
fn a_standard_item_a_reached_name_names_once_added_is_reached() {
    // The code spells `zz_name`, which the prelude declares only once the
    // second variant adds it: what the name means changed.
    let build = "fn step(s: String) -> String {\n\
                 \x20   let zz_name = s\n\
                 \x20   = zz_name\n\
                 }\n";
    let without = "def zz_unrelated() -> Int { = 1 }";
    let with = "def zz_unrelated() -> Int { = 1 }\n\
                def zz_name() -> Int { = 2 }";
    assert_ne!(
        reach_of(without, build, "step"),
        reach_of(with, build, "step")
    );
}

#[test]
fn a_context_s_default_is_reached() {
    let build = "fn step() -> String { = ^zz_ctx }\n";
    let base = "context zz_ctx: String = \"a\"";
    let edited = "context zz_ctx: String = \"b\"";
    assert_ne!(
        reach_of(base, build, "step"),
        reach_of(edited, build, "step")
    );
}

#[test]
fn a_standard_function_s_identity_is_its_reach_and_place_in_its_item() {
    let base = "def zz_used(s: String) -> String { = \"a{s}\" }";
    let marker = "def zz_used(s: String) -> String ";
    // Another item before it moves it in the file, not in its item.
    let before = "def zz_other() -> Int { = 1 }\n\
                  def zz_used(s: String) -> String { = \"a{s}\" }";
    assert_eq!(function_of(base, marker), function_of(before, marker));
    // Its own edit changes it.
    let edited = "def zz_used(s: String) -> String { = \"b{s}\" }";
    assert_ne!(function_of(base, marker), function_of(edited, marker));
}

/// [`reach_of`] beside the standard library of `table`, of the files
/// `files`, the function `name` of `app/build.gcsl` among them.
fn reach_beside(table: &[(&str, &str)], files: &[(&str, &str)], name: &str) -> [u8; 32] {
    let (_, checked) = checked_beside(table, files);
    let workspace = checked.unwrap_or_else(|errors| panic!("{errors:?}"));
    let file = workspace.file("app/build.gcsl").unwrap();
    let build = files
        .iter()
        .find(|(path, _)| *path == "app/build.gcsl")
        .unwrap()
        .1;
    let at = build.find(&format!("fn {name}(")).unwrap();
    workspace
        .standard_reach(file, u32::try_from(at).unwrap())
        .unwrap()
}

#[test]
fn a_member_of_an_imported_module_reaches_it_by_name_and_the_module_read_whole() {
    let fs = crate::STANDARD
        .iter()
        .find(|(stem, _)| *stem == "fs")
        .unwrap()
        .1;
    let prelude = prelude_entry().1;
    let added = format!("{fs}\nzz_added = 1\n");
    let edited = fs.replace(
        "exists = builtins.fs_exists",
        "exists = (builtins.fs_exists)",
    );
    assert_ne!(fs, edited);
    let reach = |fs: &str, build: &str| {
        let files = [("workspace.gcsl", ""), ("app/build.gcsl", build)];
        reach_beside(&[("fs", fs), ("prelude", prelude)], &files, "step")
    };
    // `fs.exists` reaches `exists`, by its name: its edit keys the step
    // again, another item's added to the module does not.
    let member = "import fs\nfn step(p: String) -> Bool { = fs.exists(p) }\n";
    assert_ne!(reach(fs, member), reach(&edited, member));
    assert_eq!(reach(fs, member), reach(&added, member));
    // The module read as a value is reached whole.
    let whole = "import fs\nfn step(p: String) -> Bool {\n\
                 \x20   let m = fs\n\
                 \x20   = m.exists(p)\n\
                 }\n";
    assert_ne!(reach(fs, whole), reach(&added, whole));
}

#[test]
fn a_context_a_with_binds_reaches_its_declaration() {
    // The step calls the library, so its file is the step's code, and only
    // binds the library's context: the binding reaches the declaration, and
    // what its default reads.
    let lib = "context zz_lib: String = zz_value\n\
               def lib_helper() -> String { = \"h\" }\n";
    let build = "fn step() -> String { = lib_helper() with { zz_lib = \"y\" } }\n";
    let files = [
        ("workspace.gcsl", ""),
        ("app/lib.gcsl", lib),
        ("app/build.gcsl", build),
    ];
    let prelude = |body: &str| format!("{}\nzz_value = {body}\n", prelude_entry().1);
    let (a, b) = (prelude("\"a\""), prelude("\"b\""));
    assert_ne!(
        reach_beside(&[("prelude", &a)], &files, "step"),
        reach_beside(&[("prelude", &b)], &files, "step")
    );
}

#[test]
fn a_file_s_own_helper_is_reached_through_what_of_it_is_public() {
    // `zz_helper` is the prelude's alone; the step reaches it through
    // `zz_used`, which the prelude calls it from.
    let base = "def zz_used(s: String) -> String { = zz_helper(s) }\n\
                fn zz_helper(s: String) -> String { = \"a{s}\" }";
    let edited = "def zz_used(s: String) -> String { = zz_helper(s) }\n\
                  fn zz_helper(s: String) -> String { = \"b{s}\" }";
    assert_ne!(reach_of(base, USES, "step"), reach_of(edited, USES, "step"));
    // Another file's name of that spelling is its own: the prelude's
    // helper, which it cannot name, is not reached.
    let own = "fn zz_helper(s: String) -> String { = s }\n\
               fn step(s: String) -> String { = zz_helper(s) }\n";
    let unused = "fn zz_helper(s: String) -> String { = \"a{s}\" }";
    let unused_edited = "fn zz_helper(s: String) -> String { = \"b{s}\" }";
    assert_eq!(
        reach_of(unused, own, "step"),
        reach_of(unused_edited, own, "step")
    );
}
