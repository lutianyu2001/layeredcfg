"""The Python face of layeredconf: attribute access, the layers through os.environ, the exceptions and their messages,
problems, secrets, persist, snapshots."""

import os
import stat
import unicodedata
from decimal import Decimal
from fractions import Fraction
from pathlib import Path

import pytest

import layeredconf
from layeredconf import Config, FileOnlyKey, InvalidValue, NotPersisted, UnknownKey, find_upward

DEFAULTS = """\
[server]
port = 8420
host = "0.0.0.0"

[notify]
enabled = true
topic = ""

[quota]
ratio = 1.0
hosts = []

[log]
max_mib = 10.0
dir = "data/logs"
"""


@pytest.fixture
def defaults(tmp_path):
    path = tmp_path / "defaults.toml"
    path.write_text(DEFAULTS)
    return path


def make(defaults, **kwargs):
    given = {"prefix": "APP_", "root": "/srv/app", "file_only": {"log.max_mib"}, "persisted": {"quota.ratio"},
             "secret": {"notify.topic"}, "environ": {}}
    return Config(defaults, **{**given, **kwargs})


def test_settings_are_read_as_attributes_in_their_types(defaults):
    config = make(defaults)
    assert config.server.port == 8420 and config.quota.ratio == 1.0 and config.notify.enabled is True
    assert config.quota.hosts == [] and config.get("server.host") == "0.0.0.0"
    assert config.keys() == ["server.port", "server.host", "notify.enabled", "notify.topic", "quota.ratio", "quota.hosts",
                             "log.max_mib", "log.dir"]
    assert repr(config.server) == "<config section 'server'>"
    assert config.type("quota.hosts") == "list" and config.type("server.host") == "str"
    assert config.default("server.port") == 8420 and config.variable("server.port") == "APP_SERVER_PORT"
    assert layeredconf.__version__ == "0.1.0"


def test_unknown_settings_are_errors_of_the_kinds_python_code_expects(defaults):
    config = make(defaults)
    with pytest.raises(UnknownKey) as raised:
        config.get("no.such")
    assert isinstance(raised.value, KeyError) and str(raised.value) == "unknown setting: no.such"
    with pytest.raises(AttributeError, match="unknown setting: server.nope"):
        config.server.nope
    with pytest.raises(AttributeError):
        config.nope
    with pytest.raises(AttributeError):
        config._private


def test_the_layers_through_the_environment_and_the_files(defaults, tmp_path, monkeypatch):
    near = tmp_path / "work" / ".env.local"
    near.parent.mkdir()
    near.write_text("APP_SERVER_PORT=9001\nAPP_QUOTA_HOSTS=a.lan, b.lan,\n")
    mine = tmp_path / ".env.local"
    mine.write_text("APP_SERVER_PORT=9002\n")
    monkeypatch.delenv("APP_SERVER_PORT", raising=False)
    config = make(defaults, files=[near, None, mine], environ=None)  # None: os.environ
    assert config.server.port == 9002 and config.source("server.port") == "file"
    assert config.quota.hosts == ["a.lan", "b.lan"]
    monkeypatch.setenv("APP_SERVER_PORT", "9003")
    assert config.server.port == 9003 and config.source("server.port") == "env"
    config.set("server.port", 9004)
    assert config.server.port == 9004 and config.source("server.port") == "override"
    config.unset("server.port")
    assert config.server.port == 9003
    assert config.file_values() == {"APP_SERVER_PORT": "9002", "APP_QUOTA_HOSTS": "a.lan, b.lan,"}


def test_a_value_that_does_not_convert_is_an_error_with_what_and_where(defaults):
    config = make(defaults)
    with pytest.raises(InvalidValue) as raised:
        config.set("server.port", "84x0")
    error = raised.value
    assert isinstance(error, ValueError) and (error.key, error.raw, error.expected) == ("server.port", "84x0", "int")
    assert str(error) == "invalid value for server.port: '84x0' (expected int)"
    with pytest.raises(ValueError):
        config.set("notify.enabled", "ture")
    with pytest.raises(FileOnlyKey, match="log.max_mib cannot be set on the command line: set APP_LOG_MAX_MIB in"):
        config.set("log.max_mib", "1")
    # Values of Python types are converted as Python converts them.
    config.set("server.port", 2.7)
    config.set("server.host", True)
    config.set("quota.hosts", ("a", " b "))
    config.set("log.dir", Path("/var/log"))
    assert (config.server.port, config.server.host, config.quota.hosts, config.log.dir) == (2, "True", ["a", "b"], "/var/log")


