# Security

What layeredcfg is trusted with, how it is checked, and what is left to the program that uses it.

## What It Is Trusted With

layeredcfg reads a program's settings from four layers: the TOML file of defaults (the program's own), env-style
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

## Security Audit

The security audit runs apart from the build and the tests: neither `pixi run test`, `cargo test` nor a build of the
wheel runs any of it. `cargo xtask` runs it, with the versions of its tools and of the nightly Rust it needs pinned in
`xtask/src/pins.rs`. It is the maintainer's own audit, not one by a third party.

| Check | What it covers | Command | Toolchain |
|---|---|---|---|
| `unsafe_code` lints | The binding forbids `unsafe` code; the library denies it outside `src/passwd.rs`, the lookup in the password database, whose every block clippy makes say why it is sound (`undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`); the dependencies' is counted in [SUPPLY_CHAIN.md](SUPPLY_CHAIN.md) | built in | |
| Tests | The layers, the conversions, persist, the readers of text and of env-style files; each condition of the masking of secrets and of the checks persist makes before writing | `pixi run test` | stable |
| Miri | The 45 integration tests of `layeredcfg` without undefined behaviour (not the binding, which calls the C API of Python) | `cargo xtask miri` | nightly |
| Branch coverage | Which branches the Rust tests do not take, to find conditions left untested | `cargo xtask coverage` | nightly |
| Kani | For every input up to 6 bytes, whatever is whitespace or a digit beyond ASCII: UTF-8 decoded as the standard library does, whitespace stripped only between characters, integers and booleans without panic or overflow, lists cut at their commas and nowhere else; for every ASCII input up to 6 bytes: whitespace stripped once and for all, integers as `str::parse` reads them; integers of 21 bytes at the edges of 64 bits | `cargo xtask kani` | Kani's own |
| Fuzzing | Each target an hour on a workstation, four hours in CI (see below) | `cargo xtask fuzz` | nightly |
| cargo deny | Known vulnerabilities, unmaintained and yanked crates, licenses, sources; for the fuzz workspace, vulnerabilities and sources | `cargo xtask deny` | stable |
| cargo vet | That every dependency is audited | `cargo xtask vet` | stable |
| cargo geiger | The `unsafe` code of the dependencies, of each crate of this project: a count to read, which fails nothing | `cargo xtask geiger` | stable |

The fuzz targets, and what they assert of every input:

| Target | Input | Asserted |
|---|---|---|
| `envfile` | An env-style file | No panic; the statements read lie in order inside the text, each with a name; a value written by `persist`'s quoting into the file reads back as it was, or the writing is refused (a quote never closed before it) |
| `numbers` | Any bytes, with a few runs of digits beyond ASCII set | No panic; an integer or a float read is written so that it reads back the same; `strip` is idempotent and the same on bytes and on text; the items of a list are not empty, have no comma and no whitespace around them |
| `config` | TOML text, a NUL, a value | No panic in reading the defaults, or in `get`, `shown`, `path`, `cast` and `set` of each setting with the value in the environment and as an override; a value `set` takes, `get` gives |

Not proved, and covered by the tests and the fuzzing instead: which characters beyond ASCII are whitespace or digits
(the binding asks the running Python, which no proof can see into; the Python tests compare every digit and whitespace
character of the Python they run on, and the characters next to them, with `int()`, `float()` and `str.strip()`),
`parse_float` (it builds the text it gives
`str::parse`, past what Kani can unroll), the reader of env-style files (a loop with some nine loops inside, past 16 GB
of memory at 4 bytes), and what is above the readers of text (the layers, persist).

### Running the Audit

| Command | Description |
|---|---|
| `cargo xtask setup [task…]` | Installs the pinned tools a task needs, all of them by default: cargo-deny, cargo-vet, cargo-geiger, cargo-fuzz, cargo-llvm-cov, Kani, and the nightly Rust with Miri and the LLVM tools |
| `cargo xtask <task>` | Runs one task: `deny`, `vet`, `geiger`, `miri`, `coverage`, `kani [proof…]` or `fuzz [seconds]` |
| `cargo xtask all` | Each task in that order, the cheap ones first, the next after one that fails too |
| `cargo xtask status` | The latest result of each task, when and at what commit it ran, and whether it still holds |
| `cargo xtask needed <task> [args]` | Whether the task is to run again, and why: exit 0 if it is, 1 if its results hold, 2 on an error |
| `cargo xtask outdated [--vulns]` | The pinned tools, nightly and GitHub actions due an update, and in a terminal, updates those agreed to: one with a known vulnerability at once, one whose latest release came out a month or more after it, a nightly a month old; each to a release at least a week old with no known vulnerability (`--vulns`: the vulnerabilities only) |
| `pixi run audit <task>` | The same, from the environment of pixi |

