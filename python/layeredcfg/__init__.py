"""layeredcfg: layered configuration.

A TOML file of defaults says which settings there are and the type of each, that of its default. A setting's value is
taken from the highest layer that has one,

    defaults (the TOML file) < env-style files < the environment < overrides (the command line)

and converted to its default's type; a value that does not convert is an error, not a guess. The core is Rust
(`layeredcfg._core`); this module is its Python face: attribute access (`config.server.port`), Python's exceptions,
and the messages they carry, which a program can give in its own words.
"""

from __future__ import annotations

import os
from collections.abc import Callable, Iterable, Mapping, MutableMapping
from pathlib import Path
from typing import Any

from . import _core

__version__ = _core.__version__

__all__ = [
    "MESSAGES",
    "Config",
    "ConfigError",
    "FileOnlyKey",
    "InvalidValue",
    "NotPersisted",
    "Section",
    "UnknownKey",
    "find_upward",
]

# The messages of the errors, each a str.format template or a callable, given the fields as keywords: key, raw (the
# value as given), expected (the type's name), variable (the setting's name in the environment and the files), file
# (the name of the file written into, or of the file a problem is in), allowed and allowed_keys (the settings persist
# may write, joined and as a list), error (a problem's invalid_value message). The raw value of a secret setting is
# given as the text *** (whose repr is *** too).
MESSAGES: dict[str, str | Callable[..., str]] = {
    "unknown_key": "unknown setting: {key}",
    "invalid_value": "invalid value for {key}: {raw!r} (expected {expected})",
    "file_only": "{key} cannot be set on the command line: set {variable} in {file} or in the environment",
    "not_persisted": "{key} cannot be changed here (these can: {allowed})",
    "problem_env": "{error}, written in the environment variable {variable}",
    "problem_file": "{error}, written in {variable} of {file}",
}

class _Masked(str):
    """A secret setting's value in a message: the text ***, also as its repr."""

    def __repr__(self) -> str:
        return "***"


_MASKED = _Masked("***")

# The core's names of the types, as Python names them.
_TYPES = {"bool": "bool", "int": "int", "float": "float", "string": "str", "list": "list"}


class ConfigError(Exception):
    """A setting asked for, given or written wrongly, or defaults that cannot be read. `key` is the setting."""

    def __init__(self, message: str, key: str = "") -> None:
        super().__init__(message)
        self.key = key

    def __str__(self) -> str:  # not KeyError's, which would quote the message
        return str(self.args[0])


class UnknownKey(ConfigError, KeyError):
    """No such setting."""


class InvalidValue(ConfigError, ValueError):
    """A value that does not convert to the setting's type: `raw`, `expected`."""

    def __init__(self, message: str, key: str, raw: Any, expected: str) -> None:
        super().__init__(message, key)
        self.raw = raw
        self.expected = expected


class FileOnlyKey(ConfigError, ValueError):
    """An override of a setting that only the files and the environment may change."""


class NotPersisted(ConfigError, KeyError):
    """`persist` of a setting that is not to be written."""


class Section:
    """Attribute view of one TOML table: `config.server`, then `config.server.port`."""

    __slots__ = ("_config", "_prefix")

    def __init__(self, config: Config, prefix: str) -> None:
        self._config = config
        self._prefix = prefix

    def __getattr__(self, name: str) -> Any:
        return self._config._resolve(self._prefix + name)

    def __repr__(self) -> str:
        return f"<config section {self._prefix.rstrip('.')!r}>"


def find_upward(name: str, start: str | os.PathLike[str] | None = None) -> Path | None:
    """The first file `name` in `start` (the current directory) or a directory above it."""
    return _core.find_upward(name, os.fspath(start) if start is not None else os.getcwd())


