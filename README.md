# layeredconf
<p align="right">
    <strong>English</strong> | <a href="./README/README_CN.md">中文</a>
</p>

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

A lightweight layered configuration library with a C++ core and a Python binding, that takes each setting from a TOML
file of defaults, env-style files, the environment or the command line, and converts it to its default's type.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
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

```
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
- A C++17 library, and a Python binding whose errors are Python's kinds

## Prerequisites

- [pixi](https://pixi.sh), which brings the rest into the environment of this directory:
  - A C++17 compiler
  - CMake ≥3.26
  - [toml++](https://marzer.github.io/tomlplusplus/) ≥3.4
  - pybind11 ≥2.13
  - Python 3.12, which a build of the C++ library needs too: the table of Unicode digits and whitespace is written
    from its `unicodedata` by `tools/unicode_table.py`

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

The wheel links the C++ runtime statically, so it does not depend on the build environment's.

For C++, add this directory to a CMake project with `add_subdirectory` and link to the `layeredconf` target; toml++ and
a Python interpreter must be found when it is configured.

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
environment, which every program the process starts would inherit.

### C++

```cpp
#include <layeredconf/layeredconf.hpp>

layeredconf::Options options;
options.prefix = "APP_";
options.files = {"/srv/app/.env.local"};
auto config = layeredconf::Config::load("defaults.toml", options);
auto port = std::get<std::int64_t>(config.get("server.port"));
```

The library is C++17; it reads TOML with [toml++](https://marzer.github.io/tomlplusplus/). Values are a
`std::variant<bool, std::int64_t, double, std::string, std::vector<std::string>>`; failures throw
`layeredconf::Error`, whose `kind()` says which.

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
with it, unless a file read after the written one sets the same name.

## Testing

| Command | Description |
|---|---|
| `pixi run test` | The C++ tests and the Python tests |
| `pixi run test-cpp` | The C++ tests, built in `build/` |
| `pixi run test-python` | Builds and installs the wheel, then runs the Python tests |

## Compatibility

POSIX only for now (Linux, macOS).

Tested on:

- Ubuntu 24.04.4 LTS (x86_64)
  - GCC 15.3.0
  - CMake 4.4.4
  - Python 3.12.15
  - [toml++](https://github.com/marzer/tomlplusplus) 3.4.0
  - [pybind11](https://github.com/pybind/pybind11) 3.1.0

## License

Copyright 2026 Tianyu (Sky) Lu

The entire repository is licensed under the Apache License, Version 2.0 
(the "License"); you may not use any file in this repository except in 
compliance with the License. You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

## Author

Tianyu (Sky) Lu
