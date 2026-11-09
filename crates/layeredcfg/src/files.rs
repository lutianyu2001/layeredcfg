//! Reading and writing files, and paths.

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::fs::{self, OpenOptions, Permissions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, fchown};
use std::path::{Component, Path, PathBuf};

use crate::{Environment, Error, ErrorKind, Result, passwd};

/// An error of the system as the C library says it ("No such file or directory"), without Rust's "(os error 2)".
pub(crate) fn os_message(error: &io::Error) -> String {
    let message = error.to_string();
    match error.raw_os_error() {
        Some(code) => message.strip_suffix(&format!(" (os error {code})")).unwrap_or(&message).to_owned(),
        None => message,
    }
}

pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>> {
    if !path.is_file() {
        return Err(Error::new(ErrorKind::Io, "", format!("cannot read {}: not a file", path.display())));
    }
    fs::read(path).map_err(|error| {
        Error::new(ErrorKind::Io, "", format!("cannot read {}: {}", path.display(), os_message(&error)))
    })
}

/// Six letters and digits, different for each file written at the same time (as mkstemp's XXXXXX).
fn spare_suffix() -> String {
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    let mut n = hasher.finish();
    (0..6)
        .map(|_| {
            #[allow(clippy::cast_possible_truncation)] // less than the number of letters
            let letter = LETTERS[(n % LETTERS.len() as u64) as usize];
            n /= LETTERS.len() as u64;
            char::from(letter)
        })
        .collect()
}

/// The file a path names, through symbolic links, also those whose target does not exist yet.
fn link_target(given: &Path) -> PathBuf {
    if let Ok(target) = fs::canonicalize(given) {
        return target;
    }
    let mut path = given.to_path_buf();
    for _ in 0..40 {
        let Ok(link) = fs::read_link(&path) else { break };
        path = match path.parent() {
            Some(directory) => directory.join(link),
            None => link,
        };
    }
    path
}

/// Written beside the file and put in its place at once, so that a write that fails halfway leaves the old file whole.
/// It is 0600 while it is written, and then takes the old file's permissions (read, write, execute: not set-user-ID,
/// set-group-ID or sticky) and group; when the group cannot be kept, nobody of the group the file gets may read it.
pub(crate) fn replace_file(given: &Path, text: &[u8]) -> Result<()> {
    let path = link_target(given);
    let old = fs::metadata(&path).ok();
    // A name of its own, created 0600, so that writers at the same time do not write into one another's.
    let (spare, mut file) = loop {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".writing-{}", spare_suffix()));
        let spare = PathBuf::from(name);
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&spare) {
            Ok(file) => break (spare, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(Error::new(
                    ErrorKind::Io,
                    "",
                    format!("cannot write beside {}: {}", path.display(), os_message(&error)),
                ));
            }
        }
    };
    let written = file.write_all(text).and_then(|()| file.sync_all()).and_then(|()| {
        let Some(old) = old else { return Ok(()) };
        let mut mode = old.permissions().mode() & 0o777;
        if fs::metadata(&spare)?.gid() != old.gid() && fchown(&file, None, Some(old.gid())).is_err() {
            mode &= !0o070;
        }
        file.set_permissions(Permissions::from_mode(mode))
    });
    drop(file);
    if let Err(error) = written.and_then(|()| fs::rename(&spare, &path)) {
        let _ = fs::remove_file(&spare);
        return Err(Error::new(ErrorKind::Io, "", format!("cannot write {}: {}", path.display(), os_message(&error))));
    }
    Ok(())
}

/// `~` and `~user` at the start made the home directory, as Python's os.path.expanduser makes them: `~` from HOME in
/// `environment`, else the user's entry in the password database. Unlike Python, an empty home expands nothing (Python
/// makes `~/x` `/x`).
pub(crate) fn expand_user(text: &[u8], environment: &Environment) -> PathBuf {
    let as_is = || PathBuf::from(OsString::from_vec(text.to_vec()));
    if text.first() != Some(&b'~') {
        return as_is();
    }
    let slash = text.iter().position(|&c| c == b'/').unwrap_or(text.len());
    let user = &text[1..slash];
    let home = if user.is_empty() {
        match environment.get("HOME".as_ref()) {
            Some(home) => Some(home.into_vec()),
            None => passwd::home(passwd::current()),
        }
    } else {
        // A name that is not UTF-8 is no user's here, as Python's pwd module takes only text.
        std::str::from_utf8(user).ok().and_then(|name| passwd::home_of_name(name.as_bytes()))
    };
    let Some(mut home) = home.filter(|home| !home.is_empty()) else {
        return as_is();
    };
    while home.len() > 1 && home.last() == Some(&b'/') {
        home.pop();
    }
    home.extend_from_slice(&text[slash..]);
    PathBuf::from(OsString::from_vec(home))
}

/// Whether two paths are the same file.
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// `..` and `.` taken out of an absolute path as written, without looking at the file system.
fn lexically_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The first directory from `start` upwards that has a regular file `name`, and that file.
pub fn find_upward(name: impl AsRef<Path>, start: impl AsRef<Path>) -> Option<PathBuf> {
    let name = name.as_ref();
    let mut directory = lexically_normal(&std::path::absolute(start.as_ref()).ok()?);
    loop {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !directory.pop() {
            return None;
        }
    }
}
