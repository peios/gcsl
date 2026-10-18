//! Where a document is: its path, from its URI, and the workspace it is a
//! file of, if any.
//!
//! LSP names documents by URI. A `file:` URI names a path on disk
//! ([`path_of`]), percent-decoded; a path is named by the URI
//! [`uri_of`] makes, percent-encoding every byte but the unreserved ones
//! and `/`. A document the client opened keeps the URI the client gave it,
//! since clients match diagnostics to documents by it; [`uri_of`] names
//! only the files that are not open.
//!
//! A `.gcsl` file is a file of the workspace whose root
//! [`gcsl_disk::find_workspace`] finds above it ([`locate`]). That needs the
//! file on disk, to canonicalise it; a file that is not on disk yet, a new
//! buffer not written, is placed from its directory's canonical path, by
//! the same rules: under the nearest directory holding a `workspace.gcsl`,
//! no name on its way starting with `.`, and ending in `.gcsl`. Any other
//! document, a file in no workspace or a URI that is no file, is checked
//! alone.

use std::ffi::OsStr;
use std::fmt::Write;
use std::fs;
use std::path::{Component, Path, PathBuf};

use lsp_types::Uri;

/// The bytes a path keeps as they are in a URI: the unreserved characters
/// of RFC 3986, and `/`. Every other byte is percent-encoded.
fn kept(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// The path a `file:` URI names, percent-decoded: `file:///a/b%20c.gcsl` is
/// `/a/b c.gcsl`, `file://localhost/a` is `/a`. `None` for a URI of
/// another scheme, a remote authority, or whose decoded path is relative
/// or not UTF-8. Only empty and `localhost` authorities name local files.
#[must_use]
pub fn path_of(uri: &Uri) -> Option<PathBuf> {
    let text = uri.as_str();
    let scheme = text.get(..5)?;
    if !scheme.eq_ignore_ascii_case("file:") {
        return None;
    }
    let rest = text.get(5..)?;
    // An authority, empty or `localhost`, before the path.
    let path = match rest.strip_prefix("//") {
        Some(rest) => {
            let (authority, path) = rest.split_at(rest.find('/')?);
            if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
                return None;
            }
            path
        }
        None => rest,
    };
    let path = path.split(['?', '#']).next()?;
    let mut bytes = Vec::with_capacity(path.len());
    let mut iter = path.bytes();
    while let Some(byte) = iter.next() {
        if byte == b'%' {
            let high = iter.next().and_then(hex)?;
            let low = iter.next().and_then(hex)?;
            bytes.push((high << 4) | low);
        } else {
            bytes.push(byte);
        }
    }
    let path = PathBuf::from(String::from_utf8(bytes).ok()?);
    path.is_absolute().then_some(path)
}

/// The value of the hexadecimal digit `digit`.
fn hex(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// The `file:` URI of `path`, an absolute path: `file://` and the path,
/// each byte but the unreserved ones and `/` percent-encoded. `None` for a
/// path that is not UTF-8.
#[must_use]
pub fn uri_of(path: &Path) -> Option<Uri> {
    let mut text = String::from("file://");
    for byte in path.to_str()?.bytes() {
        if kept(byte) {
            text.push(char::from(byte));
        } else {
            // Writing to a `String` cannot fail.
            let _ = write!(text, "%{byte:02X}");
        }
    }
    text.parse().ok()
}

/// Where a document is checked ([`locate`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// In the workspace under `root`, canonical, as the file at `relative`,
    /// its path relative to the root, `/` separated.
    Workspace {
        /// The workspace's root.
        root: PathBuf,
        /// The file's path relative to the root.
        relative: String,
    },
    /// Alone, as `gcsl run` checks a file outside any workspace: the file at
    /// `path`, or, for a URI that is no file, nowhere.
    Lone {
        /// The file's path, if it has one.
        path: Option<PathBuf>,
    },
}

/// Where the document named `uri` is checked: in the workspace
/// [`gcsl_disk::find_workspace`] finds for its file, or, for a file not on
/// disk yet, the workspace its directory is in, by the same rules (see the
/// module documentation); else alone.
#[must_use]
pub fn locate(uri: &Uri) -> Place {
    let Some(path) = path_of(uri) else {
        return Place::Lone { path: None };
    };
    let found = gcsl_disk::find_workspace(&path).or_else(|| unwritten(&path));
    match found {
        Some((root, relative)) => Place::Workspace { root, relative },
        None => Place::Lone { path: Some(path) },
    }
}

/// The workspace root of `file`, a file that is not on disk, and its path
/// relative to it, found from its directory's canonical path as
/// [`gcsl_disk::find_workspace`] finds a file's: `None` for a file that is
/// on disk, which that has placed already, or that would be none of a
/// workspace's files.
fn unwritten(file: &Path) -> Option<(PathBuf, String)> {
    if fs::symlink_metadata(file).is_ok() {
        return None;
    }
    let dir = fs::canonicalize(file.parent()?).ok()?;
    let file = dir.join(file.file_name()?);
    let root = gcsl_disk::find_root(&dir)?;
    let mut parts = Vec::new();
    for part in file.strip_prefix(root).ok()?.components() {
        let Component::Normal(part) = part else {
            return None;
        };
        let part = part.to_str()?;
        if part.starts_with('.') {
            return None;
        }
        parts.push(part);
    }
    if file.extension().and_then(OsStr::to_str) != Some("gcsl") {
        return None;
    }
    Some((root.to_path_buf(), parts.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_uris_round_trip() {
        let path = Path::new("/tmp/a dir/é😀/x+y.gcsl");
        let uri = uri_of(path).unwrap();
        assert_eq!(
            uri.as_str(),
            "file:///tmp/a%20dir/%C3%A9%F0%9F%98%80/x%2By.gcsl"
        );
        assert_eq!(path_of(&uri).unwrap(), path);
        // Another client's spelling: lowercase escapes, kept sub-delimiters,
        // an authority.
        let other: Uri = "file://localhost/tmp/a%20dir/(x)%c3%a9.gcsl"
            .parse()
            .unwrap();
        assert_eq!(path_of(&other).unwrap(), Path::new("/tmp/a dir/(x)é.gcsl"));
        let untitled: Uri = "untitled:Untitled-1".parse().unwrap();
        assert_eq!(path_of(&untitled), None);
    }

    #[test]
    fn only_local_absolute_file_uris_name_files() {
        for text in [
            "file://remote.example/tmp/app.gcsl",
            "file://user@localhost/tmp/app.gcsl",
            "file://localhost:80/tmp/app.gcsl",
            "file:relative.gcsl",
        ] {
            let uri: Uri = text.parse().unwrap();
            assert_eq!(path_of(&uri), None, "{text}");
            assert_eq!(locate(&uri), Place::Lone { path: None }, "{text}");
        }
        for text in [
            "file:///tmp/app.gcsl",
            "file:/tmp/app.gcsl",
            "FILE://LOCALHOST/tmp/app.gcsl",
        ] {
            let uri: Uri = text.parse().unwrap();
            assert_eq!(
                path_of(&uri),
                Some(PathBuf::from("/tmp/app.gcsl")),
                "{text}"
            );
        }
        let non_utf8: Uri = "file:///tmp/%FF.gcsl".parse().unwrap();
        assert_eq!(path_of(&non_utf8), None);
    }
}
