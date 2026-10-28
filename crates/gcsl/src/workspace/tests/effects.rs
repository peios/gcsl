//! Tests of workspaces: the effect context, through which every effect of
//! a run goes (see [Effects](crate::eval::effects#effects)).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;
use crate::eval::{Effects, eval_with};
use crate::workspace::tests::fs::fs_scratch;

/// A host that roots every path in a directory of its own, as a sandbox
/// would, gives `/work` as the working directory, records the programs it
/// is asked to run, and hides every directory named `hidden`.
#[derive(Debug)]
struct Rooted {
    /// Where `/` is.
    root: PathBuf,
    /// Each program run, its words joined by spaces.
    ran: RefCell<Vec<String>>,
    /// The number of the next temporary directory.
    temps: Cell<u32>,
}

impl Effects for Rooted {
    fn command(&self, command: Command) -> Result<Command, String> {
        let words: Vec<String> = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|word| word.to_string_lossy().into_owned())
            .collect();
        self.ran.borrow_mut().push(words.join(" "));
        Ok(command)
    }

    fn host_path(&self, path: &str) -> Result<PathBuf, String> {
        if path.starts_with("/outside") {
            return Err("it is outside the step".to_owned());
        }
        Ok(self.root.join(path.trim_start_matches('/')))
    }

    fn hides(&self, host: &Path) -> bool {
        host.file_name().is_some_and(|name| name == "hidden")
    }

    fn working_dir(&self) -> Result<PathBuf, String> {
        Ok(PathBuf::from("/work"))
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        let number = self.temps.get();
        self.temps.set(number + 1);
        let path = format!("/tmp/{number}");
        Some(
            std::fs::create_dir_all(self.root.join(path.trim_start_matches('/')))
                .map(|()| path)
                .map_err(|error| error.to_string()),
        )
    }
}

/// Runs `text`, a lone file, through `effects`, once the standard files
/// have run in its workspace.
pub(super) fn run_through(text: &str, effects: &dyn Effects) -> Result<Value, Diagnostic> {
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let (loaded, checked) = check_lone_with_standard(&source, "/");
    let mut workspace = checked.unwrap();
    for file in 0..count(loaded.len()) {
        workspace.run(file).unwrap();
    }
    let FileRun::Pending(ir) = mem::replace(&mut workspace.runs[loaded.len()], FileRun::Succeeded)
    else {
        panic!("expected pending IR for the lone file");
    };
    let ir = Rc::new(*ir);
    let size = ir.frame_size() as usize;
    workspace
        .frame
        .resize(size.max(workspace.frame.len()), Value::Empty);
    let context = (
        Some(&workspace.exec_result),
        &workspace.scan.dir_texts[..],
        effects,
    );
    eval_with(&ir, &mut workspace.frame, &mut workspace.memo, context)
}

