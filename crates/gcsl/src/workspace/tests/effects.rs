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
fn run_through(text: &str, effects: &dyn Effects) -> Result<Value, Diagnostic> {
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