def test_overrides_are_text_that_sets_them_again_in_the_order_first_set(defaults):
    config = make(defaults)
    config.set("server.port", 8421)
    config.set("quota.ratio", 0.6)
    config.set("notify.enabled", False)
    config.set("server.port", "8422")
    assert config.overrides() == {"server.port": "8422", "quota.ratio": "0.6", "notify.enabled": "False"}
    config.clear_overrides()
    assert config.overrides() == {}


def test_problems_say_what_is_wrong_and_where_it_is_written(defaults, tmp_path):
    file = tmp_path / ".env.local"
    file.write_text("APP_NOTIFY_ENABLED=maybe\n")
    config = make(defaults, files=[file], environ={"APP_SERVER_PORT": "84x0"})
    assert config.problems() == {
        "server.port": "invalid value for server.port: '84x0' (expected int), written in the environment variable "
                       "APP_SERVER_PORT",
        "notify.enabled": "invalid value for notify.enabled: 'maybe' (expected bool), written in APP_NOTIFY_ENABLED of "
                          ".env.local",
    }


def test_values_given_by_hand_are_said_to_be_in_the_write_file_and_nothing_is_relative_to_the_current_directory(
    defaults, tmp_path
):
    from layeredconf import _core

    core = _core.Config(str(defaults), files=[str(tmp_path / "x.env")], get=lambda name: None)
    core.set_file_values({"SERVER_PORT": "x"})
    assert core.problems() == [("server.port", "file", "SERVER_PORT", tmp_path / "x.env", "x", "int")]
    assert core.path("log.dir") == Path("data/logs")  # no root: still relative
    config = Config(defaults, write_file=tmp_path / "w.env", environ={})
    config.set_file_values({"SERVER_PORT": "y"})
    assert config.problems()["server.port"].endswith("written in SERVER_PORT of w.env")


def test_messages_can_be_given_in_a_program_s_own_words(defaults, tmp_path):
    words = {
        "unknown_key": "未知的配置项: {key}",
        "invalid_value": "配置项 {key} 的值无效: {raw!r}（应为 {expected}）",
        "file_only": "配置项 {key} 不能用命令行参数修改，请在 {file} 里设置 {variable}",
        "not_persisted": lambda key, allowed_keys, **_: f"{key} 不能改（能改的：{'、'.join(allowed_keys)}）",
        "problem_env": "{error}，写在环境变量 {variable} 里",
    }
    config = make(defaults, files=[tmp_path / ".env.local"], messages=words, environ={"APP_SERVER_PORT": "x"})
    with pytest.raises(KeyError, match="未知的配置项: no.such"):
        config.get("no.such")
    with pytest.raises(ValueError, match=r"配置项 log.max_mib 不能用命令行参数修改，请在 .env.local 里设置 APP_LOG_MAX_MIB"):
        config.set("log.max_mib", "1")
    with pytest.raises(KeyError, match="server.port 不能改（能改的：quota.ratio）"):
        config.persist("server.port", 1)
    assert config.problems() == {"server.port": "配置项 server.port 的值无效: 'x'（应为 int），写在环境变量 APP_SERVER_PORT 里"}


def test_secrets_are_shown_masked_and_paths_expanded(defaults):
    config = make(defaults, environ={"APP_NOTIFY_TOPIC": "my-topic", "HOME": "/home/someone"})
    assert config.shown("notify.topic") == "***" and config.notify.topic == "my-topic"
    assert config.shown("server.port") == 8420
    assert config.path("log.dir") == Path("/srv/app/data/logs")
    config.set("log.dir", "~/logs")
    assert config.path("log.dir") == Path("/home/someone/logs")


