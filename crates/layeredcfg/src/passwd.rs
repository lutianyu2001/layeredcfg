//! A user's home directory from the password database, through the C library's `getpwuid_r` and `getpwnam_r`, which
//! ask every source the system is set up with (the local files, LDAP, NIS, systemd-homed). This is the crate's only
//! `unsafe` code: Rust's standard library has no lookup by name, and a crate that wraps it (nix) is far larger than
//! these two calls.
//!
//! The `unsafe` code is kept apart from the rest, so that what can be checked without the C library is:
//! - [`attempt`] makes one call, the only code that calls into the C library;
//! - [`read`] reads what the call filled in, under a contract its tests meet with entries of their own, under Miri
//!   too;
//! - [`retry`] decides what to do with each attempt (again after a signal, with a larger buffer when the entry does
//!   not fit), in safe code that the tests drive with made-up attempts and Kani proves for every sequence of them.
//!
//! The tests against the real C library reach success, no such user and a buffer too small; a signal and the other
//! errors it can give are reached only through made-up attempts.

use std::ffi::{CStr, CString, c_char, c_int};
use std::mem::MaybeUninit;
use std::ptr;

/// The size the buffer of the strings starts at unless the system suggests one.
const FIRST_SIZE: usize = 1024;

/// How far [`retry`] goes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// The most the buffer grows to.
    pub(crate) most_size: usize,
    /// How many times a call interrupted by a signal is made again before the lookup counts as failed.
    pub(crate) most_interruptions: u32,
}

/// An entry longer than a mebibyte is no user's.
pub(crate) const LIMITS: Limits = Limits { most_size: 1 << 20, most_interruptions: 16 };

/// What one call found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// The user's entry, and its home directory, when it has one.
    Found(Option<Vec<u8>>),
    /// No such user.
    NoUser,
    /// The call failed with this error: `ERANGE` when the buffer is too small, `EINTR` when a signal came.
    Failed(c_int),
}

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
    // SAFETY: sysconf has no preconditions; any name is valid to ask for (it gives -1 for one it does not know).
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    retry(first_size(suggested), &LIMITS, |size| attempt(user, size))
}

/// The size the buffer starts at: the one the system suggests (glibc does), else [`FIRST_SIZE`] (musl gives -1).
fn first_size(suggested: libc::c_long) -> usize {
    usize::try_from(suggested).ok().filter(|&size| size > 0).unwrap_or(FIRST_SIZE)
}

/// The home directory from the attempts `attempt(size)` makes, each with a buffer of `size` bytes: from `first`
/// (between 1 and `limits.most_size`), doubling while the entry does not fit, and made again after a signal, up to
/// `limits.most_interruptions` times. Nothing when there is no such user or the entry has no directory, or an attempt
/// fails otherwise, or the limits are reached.
pub(crate) fn retry(first: usize, limits: &Limits, mut attempt: impl FnMut(usize) -> Attempt) -> Option<Vec<u8>> {
    let mut size = first.clamp(1, limits.most_size.max(1)); // doubling 0 would never grow
    let mut interruptions = 0;
    loop {
        match attempt(size) {
            Attempt::Found(home) => return home,
            Attempt::Failed(libc::EINTR) if interruptions < limits.most_interruptions => interruptions += 1,
            Attempt::Failed(libc::ERANGE) if size < limits.most_size => {
                size = size.saturating_mul(2).min(limits.most_size);
            }
            Attempt::NoUser | Attempt::Failed(_) => return None,
        }
    }
}

/// One call of `getpwuid_r` or `getpwnam_r` with a buffer of `size` bytes.
#[allow(unsafe_code)]
fn attempt(user: User<'_>, size: usize) -> Attempt {
    let mut buffer: Vec<c_char> = vec![0; size];
    let mut entry = MaybeUninit::<libc::passwd>::uninit();
    let mut found: *mut libc::passwd = ptr::null_mut();
    let code = match user {
        // SAFETY: entry is writable storage for one passwd, buffer is writable for buffer.len() bytes, and found is
        // a writable pointer: what getpwuid_r takes (POSIX). Of the memory Rust owns, it writes only these three (the
        // C library keeps state of its own besides: errno, its NSS modules, their files).
        User::Id(id) => unsafe {
            libc::getpwuid_r(id, entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &raw mut found)
        },
        // SAFETY: as above, and name is a NUL-terminated string that outlives the call.
        User::Name(name) => unsafe {
            libc::getpwnam_r(name.as_ptr(), entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &raw mut found)
        },
    };
    // SAFETY: what read takes, as getpwuid_r and getpwnam_r leave it: found is null, or, when code is 0, points to
    // entry (glibc, musl and macOS set *result to the entry they were given, or to null), filled in, whose pw_dir is
    // null or a NUL-terminated string. POSIX puts it in buffer, which is borrowed, unchanged, while read runs; an NSS
    // module may point it elsewhere instead (nss-systemd up to v245 gave root's and nobody's as static strings),
    // which lives as long as the module.
    unsafe { read(code, found, &entry, &buffer) }
}

