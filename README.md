# layeredconf
<p align="right">
    <strong>English</strong> | <a href="./README/README_CN.md">中文</a>
</p>

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)<br>
[![audit](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Faudit.json)](SECURITY.md#ci)
[![pins](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Fpins.json)](xtask/src/pins.rs)

A lightweight layered configuration library with a Rust core and a Python binding, that takes each setting from a TOML
file of defaults, env-style files, the environment or the command line, and converts it to its default's type.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Security Audit](#security-audit)
- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Usage](#usage)
- [What the Layers Take](#what-the-layers-take)
- [Env-Style Files](#env-style-files)
- [Testing](#testing)
- [Compatibility](#compatibility)
- [License](#license)
- [Author](#author)

## Overview

A TOML file of defaults says which settings there are and the type of each: the type of its default. A setting's
value is taken from the highest layer that has one,

```text
defaults (the TOML file)  <  env-style files  <  the environment  <  overrides (the command line)
```

and converted to its default's type. A value that does not convert is an error, never a guess: `ture` for a boolean
is a mistake, not false. A setting's name in the environment and in the files is a prefix and its key upper-cased,
dots made underscores: with the prefix `APP_`, `server.port` is `APP_SERVER_PORT`.

## Features

- No schema to write in code: adding a setting is adding a line, with a comment, to the TOML file
- Four layers: defaults, env-style files, the environment, overrides
- Strict conversion to the default's type, as Python converts text
- Env-style files read as python-dotenv reads them for the most part
- A value written back into its file in place, every other byte kept (`persist`)
- Secrets shown masked; settings that only the files may set
- A Rust library without unsafe code of its own, and a Python binding whose errors are Python's kinds

## Security Audit

[![deny](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Fdeny.json)](https://github.com/lutianyu2001/layeredconf/actions/workflows/advisories.yml)
[![vet](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Fvet.json)](audit/vet)
[![geiger](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Fgeiger.json)](audit/geiger)
[![miri](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Fmiri.json)](audit/miri)
[![coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Fcoverage.json)](audit/coverage)
[![kani](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Fkani.json)](audit/kani)
[![fuzz](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredconf%2Fbadges%2Ffuzz.json)](audit/fuzz)

The maintainer's own security audit of the code, not one by a third party: it says what was checked and how, not that
nothing is wrong. The latest results of each check are committed in [`audit/`](audit) with the commit they ran at, and
the badges above say whether each still holds for the code on `main`. The checks whose length is known beforehand
(deny, vet, geiger, coverage and the fuzzing) run again on GitHub at each release; Kani and Miri are run by hand. How
to run them, and what each covers, is in [SECURITY.md](SECURITY.md); the dependencies are in
[SUPPLY_CHAIN.md](SUPPLY_CHAIN.md).

- **Review against a threat model.** Text in every layer, from any bytes; secrets in what is shown, in the messages of
  errors and in Python's exceptions; `persist` writing files that other users of the machine can reach (permissions,
  links, a file half written) and several writers at once; `${NAME}` making a small file's values huge. What was found
  is fixed, or written down with what it leaves open, in SECURITY.md.
- **`unsafe` code.** None in either crate of this project, which `#![forbid(unsafe_code)]`; the dependencies' is
  counted by cargo geiger, and PyO3, which calls Python's C API, has most of it.
- **Dependencies.** 29 crates, from crates.io only, checked by cargo deny for known vulnerabilities, unmaintained or
  yanked crates, licenses and sources. In cargo vet, 3 are covered by audits imported from other organisations and
  the other 26 by the maintainer's own certification, none exempted. They are not vendored: this is a library,
  and `Cargo.lock` pins every version and checksum.
- **Miri.** The 44 tests of the Rust library (the layers, persist, env-style files, the readers of text) run under Miri
  without undefined behaviour; not the binding, which calls Python's C API.
- **Fuzzing.** Three targets, an hour each on a workstation and four hours each at a release: `envfile` (statements in
  order inside the text, values written by `persist` read back the same), `numbers` (integers and floats written so
  that they read back the same, `strip` idempotent, list items without commas or whitespace around them) and `config`
  (reading defaults and every setting's `get`, `shown`, `path` and `set` without a panic).
- **Kani.** 13 proofs: for every input up to 6 bytes, UTF-8 is decoded as the standard library does, whitespace is
  stripped once and for all and only between characters, integers are read without panic or overflow and as
  `str::parse` reads them, booleans without panic, and lists are cut at their commas and nowhere else; and integers of
  21 bytes are read exactly at the edges of 64 bits. Each proof ran alone, compiled before, on 2026-11-04 at 5970f0e,
  on an AMD Ryzen 9 5950X with 32 GB:

  | Proof | Time | Peak memory |
  |---|---|---|
  | `decode_past_the_end_is_no_character` | 1 s | 96 MiB |
  | `decode_reads_utf8_as_the_standard_library_does` | 55 s | 1.3 GiB |
  | `parse_bool_does_not_panic` | 20 s | 669 MiB |
  | `parse_int_does_not_panic_or_overflow` | 50 s | 1.7 GiB |
  | `parse_int_is_exact_at_the_largest_i64` | 16 min 58 s | 9.7 GiB |
  | `parse_int_is_exact_at_the_smallest_i64` | 15 min 36 s | 9.7 GiB |
  | `parse_int_is_exact_past_u64` | 15 min 40 s | 10.8 GiB |
  | `parse_int_is_exact_past_u64_negative` | 14 min 5 s | 10.3 GiB |
  | `parse_int_reads_plain_numbers_as_the_standard_library_does` | 1 min 24 s | 2.5 GiB |
  | `space_at_stays_in_the_text` | 1 s | 100 MiB |
  | `split_list_cuts_at_the_commas_and_nowhere_else` | 54 s | 2.1 GiB |
  | `strip_str_cuts_at_the_boundaries_of_characters` | 24 s | 1.1 GiB |
  | `strip_takes_whitespace_off_once_and_for_all` | 26 s | 907 MiB |
- **Branch coverage.** The Rust tests take 85.81 % of the 310 branches of the Rust code, 90.48 % of the library's 294
  (`audit/coverage/summary.txt`); the binding's 16 are left to the Python tests.
- **Two reviews.** Every change went through a code review, and the most sensitive code (the reader of env-style files,
  `persist`, the masking of secrets) was read whole by separate reviewers who were given the threat model, not the
  findings.

What this shows: this project's own code has no memory errors, being Rust without `unsafe` (unless the compiler or a
dependency's `unsafe` code is wrong), and none of the checks above found a panic, an overflow or undefined behaviour. What it does not: Rust does not rule out mistakes
of logic; the dependencies' `unsafe` code is left to them and their audits; Kani proves only the inputs within its
bounds, and fuzzing tries only some. Not proved, and covered by the tests and the fuzzing alone: `parse_float`, the
reader of env-style files, and what is above the readers of text (the layers, `persist`).

## Prerequisites

- [rustup](https://rustup.rs), which installs the Rust version `rust-toolchain.toml` pins (1.99.1) when it is first
  needed
- [pixi](https://pixi.sh), which brings the rest into the environment of this directory:
  - [maturin](https://www.maturin.rs) ≥1.9
  - Python 3.12, which a build of the Rust library needs too: the table of Unicode digits and whitespace is written
    from its `unicodedata` by `tools/unicode_table.py` (the Python `PYO3_PYTHON` names, else `PYTHON`, else
    `python3`)

## Installation

layeredconf is not published yet, so it is built from source.

1. Clone the repository:

   ```bash
   git clone https://github.com/lutianyu2001/layeredconf.git
   cd layeredconf
   ```

2. Build the wheel, into `dist/`:

   ```bash
   pixi run wheel
   ```

3. Install it into your Python:

   ```bash
   pip install dist/layeredconf-*.whl
   ```

For Rust, depend on the crate in `crates/layeredconf` by its path or by this repository's git URL; its build needs a
Python 3, as above.

## Usage

### Python

```python
from layeredconf import Config, find_upward

config = Config(
    "defaults.toml",
    prefix="APP_",
    files=[find_upward(".env.local"), "/srv/app/.env.local"],  # a later file wins
    root="/srv/app",                    # what a relative path is relative to
    file_only={"log.max_dir_size_mib"}, # not to be overridden on the command line
    persisted={"subagent.timeout_minutes"},
    secret={"notify.token"},
)

config.server.port                   # 8420, an int, as the default is
config.get("server.port")
config.set("server.port", "8421")    # an override, converted at once
config.path("data.dir")              # ~ expanded, a relative path joined to the root
config.source("server.port")         # "override", "env", "file" or "default"
config.shown("notify.token")         # "***" when it has a value
config.problems()                    # every value of the environment and the files that does not convert
config.persist("subagent.timeout_minutes", 45)  # written into the file for good, in force at once
```

The errors are Python's kinds too: `UnknownKey` is a `KeyError`, `InvalidValue` and `FileOnlyKey` are `ValueError`s,
`NotPersisted` is a `KeyError`, all of them `ConfigError`s; an unknown attribute is an `AttributeError`. Their messages
are templates a program can replace with its own words (`messages=`, see `layeredconf.MESSAGES`).

The environment is `os.environ` (or the mapping given as `environ=`), read when a value is asked for: a variable set by
Python code holds at once. The files' values are read into the configuration only, never into the process's
environment, which every program the process starts would inherit, and `persist` does not change the environment
either (see [Env-Style Files](#env-style-files)).

### Rust

```rust,no_run
use layeredconf::{Config, Options, Value};

fn main() -> Result<(), layeredconf::Error> {
    let options = Options {
        prefix: "APP_".to_owned(),
        files: vec!["/srv/app/.env.local".into()],
        ..Options::default()
    };
    let mut config = Config::load("defaults.toml", options)?;
    let Value::Int(port) = config.get("server.port")? else { unreachable!() };
    config.set("server.port", "8421")?;
    println!("{port}");
    Ok(())
}
```

Values are a `layeredconf::Value` (`Bool`, `Int`, `Float`, `Str`, `List`); what a caller gives is a `Raw`, which text
(`&str`, `String`, `OsString`), the other types and a `Value` convert into. Text from the environment and the files is
an `OsString`, which need not be UTF-8. Failures are a `layeredconf::Error`, whose `kind()` says which. The environment
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
and in the files alike. Which characters they are comes from the `unicodedata` of the Python the library is built with,
so a wheel has the Unicode version of the Python it is for. A value given from Python that is not text is converted as
Python converts it (`int()`, `float()`, `bool()`, `str()`, `list()`), a `Decimal` or `None` included. A TOML default of
another type (a date, a list of numbers) is refused when the file is read: no other layer could write it. Floats are
written back as Python writes them (`45.0`, `1e+16`), so a binding behaves as Python code did.

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
  - Python 3.12.15
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