def test_a_secret_value_is_kept_out_of_the_messages(defaults):
    config = make(defaults, secret={"server.port"}, environ={"APP_SERVER_PORT": "84x0"})
    assert config.problems() == {
        "server.port": "invalid value for server.port: *** (expected int), written in the environment variable "
                       "APP_SERVER_PORT",
    }
    with pytest.raises(InvalidValue) as raised:
        config.server.port
    assert str(raised.value) == "invalid value for server.port: *** (expected int)" and raised.value.raw == "84x0"
    # Nor is the core's error, which has it, kept as the context.
    assert raised.value.__context__ is None and raised.value.__cause__ is None
    # Converted in Python too.
    with pytest.raises(InvalidValue, match=r"^invalid value for server.port: \*\*\* \(expected int\)$") as raised:
        config.set("server.port", object())
    assert raised.value.__context__ is None
    # A message of one's own is given *** as text.
    config = make(defaults, secret={"server.port"}, messages={"invalid_value": lambda key, raw, expected: raw.lower()})
    with pytest.raises(InvalidValue, match=r"^\*\*\*$"):
        config.set("server.port", "84x0")


def test_persist_writes_the_line_keeps_the_rest_and_holds_at_once(defaults, tmp_path):
    file = tmp_path / ".env.local"
    file.write_text("# ntfy\nAPP_NOTIFY_TOPIC=secret\nexport APP_QUOTA_RATIO=0.1\n")
    file.chmod(0o600)
    environ = {}
    config = make(defaults, files=[file], environ=environ)
    assert config.persist("quota.ratio", "0.45") == 0.45
    assert file.read_text() == "# ntfy\nAPP_NOTIFY_TOPIC=secret\nAPP_QUOTA_RATIO=0.45\n"
    assert stat.S_IMODE(file.stat().st_mode) == 0o600
    assert config.quota.ratio == 0.45 and environ == {}
    # The environment's variable, which wins over the file, is replaced too, for this configuration only (the
    # environment is not changed), whenever the variable has the value it had then.
    environ["APP_QUOTA_RATIO"] = "0.9"
    config.persist("quota.ratio", 0.3)
    assert environ["APP_QUOTA_RATIO"] == "0.9" and config.quota.ratio == 0.3
    assert config.source("quota.ratio") == "env"
    environ["APP_QUOTA_RATIO"] = "0.7"
    assert config.quota.ratio == 0.7
    environ["APP_QUOTA_RATIO"] = "0.9"
    assert config.quota.ratio == 0.3
    # Only the settings to persist; another file when given one.
    with pytest.raises(NotPersisted):
        config.persist("server.port", 1)
    other = tmp_path / "other.env"
    config.persist("quota.ratio", 2, other)
    assert other.read_text() == "APP_QUOTA_RATIO=2.0\n"


def test_a_snapshot_puts_the_overrides_and_the_files_back(defaults, tmp_path):
    file = tmp_path / ".env.local"
    file.write_text("APP_SERVER_PORT=9001\n")
    config = make(defaults, files=[file])
    config.set("quota.ratio", 0.5)
    saved = config.snapshot()
    config.clear_overrides()
    config.set_file_values({})
    config.set("server.host", "localhost")
    assert (config.quota.ratio, config.server.port, config.server.host) == (1.0, 8420, "localhost")
    config.restore(saved)
    assert (config.quota.ratio, config.server.port, config.server.host) == (0.5, 9001, "0.0.0.0")


def test_bad_defaults_and_settings_that_do_not_exist_are_refused(tmp_path):
    bad = tmp_path / "bad.toml"
    bad.write_text("when = 1979-05-27\n")
    with pytest.raises(layeredconf.ConfigError, match="date or a time"):
        Config(bad)
    with pytest.raises(layeredconf.ConfigError):
        Config(tmp_path / "missing.toml")
    good = tmp_path / "good.toml"
    good.write_text(DEFAULTS)
    with pytest.raises(UnknownKey):
        Config(good, secret={"no.such"})
    # Paths as bytes too, as os.fspath gives them.
    assert Config(os.fsencode(good)).server.port == 8420


def test_a_file_is_found_upwards(tmp_path, monkeypatch):
    (tmp_path / ".env.local").write_text("A=1\n")
    (tmp_path / "a" / "b").mkdir(parents=True)
    assert find_upward(".env.local", tmp_path / "a" / "b") == tmp_path / ".env.local"
    monkeypatch.chdir(tmp_path / "a")
    assert find_upward(".env.local") == tmp_path / ".env.local"
    assert find_upward(".no-such-file-anywhere") is None
    assert os.getcwd() == str(tmp_path / "a")