/// What a call of `getpwuid_r` or `getpwnam_r` that returned `code` and set `found` found, with the home directory
/// copied out.
///
/// # Safety
///
/// When `code` is 0 and `found` is not null, `found` points to `entry` (asserted in debug builds), which is filled in,
/// and its `pw_dir` is null or points to a NUL-terminated string that stays valid and unchanged while `read` runs:
/// within `buffer`, where POSIX puts it, or wherever else the C library keeps it.
#[allow(unsafe_code)]
pub(crate) unsafe fn read(
    code: c_int,
    found: *const libc::passwd,
    entry: &MaybeUninit<libc::passwd>,
    // Not read: borrowed so that what the strings are in stays as it is while they are read.
    _buffer: &[c_char],
) -> Attempt {
    if code != 0 {
        return Attempt::Failed(code);
    }
    if found.is_null() {
        return Attempt::NoUser;
    }
    debug_assert!(ptr::eq(found, entry.as_ptr()), "the entry found is not the one given");
    // SAFETY: by the contract, found points to entry, which is filled in and borrowed for this call.
    let directory = unsafe { (*found).pw_dir };
    if directory.is_null() {
        return Attempt::Found(None);
    }
    // SAFETY: by the contract, a NUL-terminated string that stays valid and unchanged until the copy is made.
    let directory = unsafe { CStr::from_ptr(directory) };
    Attempt::Found(Some(directory.to_bytes().to_vec()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::cell::Cell;
    use std::ffi::{c_char, c_int};
    use std::mem::MaybeUninit;
    use std::ptr;

    use super::{Attempt, FIRST_SIZE, Limits, first_size, retry};

    const SMALL: Limits = Limits { most_size: 64, most_interruptions: 3 };

    /// What `retry` does with these attempts, one after the other (the last again once they run out): the sizes it
    /// asked for, and what it found.
    fn driven(first: usize, limits: &Limits, script: &[Attempt]) -> (Vec<usize>, Option<Vec<u8>>) {
        let calls = Cell::new(Vec::new());
        let found = retry(first, limits, |size| {
            let mut sizes = calls.take();
            sizes.push(size);
            let next = script[(sizes.len() - 1).min(script.len() - 1)].clone();
            calls.set(sizes);
            next
        });
        (calls.take(), found)
    }

    fn failed(code: c_int) -> Attempt {
        Attempt::Failed(code)
    }

    #[test]
    fn an_entry_found_is_its_directory_and_no_user_or_no_directory_is_nothing() {
        let home = Attempt::Found(Some(b"/home/a".to_vec()));
        assert_eq!(driven(16, &SMALL, &[home]), (vec![16], Some(b"/home/a".to_vec())));
        assert_eq!(driven(16, &SMALL, &[Attempt::NoUser]), (vec![16], None));
        assert_eq!(driven(16, &SMALL, &[Attempt::Found(None)]), (vec![16], None));
    }

    #[test]
    fn a_buffer_too_small_doubles_up_to_the_limit_and_then_the_lookup_fails() {
        let home = Attempt::Found(Some(b"/h".to_vec()));
        let (sizes, found) = driven(5, &SMALL, &[failed(libc::ERANGE), failed(libc::ERANGE), home]);
        assert_eq!((sizes, found), (vec![5, 10, 20], Some(b"/h".to_vec())));
        assert_eq!(driven(5, &SMALL, &[failed(libc::ERANGE)]), (vec![5, 10, 20, 40, 64], None));
        // 0 would never grow, and a start past the limit is the limit.
        assert_eq!(driven(0, &SMALL, &[failed(libc::ERANGE)]).0, [1, 2, 4, 8, 16, 32, 64]);
        assert_eq!(driven(1000, &SMALL, &[failed(libc::ERANGE)]), (vec![64], None));
        let no_room = Limits { most_size: 0, most_interruptions: 0 };
        assert_eq!(driven(0, &no_room, &[failed(libc::ERANGE)]), (vec![1], None));
        assert_eq!(driven(usize::MAX, &LIMITS_FULL, &[failed(libc::ERANGE)]).0, [1 << 20]);
    }

    const LIMITS_FULL: Limits = super::LIMITS;

    #[test]
    fn the_buffer_starts_at_the_size_the_system_suggests_or_else_at_a_kibibyte() {
        assert_eq!(first_size(4096), 4096);
        assert_eq!(first_size(-1), FIRST_SIZE); // musl, and a system that suggests none
        assert_eq!(first_size(0), FIRST_SIZE);
    }

    #[test]
    fn a_signal_makes_the_call_again_a_few_times_and_another_error_fails_at_once() {
        let home = Attempt::Found(Some(b"/h".to_vec()));
        let (sizes, found) = driven(8, &SMALL, &[failed(libc::EINTR), failed(libc::EINTR), home]);
        assert_eq!((sizes, found), (vec![8, 8, 8], Some(b"/h".to_vec())));
        assert_eq!(driven(8, &SMALL, &[failed(libc::EINTR)]), (vec![8; 4], None));
        assert_eq!(driven(8, &SMALL, &[failed(libc::EIO)]), (vec![8], None));
        assert_eq!(driven(8, &SMALL, &[failed(libc::ENOMEM)]), (vec![8], None));
        // Signals and a buffer too small, together.
        let script = [failed(libc::EINTR), failed(libc::ERANGE), failed(libc::EINTR), Attempt::NoUser];
        assert_eq!(driven(8, &SMALL, &script), (vec![8, 8, 16, 16], None));
    }

    /// An entry as the C library fills it in: the directory, NUL-terminated, at `at` in a buffer of `size` bytes.
    #[allow(unsafe_code)]
    fn filled(directory: Option<&[u8]>, at: usize, size: usize) -> (Vec<c_char>, MaybeUninit<libc::passwd>) {
        let mut buffer: Vec<c_char> = vec![0x55; size];
        // SAFETY: every field of passwd is an integer or a pointer, for which all zeros is valid (0, null).
        let mut entry: libc::passwd = unsafe { MaybeUninit::zeroed().assume_init() };
        if let Some(directory) = directory {
            for (i, &byte) in directory.iter().enumerate() {
                buffer[at + i] = c_char::from_ne_bytes([byte]);
            }
            buffer[at + directory.len()] = 0;
            entry.pw_dir = buffer[at..].as_mut_ptr();
        }
        (buffer, MaybeUninit::new(entry))
    }

    #[test]
    #[allow(unsafe_code)]
    fn read_copies_the_directory_out_of_the_buffer_and_reports_the_rest() {
        let (buffer, entry) = filled(Some(b"/home/a"), 3, 16);
        // SAFETY: entry is filled in, its pw_dir a NUL-terminated string in buffer, and found points to it.
        let found = unsafe { super::read(0, entry.as_ptr(), &entry, &buffer) };
        assert_eq!(found, Attempt::Found(Some(b"/home/a".to_vec())));
        // The string ends at the buffer's last byte.
        let (buffer, entry) = filled(Some(b"/x"), 13, 16);
        // SAFETY: as above.
        let found = unsafe { super::read(0, entry.as_ptr(), &entry, &buffer) };
        assert_eq!(found, Attempt::Found(Some(b"/x".to_vec())));
        // A directory outside the buffer, as an NSS module may give.
        let (elsewhere, entry) = filled(Some(b"/root"), 0, 8);
        // SAFETY: entry is filled in, its pw_dir a NUL-terminated string in elsewhere, alive until after the call.
        let found = unsafe { super::read(0, entry.as_ptr(), &entry, &[]) };
        assert_eq!(found, Attempt::Found(Some(b"/root".to_vec())));
        drop(elsewhere);
        let (buffer, entry) = filled(None, 0, 4);
        // SAFETY: entry is filled in, with no directory, and found points to it.
        let found = unsafe { super::read(0, entry.as_ptr(), &entry, &buffer) };
        assert_eq!(found, Attempt::Found(None));
        let entry = MaybeUninit::uninit();
        // SAFETY: found is null, so nothing is read.
        let none = unsafe { super::read(0, ptr::null(), &entry, &[]) };
        assert_eq!(none, Attempt::NoUser);
        // SAFETY: nothing is read when the code is not 0.
        let failed = unsafe { super::read(libc::ERANGE, entry.as_ptr(), &entry, &[]) };
        assert_eq!(failed, Attempt::Failed(libc::ERANGE));
    }

    // The C library's password database: not under Miri, which cannot call into it.
    #[cfg(not(miri))]
    mod system {
        use std::process::Command;

        use super::super::{User, attempt, current, home, home_of_name, retry};
        use super::LIMITS_FULL;

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
            let user = format!("/Users/{name}");
            if let Some(record) = output("dscl", &[".", "-read", &user, "NFSHomeDirectory"]) {
                assert_eq!(record.strip_prefix("NFSHomeDirectory: ").map(str::as_bytes), Some(&by_id[..]), "{record}");
            }
        }

        #[test]
        fn a_buffer_too_small_for_the_real_entry_grows_until_it_fits() {
            for user in [current(), User::Name(c"root")] {
                // One byte holds no entry: the C library says ERANGE, and the buffer grows.
                assert_eq!(attempt(user, 1), super::Attempt::Failed(libc::ERANGE));
                let grown = retry(1, &LIMITS_FULL, |size| attempt(user, size));
                assert!(grown.as_ref().is_some_and(|home| home.starts_with(b"/")), "{grown:?}");
                assert_eq!(grown, home(user));
            }
        }

        #[test]
        fn no_such_user_and_a_name_no_user_can_have_are_nothing() {
            assert_eq!(home_of_name(b"layeredcfg-no-such-user"), None);
            assert_eq!(attempt(User::Name(c"layeredcfg-no-such-user"), 4096), super::Attempt::NoUser);
            assert_eq!(home_of_name(b"root\0x"), None);
            assert_eq!(home_of_name(b""), None);
            assert!(home_of_name(b"root").is_some());
        }
    }
}
