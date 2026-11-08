//! A directory's files, listed from a tree deeper than this process may open
//! descriptors. Its own test binary, since it lowers the process's limit.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use gcsl::SourceFile;
use nix::sys::resource::{Resource, getrlimit, setrlimit};

/// How many directories deep the tree goes: well past `LIMIT`.
const DEPTH: usize = 400;

/// The open-files limit the listing runs under.
const LIMIT: u64 = 64;

#[test]
fn a_tree_deeper_than_the_descriptor_limit_is_listed() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("gcsl-deep-tree-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let mut expected = Vec::new();
    let mut dir = root.clone();
    let mut under = String::new();
    for level in 0..DEPTH {
        dir.push("level");
        under.push_str("level/");
        fs::create_dir_all(&dir).unwrap();
        if level % 100 == 0 {
            fs::write(dir.join("here.txt"), "").unwrap();
            expected.push(format!("{under}here.txt"));
            // A link to a directory is a file of the tree, not followed.
            symlink("..", dir.join("up")).unwrap();
            expected.push(format!("{under}up"));
        }
    }
    fs::write(dir.join("deepest.txt"), "").unwrap();
    expected.push(format!("{under}deepest.txt"));
    expected.sort_unstable();

    let root_path = root.to_str().unwrap();
    let text = format!(
        "= for f in dir(\"{root_path}\").files() {{ strip_prefix(f.path, \"{root_path}/\") }}"
    );
    let (soft, hard) = getrlimit(Resource::RLIMIT_NOFILE).unwrap();
    setrlimit(Resource::RLIMIT_NOFILE, LIMIT, hard).unwrap();
    let listed = gcsl::run(&SourceFile::new("deep.gcsl", &text).unwrap());
    // std's `remove_dir_all` holds a descriptor per level itself.
    setrlimit(Resource::RLIMIT_NOFILE, soft, hard).unwrap();
    fs::remove_dir_all(&root).unwrap();

    let listed = listed.unwrap_or_else(|errors| panic!("{errors:?}")).value;
    let expected: Vec<String> = expected.iter().map(|path| format!("\"{path}\"")).collect();
    assert_eq!(
        listed.repr().to_string(),
        format!("[{}]", expected.join(", "))
    );
}