def test_python_values_are_converted_as_python_converts_them(defaults):
    config = make(defaults)
    assert config.cast("quota.ratio", Decimal("2.5")) == 2.5 and config.cast("quota.ratio", Fraction(1, 2)) == 0.5
    assert config.cast("server.port", Decimal("7")) == 7 and config.cast("server.port", True) == 1
    assert config.cast("notify.enabled", []) is False and config.cast("server.host", None) == "None"
    for key, value in (("server.port", None), ("quota.ratio", None), ("quota.hosts", 5), ("server.port", 2**63),
                       ("server.port", float("nan"))):
        with pytest.raises(InvalidValue):
            config.cast(key, value)
    with pytest.raises(FileOnlyKey):  # refused for being file-only before its value is looked at
        config.set("log.max_mib", None)
    config.set("server.port", 8421.0)
    config.set("notify.enabled", 2)
    assert config.overrides() == {"server.port": "8421", "notify.enabled": "True"}


def outcome(function, *args):
    """What a conversion gives, as its repr, or ValueError."""
    try:
        return repr(function(*args))
    except ValueError:
        return "ValueError"


def python_bool(text):
    word = text.strip().lower()
    if word not in ("1", "true", "yes", "on", "0", "false", "no", "off", ""):
        raise ValueError(text)
    return word in ("1", "true", "yes", "on")


def test_digits_and_whitespace_beyond_ascii_are_read_as_this_python_reads_them(defaults):
    config = make(defaults)
    zeros = [code for code in range(0x110000) if unicodedata.decimal(chr(code), -1) == 0]
    spaces = [code for code in range(0x110000) if chr(code).isspace()]
    # Every decimal digit and every whitespace character of this Python, the characters next to them, and a few more.
    codes = {zero + k for zero in zeros for k in range(-1, 11)} | {space + k for space in spaces for k in (-1, 0, 1)}
    codes |= {0x7F, 0xB2, 0x180E, 0x200B, 0xFEFF, 0xFF0B, 0xFF0E, 0xFF45}
    for char in (chr(code) for code in sorted(codes) if not 0xD800 <= code <= 0xDFFF):
        for text in (char, char + "5", "5" + char, "-" + char, char + "_1", "1." + char, char + "e1", char + "on"):
            assert outcome(config.cast, "server.port", text) == outcome(int, text), ascii(text)
            assert outcome(config.cast, "quota.ratio", text) == outcome(float, text), ascii(text)
            assert outcome(config.cast, "notify.enabled", text) == outcome(python_bool, text), ascii(text)
            items = [item.strip() for item in (text + "," + text).split(",") if item.strip()]
            assert config.cast("quota.hosts", text + "," + text) == items, ascii(text)


def test_a_file_written_with_full_width_digits_and_spaces_reads_as_python_reads_it(defaults, tmp_path):
    file = tmp_path / ".env.local"
    # ８４２１, an ideographic space, a comment in Chinese; a line that begins with that space, ０.５
    file.write_text("APP_SERVER_PORT=\uff18\uff14\uff12\uff11\u3000# \u5907\u6ce8\n"
                    "\u3000APP_QUOTA_RATIO = \uff10.\uff15\n")
    config = make(defaults, files=[file])
    assert config.server.port == 8421 and config.quota.ratio == 0.5 and config.problems() == {}


def test_text_that_is_not_utf8_is_an_invalid_value_not_a_crash(defaults, tmp_path):
    file = tmp_path / ".env.local"
    file.write_bytes(b"APP_SERVER_HOST=caf\xe9\n")
    config = make(defaults, files=[file], environ={"APP_LOG_DIR": "caf\udce9"})
    with pytest.raises(InvalidValue) as raised:
        config.server.host
    assert raised.value.raw == "caf\udce9"
    assert set(config.problems()) == {"server.host", "log.dir"}
    assert config.file_values() == {"APP_SERVER_HOST": "caf\udce9"}


def test_a_snapshot_keeps_which_file_each_value_came_from(defaults, tmp_path):
    first, second = tmp_path / "a.env", tmp_path / "b.env"
    first.write_text("APP_SERVER_PORT=x\n")
    second.write_text("APP_QUOTA_RATIO=0.2\n")
    config = make(defaults, files=[first, second])
    saved = config.snapshot()
    config.set_file_values({})
    config.restore(saved)
    assert config.problems()["server.port"].endswith("of a.env")
