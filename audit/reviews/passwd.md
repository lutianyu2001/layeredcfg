# Review of the lookup in the password database

- file: crates/layeredcfg/src/passwd.rs sha256:925923fa542e5155b62b1b1361d9e1399f782e8c88258d554a3b17baba960409
- reviewer: three independent reviewers, each given the code alone, without the findings of the others
- date: 2026-11-08
- commit: 76b5f65

The module is the crate's only `unsafe` code: `getuid`, `sysconf`, `getpwuid_r`, `getpwnam_r`, and the reading
of what the last two fill in. Each reviewer checked every `unsafe` block, every `// SAFETY:` comment and the
`# Safety` contract of `read` against POSIX and the sources of glibc, musl and macOS's Libinfo: pointers, lengths,
initialisation, provenance and lifetimes, every return code, thread safety, the signedness of `c_char`, and the
tests and the Kani proof.

## Verdict

Sound: no undefined behaviour and no false safety claim in the code as reviewed.

## Findings, and what was done

- **The buffer could start at 0 bytes and never grow** (found by a test before the reviews): doubling 0 stays 0, a
  loop without end. The size is clamped to at least 1.
- **The retry after a signal had no bound** (first review): a module answering EINTR for ever would spin. At most 16
  retries now.
- **The contract asked too much** (second review): it said the home directory is a string in the caller's buffer,
  as POSIX puts it, and a debug build asserted it; nss-systemd up to v245 gave root's and nobody's home as static
  strings outside it, so the assertion could fire on a legitimate system. The contract now asks for a NUL-terminated
  string that stays valid and unchanged while it is read, and a test reads one outside the buffer, under Miri too.
- **The Kani proof bounded the calls loosely and did not show that retry stops at a final answer or retries at all**
  (second and third reviews). It now asserts, for every sequence of answers, that a call follows only an answer that
  allows one, with the size it should have, and that the lookup ends at an answer it cannot go on from, giving what
  that answer found. Breaking retry to give up at the first signal makes it fail.
- **A comment said the call writes only into the three pointers given** (third review): the C library keeps state
  of its own. It now says that of the memory Rust owns, only those three are written.
- **Untested paths** (second review): `~` without HOME, `~name`, an unknown name, trailing slashes in HOME, and the
  size the buffer starts at when the system suggests none (musl) are tested now.

## What this leaves open

- The real C library is reached only by the tests that compare it with `getent` and `dscl` (glibc, musl, macOS)
  and that run under AddressSanitizer; a signal and the errors other than ERANGE are reached through made-up answers
  alone. Miri and Kani cannot follow the calls into it.
- The Kani proof uses small limits (8 bytes, 2 signals); that the real ones (1 MiB, 16) behave the same differs only
  in the number of steps, which is argued, not proved.
- An NSS module that returns a home directory it frees or changes while it is read would break the contract; none
  is known to.
