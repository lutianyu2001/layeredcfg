# layeredcfg
<p align="right">
    <strong>English</strong> | <a href="https://github.com/lutianyu2001/layeredcfg/blob/main/README/README_CN.md">中文</a>
</p>

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Release](https://img.shields.io/github/v/release/lutianyu2001/layeredcfg)](https://github.com/lutianyu2001/layeredcfg/releases/latest)
[![Crates.io Version](https://img.shields.io/crates/v/layeredcfg)](https://crates.io/crates/layeredcfg)
[![PyPI Version](https://img.shields.io/pypi/v/layeredcfg)](https://pypi.org/project/layeredcfg/)<br>
[![CI](https://github.com/lutianyu2001/layeredcfg/actions/workflows/ci.yml/badge.svg)](https://github.com/lutianyu2001/layeredcfg/actions/workflows/ci.yml)
[![audit](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Faudit.json)](https://github.com/lutianyu2001/layeredcfg/blob/main/SECURITY.md#ci)
[![pins](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fpins.json)](https://github.com/lutianyu2001/layeredcfg/blob/main/xtask/src/pins.rs)

A lightweight layered configuration library with a Rust core and a Python binding, that takes each setting from a TOML
file of defaults, the user's TOML files, env-style files, the environment or the command line, and converts it to its
default's type.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Security Audit](#security-audit)
- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Usage](#usage)
- [What the Layers Take](#what-the-layers-take)
- [The User's TOML Files](#the-users-toml-files)
- [Env-Style Files](#env-style-files)
- [Turning Layers Off](#turning-layers-off)
- [Testing](#testing)
- [Compatibility](#compatibility)
- [License](#license)
- [Author](#author)

## Overview

A TOML file of defaults says which settings there are and the type of each: the type of its default. A setting's
value is taken from the highest layer that has one,

```text
defaults (the TOML file)  <  the user's TOML files  <  env-style files  <  the environment  <  overrides (the command line)
```

and converted to its default's type. A value that does not convert is an error, never a guess: `ture` for a boolean
is a mistake, not false. The user's TOML files have the keys of the defaults. A setting's name in the environment and
in the env-style files is a prefix and its key upper-cased, dots made underscores: with the prefix `APP_`,
`server.port` is `APP_SERVER_PORT`. A program that has no use for a layer turns it off.

## Features

- No schema to write in code: adding a setting is adding a line, with a comment, to the TOML file
- Five layers: defaults, the user's TOML files, env-style files, the environment, overrides; any but the first and
  the last can be turned off
- Strict conversion to the default's type, as Python converts text
- Env-style files read as python-dotenv reads them for the most part
- A value written back into its file in place, every other byte kept (`persist`)
- Secrets shown masked; settings that only the files may set
- A Rust library whose only `unsafe` code is the lookup of a home directory in the password database, and a Python
  binding without any, whose errors are Python's kinds

## Security Audit

[![review](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Freview.json)](https://github.com/lutianyu2001/layeredcfg/tree/main/audit/review)
[![deny](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fdeny.json)](https://github.com/lutianyu2001/layeredcfg/actions/workflows/advisories.yml)
[![vet](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fvet.json)](https://github.com/lutianyu2001/layeredcfg/tree/main/audit/vet)
[![geiger](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fgeiger.json)](https://github.com/lutianyu2001/layeredcfg/tree/main/audit/geiger)
[![miri](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fmiri.json)](https://github.com/lutianyu2001/layeredcfg/tree/main/audit/miri)
[![asan](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fasan.json)](https://github.com/lutianyu2001/layeredcfg/tree/main/audit/asan)
[![coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fcoverage.json)](https://github.com/lutianyu2001/layeredcfg/tree/main/audit/coverage)
[![kani](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fkani.json)](https://github.com/lutianyu2001/layeredcfg/tree/main/audit/kani)
[![fuzz](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Ffuzz.json)](https://github.com/lutianyu2001/layeredcfg/tree/main/audit/fuzz)

The maintainer's own security audit of the code, not one by a third party: it says what was checked and how, not that
nothing is wrong. The latest results of each check are committed in [`audit/`](https://github.com/lutianyu2001/layeredcfg/tree/main/audit) with the commit they ran at, and
the badges above say whether each still holds for the code on `main`. The checks whose length is known beforehand
(review, deny, vet, geiger, AddressSanitizer, coverage and the fuzzing), and Miri, which takes minutes, run again on
GitHub at each release;
Kani is run by hand. How
to run them, and what each covers, is in [SECURITY.md](https://github.com/lutianyu2001/layeredcfg/blob/main/SECURITY.md); the dependencies are in
[SUPPLY_CHAIN.md](https://github.com/lutianyu2001/layeredcfg/blob/main/SUPPLY_CHAIN.md).

- **Review against a threat model.** Text in every layer, from any bytes; secrets in what is shown, in the messages of
  errors and in Python's exceptions; `persist` writing files that other users of the machine can reach (permissions,
  links, a file half written) and several writers at once; `${NAME}` making a small file's values huge. What was found
  is fixed, or written down with what it leaves open, in SECURITY.md.
- **`unsafe` code.** The binding has none (`#![forbid(unsafe_code)]`). The library has it in one module alone,
  `src/passwd.rs`, which looks a home directory up in the password database for `~` and `~name` (the C library's
  `getpwuid_r` and `getpwnam_r`, which Rust's standard library has no lookup by name for). Each block makes one call
  or reads one pointer and says why that is sound. The call, the reading of what it filled in (under a stated
  contract) and the retries are apart: the retries are safe code, tested against every answer the C library can give
  and proved by Kani for every sequence of them; the reading is tested under Miri with entries made as the C library
  makes them; the real calls are tested against `getent` (Linux) and `dscl` (macOS) on glibc, musl and macOS, and run
  under AddressSanitizer. Two reviewers read the module, and the review is recorded with the file's hash, so that a
  change makes it due again (`cargo xtask review`). The dependencies' `unsafe` code is counted by cargo geiger, and
  PyO3, which calls Python's C API, has most of it.
- **Dependencies.** 25 crates, from crates.io only, checked by cargo deny for known vulnerabilities, unmaintained or
  yanked crates, licenses and sources. In cargo vet, 2 are covered by audits imported from other organisations and
  the other 23 by the maintainer's own certification, none exempted. They are not vendored: this is a library,
  and `Cargo.lock` pins every version and checksum.
- **Miri.** The 45 tests of the Rust library (the layers, persist, env-style files, the readers of text) run under Miri
  without undefined behaviour; not the binding, which calls Python's C API.
- **Fuzzing.** Three targets, an hour each on a workstation and four hours each at a release: `envfile` (statements in
  order inside the text, values written by `persist` read back the same), `numbers` (integers and floats written so
  that they read back the same, digits beyond ASCII included, `strip` idempotent, list items without commas or
  whitespace around them) and `config`
  (reading defaults and every setting's `get`, `shown`, `path` and `set` without a panic).
- **Kani.** 13 proofs: for every input up to 6 bytes, whatever the characters beyond ASCII are taken for, UTF-8 is
  decoded as the standard library does, whitespace is stripped only between characters, integers and booleans are read
  without panic or overflow, and lists are cut at their commas and nowhere else; for every ASCII input up to 6 bytes,
  whitespace is stripped once and for all and integers are read as `str::parse` reads them; and integers of 21 bytes
  are read exactly at the edges of 64 bits. Each proof ran alone, compiled before, on 2026-11-05 at 1466eab, on an AMD
  Ryzen 9 5950X with 32 GB:

  | Proof | Time | Peak memory |
  |---|---|---|
  | `decode_past_the_end_is_no_character` | 1 s | 95 MiB |
  | `decode_reads_utf8_as_the_standard_library_does` | 56 s | 1.3 GiB |
  | `parse_bool_does_not_panic` | 19 s | 614 MiB |
  | `parse_int_does_not_panic_or_overflow` | 43 s | 1.3 GiB |
  | `parse_int_is_exact_at_the_largest_i64` | 19 min 0 s | 9.6 GiB |
  | `parse_int_is_exact_at_the_smallest_i64` | 16 min 13 s | 9.6 GiB |
  | `parse_int_is_exact_past_u64` | 14 min 28 s | 9.6 GiB |
  | `parse_int_is_exact_past_u64_negative` | 14 min 19 s | 9.6 GiB |
  | `parse_int_reads_plain_numbers_as_the_standard_library_does` | 59 s | 2.2 GiB |
  | `space_at_stays_in_the_text` | 1 s | 103 MiB |
  | `split_list_cuts_at_the_commas_and_nowhere_else` | 53 s | 2.1 GiB |
  | `strip_str_cuts_at_the_boundaries_of_characters` | 23 s | 1.1 GiB |
  | `strip_takes_whitespace_off_once_and_for_all` | 19 s | 1.0 GiB |
- **Branch coverage.** The Rust tests take 85.81 % of the 310 branches of the Rust code, 90.48 % of the library's 294
  (`audit/coverage/summary.txt`); the binding's 16 are left to the Python tests.
- **Two reviews.** Every change went through a code review, and the most sensitive code (the reader of env-style files,
  `persist`, the masking of secrets) was read whole by separate reviewers who were given the threat model, not the
  findings.

What this shows: this project's own code has no memory errors, being Rust whose `unsafe` code is the two calls into
the password database (unless the compiler, the C library or a dependency's `unsafe` code is wrong), and none of the checks above found a panic, an overflow or undefined behaviour. What it does not: Rust does not rule out mistakes
of logic; the dependencies' `unsafe` code is left to them and their audits; neither Miri nor Kani can follow the calls
into the C library, which the tests alone cover; Kani proves only the inputs within its
bounds, and fuzzing tries only some. Not proved, and covered by the tests and the fuzzing alone: which characters
beyond ASCII are whitespace or digits (the running Python's, which the Python tests compare for every digit and
whitespace character it has), `parse_float`, the reader of env-style files, and what is above the readers of text
(the layers, `persist`).

## Prerequisites

- Python 3.10 or later for the Python package, Rust 1.88 or later for the crate
- To build from source and run the tests:
  - [rustup](https://rustup.rs), which installs the Rust version `rust-toolchain.toml` pins (1.99.1) when it is first
    needed
  - [pixi](https://pixi.sh), which brings the rest into the environment of this directory:
    - [maturin](https://www.maturin.rs) ≥1.9
    - Python 3.12, for the binding and its tests

## Installation

### From PyPI (Recommended)

```bash
pip install layeredcfg
```

The wheels are for Linux (x86_64 and aarch64, glibc and musl), one for every Python from 3.10. Elsewhere (macOS) pip
builds the package from its source distribution, which needs Rust 1.88 or later.

### From crates.io

```bash
cargo add layeredcfg
```

### Building from Source

1. Clone the repository:

   ```bash
   git clone https://github.com/lutianyu2001/layeredcfg.git
   cd layeredcfg
   ```

2. Build the wheel, into `dist/`:

   ```bash
   pixi run wheel
   ```

3. Install it into your Python:

   ```bash
   pip install dist/layeredcfg-*.whl
   ```

For the crate from this repository instead of crates.io, depend on `crates/layeredcfg` by its path or by this
repository's git URL.

The files published to PyPI, the source distribution and the wheels of every platform, are built from a release's tag
with `pixi run release-files <tag>`, into `dist/`; it needs [zig](https://ziglang.org), the Rust targets of the
wheels (`rustup target add`) and [cargo-about](https://github.com/EmbarkStudios/cargo-about) 0.9.3, which collects the
license texts of the crates compiled into the wheels into each of them. It fails if a file names a path of the machine
that built it, or lacks those license texts.

## Usage

### Python

```python
from pathlib import Path

from layeredcfg import Config, find_upward

config = Config(
    "defaults.toml",
    prefix="APP_",
    user_files=["/etc/app/config.toml", Path.home() / ".config/app/config.toml"],  # a later file wins
    files=[find_upward(".env.local"), "/srv/app/.env.local"],  # a later file wins
    root="/srv/app",                    # what a relative path is relative to
    file_only={"log.max_dir_size_mib"}, # not to be overridden on the command line
    persisted={"subagent.timeout_minutes"},
    secret={"notify.token"},
    disabled=set(),                     # layers turned off: "user", "file", "env"
)

config.server.port                   # 8420, an int, as the default is
config.get("server.port")
config.set("server.port", "8421")    # an override, converted at once
config.path("data.dir")              # ~ expanded, a relative path joined to the root
config.source("server.port")         # "override", "env", "file", "user" or "default"
config.shown("notify.token")         # "***" when it has a value
config.problems()                    # every value of the environment and the files that does not convert
config.persist("subagent.timeout_minutes", 45)  # written into the file for good, in force at once
```

The errors are Python's kinds too: `UnknownKey` is a `KeyError`, `InvalidValue` and `FileOnlyKey` are `ValueError`s,
`NotPersisted` is a `KeyError`, `BadUserFile` is a `ValueError`, all of them `ConfigError`s; an unknown attribute is an `AttributeError`. Their messages
are templates a program can replace with its own words (`messages=`, see `layeredcfg.MESSAGES`).

The environment is `os.environ` (or the mapping given as `environ=`), read when a value is asked for: a variable set by
Python code holds at once. The files' values are read into the configuration only, never into the process's
environment, which every program the process starts would inherit, and `persist` does not change the environment
either (see [Env-Style Files](#env-style-files)).

### Rust

```rust,no_run
use layeredcfg::{Config, Options, Source, Value};

fn main() -> Result<(), layeredcfg::Error> {
    let options = Options {
        prefix: "APP_".to_owned(),
        user_files: vec!["/etc/app/config.toml".into()],
        files: vec!["/srv/app/.env.local".into()],
        disabled: [Source::Env].into(), // the environment's variables are no settings here
        ..Options::default()
    };
    let mut config = Config::load("defaults.toml", options)?;
    let Value::Int(port) = config.get("server.port")? else { unreachable!() };
    config.set("server.port", "8421")?;
    println!("{port}");
    Ok(())
}
```

Values are a `layeredcfg::Value` (`Bool`, `Int`, `Float`, `Str`, `List`); what a caller gives is a `Raw`, which text
(`&str`, `String`, `OsString`), the other types and a `Value` convert into. Text from the environment and the files is
an `OsString`, which need not be UTF-8. Failures are a `layeredcfg::Error`, whose `kind()` says which. The environment
is the process's unless `Options::environment` gives another (`Environment::new`), and it is only ever read. TOML is
read with the [toml](https://crates.io/crates/toml) crate.

## What the Layers Take

| Default's type | A value as text |
|---|---|
| boolean | `1`, `true`, `yes`, `on`; `0`, `false`, `no`, `off` or nothing (any case) |
| integer | as Python's `int()`: whitespace around, a sign, digits, underscores between digits |
| float | as Python's `float()`: as for an integer, and a point, an exponent, `inf`, `nan` |
| string | as it is |
| list of strings | comma-separated, each item stripped, empty ones left out |

Text must be UTF-8 (a value that is not is an invalid value). Digits and whitespace are Python's, beyond ASCII too:
`８４２０` is 8420, as it is for `int()`, and a no-break space or an ideographic space (U+3000) is whitespace, in values
and in the files alike. Beyond ASCII, the binding asks the running Python (`str.isspace()`, `unicodedata.decimal()`), so
the one wheel of each platform reads text with the Unicode version of whichever Python runs it. In Rust, whitespace
beyond ASCII is Unicode's (`char::is_whitespace`, the same characters) and there is no digit beyond ASCII, as for
`str::parse`, unless `text::set_beyond_ascii` says otherwise. A value given from Python that is not text is converted as
Python converts it (`int()`, `float()`, `bool()`, `str()`, `list()`), a `Decimal` or `None` included. A TOML default of
another type (a date, a list of numbers) is refused when the file is read: no other layer could write it. Floats are
written back as Python writes them (`45.0`, `1e+16`), so a binding behaves as Python code did.

## The User's TOML Files

The defaults are the program's; the user writes the settings to change in TOML files of the same keys, which may leave
any of them out:

```toml
[server]
port = 9000
```

A file that is not there is passed over, and a later file wins over an earlier one (a system-wide file, then the
user's own). A value is converted to its default's type as the text of the other layers is (`"9000"` for an integer is
9000, `"no"` for a boolean is false). The files are read when the configuration is made and by `reload_files`, and a
mistake in one is an error then, a `BadUserFile` whose message says the file, the line and the column: TOML that does
not parse, a key or a table that is no setting, a value that does not convert (a secret's shown as `***`). What a
caller read itself is given with `set_user_text`; `user_values` is what was read.

## Env-Style Files

`NAME=value` lines, as python-dotenv reads them for the most part: comments, `export `, unquoted values (a `#` after
whitespace begins a comment, so `NAME= # note` is empty), `'single-quoted'` values taken as they are, `"double-quoted"`
ones with escapes, quoted values over several lines, and `${NAME}` or `${NAME:-default}` in unquoted and double-quoted
values (the file's own earlier value, else the environment's; single quotes keep them as they are, where python-dotenv
expands them). A byte order mark and `\r\n` or `\r` line ends are read too. Whitespace is Python's here as well, as it
is for python-dotenv's `\s`: an ideographic space before a `#` begins a comment as a space does.

Write names without quotes. A quoted name (`'NAME'=value`, which python-dotenv reads as `NAME`) is not standard, and
this library does not read it and is not going to: the quotes are part of the name here, which no setting has, so the
line does nothing, and nothing says so.

`persist` replaces the setting's assignment (an `export ` before it allowed, all the lines of a quoted value), drops
later ones of the same name, keeps every other byte as it was, and puts the file in place at once, with its permissions
(0600 for a new one) from the moment it is written; a symbolic link stays and its target is written. Processes writing
at the same time do not corrupt the file, but the last one's file is what stays. What the configuration reads changes
with it, unless a file read after the written one sets the same name. Where the environment has the variable, which wins
over the files, the configuration takes the written value instead whenever the environment has the value it had when
`persist` wrote (set it to another value and that one is read); the environment itself is not changed, so other code in
the process and the programs it starts still see the old value. A quote in the file that is never closed makes the
reader leave its line out; when the value written has that quote, which would close it and take in the lines between,
`persist` refuses with an error and leaves the file as it was.

## Turning Layers Off

`disabled` (`Options::disabled` in Rust) names the layers a program has no use for: `user`, `file` and `env`
(`Source::User`, `Source::File`, `Source::Env`). None of their values is read, and their mistakes are no problems. The
environment is still read for what is not a setting: `HOME` for `~` in a path, and `${NAME}` in an env-style file.
`persist`, which writes an env-style file, is refused while those are off. The defaults and the overrides cannot be
turned off; a program that takes no overrides calls no `set`.

## Testing

| Command | Description |
|---|---|
| `pixi run test` | The Rust tests and the Python tests |
| `pixi run test-rust` | The Rust tests |
| `pixi run test-python` | Builds and installs the wheel, then runs the Python tests |
| `pixi run lint` | rustfmt and clippy |

## Compatibility

POSIX only for now (Linux, macOS).

Tested on:

- Ubuntu 24.04.4 LTS (x86_64)
  - Rust 1.99.1
  - Python 3.12.15, and the manylinux wheel on Python 3.15.0
  - [maturin](https://github.com/PyO3/maturin) 1.15.0
  - [PyO3](https://github.com/PyO3/pyo3) 0.30.1
  - [toml](https://crates.io/crates/toml) 1.1.8

## License

Copyright 2026 Tianyu (Sky) Lu

The entire repository is licensed under the Apache License, Version 2.0 
(the "License"); you may not use any file in this repository except in 
compliance with the License. You may obtain a copy of the License at

```plaintext
http://www.apache.org/licenses/LICENSE-2.0
```

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

## Author

Tianyu (Sky) Lu