#[test]
fn every_effect_goes_through_the_host_s_context() {
    let root = PathBuf::from(fs_scratch("effects-rooted"));
    let effects = Rooted {
        root: root.clone(),
        ran: RefCell::new(Vec::new()),
        temps: Cell::new(0),
    };
    let text = "import fs\n\
                fs.mkdir(\"/d\")\n\
                fs.write(\"/d/x.txt\", \"hi\")\n\
                let composed = fs.compose_dir({\"out/x.txt\" = file(\"/d/x.txt\")})\n\
                = [\n\
                  fs.read(\"/d/x.txt\"),\n\
                  \"{fs.exists(\"/d/x.txt\")}\",\n\
                  file(\"rel\").path,\n\
                  working_dir().path,\n\
                  \"{exec([\"true\"]).status}\",\n\
                  dir(\"/d\").files()[0].path,\n\
                  composed.path,\n\
                  file(\"{composed.path}/out/x.txt\").read(),\n\
                  fs.temp_dir(),\n\
                ]";
    let value = run_through(text, &effects).unwrap();
    // GCSL's values hold paths as its code names them: a relative one in
    // its file's directory, the root, and the working directory the
    // host's.
    assert_eq!(
        value.to_string(),
        "[\"hi\", \"true\", \"/rel\", \"/work\", \"0\", \"/d/x.txt\", \"/tmp/0\", \"hi\", \"/tmp/1\"]"
    );
    // What it did to the file system is under the host's root.
    assert_eq!(std::fs::read_to_string(root.join("d/x.txt")).unwrap(), "hi");
    assert!(root.join("tmp/0/out/x.txt").is_file());
    // Its command went through the host.
    assert_eq!(*effects.ran.borrow(), ["true"]);
    // A path the host gives no place is an error at the call.
    let error = run_through("import fs\n= fs.read(\"/outside/x\")", &effects).unwrap_err();
    assert_eq!(
        error.message,
        "`fs.read` cannot use `/outside/x`: it is outside the step"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_directory_the_host_hides_is_neither_listed_nor_copied() {
    let root = PathBuf::from(fs_scratch("effects-hidden"));
    let effects = Rooted {
        root: root.clone(),
        ran: RefCell::new(Vec::new()),
        temps: Cell::new(0),
    };
    let text = "import fs\n\
                fs.mkdir(\"/d/hidden/deep\")\n\
                fs.mkdir(\"/d/sub\")\n\
                fs.write(\"/d/hidden/deep/x\", \"h\")\n\
                fs.write(\"/d/sub/kept\", \"k\")\n\
                let composed = fs.compose_dir({\"c\" = dir(\"/d\")})\n\
                let listed = dir(\"/d\").files()\n\
                = [\"{len(listed)}\", listed[0].path, composed.path]";
    let value = run_through(text, &effects).unwrap();
    assert_eq!(value.to_string(), "[\"1\", \"/d/sub/kept\", \"/tmp/0\"]");
    // The copy has what is not hidden, and nothing of what is.
    assert!(root.join("tmp/0/c/sub/kept").is_file());
    assert!(!root.join("tmp/0/c/hidden").exists());
    // It is still there, to be read by its path.
    assert!(root.join("d/hidden/deep/x").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

/// A host as [`Rooted`] is, which links files into compositions, is told
/// of each finished one, and makes sandboxes itself, recording each.
#[derive(Debug)]
struct Hooked {
    /// Where paths are rooted, and how programs run.
    rooted: Rooted,
    /// The compositions it was told of.
    composed: RefCell<Vec<String>>,
    /// The sandboxes it made, each its directory and its copy's root.
    sandboxes: RefCell<Vec<(String, String)>>,
}

impl Effects for Hooked {
    fn command(&self, command: Command) -> Result<Command, String> {
        self.rooted.command(command)
    }

    fn place_file(
        &self,
        source: &std::path::Path,
        target: &std::path::Path,
    ) -> std::io::Result<()> {
        std::fs::hard_link(source, target)
    }

    fn composed(&self, root: &str) {
        self.composed.borrow_mut().push(root.to_owned());
    }

    fn make_sandbox(&self, source: &str, root: &str) -> Option<Result<(), String>> {
        self.sandboxes
            .borrow_mut()
            .push((source.to_owned(), root.to_owned()));
        Some(Ok(()))
    }

    fn host_path(&self, path: &str) -> Result<PathBuf, String> {
        self.rooted.host_path(path)
    }

    fn working_dir(&self) -> Result<PathBuf, String> {
        self.rooted.working_dir()
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        self.rooted.temp_dir()
    }
}

#[test]
fn a_host_can_place_compositions_files_and_make_sandboxes_itself() {
    use std::os::unix::fs::MetadataExt;
    let root = PathBuf::from(fs_scratch("effects-hooked"));
    let effects = Hooked {
        rooted: Rooted {
            root: root.clone(),
            ran: RefCell::new(Vec::new()),
            temps: Cell::new(0),
        },
        composed: RefCell::new(Vec::new()),
        sandboxes: RefCell::new(Vec::new()),
    };
    let text = "import fs\n\
                fs.mkdir(\"/d\")\n\
                fs.write(\"/d/x.txt\", \"hi\")\n\
                let composed = fs.compose_dir({\"out/x.txt\" = file(\"/d/x.txt\")})\n\
                let s = composed.sandbox(cwd = \"/work\")\n\
                = [composed.path, \"{fs.exists(\"/tmp/1/out/x.txt\")}\"]";
    let value = run_through(text, &effects).unwrap();
    // The composition's file is the host's link, and the host was told of
    // it once it was done.
    let source = std::fs::metadata(root.join("d/x.txt")).unwrap();
    let placed = std::fs::metadata(root.join("tmp/0/out/x.txt")).unwrap();
    assert_eq!(source.ino(), placed.ino());
    assert_eq!(*effects.composed.borrow(), ["/tmp/0"]);
    // The host made the sandbox: GCSL copied nothing into it, but made its
    // working directory.
    assert_eq!(
        *effects.sandboxes.borrow(),
        [("/tmp/0".to_owned(), "/tmp/1".to_owned())]
    );
    assert_eq!(value.to_string(), "[\"/tmp/0\", \"false\"]");
    assert!(root.join("tmp/1/work").is_dir());
}

/// A host as [`Rooted`] is, which places whole each directory named `big`
/// it is offered, and no other, recording each it is offered, and finds
/// what each holds where it was; and which will not have anything under
/// `/d/big` changed.
#[derive(Debug)]
struct Placing {
    /// Where paths are rooted, and how programs run.
    rooted: Rooted,
    /// Each directory offered whole, its path and where it was to be placed,
    /// both relative to the root, and whether it was taken.
    offered: RefCell<Vec<(String, String, bool)>>,
    /// Each directory placed whole, where it is and what it holds, on the
    /// host.
    placed: RefCell<Vec<(PathBuf, PathBuf)>>,
}

impl Placing {
    /// `host`, a path under the root, relative to it.
    fn shown(&self, host: &Path) -> String {
        host.strip_prefix(&self.rooted.root).map_or_else(
            |_| host.display().to_string(),
            |path| path.display().to_string(),
        )
    }
}

impl Effects for Placing {
    fn command(&self, command: Command) -> Result<Command, String> {
        self.rooted.command(command)
    }

    fn place_tree(&self, source: &Path, target: &Path) -> Option<std::io::Result<()>> {
        let taken = source.file_name().is_some_and(|name| name == "big");
        self.offered
            .borrow_mut()
            .push((self.shown(source), self.shown(target), taken));
        if !taken {
            return None;
        }
        self.placed
            .borrow_mut()
            .push((target.to_path_buf(), source.to_path_buf()));
        Some(std::fs::create_dir(target))
    }

    fn contents(&self, host: &Path) -> Option<PathBuf> {
        self.placed
            .borrow()
            .iter()
            .find(|(at, _)| at == host)
            .map(|(_, source)| source.clone())
    }

    fn host_path(&self, path: &str) -> Result<PathBuf, String> {
        self.rooted.host_path(path)
    }

    fn write_path(&self, path: &str) -> Result<PathBuf, String> {
        if path.starts_with("/d/big") {
            return Err("it is the step's input".to_owned());
        }
        self.rooted.host_path(path)
    }

    fn working_dir(&self) -> Result<PathBuf, String> {
        self.rooted.working_dir()
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        self.rooted.temp_dir()
    }
}

/// A [`Placing`] host rooted in a new scratch directory named `name`, of
/// `/d/big/a`, `/d/big/sub/b` and `/d/small/c`.
fn placing(name: &str) -> Placing {
    let root = PathBuf::from(fs_scratch(name));
    for (path, contents) in [("d/big/a", "a"), ("d/big/sub/b", "b"), ("d/small/c", "c")] {
        let at = root.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, contents).unwrap();
    }
    Placing {
        rooted: Rooted {
            root,
            ran: RefCell::new(Vec::new()),
            temps: Cell::new(0),
        },
        offered: RefCell::new(Vec::new()),
        placed: RefCell::new(Vec::new()),
    }
}

#[test]
fn a_host_can_place_a_composition_s_directories_whole() {
    let effects = placing("effects-placing");
    let text = "import fs\n\
                let composed = fs.compose_dir({\"t\" = dir(\"/d/big\"); \"s\" = dir(\"/d/small\"); \
                \"u/x\" = file(\"/d/small/c\")})\n\
                let again = fs.compose_dir({\"in\" = composed})\n\
                = for each in [...composed.files(), ...again.files()] { = each.path }";
    let value = run_through(text, &effects).unwrap();
    // Each directory is offered once nothing else can be placed in it, and
    // what one the host took holds is found where it was.
    assert_eq!(
        value.to_string(),
        "[\"/tmp/0/s/c\", \"/tmp/0/t/a\", \"/tmp/0/t/sub/b\", \"/tmp/0/u/x\", \
         \"/tmp/1/in/s/c\", \"/tmp/1/in/t/a\", \"/tmp/1/in/t/sub/b\", \"/tmp/1/in/u/x\"]"
    );
    let offered =
        |source: &str, target: &str, taken: bool| (source.to_owned(), target.to_owned(), taken);
    assert_eq!(
        *effects.offered.borrow(),
        [
            offered("d/big", "tmp/0/t", true),
            offered("d/small", "tmp/0/s", false),
            offered("tmp/0", "tmp/1/in", false),
            offered("tmp/0/s", "tmp/1/in/s", false),
            offered("d/big", "tmp/1/in/t", true),
            offered("tmp/0/u", "tmp/1/in/u", false),
        ]
    );
    let root = &effects.rooted.root;
    assert!(root.join("tmp/1/in/s/c").is_file());
    assert!(!root.join("tmp/1/in/t/a").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_directory_something_is_placed_in_is_placed_entry_by_entry() {
    let effects = placing("effects-placing-in");
    let text = "import fs\n\
                let composed = fs.compose([{at = \"t\"; place = dir(\"/d/big\")}, \
                {at = \"t/x\"; place = file(\"/d/small/c\")}, \
                {at = \"v\"; place = dir(\"/d/big\")}, {at = \"v\"; place = dir(\"/d/small\")}])\n\
                = for each in composed.files() { = each.path }";
    let value = run_through(text, &effects).unwrap();
    assert_eq!(
        value.to_string(),
        "[\"/tmp/0/t/a\", \"/tmp/0/t/sub/b\", \"/tmp/0/t/x\", \"/tmp/0/v/a\", \
         \"/tmp/0/v/c\", \"/tmp/0/v/sub/b\"]"
    );
    let offered: Vec<String> = effects
        .offered
        .borrow()
        .iter()
        .map(|(source, target, _)| format!("{source} {target}"))
        .collect();
    assert_eq!(offered, ["d/big/sub tmp/0/t/sub", "d/big/sub tmp/0/v/sub"]);
    // A file placed where a placed directory has one is placed twice.
    let text = "import fs\n\
                = fs.compose([{at = \"t\"; place = dir(\"/d/big\")}, \
                {at = \"t/a\"; place = file(\"/d/small/c\")}])";
    let error = run_through(text, &effects).unwrap_err();
    assert!(
        error.message.contains("two entries place `t/a`"),
        "{}",
        error.message
    );
    let _ = std::fs::remove_dir_all(&effects.rooted.root);
}

#[test]
fn a_host_can_refuse_to_have_a_path_changed() {
    let effects = placing("effects-placing-write");
    for call in [
        "fs.write(\"/d/big/z\", \"z\")",
        "fs.mkdir(\"/d/big/z\")",
        "fs.copy(\"/d/small/c\", \"/d/big/z\")",
        "fs.set_executable(\"/d/big/a\", true)",
    ] {
        let error = run_through(&format!("import fs\n{call}"), &effects).unwrap_err();
        assert!(
            error.message.contains("it is the step's input"),
            "{}",
            error.message
        );
    }
    assert!(!effects.rooted.root.join("d/big/z").exists());
    let _ = std::fs::remove_dir_all(&effects.rooted.root);
}
