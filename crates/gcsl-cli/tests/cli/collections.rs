//! Tests of the `gcsl` command: lists, ranges, mutable variables, loops,
//! unions, updates and spreads, run and dumped.

use super::*;

#[test]
fn run_builds_a_deployment_of_lists() {
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{count = 3; first = 22; report = \"[\\\"web\\\", \\\"db\\\"]: [8080, 5432]\"; \
         same = true; services = [{name = \"web\"; replicas = 2}, {name = \"db\"; replicas = \
         1}]}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_lists() {
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in [
        "\nIdent(String) @230..236\nLBracket @236..237\nRBracket @237..238\n",
        "\nDotDotDot @576..579\n",
    ] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n      (params (names String[]))\n      (ret impure Int[])\n",
        "\n    (list @445..469\n\
         \x20     (int 22 @446..448)\n\
         \x20     (index @450..458\n\
         \x20       (name found @450..455)\n\
         \x20       (int 0 @456..457))\n",
        "\n    (list @575..643\n\
         \x20     (spread\n\
         \x20       (list @579..611\n",
        "(let open : List<Present<Int>> @414..469\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        // A bare list parameter of an impure function is the either shape,
        // element and all; its result is inferred from its body's list.
        "(name ports : fn(names: List<String | Future<String>> | Future<List<String | \
         Future<String>>>) -> List<Future<Int>> @334..339)",
        "(list : List<String> @308..321\n",
        // An element is read as it is.
        "(index : Future<Int> @450..458\n",
        "(name len : fn(List<Int>): Int @478..481)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        "\n    (make-list @445..469\n\
         \x20     (int 22 @446..448)\n\
         \x20     (await @450..458\n\
         \x20       (index @450..458\n\
         \x20         (load found#127 @450..455)\n\
         \x20         (int 0 @456..457)))\n",
        "\n    (list-len @478..487\n\
         \x20     (load open#128 @482..486)))\n",
        "\n      (await *! @524..529\n\
         \x20       (load found#127 @524..529))\n",
        "\n    (list-eq == * @539..563\n",
        "\n    (make-list @575..643\n\
         \x20     (spread @576..611\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn an_index_out_of_range_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/index_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: index out of range\n\
         \x20--> tests/fixtures/index_error.gcsl:2:1\n\
         \x20 |\n\
         2 | ports[2]\n\
         \x20 | ^^^^^^^^ the index is 2, and the list has 2 elements\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_rolls_out_a_fleet_with_fors() {
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{count = 3; plan = \"[\\\"web-1\\\", \\\"web-2\\\"] on [8000, 8001, 8002]\"; squares = \
         [1, 4, 9]; same = true}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_fors_and_ranges() {
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in [
        "\nFor @213..216\nIdent(i) @217..218\nIn @219..221\nInt(0) @222..223\nDotDot @223..225\n",
        "\nInt(1) @687..688\nDotDotEq @688..691\nInt(3) @691..692\n",
    ] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n    (for host @310..354\n\
         \x20     (name hosts @322..327)\n\
         \x20     (guard\n\
         \x20       (!= @331..345\n\
         \x20         (name host @331..335)\n\
         \x20         (str \"db-1\" @339..345)))\n\
         \x20     (block @346..354\n",
        "\n    (for i @678..702\n\
         \x20     (..= @687..692\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "(for i : List<Int> @213..248\n\
         \x20     (.. : List<Int> @222..235\n",
        "(for host : List<String> @310..354\n",
        // A statement of `{}`s, and one of futures.
        "(for host : List<{}> @426..459\n",
        "(for host : List<Future<{}>> @524..562\n",
        // The body is awaited to fit the expected element type, so the
        // list holds what it is once awaited.
        "(for p : List<Int> @596..621\n",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // A `for` shows its variable's slot, named, as every slot is, by
        // the last variable stored in it.
        "\n    (for ports#125 @213..248\n\
         \x20     (range @222..235\n\
         \x20       (int 0 @222..223)\n\
         \x20       (list-len @225..235\n",
        "\n    (for webs#126 if @310..354\n\
         \x20     (load hosts#124 @322..327)\n\
         \x20     (str-cmp != @331..345\n",
        // The statement of `{}`s discards its body's values; the statement
        // of futures builds its list, and awaits it deeply.
        "\n  (for-each checks#127 @426..459\n\
         \x20   (load webs#126 @438..442)\n",
        "\n  (await *! @524..562\n\
         \x20   (for checks#127 @524..562\n",
        "\n    (for checks#127 @596..621\n\
         \x20     (load ports#125 @605..610)\n\
         \x20     (await @611..621\n",
        "\n    (for i#128 @678..702\n\
         \x20     (range-inclusive @687..692\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_range_too_large_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/range_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: range too large\n\
         \x20--> tests/fixtures/range_error.gcsl:1:11\n\
         \x20 |\n\
         1 | let ids = 0..=9223372036854775807\n\
         \x20 |           ^^^^^^^^^^^^^^^^^^^^^^^ this range has 9223372036854775808 elements\n\
         \x20 |\n\
         \x20 = note: a range used as a list holds every element, and a list cannot hold more \
         than 384307168202282325. A `for` over a range directly builds no list\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn iterating_over_what_is_not_a_list_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/iterate_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot iterate over a value of type Int: it is not a list\n\
         \x20--> tests/fixtures/iterate_error.gcsl:2:10\n\
         \x20 |\n\
         2 | for r in replicas { r }\n\
         \x20 |          ^^^^^^^^ this is Int\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn repl_shows_fors_and_ranges() {
    let output = repl(
        "let xs = for i in 0..3 { i * 2 }\n\
         xs\n\
         for x in xs if x > 0 { lazy x }\n\
         0..=2\n",
    );
    assert_eq!(
        stdout(&output),
        "[0, 2, 4] : List<Int>\n\
         [Future<Int>, Future<Int>] : List<Future<Int>>\n\
         [0, 1, 2] : List<Int>\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_tallies_checks_in_mutable_variables() {
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{checked = 550; slow = \"web-2\"; port = 443; hosts = [\"web-1\", \"db-1\"]; average = \
         183}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_mutable_variables() {
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    let token = "\nLet @159..162\nMut @163..166\nIdent(total) @167..172\nEq @173..174\n";
    assert!(tokens.contains(token), "{token}: {tokens}");
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n  (let mut total @159..176\n\
         \x20   (int 0 @175..176))\n",
        "\n      (block @299..366\n\
         \x20       (rebind total @303..324\n",
        "\n        (rebind conf.tls @491..507\n",
        "\n        (let mut sum @793..808\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "(for c : List<{}> @283..366\n",
        "(name slowest : {host: String, ms: Int} @337..344)",
        // A dotted rebind leaves the variable's type as it was declared.
        "(name conf : {port: Int, tls: Bool} @907..911)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // A rebind in a `for`'s body, and in an `if` in it, stores into the
        // variable's one slot.
        "\n  (store total#126 @159..176\n\
         \x20   (int 0 @175..176))\n",
        "\n    (block @299..366\n\
         \x20     (store total#126 @303..324\n\
         \x20       (int-add @312..324\n\
         \x20         (load total#126 @312..317)\n",
        "\n        (block @348..364\n\
         \x20         (store slowest#127 @350..362\n\
         \x20           (load conf#128 @361..362))\n",
        // The guard reads the slot the body writes.
        "\n      (int-cmp >= @621..635\n\
         \x20       (load budget#129 @621..627)\n",
        "\n        (store budget#129 @638..661\n",
        // The closure copies the variable when it is made.
        "\n  (store report#131 @721..753\n\
         \x20   (make-closure #0 @734..753\n\
         \x20     (load total#126 @734..753)))\n\
         \x20 (store total#126 @754..764\n",
        // A function's own variable, in its own frame.
        "\n      (block @823..841\n\
         \x20       (store sum#1 @825..839\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn rebinding_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/rebind_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot rebind `total` here: it is declared in an enclosing block\n\
         \x20--> tests/fixtures/rebind_error.gcsl:2:19\n\
         \x20 |\n\
         1 | let total = 0\n\
         \x20 | ------------- declared here\n\
         2 | for x in [1, 2] { total := total + x }\n\
         \x20 |                   ^^^^^\n\
         \x20 |\n\
         \x20 = note: shadow it with `let total = ...` instead; only a variable declared with \
         `let mut` can be rebound from an inner block\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl(&["run", "tests/fixtures/mut_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot rebind `total` here: it is declared outside this function\n\
         \x20--> tests/fixtures/mut_error.gcsl:2:29\n\
         \x20 |\n\
         1 | let mut total = 0\n\
         \x20 | ----------------- declared here\n\
         2 | let add = fn(x: Int): Int { total := total + x; total }\n\
         \x20 |                             ^^^^^\n\
         \x20 |\n\
         \x20 = note: a function holds its own copy of each variable it uses, made when the \
         function is created; rebinding the copy would not change `total`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_works_through_a_queue_with_every_kind_of_loop() {
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{done = 2; costs = [3, 8, 5]; loose = [{}, \"build\"]; first = [\"build\"]; spent = [6, \
         2]; attempts = [1, 2]; rows = [[10, 20], [50]]}\n"
    );
    // Find-first without a final `continue` collects `{}` too: a warning.
    // With one, none.
    assert_eq!(
        stderr(&output),
        "warning: the values this loop collects have different types\n\
         \x20--> tests/fixtures/loops.gcsl:9:13\n\
         \x20 |\n\
         9 | let loose = for j in jobs { if j.cost > 5 { break j.name } }\n\
         \x20 |             ^^^             ------------------------------\n\
         \x20 |                             |                     |\n\
         \x20 |                             |                     this is String\n\
         \x20 |                             this is {}\n\
         \x20 |\n\
         \x20 = note: the loop is `List<String | {}>`; annotate it with that type if all are \
         meant, or tell them apart with `is`\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_loops_and_their_stops() {
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in [
        "\nDo @825..827\n",
        "\nBreak @473..478\n",
        "\nContinue @1017..1025\nLabel(rows) @1025..1030\n",
        "\nFor @972..975\nLabel(rows) @975..980\n",
    ] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n    (for @306..363\n\
         \x20     (let i @310..319\n\
         \x20       (int 0 @318..319))\n\
         \x20     (cond\n",
        "\n      (rebind i @336..346\n",
        "\n    (do @825..871\n\
         \x20     (block @828..857\n",
        "\n      (cond\n\
         \x20       (< @862..871\n",
        "(for'rows r @972..1044\n",
        "(continue'rows @1017..1030)",
        "(break @473..485\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "(for : List<{}> @205..238\n",
        "(for : List<Int> @306..363\n",
        "(for j : List<String | {}> @441..489\n",
        "(for j : List<String> @558..616\n",
        "(do : List<Int> @825..871\n",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // A statement loop collects nothing.
        "\n  (loop-each @205..238\n",
        // A header's items before the condition run once, in a block
        // around the loop; those after it are the loop's last parts.
        "\n    (block @306..363\n\
         \x20     (store costs#121 @310..319\n",
        "\n      (loop @306..363\n",
        // `for { }` tests `true`.
        "\n    (loop @692..753\n\
         \x20     (bool true @692..695)\n",
        "\n    (do @825..871\n",
        // A stop counts the loops it leaves.
        "(break 0 @473..485\n",
        "(continue 0 @606..614)",
        "(continue 1 @1017..1030)",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn loop_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/loop_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot rebind `i` here: it is declared in the header of a `for`\n\
         \x20--> tests/fixtures/loop_error.gcsl:1:50\n\
         \x20 |\n\
         1 | let squares = for let i = 0; i < 3; i := i + 1 { i := i * i }\n\
         \x20 |                   --------- declared here        ^\n\
         \x20 |\n\
         \x20 = note: a variable of a `for` header is rebound in the header, after the \
         condition; declare it with `let mut i = ...` in the header to rebind it from the body \
         too\n\
         \n\
         error: `break` cannot leave a function\n\
         \x20--> tests/fixtures/loop_error.gcsl:2:56\n\
         \x20 |\n\
         2 | let found = for x in [1, 2] { let check = fn(): Bool { break x }; x }\n\
         \x20 |                                           --           ^^^^^\n\
         \x20 |                                           |\n\
         \x20 |                                           inside a function here\n\
         \x20 |\n\
         \x20 = note: the body of a function runs when it is called, which may be after the \
         loop has ended\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_tells_the_members_of_unions_apart() {
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{ports = [8080, 5432]; label = 2; first = 8081; same = false}\n"
    );
    // The two services' ports differ: a union, and a warning.
    assert_eq!(
        stderr(&output),
        "warning: the elements of this list have different types\n\
         \x20--> tests/fixtures/unions.gcsl:2:48\n\
         \x20 |\n\
         2 | let services = [{ name = \"web\"; port = 8080 }, { name = \"db\"; port = \
         \"postgres\" }]\n\
         \x20 |                 -----------------------------  \
         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ this is {name: String, port: String}\n\
         \x20 |                 |\n\
         \x20 |                 the elements so far are {name: String, port: Int}\n\
         \x20 |\n\
         \x20 = note: the list is `List<{name: String, port: Int | String}>`; annotate it with \
         that type if both are meant, or tell them apart with `is`\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_unions_tests_and_narrowing() {
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in ["\nIs @326..328\n", "\nAs @629..631\n"] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    let node = "(is Int @324..332\n\
                \x20             (name p @324..325))\n";
    assert!(ast.contains(node), "{node}: {ast}");
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        // The field the elements disagree on is the union of both.
        "(name s : {name: String, port: Int | String} @312..313)",
        // `p` is an Int in one branch and a String in the other.
        "(name p : Int | String @324..325))",
        "(name p : Int @335..336)",
        "(name p : String @347..348)",
        "(as Int : Int @612..635",
        "(=== : Bool @708..745",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // The test looks at the kind alone, and writes the slot the
        // branches read, which a later variable takes once it is free.
        "(is int first#123 @324..332\n\
         \x20           (load label#122 @324..325))\n\
         \x20         (block @333..338\n\
         \x20           (load first#123 @335..336))\n",
        "(as int @612..635\n",
        "(value-same === @708..745\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn union_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/union_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "warning: the branches of this `if` have different types\n\
         \x20--> tests/fixtures/union_error.gcsl:1:36\n\
         \x20 |\n\
         1 | let port = if true { 8080 } else { \"http\" }\n\
         \x20 |                      ----          ^^^^^^ this is String\n\
         \x20 |                      |\n\
         \x20 |                      this is Int\n\
         \x20 |\n\
         \x20 = note: the `if` is `Int | String`; annotate it with that type if both are meant, \
         or tell them apart with `is`\n\
         \n\
         error: cannot add `Int | String` and Int\n\
         \x20--> tests/fixtures/union_error.gcsl:2:6\n\
         \x20 |\n\
         2 | port + 1\n\
         \x20 | ---- ^ - Int\n\
         \x20 | |\n\
         \x20 | Int | String\n\
         \x20 |\n\
         \x20 = note: tell them apart first, as in `if port is Int { ... }`\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl(&["run", "tests/fixtures/as_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `port` is not an Int\n\
         \x20--> tests/fixtures/as_error.gcsl:2:1\n\
         \x20 |\n\
         2 | port as Int\n\
         \x20 | ^^^^^^^^^^^ it is a String\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_updates_elements_of_lists() {
    let output = gcsl(&["run", "tests/fixtures/update.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{up = true; ports = [81, 5433]; cells = [[0, 1], [2, 3]]; counts = [0, 5, 0]; before = \
         [0, 0, 0]; r = {a = 8; b = 7}}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_element_updates() {
    let output = gcsl(&["run", "tests/fixtures/update.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    let token = "\nIdent(hosts) @247..252\nLBracket @252..253\nInt(0) @253..254\nRBracket \
                 @254..255\nDot @255..256\nIdent(up) @256..258\nColonEq @259..261\n";
    assert!(tokens.contains(token), "{token}: {tokens}");
    let output = gcsl(&["run", "tests/fixtures/update.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        // A path shows as it is written, and each position before the
        // value.
        "\n  (rebind hosts[0].up @247..266\n\
         \x20   (at\n\
         \x20     (int 0 @253..254))\n\
         \x20   (bool true @262..266))\n",
        "\n        (rebind hosts[i].ports[0] @364..406\n\
         \x20         (at\n\
         \x20           (name i @370..371))\n\
         \x20         (at\n\
         \x20           (int 0 @379..380))\n",
        "\n              (rebind grid[y][x] @497..520\n\
         \x20               (at\n\
         \x20                 (name y @502..503))\n\
         \x20               (at\n\
         \x20                 (name x @505..506))\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/update.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "\n  (rebind counts[1] @628..642\n\
         \x20   (at\n\
         \x20     (int 1 : Int @635..636))\n",
        // The list another variable holds keeps its type, and so does the
        // one updated.
        "(name before : List<Int> @884..890)",
        "(name counts : List<Int> @866..872)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/update.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // One update for the whole path: its value, its list and its
        // positions, and its steps.
        "\n  (store hosts#119 @247..266\n\
         \x20   (update [].up @247..258\n\
         \x20     (bool true @262..266)\n\
         \x20     (load hosts#119 @247..252)\n\
         \x20     (int 0 @253..254)))\n",
        "\n      (store hosts#119 @364..406\n\
         \x20       (update [].ports[] @364..381\n\
         \x20         (int-add @385..406\n",
        "\n            (update [][] @497..507\n",
        // The value, which rebinds `r`, comes before the read of `r`.
        "\n  (store r#123 @735..778\n\
         \x20   (update .a @735..738\n\
         \x20     (block @742..778\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn update_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/update_range_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: index out of range\n\
         \x20--> tests/fixtures/update_range_error.gcsl:2:1\n\
         \x20 |\n\
         2 | ports[2] := 8080\n\
         \x20 | ^^^^^^^^ the index is 2, and the list has 2 elements\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl(&["run", "tests/fixtures/update_list_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot update `replicas[0]`: `replicas` is Int, not a list\n\
         \x20--> tests/fixtures/update_list_error.gcsl:2:1\n\
         \x20 |\n\
         2 | replicas[0] := 4\n\
         \x20 | --------^^^\n\
         \x20 | |\n\
         \x20 | this is Int\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn repl_rebinds_mutable_variables_across_lines() {
    let output = repl(
        "let mut n = 0\n\
         for i in 0..4 { n := n + i }\n\
         n\n\
         let f = fn(): Int { n := 1; n }\n\
         { n := n * 2 }\n\
         n\n",
    );
    // The `for` is its input's result, a list of its body's `{}`s.
    assert_eq!(
        stdout(&output),
        "[{}, {}, {}, {}] : List<{}>\n6 : Int\n12 : Int\n"
    );
    assert!(
        stderr(&output)
            .starts_with("error: cannot rebind `n` here: it is declared outside this function\n"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_spreads_defaults_into_a_configuration() {
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{host = \"localhost\"; port = 8080; url = \"localhost:8080\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_spreads() {
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    assert!(
        tokens.contains("\nDotDotDot @176..179\nIdent(defaults) @179..187\n"),
        "{tokens}"
    );
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    let node = "\n    (block @172..227\n\
                \x20     (spread @176..187\n\
                \x20       (name defaults @179..187))\n\
                \x20     (field port @190..201\n";
    assert!(ast.contains(node), "{node}: {ast}");
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "\n    (block : {host: String, port: Int, url: String} @172..227\n\
         \x20     (spread @176..187\n\
         \x20       (name defaults : {host: String, port: Int} @179..187))\n",
        // The names the spread defines, one of them overridden.
        "(name host : String @212..216)",
        "(name port : Int @219..223)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    // The operand is stored once, in a slot of its own, and each field is
    // read from it by name into the field's slot; the override stores into
    // the same slot, and the field keeps its place.
    let node = "\n      (store url#122 @176..187\n\
                \x20       (load defaults#119 @179..187))\n\
                \x20     (store conf#120 @176..187\n\
                \x20       (field host @179..187\n\
                \x20         (load url#122 @179..187)))\n\
                \x20     (store port#121 @176..187\n\
                \x20       (field port @179..187\n\
                \x20         (load url#122 @179..187)))\n\
                \x20     (store port#121 @190..201\n\
                \x20       (int 8080 @197..201))\n";
    assert!(ir.contains(node), "{node}: {ir}");
    let node = "\n      (make-record [host port url] @172..227\n\
                \x20       (load conf#120 @179..187)\n\
                \x20       (load port#121 @190..194)\n\
                \x20       (load url#122 @204..207))))\n";
    assert!(ir.contains(node), "{node}: {ir}");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_spread_of_a_field_defined_before_it_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/spread_twice.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: field `port` is defined twice\n\
         \x20--> tests/fixtures/spread_twice.gcsl:4:6\n\
         \x20 |\n\
         3 |   port = 8080\n\
         \x20 |   ---- first defined here\n\
         4 |   ...base\n\
         \x20 |      ^^^^ `port` is a field of this, of type Int\n\
         \x20 |\n\
         \x20 = note: a spread cannot replace a field defined before it; put the spread first\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn repl_shows_lists_and_their_types() {
    let output = repl(
        "let xs = [1, lazy 2]\n\
         xs\n\
         \"{xs}\"\n\
         xs[1] + len(xs)\n\
         [...xs, 3][0]\n",
    );
    assert_eq!(
        stdout(&output),
        "[1, Future<Int>] : List<Int | Future<Int>>\n\
         \"[1, 2]\" : String\n\
         4 : Int\n\
         1 : Int | Future<Int>\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_shows_futures_as_their_types() {
    let output = repl(
        "let x = lazy 20 + 1\n\
         x\n\
         x * 2\n\
         await x\n\
         { f = x }\n",
    );
    assert_eq!(
        stdout(&output),
        "Future<Int> : Future<Int>\n\
         42 : Int\n\
         21 : Int\n\
         {f = Future<Int>} : {f: Future<Int>}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}
