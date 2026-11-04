# Supply Chain

What the Rust dependencies are, how the security audit checks them, and what is left to review. The dependencies are not vendored:
this is a library, and `Cargo.lock` (committed) pins every crate's version and checksum.

## Security Audit

| Command | What it checks | Configuration |
|---|---|---|
| `cargo xtask deny` | `cargo deny --workspace check` (xtask too): known vulnerabilities and unmaintained or yanked crates (RustSec), licenses, duplicate crates, sources; and for the fuzz workspace, vulnerabilities and sources | `deny.toml` |
| `cargo xtask vet` | `cargo vet`: that every crate is audited, by an audit of this project or one imported from another organisation | `supply-chain/` |
| `cargo xtask geiger` | `cargo geiger` of each crate of this project, `crates/layeredcfg` and `crates/layeredcfg-python`, a table each: how much `unsafe` code each dependency has and uses | none |

`cargo xtask setup` installs these tools at the versions `xtask/src/pins.rs` pins, and `cargo xtask outdated` says
when one is due an upgrade (a known vulnerability in it, or a latest release that came out a month or more after it,
with a release a week old to upgrade to), and so for the GitHub actions the workflows use. `xtask/` is a member of the
workspace whose only dependency, proc-macro2 (to compare Rust files by their tokens), is one PyO3's macros already
have, so it adds no crate to `Cargo.lock`; it is not a default member, so that `cargo build` and `cargo test` do not
build it, and it is not in the package. It asks crates.io, [OSV](https://osv.dev) and GitHub about the versions
through `curl`, and reads their JSON with a parser of its own.

The licenses allowed are Apache-2.0 (with or without the LLVM exception), MIT and Unicode-3.0, which are those the
dependencies have; crates come from crates.io only.

## Audits

`cargo vet` imports the audits of Mozilla, Google, the Bytecode Alliance, Zcash, ISRG, Embark Studios and Ariel OS.
Three crates are covered by them (cfg_aliases, heck, quote); the maintainer certified the other 26 at their locked
versions, for `safe-to-deploy`, in `supply-chain/audits.toml`. There are no exemptions. A crate added or updated
later is audited the same way (`cargo vet certify <crate> <version>`, or `<crate> <old> <new>` for the changes
between two versions).

## Unsafe Code

The two crates of this project, `layeredcfg` and `layeredcfg-python`, have `#![forbid(unsafe_code)]`: neither has any
`unsafe` code of its own. What `unsafe` code runs is the dependencies'. As `cargo geiger` counts it (used / present):

| Crate | Expressions | Why it has `unsafe` code |
|---|---|---|
| pyo3 0.30.1, pyo3-ffi 0.30.1 | 3179 / 3525, 1319 / 1319 | Python's C API |
| hashbrown 0.17.1, indexmap 2.14.2 | 1343 / 1612, 80 / 85 | Hash tables (the order of a TOML table) |
| libc 0.2.191 | 76 / 859 | The C library's declarations |
| nix 0.31.3 | 57 / 2054 | The password database (`~user`, and `~` without HOME) |
| once_cell 1.21.4 | 81 / 128 | PyO3's one-time initialisation |
| toml_parser 1.1.5, winnow 1.0.4 | 66 / 66, 66 / 70 | The TOML parser's hot paths |
| proc-macro2, syn, unicode-ident, pyo3-macros-backend | 14, 88, 4, 23 | Build time only (PyO3's macros) |

## Code Run at Build Time

These crates run code when the project is built: build scripts (`custom-build`) and procedural macros (`proc-macro`).
They are where a compromised dependency would act first.

```text
libc 0.2.191 [custom-build]
nix 0.31.3 [custom-build]
portable-atomic 1.15.0 [custom-build]
proc-macro2 1.0.107 [custom-build]
pyo3 0.30.1 [custom-build]
pyo3-ffi 0.30.1 [custom-build]
pyo3-macros 0.30.1 [proc-macro]
pyo3-macros-backend 0.30.1 [custom-build]
quote 1.0.47 [custom-build]
serde_core 1.0.229 [custom-build]
serde_derive 1.0.229 [proc-macro]
target-lexicon 0.13.5 [custom-build]
```

The list is made with:

```bash
cargo metadata --format-version 1 --locked | jq -r '
  .packages[]
  | select(.source != null)
  | select(any(.targets[]; .kind[] == "custom-build" or .kind[] == "proc-macro"))
  | "\(.name) \(.version) [\([.targets[].kind[]] | unique | map(select(. == "custom-build" or . == "proc-macro")) | join(","))]"' | sort
```

The project's own crates have no build script.

## The Fuzz Targets

`fuzz/` is a workspace of its own (the main one excludes it), built with the nightly Rust by `cargo fuzz`, with its
own `Cargo.lock`. It adds `libfuzzer-sys` (which builds libFuzzer, C++ code) and is not part of what a user of the
library builds: `cargo vet` does not cover it, and `cargo xtask deny` checks its vulnerabilities and sources only,
before it is built and run. Its `Cargo.lock` is started from the main one, so that the crates both have are at the
versions a user builds; after the main one changes, the same again:

```bash
cp Cargo.lock fuzz/ && (cd fuzz && cargo metadata --offline > /dev/null)
```
