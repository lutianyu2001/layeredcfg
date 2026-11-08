//! A user's home directory from the password database, through the C library's `getpwuid_r` and `getpwnam_r`, which
//! ask every source the system is set up with (the local files, LDAP, NIS, systemd-homed). This is the crate's only
//! `unsafe` code: Rust's standard library has no lookup by name, and a crate that wraps it (nix) is far larger than
//! these two calls. Each `unsafe` block makes one call or reads one pointer, and says why it holds.

use std::ffi::{CStr, CString, c_char};
use std::mem::MaybeUninit;
use std::ptr;

/// The size the buffer of the strings starts at, unless the system suggests one, and the most it grows to: an entry
/// longer than a mebibyte is no user's.
const FIRST_SIZE: usize = 1024;
const MOST_SIZE: usize = 1 << 20;

/// How many times a lookup interrupted by a signal is tried again before it counts as failed.
const MOST_INTERRUPTIONS: u32 = 16;

/// Whose entry: the user with an id, or with a name.
#[derive(Clone, Copy)]
pub(crate) enum User<'a> {
    Id(libc::uid_t),
    Name(&'a CStr),
}

/// The user the process runs as.
#[allow(unsafe_code)]
pub(crate) fn current() -> User<'static> {
    // SAFETY: getuid has no preconditions and cannot fail (POSIX).
    User::Id(unsafe { libc::getuid() })
}

/// The home directory of the user named `name`; nothing for a name with a NUL in it, which no user has.
pub(crate) fn home_of_name(name: &[u8]) -> Option<Vec<u8>> {
    let name = CString::new(name).ok()?;
    home(User::Name(&name))
}

/// The home directory in the user's entry: nothing when there is no such user, the entry has no directory, or the
/// database cannot be read.
#[allow(unsafe_code)]
pub(crate) fn home(user: User<'_>) -> Option<Vec<u8>> {
    // The size the system suggests, when it does (sysconf gives -1 for none).
    // SAFETY: sysconf has no preconditions; any name is valid to ask for.
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let first = usize::try_from(suggested).ok().filter(|&size| size > 0).unwrap_or(FIRST_SIZE);
    home_with_buffer(user, first.min(MOST_SIZE))
}

/// As [`home`], with the buffer starting at `size` bytes and doubling while the entry does not fit (tests start it
/// small so that it grows).
#[allow(unsafe_code)]
pub(crate) fn home_with_buffer(user: User<'_>, size: usize) -> Option<Vec<u8>> {
    let mut size = size.clamp(1, MOST_SIZE); // doubling 0 would never grow
    let mut interruptions = 0;
    loop {
        let mut buffer: Vec<c_char> = vec![0; size];
        let mut entry = MaybeUninit::<libc::passwd>::uninit();
        let mut found: *mut libc::passwd = ptr::null_mut();
        let code = match user {
            // SAFETY: entry is writable storage for one passwd, buffer is writable for buffer.len() bytes, and found
            // is a writable pointer: what getpwuid_r takes (POSIX). Besides errno, it writes only into the three.
            User::Id(id) => unsafe {
                libc::getpwuid_r(id, entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &raw mut found)
            },
            // SAFETY: as above, and name is a NUL-terminated string that outlives the call.
            User::Name(name) => unsafe {
                libc::getpwnam_r(name.as_ptr(), entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &raw mut found)
            },
        };
        match code {
            0 => {}
            libc::EINTR if interruptions < MOST_INTERRUPTIONS => {
                interruptions += 1;
                continue;
            }
            libc::ERANGE if size < MOST_SIZE => {
                size = (size * 2).min(MOST_SIZE);
                continue;
            }
            _ => return None,
        }
        if found.is_null() {
            return None; // no such user
        }
        // SAFETY: getpwuid_r and getpwnam_r returned 0 with found not null, so found points to entry, which they
        // filled in (POSIX); entry is alive and not moved.
        let directory = unsafe { (*found).pw_dir };
        if directory.is_null() {
            return None;
        }
        // SAFETY: pw_dir of a filled-in entry points to a NUL-terminated string (POSIX puts it in buffer, as glibc,
        // musl and macOS do), and what it points to stays alive and unchanged, buffer and entry with it, until this
        // copy of it is made.
        let directory = unsafe { CStr::from_ptr(directory) }.to_bytes().to_vec();
        return Some(directory);
    }
}

// Miri runs none of these: it cannot call into the C library's password database.
#[cfg(all(test, not(miri)))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::process::Command;

    use super::{User, current, home, home_of_name, home_with_buffer};

    /// What a command prints, without the line end; nothing when it is not there or fails.
    fn output(program: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(program).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
    }

    #[test]
    fn the_current_user_is_found_by_id_and_by_name_with_the_home_the_system_gives() {
        let by_id = home(current()).expect("the current user has an entry");
        let name = output("id", &["-un"]).expect("id -un names the current user");
        assert_eq!(home_of_name(name.as_bytes()), Some(by_id.clone()));
        // getent asks the same sources; macOS has none, and dscl there.
        if let Some(entry) = output("getent", &["passwd", &name]) {
            assert_eq!(entry.split(':').nth(5).map(str::as_bytes), Some(&by_id[..]), "{entry}");
        }
        if let Some(record) = output("dscl", &[".", "-read", &format!("/Users/{name}"), "NFSHomeDirectory"]) {
            assert_eq!(record.strip_prefix("NFSHomeDirectory: ").map(str::as_bytes), Some(&by_id[..]), "{record}");
        }
    }

    #[test]
    fn a_buffer_too_small_grows_until_the_entry_fits() {
        let user = current();
        assert_eq!(home_with_buffer(user, 1), home(user));
        assert_eq!(home_with_buffer(user, 0), home(user));
        let root = User::Name(c"root");
        assert!(home_with_buffer(root, 1).is_some_and(|home| home.starts_with(b"/")));
    }

    #[test]
    fn no_such_user_and_a_name_no_user_can_have_are_nothing() {
        assert_eq!(home_of_name(b"layeredcfg-no-such-user"), None);
        assert_eq!(home_of_name(b"root\0x"), None);
        assert_eq!(home_of_name(b""), None);
        assert!(home_of_name(b"root").is_some());
    }
}