Each heavy task runs in a systemd user scope with a cap of memory and no swap, so that one going too far is killed and
not the machine: 4 GB (`RUST_XTASK_MEMORY` changes it), 17 GB for each Kani proof, and 1.25 GB for each fuzz target
running at once (`RUST_XTASK_FUZZ_MEMORY`; `RUST_XTASK_JOBS` for how many at once). Without a scope that caps, the
heavy tasks do not run, unless `RUST_XTASK_NO_SCOPE=1` (in CI, where each job has a machine of its own). One task runs
at a time, and a second started in another terminal is refused; with `RUST_XTASK_PARALLEL=1` another may run beside it
while their caps fit in the memory, Kani and the fuzzing never together. The other tasks check for a known
vulnerability in a pinned tool once a day, and for releases once a week, and say so in a line.

After a change to a proof, to what it proves or to a fuzz target, try it first with `cargo xtask kani --no-record`
(and `cargo xtask fuzz 600 --no-record`): in `all`, the fuzzing runs its hour even when Kani failed.

### Results

The latest run of each task is committed in `audit/<task>/`: `run.txt` (when it started, how long it took, the commit
it ran at, its arguments, the tools and how it went), `run.log` (all it wrote), `inputs.txt` (the files it read) and
what the task makes: the tables of `geiger`, `coverage/summary.txt`, the proofs' `kani/harnesses.txt` and each proof's
log, the fuzz targets' logs. When the latest run failed, the last that succeeded is kept in
`audit/<task>/last_succeeded/`.

A run is recorded only from a working tree with everything committed (untracked files too, `audit/` aside), so that it
is of a commit, and fails if the tree or HEAD changed while it ran. Its results hold, for `cargo xtask status`,
`needed` and the badges, as long as it succeeded, the tools pinned for it and its arguments are the same, none of the
files it reads changed, and, for `deny`, whose advisories change every day, for a day. Rust files are compared by
their tokens, so that a change to a comment or to the layout changes nothing; the others as they are. Which files a
task reads is `affects` in `xtask/src/main.rs`: documentation reads none of them, a task's own module in `xtask/src/tasks/`
only that task, the proofs only `kani`, the integration tests `miri` and `coverage`, the binding `geiger` and `coverage`
(the others do not build it), the Python tests only `coverage`; the crate's own version, which only the Python tests
see, for none.

`--if-needed` (`cargo xtask all --if-needed`) skips the tasks whose results hold. `--no-record` runs a task on any
working tree, into `target/audit/<task>/`, with `audit/` left as it is; some proofs only (`kani <proof…>`) or fuzzing
for less than an hour run only so. To record: commit the code, run on the clean tree, then commit `audit/`.

### CI

`.github/workflows/verify.yml` runs the tasks on GitHub, each in a job of its own, as on a workstation, with the same
pinned tools:

- At each release published on GitHub: `deny`, `vet`, `geiger`, `coverage` and `fuzz`, whose length is known before
  they start, and `miri`, which takes two minutes. The fuzzing runs four hours a target, all of them at once, from the
  corpus of the last run.
- Kani never by itself: it takes an hour or more, and one new proof can take it from seconds to hours (so can a test
  for Miri, which is measured again after a change to its tests, and leaves the release's tasks when it grows long).
  Kani runs on a workstation, or when the workflow is started by hand (Actions, Verify, Run workflow) with the task
  `kani`.
- Never on a push.

Only the tasks whose committed results no longer hold run. The last jobs commit the results to `audit/` on `main`, as
the results of the release's commit (results committed there since that commit are newer, and stay), and start
`.github/workflows/advisories.yml`, which writes the badges of the README. Advisories also runs daily (`cargo xtask
deny --no-record` against the advisories of that day, and the pinned tools' and actions' vulnerabilities) and weekly
(`cargo xtask outdated`).

The workflows can read the repository, and little more. The jobs that build and run the project's code, whose
dependencies' build scripts and macros run with them, cannot write. Three jobs can, and run none of it: `record` in
Verify commits `audit/` and nothing else; `badges` in Advisories pushes the badges to the `badges` branch, which holds
nothing else, after checking each is one of the few files and words a badge can be; and `badges` in Verify can only
start Advisories. Every action is pinned to a commit, and `cargo xtask outdated` checks them against their releases
and advisories.

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
  The lookup is this project's only `unsafe` code (`src/passwd.rs`, the C library's `getpwuid_r` and `getpwnam_r`):
  Miri and Kani cannot follow it into the C library, so the tests alone check it, against `getent` and `dscl`, with
  a buffer that has to grow; a name with a NUL is no user's, and the buffer stops growing at a mebibyte.
- **Messages have names and values as they are.** The core escapes only `"` and `\` in a value; Python's
  `invalid_value` message has the value's `repr`, but names of files and settings, and the core's messages about files
  and the defaults, are as they are. A message printed to a terminal can carry control characters, escape sequences
  among them.
- **The dependencies' `unsafe` code** (PyO3, libc, hashbrown and others) is theirs, counted and left to their audits:
  see [SUPPLY_CHAIN.md](SUPPLY_CHAIN.md).
