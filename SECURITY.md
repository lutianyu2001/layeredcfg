# Security

What layeredconf is trusted with, how it is checked, and what is left to the program that uses it.

## What It Is Trusted With

layeredconf reads a program's settings from four layers: the TOML file of defaults (the program's own), env-style
files, the environment and the command line. All of them come from the program or from the user who runs it; they are
trusted, but may be wrong. It gives back values and errors, whose messages programs tend to log or print, and
`persist` writes files.

What it holds against:

- **Text that is wrong.** Any bytes, in any layer, are read or refused with an error: no panic, no overflow, no
  undefined behaviour. The tests, Kani, Miri and the fuzzing below check this. `${NAME}` cannot make a small file's
  values huge (`A=${A}${A}` line after line would double them): past 16 MiB of what it puts in, in all, the
  statements are left out.
- **Secrets in logs.** The value of a `secret` setting is `***` in what `shown` gives, and in the message of an error
  about it (`InvalidValue`, and the messages of Python's `problems()`); `{:?}` of an error leaves the value out, and
  Python's errors do not keep the core's error, which has it, as their context.
- **Other users of the machine.** `persist` writes beside the file, 0600 until the text is written and synced, and
  renames it into place, so that nobody reads a file half written. A new file stays 0600; one it replaces keeps its
  read, write and execute permissions and its group (see below). A file it cannot look at, such as one behind a link
  the system will not follow (`fs.protected_symlinks`, in a directory others can write), is not written.
- **Several writers at once.** Each writes a file of its own name beside the target, so none writes into another's;
  the last one's file is the one that stays.

What it does not hold against is under [Design Decisions](#design-decisions).

## Checks

The audit runs apart from the build and the tests, with `cargo xtask` (see the README); `target/audit/` keeps the
logs. The versions of its tools are pinned in `xtask/src/tools.rs`.

| Check | What it covers | Command |
|---|---|---|
| `#![forbid(unsafe_code)]` | Neither crate of this project has `unsafe` code; the dependencies' is counted in [SUPPLY_CHAIN.md](SUPPLY_CHAIN.md) | built in |
| Tests | The layers, the conversions, persist, the readers of text and of env-style files; each condition of the masking of secrets and of the checks persist makes before writing | `pixi run test` |
| Miri | The Rust tests of `layeredconf` without undefined behaviour (not the binding, which calls the C API of Python) | `cargo xtask miri` |
| Branch coverage | Which branches the Rust tests do not take, to find conditions left untested | `cargo xtask coverage` |
| Kani | For every input up to 6 bytes: UTF-8 decoded as the standard library does, whitespace stripped once and for all and only between characters, integers without panic or overflow and as `str::parse` reads them, booleans without panic, lists cut at their commas and nowhere else; integers of 21 bytes at the edges of 64 bits | `cargo xtask kani` |
| Fuzzing | Each target an hour (see below) | `cargo xtask fuzz` |
| cargo deny | Known vulnerabilities, unmaintained and yanked crates, licenses, sources; for the fuzz workspace, vulnerabilities and sources | `cargo xtask deny` |
| cargo vet | That every dependency is audited or exempted | `cargo xtask vet` |
| cargo geiger | The `unsafe` code of the dependencies: a count to read, which fails nothing | `cargo xtask geiger` |

The fuzz targets, and what they assert of every input:

| Target | Input | Asserted |
|---|---|---|
| `envfile` | An env-style file | No panic; the statements read lie in order inside the text, each with a name; a value written by `persist`'s quoting into the file reads back as it was, or the writing is refused (a quote never closed before it) |
| `numbers` | Any bytes | No panic; an integer or a float read is written so that it reads back the same; `strip` is idempotent and the same on bytes and on text; the items of a list are not empty, have no comma and no whitespace around them |
| `config` | TOML text, a NUL, a value | No panic in reading the defaults, or in `get`, `shown`, `path`, `cast` and `set` of each setting with the value in the environment and as an override; a value `set` takes, `get` gives |

Not proved, and covered by the tests and the fuzzing instead: `parse_float` (it builds the text it gives
`str::parse`, past what Kani can unroll), the reader of env-style files (a loop with some nine loops inside, past 16 GB
of memory at 4 bytes), and what is above the readers of text (the layers, persist).

## Design Decisions

Each says what it does not cover.

- **The process's environment is only read.** `persist` keeps what it writes in the configuration and does not set
  the variable: `std::env::set_var` is not safe while other threads read the environment. A child process the program
  starts does not see the value written until it is read from the file again.
- **What is masked.** `***` takes the place of a secret's value in `shown` and in messages, nowhere else: `get` gives
  the value (that is what it is for), as do `Error::raw()` and Python's `InvalidValue.raw`; a Python `str` with a
  surrogate that surrogateescape does not make a byte (one outside U+DC80 to U+DCFF) is refused with Python's
  `UnicodeEncodeError`, which has the whole value; `problems()` in Rust gives each value as written, also a secret's
  (Python's gives messages, masked); `{:?}` of a `State` or a `Problem` shows values; and `persist` writes the value
  into the file.
- **A file keeps its permissions, not all of them.** A secret persisted into a file others can read is readable by
  them. The file is replaced by rename, so its owner becomes whoever wrote it; it keeps its group where the writer may
  give it that group, and otherwise gets the writer's, without permissions for it. Set-user-ID, set-group-ID and
  sticky are not kept, nor are ACLs or extended attributes. A process killed while it writes leaves the file beside
  (`.writing-` and six letters), 0600, with the new text; the directory is not synced after the rename.
- **Symbolic links are followed.** `persist` writes the file a link leads to and leaves the link, so that files kept
  elsewhere (dotfiles) stay where they are. Whoever can change the link can send the write to another file of the
  writer's, where the system lets the link be followed.
- **The files are not shell scripts.** `persist` writes a value with a quote in it single-quoted with `\'`, as
  python-dotenv reads it; a shell does not, so sourcing (`. .env`) a file `persist` wrote can run what a value has in
  it. A file is read with `export ` before a name only so that it can be shared with tools that write it so.
- **The files are as trusted as the program.** They are read whole, without a limit of size, and a file is checked to
  be one before it is opened: in a directory others can write, it can be swapped for a pipe in between, and the read
  waits. `${NAME}` in a file takes any variable of the environment, as python-dotenv does, so a file can put, say, a
  token from the environment into a setting that is shown or logged.
- **`find_upward` does not check who owns what it finds.** A file found in a directory others can write into (under
  `/tmp`, say) is theirs: a program should look upward only from directories of its user.
- **`~name` in a path looks the user up** in the password database, which can go over the network (LDAP, NIS).
- **Messages have names and values as they are.** The core escapes only `"` and `\` in a value; Python's
  `invalid_value` message has the value's `repr`, but names of files and settings, and the core's messages about files
  and the defaults, are as they are. A message printed to a terminal can carry control characters, escape sequences
  among them.
- **The dependencies' `unsafe` code** (PyO3, nix, hashbrown and others) is theirs, counted and left to their audits:
  see [SUPPLY_CHAIN.md](SUPPLY_CHAIN.md).