class Config:
    """The settings of a program. `defaults` is the TOML file; `prefix` begins every setting's name in the environment
    and the files; `files` are env-style files (`NAME=value` lines), a later one winning; `write_file` is where `persist`
    writes (else the last of `files`); `root` is what a relative `path` is relative to. `file_only` settings cannot be
    overridden, `persisted` ones can be written with `persist`, `secret` ones are masked by `shown` and in the messages
    of errors. `environ` is the environment (`os.environ`); `messages` replaces messages of `MESSAGES` by kind.

    Settings are read as attributes (`config.server.port`) or with `get`; a table's name that is also a method's (`get`,
    `path`, ...) is reached with `get` only."""

    def __init__(
        self,
        defaults: str | os.PathLike[str],
        *,
        prefix: str = "",
        files: Iterable[str | os.PathLike[str] | None] = (),
        write_file: str | os.PathLike[str] | None = None,
        root: str | os.PathLike[str] | None = None,
        file_only: Iterable[str] = (),
        persisted: Iterable[str] = (),
        secret: Iterable[str] = (),
        environ: MutableMapping[str, str] | None = None,
        messages: Mapping[str, str | Callable[..., str]] | None = None,
    ) -> None:
        self._messages = {**MESSAGES, **(messages or {})}
        environment = os.environ if environ is None else environ
        given = [os.fspath(f) for f in files if f]
        # The file `persist` writes into, named by the message of an override that is refused.
        self._written = Path(write_file) if write_file else Path(given[-1]) if given else None
        self._persisted = sorted(persisted)
        self._file_only = set(file_only)
        self._secret = set(secret)
        self._core = self._call(
            _core.Config,
            os.fspath(defaults),
            prefix=prefix,
            files=given,
            write_file=os.fspath(write_file) if write_file else "",
            root=os.fspath(root) if root else "",
            file_only=self._file_only,
            persisted=set(self._persisted),
            secret=self._secret,
            get=environment.get,
        )

    # -- the settings

    def __getattr__(self, name: str) -> Any:
        if name.startswith("_"):
            raise AttributeError(name)
        return self._resolve(name)

    def _resolve(self, dotted: str) -> Any:
        if self._core.has(dotted):
            return self.get(dotted)
        if self._core.is_section(dotted):
            return Section(self, dotted + ".")
        raise AttributeError(self._message("unknown_key", key=dotted))

    def keys(self) -> list[str]:
        """Every setting, in the order of the TOML file."""
        return list(self._core.keys())

    def default(self, key: str) -> Any:
        return self._call(self._core.default, key)

    def type(self, key: str) -> str:
        """The setting's type: bool, int, float, str or list."""
        return _TYPES[self._call(self._core.type, key)]

    def variable(self, key: str) -> str:
        """The setting's name in the environment and the files: the prefix and the key upper-cased, dots to underscores."""
        return self._core.variable(key)

    def get(self, key: str) -> Any:
        """The value from the highest layer that has one, converted to the setting's type."""
        return self._call(self._core.get, key)

    def path(self, key: str) -> Path:
        """A path setting: `~` expanded, a relative one joined to the root."""
        return Path(self._call(self._core.path, key))

    def cast(self, key: str, value: Any) -> Any:
        """A value converted to the setting's type, as the layers would convert it."""
        return self._call(self._core.cast, key, self._raw(key, value))

    def source(self, key: str) -> str:
        """Where the value comes from: override, env, file or default."""
        return self._call(self._core.source, key)

    def shown(self, key: str) -> Any:
        """The value as it is shown: `***` for a secret setting that has one."""
        return self._call(self._core.shown, key)

    def problems(self) -> dict[str, str]:
        """Every value of the environment and the files that does not convert, by setting: what is wrong and where it is
        written. An override is checked as it is set."""
        found = {}
        for key, source, variable, file, text, expected in self._core.problems():
            shown = self._in_message(key, text)
            error = self._message("invalid_value", key=key, raw=shown, expected=_TYPES[expected])
            if source == "env":
                found[key] = self._message("problem_env", error=error, key=key, variable=variable)
            else:
                found[key] = self._message("problem_file", error=error, key=key, variable=variable, file=Path(file).name)
        return found

    # -- the overrides

    def set(self, key: str, value: Any) -> None:
        """Override a setting (the command line's `-c key=value`). The value is converted at once."""
        # A setting that cannot be overridden says so before its value is looked at.
        self._call(self._core.set, key, "" if key in self._file_only else self._raw(key, value))

    def unset(self, key: str) -> None:
        self._call(self._core.unset, key)

    def clear_overrides(self) -> None:
        self._core.clear_overrides()

    def overrides(self) -> dict[str, str]:
        """The overrides as text that `set` takes again, in the order they were first set."""
        return dict(self._core.overrides())

    # -- the files

    def file_values(self) -> dict[str, str]:
        """What was read from the files: name -> text."""
        return dict(self._core.file_values())

    def set_file_values(self, values: Mapping[str, str]) -> None:
        """Replace what was read from the files."""
        self._core.set_file_values(dict(values))

    def reload_files(self) -> None:
        self._call(self._core.reload_files)

    def persist(self, key: str, value: Any, file: str | os.PathLike[str] | None = None) -> Any:
        """Write a setting of `persisted` into `file` (else the write file) for good: its line replaced, every other line
        kept, the file replaced at once with its permissions. It holds at once: what was read from the files changes,
        and so does the environment's variable if it has one, for this configuration only (the environment itself is
        not changed) and whenever the variable has the value it had then; an override still wins. Returns the value as the setting's
        type."""
        return self._call(self._core.persist, key, self._raw(key, value), os.fspath(file) if file else "")

    # -- for tests

    def snapshot(self) -> Any:
        """The overrides and what was read from the files, with the file each came from, for `restore`."""
        return self._core.state()

    def restore(self, snapshot: Any) -> None:
        self._core.restore(snapshot)

    def _raw(self, key: str, value: Any) -> Any:
        """A value as the core takes it. Text (a path as its text) goes as it is, for the core to read; anything else is
        converted here as Python converts it, int(), float(), bool(), str() or list() by the setting's type, which the
        core could not do for every Python type (a Decimal, a Fraction, None)."""
        if isinstance(value, os.PathLike):
            value = os.fspath(value)
        if isinstance(value, str):
            return value
        kind = self.type(key)
        try:
            if kind == "list":
                return [str(item) for item in value]
            converted = {"bool": bool, "int": int, "float": float, "str": str}[kind](value)
            if kind == "int" and not -(2**63) <= converted < 2**63:
                raise OverflowError(value)
            return converted
        except (TypeError, ValueError, OverflowError):
            pass
        # Outside the except block: the error of the conversion may have the value in it.
        raise self._invalid(key, value, kind)

    # -- errors

    def _message(self, kind: str, **fields: Any) -> str:
        message = self._messages[kind]
        return message(**fields) if callable(message) else message.format(**fields)

    def _invalid(self, key: str, raw: Any, expected: str) -> InvalidValue:
        """The error of a value that does not convert; its message has *** for a secret setting's value."""
        message = self._message("invalid_value", key=key, raw=self._in_message(key, raw), expected=expected)
        return InvalidValue(message, key, raw, expected)

    def _in_message(self, key: str, raw: Any) -> Any:
        return _MASKED if key in self._secret else raw

    def _call(self, function: Callable[..., Any], *args: Any, **kwargs: Any) -> Any:
        try:
            return function(*args, **kwargs)
        except _core.Error as exc:
            failed = exc.args
        # Raised outside the except block, so that the core's error, whose args have the value (a secret's too), is not
        # kept as its __context__.
        raise self._error(*failed)

    def _error(self, kind: str, key: str, raw: Any, expected: str | None, message: str) -> ConfigError:
        if kind == "unknown_key":
            return UnknownKey(self._message("unknown_key", key=key), key)
        if kind == "invalid_value":
            return self._invalid(key, raw, _TYPES.get(expected or "", expected or ""))
        if kind == "file_only":
            file = self._written.name if self._written else ""
            return FileOnlyKey(self._message("file_only", key=key, variable=self.variable(key), file=file), key)
        if kind == "not_persisted":
            allowed = ", ".join(self._persisted)
            return NotPersisted(
                self._message("not_persisted", key=key, allowed=allowed, allowed_keys=list(self._persisted)), key
            )
        return ConfigError(message, key)

