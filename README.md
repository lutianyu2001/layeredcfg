# layeredconf

Layered configuration with a C++ core and a Python binding. [中文说明](README.zh-CN.md)

A TOML file of defaults says which settings there are and the type of each: the type of its default. A setting's
value is taken from the highest layer that has one,

```
defaults (the TOML file)  <  env-style files  <  the environment  <  overrides (the command line)
```

and converted to its default's type. A value that does not convert is an error, never a guess: `ture` for a boolean
is a mistake, not false. A setting's name in the environment and in the files is a prefix and its key upper-cased,
dots made underscores: with the prefix `APP_`, `server.port` is `APP_SERVER_PORT`.

There is no schema to write in code: adding a setting is adding a line, with a comment, to the TOML file.

## Python

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

## C++

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

## What the layers take

| Default's type | A value as text |
|---|---|
| boolean | `1`, `true`, `yes`, `on`; `0`, `false`, `no`, `off` or nothing (any case) |
| integer | as Python's `int()`: a sign, digits, underscores between digits |
| float | as Python's `float()`: a point, an exponent, `inf`, `nan` |
| string | as it is |
| list of strings | comma-separated, each item stripped, empty ones left out |

A TOML default of another type (a date, a list of numbers) is refused when the file is read: no other layer could
write it. Floats are written back as Python writes them (`45.0`, `1e+16`), so a binding behaves as Python code did.

## Env-style files

`NAME=value` lines, as python-dotenv reads them for the most part: comments, `export `, unquoted values (a `#` after
whitespace begins a comment), `'single-quoted'` values taken as they are, `"double-quoted"` ones with escapes, quoted
values over several lines, and `${NAME}` or `${NAME:-default}` in unquoted and double-quoted values (the file's own
earlier value, else the environment's).

`persist` replaces the setting's line (an `export ` before it allowed), drops later lines of the same name, keeps every
other line as it was, and puts the file in place at once, with its permissions (0600 for a new one) from the moment it
is written.

## Building

Everything runs in the [pixi](https://pixi.sh) environment of this directory (compilers, CMake, toml++, pybind11):

```bash
pixi run test          # the C++ tests and the Python tests
pixi run wheel         # the wheel, in dist/
```

The wheel links the C++ runtime statically, so it does not depend on the build environment's.

## Status

Not published yet. POSIX only for now (Linux, macOS).
