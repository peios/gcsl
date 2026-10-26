//! Tests of workspaces: the effect context, through which every effect of
//! a run goes (see [Effects](crate::eval::effects#effects)).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::Command;

use super::*;
use crate::eval::{Effects, eval_with};
use crate::workspace::tests::fs::fs_scratch;

/// A host that roots every path in a directory of its own, as a sandbox
/// would, gives `/work` as the working directory, and records the
/// programs it is asked to run.
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
                  \"{exec([\"true\"]).status}\",\n\
                  dir(\"/d\").files()[0].path,\n\
                  composed.path,\n\
                  file(\"{composed.path}/out/x.txt\").read(),\n\
                  fs.temp_dir(),\n\
                ]";
    let value = run_through(text, &effects).unwrap();
    // GCSL's values hold paths as its code names them.
    assert_eq!(
        value.to_string(),
        "[\"hi\", \"true\", \"/work/rel\", \"0\", \"/d/x.txt\", \"/tmp/0\", \"hi\", \"/tmp/1\"]"
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
