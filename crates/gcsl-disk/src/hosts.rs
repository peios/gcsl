//! Hosts: the programs whose files are GCSL with a prelude of their own,
//! found by the extension of their files.
//!
//! A *host* is named by the extension of its files, `prvm` for Provium's
//! `.prvm` tests. Its *host directory*, named the same, is a library root
//! (see the crate's "Libraries") that holds the libraries of its prelude,
//! and a file `prelude` ([`PRELUDE_FILE`]) whose one line names the
//! library that is its files' prelude, `provium`, which the library gives
//! each of them ([`SourceFile::with_prelude`]):
//!
//! ```text
//! .gcsl/hosts/prvm/
//!   prelude              provium
//!   provium/core.gcsl
//! ```
//!
//! A host with no `prelude` file gives its files no prelude: they are
//! checked as `.gcsl` files are. Hosts are found ([`find_hosts`]) in two
//! places, by the name of each directory there: `.gcsl/hosts/` under the
//! workspace's root ([`WORKSPACE_HOSTS`]), which the host writes when it
//! loads a workspace, and then `/usr/share/gcsl/hosts/` ([`SYSTEM_HOSTS`]),
//! which a host's package installs; the first found of an extension stands.
//! Any directory there is a host, whoever wrote it.
//!
//! Two hosts are known by name ([`KNOWN_HOSTS`]), so that their files are
//! recognised when the host is not installed: `pekit`, built in until its
//! prelude leaves GCSL's, with none, and `prvm`, Provium's, whose prelude
//! is `provium`. A file of a known host that is not installed is read and
//! given its prelude still, and the library's error that it is not found is
//! said as the host's ([`explain_missing_hosts`]): "Provium is not
//! installed, so the prelude of this `.prvm` file cannot be found". A file
//! of any other extension is no GCSL file, and is not read.

use std::fs;
use std::path::{Path, PathBuf};

use gcsl::{Diagnostic, SourceFile};

/// The directory, under a workspace's root, of the hosts a host program
/// wrote there.
pub const WORKSPACE_HOSTS: &str = ".gcsl/hosts";

/// The directory of the hosts a host's package installs.
pub const SYSTEM_HOSTS: &str = "/usr/share/gcsl/hosts";

/// The file of a host directory that names its prelude's library.
pub const PRELUDE_FILE: &str = "prelude";

/// A host known by name, so that its files are recognised when it is not
/// installed (see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownHost {
    /// The extension of its files, without the dot: `prvm`.
    pub extension: &'static str,
    /// Its name, as a person knows it: `Provium`.
    pub name: &'static str,
    /// The library that is its files' prelude, if they have one.
    pub prelude: Option<&'static str>,
    /// Whether GCSL's tools carry it, so that it is never missing.
    pub built_in: bool,
    /// How to install it, for the error of a file whose host is missing.
    pub install: &'static str,
}

/// The hosts known by name (see the module documentation).
pub const KNOWN_HOSTS: [KnownHost; 2] = [
    KnownHost {
        extension: "pekit",
        name: "pekit",
        prelude: None,
        built_in: true,
        install: "",
    },
    KnownHost {
        extension: "prvm",
        name: "Provium",
        prelude: Some("provium"),
        built_in: false,
        install: "run `provium prelude --write` in this workspace, or install Provium",
    },
];

/// Where a host was found ([`Host::place`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// In the workspace's `.gcsl/hosts/` ([`WORKSPACE_HOSTS`]).
    Workspace,
    /// In `/usr/share/gcsl/hosts/` ([`SYSTEM_HOSTS`]).
    System,
    /// Carried by GCSL's tools ([`KnownHost::built_in`]).
    BuiltIn,
    /// Nowhere: a known host that is not installed.
    Missing,
}

impl Place {
    /// How the place is shown by `gcsl hosts`: `workspace`, `system`,
    /// `built in` or `not installed`.
    #[must_use]
    pub const fn shown(self) -> &'static str {
        match self {
            Place::Workspace => "workspace",
            Place::System => "system",
            Place::BuiltIn => "built in",
            Place::Missing => "not installed",
        }
    }
}

/// A host, found or known ([`find_hosts`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// The extension of its files, without the dot: `prvm`.
    pub extension: String,
    /// Where it was found.
    pub place: Place,
    /// Its directory, a library root, if it has one on disk.
    pub dir: Option<PathBuf>,
    /// The library that is its files' prelude, if they have one: what its
    /// `prelude` file names, or a known host's.
    pub prelude: Option<String>,
}

