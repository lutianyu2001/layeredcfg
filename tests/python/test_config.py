"""The Python face of layeredconf: attribute access, the layers through os.environ, the exceptions and their messages,
problems, secrets, persist, snapshots."""

import os
import stat
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
    # The environment's variable, which wins over the file, is replaced too.
    environ["APP_QUOTA_RATIO"] = "0.9"
    config.persist("quota.ratio", 0.3)
    assert environ["APP_QUOTA_RATIO"] == "0.3" and config.quota.ratio == 0.3
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


def test_a_file_is_found_upwards(tmp_path, monkeypatch):
    (tmp_path / ".env.local").write_text("A=1\n")
    (tmp_path / "a" / "b").mkdir(parents=True)
    assert find_upward(".env.local", tmp_path / "a" / "b") == tmp_path / ".env.local"
    monkeypatch.chdir(tmp_path / "a")
    assert find_upward(".env.local") == tmp_path / ".env.local"
    assert find_upward(".no-such-file-anywhere") is None
    assert os.getcwd() == str(tmp_path / "a")
