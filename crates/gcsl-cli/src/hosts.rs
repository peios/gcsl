//! `gcsl hosts [--json]`: list the hosts whose files GCSL's tools read, for
//! editor integrations and for a person writing a file association.
//!
//! A host is named by the extension of its files and found by its
//! directory (see `gcsl-disk`'s "Hosts", [`gcsl_disk::find_hosts`]): in
//! `.gcsl/hosts/` under the root of the workspace the working directory is
//! in, if it is in one, then in `/usr/share/gcsl/hosts/`; pekit's is built
//! in, and a known host that is installed nowhere is listed as not
//! installed. Each is one line on standard output, its extension, where it
//! was found, the libraries its prelude imports or `no prelude`, and its
//! directory if it has one, in columns:
//!
//! ```text
//! pekit  built in       no prelude
//! prvm   workspace      provium     .gcsl/hosts/prvm
//! ```
//!
//! With `--json`, the hosts are one JSON list on standard output, each
//! `{"extension", "place", "prelude", "path"}`, `place` one of
//! `"workspace"`, `"system"`, `"built in"` and `"not installed"`, `prelude`
//! the list of its prelude's imports, empty if it has none, and `path`
//! `null` where it has none. The exit code is 0, or 2 if the
//! output cannot be written.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

use clap::Args;
use gcsl_disk::Host;
use serde_json::{Value, json};

use crate::{EXIT_USAGE_OR_IO, output};

/// Arguments of `gcsl hosts`.
#[derive(Args)]
pub struct HostsArgs {
    /// Print the hosts as one JSON list.
    #[arg(long)]
    json: bool,
}

/// Runs `gcsl hosts`.
pub fn main(args: &HostsArgs) -> ExitCode {
    let cwd = std::env::current_dir().and_then(fs::canonicalize).ok();
    let root = cwd.as_deref().and_then(gcsl_disk::find_root);
    let hosts = gcsl_disk::find_hosts(root);
    let text = if args.json {
        let listed: Vec<Value> = hosts.iter().map(json_of).collect();
        format!("{}\n", Value::Array(listed))
    } else {
        table(&hosts)
    };
    match io::stdout().lock().write_all(text.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::error(format_args!("cannot write the hosts: {error}"));
            ExitCode::from(EXIT_USAGE_OR_IO)
        }
    }
}

/// `host` as `--json` lists it.
fn json_of(host: &Host) -> Value {
    json!({
        "extension": host.extension,
        "place": host.place.shown(),
        "prelude": host.prelude,
        "path": gcsl_disk::shown_dir(host).map(|path| path.display().to_string()),
    })
}

/// `hosts` as lines of columns, each column as wide as its widest cell.
fn table(hosts: &[Host]) -> String {
    let rows: Vec<[String; 4]> = hosts
        .iter()
        .map(|host| {
            [
                host.extension.clone(),
                host.place.shown().to_owned(),
                match host.libraries().join(", ") {
                    libraries if libraries.is_empty() => "no prelude".to_owned(),
                    libraries => libraries,
                },
                gcsl_disk::shown_dir(host)
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
            ]
        })
        .collect();
    let width = |column: usize| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2)];
    let mut text = String::new();
    for row in &rows {
        let mut line = String::new();
        for (cell, width) in row.iter().zip(widths) {
            let _ = write!(line, "{cell:width$}  ");
        }
        line.push_str(&row[3]);
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}