impl Host {
    /// The host known by name of the extension `extension`, if there is
    /// one, as it is when it is not installed, or built in.
    #[must_use]
    pub fn known(extension: &str) -> Option<Host> {
        let known = KNOWN_HOSTS
            .iter()
            .find(|known| known.extension == extension)?;
        Some(Host {
            extension: known.extension.to_owned(),
            place: if known.built_in {
                Place::BuiltIn
            } else {
                Place::Missing
            },
            dir: None,
            prelude: known.prelude.map(str::to_owned),
        })
    }
}

/// The hosts of the workspace under `root`, or of no workspace for `None`:
/// each found in its `.gcsl/hosts/`, then each in `/usr/share/gcsl/hosts/`
/// of an extension not found there, then each known host found in
/// neither, built in or missing; sorted by extension (see the module
/// documentation). A directory that cannot be listed has no hosts.
#[must_use]
pub fn find_hosts(root: Option<&Path>) -> Vec<Host> {
    let workspace = root.map(|root| root.join(WORKSPACE_HOSTS));
    find_hosts_in(workspace.as_deref(), Path::new(SYSTEM_HOSTS))
}

/// [`find_hosts`], with the workspace's hosts in `workspace`, if it has
/// any, and the system's in `system`.
#[must_use]
pub fn find_hosts_in(workspace: Option<&Path>, system: &Path) -> Vec<Host> {
    let mut hosts: Vec<Host> = Vec::new();
    let places = workspace
        .map(|dir| (dir, Place::Workspace))
        .into_iter()
        .chain([(system, Place::System)]);
    for (dir, place) in places {
        for host in hosts_in(dir, place) {
            if !hosts.iter().any(|found| found.extension == host.extension) {
                hosts.push(host);
            }
        }
    }
    for known in KNOWN_HOSTS {
        if !hosts.iter().any(|found| found.extension == known.extension) {
            hosts.extend(Host::known(known.extension));
        }
    }
    hosts.sort_by(|a, b| a.extension.cmp(&b.extension));
    hosts
}

/// The hosts whose directories are in `dir`, found at `place`, in no
/// order: each directory whose name is an extension, `gcsl` none, and whose
/// name does not start with `.`.
fn hosts_in(dir: &Path, place: Place) -> Vec<Host> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_str()?.to_owned();
            let usable =
                !name.starts_with('.') && name != "gcsl" && !name.contains('.') && path.is_dir();
            if !usable {
                return None;
            }
            let prelude = fs::read_to_string(path.join(PRELUDE_FILE))
                .ok()
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty());
            Some(Host {
                extension: name,
                place,
                dir: Some(path),
                prelude,
            })
        })
        .collect()
}

/// The host of `hosts` whose files have the extension of `path`, if one
/// does.
#[must_use]
pub fn host_of<'a>(hosts: &'a [Host], path: &Path) -> Option<&'a Host> {
    let extension = path.extension()?.to_str()?;
    hosts.iter().find(|host| host.extension == extension)
}

/// `source`, given the prelude of its host among `hosts`, if it is a host's
/// file and the host has one.
#[must_use]
pub fn with_host_prelude(hosts: &[Host], path: &Path, source: SourceFile) -> SourceFile {
    match host_of(hosts, path).and_then(|host| host.prelude.as_deref()) {
        Some(library) => source.with_prelude(library),
        None => source,
    }
}

/// Says each error of `errors` that a file's prelude is not found, which
/// the library reports for a file whose host is missing ([`Place::Missing`])
/// among `hosts`, as the host's: "Provium is not installed, so the prelude
/// of this `.prvm` file cannot be found", noted with how to install it (see
/// the module documentation).
pub fn explain_missing_hosts(hosts: &[Host], errors: &mut [Diagnostic]) {
    for host in hosts.iter().filter(|host| host.place == Place::Missing) {
        let Some(library) = host.prelude.as_deref() else {
            continue;
        };
        let message = gcsl::workspace::prelude_not_given_message(library);
        let known = KNOWN_HOSTS
            .iter()
            .find(|known| known.extension == host.extension);
        let (name, install) = known.map_or((host.extension.as_str(), ""), |known| {
            (known.name, known.install)
        });
        for error in errors.iter_mut().filter(|error| error.message == message) {
            error.message = format!(
                "{name} is not installed, so the prelude of this `.{}` file cannot be found",
                host.extension
            );
            if !install.is_empty() {
                error.notes.push(install.to_owned());
            }
        }
    }
}

/// The library roots the directories of `hosts` give: each host's that has
/// one, in the order of `hosts`.
pub(crate) fn host_roots(hosts: &[Host]) -> Vec<PathBuf> {
    hosts.iter().filter_map(|host| host.dir.clone()).collect()
}

/// The paths of `hosts`, as the tools say where a host's files are looked
/// for, for a person: `.gcsl/hosts/prvm`.
#[must_use]
pub fn shown_dir(host: &Host) -> Option<PathBuf> {
    host.dir.as_deref().map(crate::shown)
}
