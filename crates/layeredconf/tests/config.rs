//! The layers, the conversions, the checks, and persist, with an environment of the test's own.

// The helpers of the tests fail the test they are called from by panicking, as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use layeredconf::{Config, Environment, ErrorKind, Options, Raw, Source, Type, Value, find_upward};

const DEFAULTS: &str = r#"# The test's defaults.
[server]
port = 8420
host = "0.0.0.0"

[notify]
enabled = true
topic = ""
token = ""

[quota]
ratio = 1.0
hosts = []

[log]
max_mib = 10.0
dir = "data/logs"
"#;

/// A directory of the test's own, removed when it is dropped.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos());
        let name = format!("layeredconf-test-{}-{}-{nanos}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
        let dir = std::env::temp_dir().join(name);
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn write(&self, name: &str, text: impl AsRef<[u8]>) -> PathBuf {
        let path = self.dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// An environment the test sets as it likes.
#[derive(Clone, Default)]
struct FakeEnvironment {
    values: Arc<Mutex<BTreeMap<OsString, OsString>>>,
}

impl FakeEnvironment {
    fn set(&self, name: &str, value: &str) {
        self.values.lock().unwrap().insert(name.into(), value.into());
    }

    fn value(&self, name: &str) -> Option<OsString> {
        self.values.lock().unwrap().get(OsStr::new(name)).cloned()
    }

    fn get(&self) -> Environment {
        let values = Arc::clone(&self.values);
        Environment::new(move |name: &OsStr| values.lock().unwrap().get(name).cloned())
    }
}

fn options(environment: &FakeEnvironment, files: &[&Path]) -> Options {
    Options {
        prefix: "APP_".to_owned(),
        files: files.iter().map(|file| file.to_path_buf()).collect(),
        root: Some("/srv/app".into()),
        file_only: ["log.max_mib".to_owned()].into(),
        persisted: ["quota.ratio".to_owned()].into(),
        secret: ["notify.topic".to_owned(), "notify.token".to_owned()].into(),
        environment: environment.get(),
        ..Options::default()
    }
}

fn plain(environment: &FakeEnvironment) -> Options {
    Options { environment: environment.get(), ..Options::default() }
}

fn parse(toml: &str, options: Options) -> Config {
    Config::parse(toml, options, "defaults").unwrap()
}

fn get(config: &Config, key: &str) -> Value {
    config.get(key).unwrap()
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn the_defaults_say_which_settings_there_are_in_the_file_order_and_their_types() {
    let environment = FakeEnvironment::default();
    let config = parse(DEFAULTS, options(&environment, &[]));
    assert_eq!(
        config.keys(),
        [
            "server.port",
            "server.host",
            "notify.enabled",
            "notify.topic",
            "notify.token",
            "quota.ratio",
            "quota.hosts",
            "log.max_mib",
            "log.dir"
        ]
    );
    assert_eq!(config.type_of("server.port").unwrap(), Type::Int);
    assert_eq!(config.type_of("quota.hosts").unwrap(), Type::List);
    assert!(config.is_section("server"));
    assert!(!config.is_section("server.port"));
    assert_eq!(get(&config, "server.port"), Value::Int(8420));
    assert_eq!(config.source("server.port").unwrap(), Source::Default);
    assert_eq!(config.variable("server.port"), "APP_SERVER_PORT");
    assert!(config.get("server.nope").is_err());
    // A table's entries together, in the file's order, as Python's tomllib has them.
    let reopened = parse("[b]\nx = 1\n[a]\ny = 2\n[b.c]\nz = 3\n", plain(&environment));
    assert_eq!(reopened.keys(), ["b.x", "b.c.z", "a.y"]);
}

#[test]
fn defaults_that_no_other_layer_could_write_are_refused() {
    let environment = FakeEnvironment::default();
    assert!(Config::parse("when = 1979-05-27\n", plain(&environment), "defaults").is_err());
    assert!(Config::parse("ports = [1, 2]\n", plain(&environment), "defaults").is_err());
    assert!(Config::parse("broken = \n", plain(&environment), "defaults").is_err());
    // Settings named in the options must exist.
    let named = Options { secret: ["no.such".to_owned()].into(), ..plain(&environment) };
    assert!(Config::parse(DEFAULTS, named, "defaults").is_err());
}

#[test]
fn each_layer_wins_over_the_one_below_and_each_value_is_converted_to_its_default_type() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let near = scratch.write("work/.env.local", "APP_SERVER_PORT=9001\nAPP_QUOTA_HOSTS=x.lan\n");
    let mine = scratch.write("repo/.env.local", "APP_SERVER_PORT=9002\nAPP_QUOTA_RATIO=0.5\n");
    let mut config = parse(DEFAULTS, options(&environment, &[&near, &mine]));
    assert_eq!(get(&config, "server.port"), Value::Int(9002)); // the later file
    assert_eq!(get(&config, "quota.hosts"), Value::List(vec!["x.lan".to_owned()]));
    assert_eq!(get(&config, "quota.ratio"), Value::Float(0.5));
    assert_eq!(config.source("server.port").unwrap(), Source::File);
    environment.set("APP_SERVER_PORT", "9003");
    assert_eq!(get(&config, "server.port"), Value::Int(9003));
    assert_eq!(config.source("server.port").unwrap(), Source::Env);
    config.set("server.port", "9004").unwrap();
    assert_eq!(get(&config, "server.port"), Value::Int(9004));
    assert_eq!(config.source("server.port").unwrap(), Source::Override);
    config.unset("server.port").unwrap();
    assert_eq!(get(&config, "server.port"), Value::Int(9003));
    // Values that are not text are converted as Python converts them.
    config.set("server.port", 2.7).unwrap();
    assert_eq!(get(&config, "server.port"), Value::Int(2));
    config.set("notify.enabled", 0i64).unwrap();
    assert_eq!(get(&config, "notify.enabled"), Value::Bool(false));
    config.set("server.host", true).unwrap();
    assert_eq!(get(&config, "server.host"), Value::Str("True".to_owned()));
    config.set("quota.ratio", 3i64).unwrap();
    assert_eq!(get(&config, "quota.ratio"), Value::Float(3.0));
}

#[test]
fn an_override_is_checked_as_it_is_set_and_kept_in_the_order_first_set_as_text_that_sets_it_again() {
    let environment = FakeEnvironment::default();
    let mut config = parse(DEFAULTS, options(&environment, &[]));
    assert!(config.set("server.port", "84x0").is_err());
    assert!(config.set("no.such", "1").is_err());
    let error = config.set("log.max_mib", "1").expect_err("a file-only setting was overridden");
    assert_eq!(error.kind(), ErrorKind::FileOnly);
    let error = config.set("notify.enabled", "maybe").expect_err("a word that is no boolean was taken");
    assert_eq!(error.kind(), ErrorKind::InvalidValue);
    assert_eq!(error.key(), "notify.enabled");
    assert_eq!(error.raw(), Some(&Raw::from("maybe")));
    assert_eq!(error.expected(), Some(Type::Bool));
    config.set("server.port", 8421i64).unwrap();
    config.set("quota.hosts", vec![" a ".to_owned(), String::new(), "b".to_owned()]).unwrap();
    config.set("quota.ratio", 0.6).unwrap();
    config.set("server.port", "8422").unwrap();
    let expected: Vec<(String, OsString)> = [("server.port", "8422"), ("quota.hosts", "a,b"), ("quota.ratio", "0.6")]
        .into_iter()
        .map(|(key, text)| (key.to_owned(), text.into()))
        .collect();
    assert_eq!(config.overrides(), expected);
    // As text that set takes again: what is not text is written as its converted value.
    config.set("server.port", 8421.0).unwrap();
    config.set("notify.enabled", 2i64).unwrap();
    assert_eq!(config.overrides()[0].1, "8421");
    assert_eq!(config.overrides()[3].1, "True");
    for (key, text) in config.overrides() {
        assert!(config.cast(&key, text).is_ok(), "{key}");
    }
    config.clear_overrides();
    assert!(config.overrides().is_empty());
}

#[test]
fn problems_are_the_values_of_the_environment_and_the_files_that_do_not_convert_with_where_they_are() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let file = scratch.write(".env.local", "APP_NOTIFY_ENABLED=maybe\nAPP_SERVER_HOST=anything\n");
    let mut config = parse(DEFAULTS, options(&environment, &[&file]));
    environment.set("APP_SERVER_PORT", "84x0");
    let found = config.problems();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].key, "server.port");
    assert_eq!(found[0].source, Source::Env);
    assert_eq!(found[0].text, "84x0");
    assert_eq!(found[1].key, "notify.enabled");
    assert_eq!(found[1].source, Source::File);
    assert_eq!(found[1].file.as_deref(), Some(file.as_path()));
    assert_eq!(found[1].variable, "APP_NOTIFY_ENABLED");
    // An override hides what is under it.
    config.set("server.port", 1i64).unwrap();
    assert_eq!(config.problems().len(), 1);
    // Values given by a caller are said to be in the file persist writes into.
    config.set_file_values([("APP_QUOTA_RATIO".into(), "x".into())].into(), None);
    assert_eq!(config.problems().len(), 1);
    assert_eq!(config.problems()[0].file.as_deref(), Some(file.as_path()));
}

#[test]
fn secrets_are_shown_masked_when_they_have_a_value() {
    let environment = FakeEnvironment::default();
    let config = parse(DEFAULTS, options(&environment, &[]));
    assert_eq!(config.shown("notify.topic").unwrap(), Value::Str(String::new()));
    environment.set("APP_NOTIFY_TOPIC", "my-topic");
    assert_eq!(config.shown("notify.topic").unwrap(), Value::Str("***".to_owned()));
    assert_eq!(get(&config, "notify.topic"), Value::Str("my-topic".to_owned()));
    assert_eq!(config.shown("server.port").unwrap(), Value::Int(8420));
}

#[test]
fn a_secret_value_is_kept_out_of_the_message_of_its_error() {
    let environment = FakeEnvironment::default();
    let options = Options { secret: ["pin".to_owned()].into(), ..plain(&environment) };
    let mut config = parse("pin = 0\nport = 0\n", options);
    let error = config.set("pin", "12x4").unwrap_err();
    assert_eq!(error.message(), "invalid value for pin: *** (expected int)");
    assert_eq!(error.raw(), Some(&Raw::from("12x4")));
    assert!(!format!("{error:?}").contains("12x4"), "{error:?}");
    environment.set("PIN", "56y7");
    assert_eq!(config.get("pin").unwrap_err().message(), "invalid value for pin: *** (expected int)");
    // Any other setting's value is in it, quoted.
    let error = config.set("port", r#"8"4\2"#).unwrap_err();
    assert_eq!(error.message(), r#"invalid value for port: "8\"4\\2" (expected int)"#);
}

#[test]
fn persist_keeps_the_permissions_but_not_the_special_bits_and_writes_no_file_it_cannot_look_at() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let file = scratch.write(".env", "APP_QUOTA_RATIO=0.1\n");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o4640)).unwrap();
    let mut config = parse(DEFAULTS, options(&environment, &[&file]));
    config.persist("quota.ratio", "0.2", None).unwrap();
    assert_eq!(mode(&file), 0o640);
    // Behind a directory that may not be looked into: an error, not a new file over whatever is there.
    let locked = scratch.write("locked/.env", "APP_QUOTA_RATIO=0.1\n");
    let directory = locked.parent().unwrap().to_path_buf();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o000)).unwrap();
    let root = fs::metadata(&locked).is_ok(); // who looks into any directory
    let refused = config.persist("quota.ratio", "0.3", Some(&locked));
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    if root {
        return;
    }
    let error = refused.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert!(error.message().contains("cannot read"), "{}", error.message());
    assert_eq!(Scratch::read(&locked), "APP_QUOTA_RATIO=0.1\n");
}

#[test]
fn overrides_of_a_snapshot_of_another_configuration_are_left_out_without_a_panic() {
    let environment = FakeEnvironment::default();
    let mut other = parse("elsewhere = 1\n", plain(&environment));
    other.set("elsewhere", 2_i64).unwrap();
    let mut config = parse(DEFAULTS, plain(&environment));
    config.restore(&other.state());
    assert_eq!(config.overrides(), vec![("elsewhere".to_owned(), OsString::new())]);
}

#[test]
fn the_process_environment_has_no_variable_of_a_name_none_can_have() {
    let environment = Environment::process();
    for name in ["", "A=B", "A\0B"] {
        assert_eq!(environment.get(OsStr::new(name)), None, "{name:?}");
    }
    assert_eq!(environment.get(OsStr::new("PATH")), std::env::var_os("PATH"));
}

#[test]
fn files_that_are_not_there_or_are_no_files_are_passed_over() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let file = scratch.write(".env", "APP_SERVER_PORT=9000\n");
    let files = vec![PathBuf::new(), scratch.dir.join("missing.env"), scratch.dir.clone(), file];
    let config = parse(DEFAULTS, Options { files, ..options(&environment, &[]) });
    assert_eq!(get(&config, "server.port"), Value::Int(9000));
}

#[test]
fn a_path_setting_has_the_home_expanded_and_a_relative_one_joined_to_the_root() {
    let environment = FakeEnvironment::default();
    environment.set("HOME", "/home/someone/");
    let mut config = parse(DEFAULTS, options(&environment, &[]));
    assert_eq!(config.path("log.dir").unwrap(), Path::new("/srv/app/data/logs"));
    config.set("log.dir", "~/logs").unwrap();
    assert_eq!(config.path("log.dir").unwrap(), Path::new("/home/someone/logs"));
    config.set("log.dir", "/var/log/app").unwrap();
    assert_eq!(config.path("log.dir").unwrap(), Path::new("/var/log/app"));
}

#[test]
fn persist_replaces_the_setting_line_keeps_the_file_other_lines_and_permissions_and_holds_at_once() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let file = scratch
        .write(".env.local", "# ntfy\nAPP_NOTIFY_TOPIC=secret\nexport APP_QUOTA_RATIO=0.1\nAPP_QUOTA_RATIO=0.2\n");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let mut config = parse(DEFAULTS, options(&environment, &[&file]));
    assert_eq!(config.persist("quota.ratio", "0.45", None).unwrap(), Value::Float(0.45));
    assert_eq!(Scratch::read(&file), "# ntfy\nAPP_NOTIFY_TOPIC=secret\nAPP_QUOTA_RATIO=0.45\n");
    assert_eq!(mode(&file), 0o600);
    let mut writing = file.as_os_str().to_owned();
    writing.push(".writing");
    assert!(!Path::new(&writing).exists());
    let names: Vec<OsString> = fs::read_dir(&scratch.dir).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(names, [".env.local"]); // no file left beside it
    assert_eq!(get(&config, "quota.ratio"), Value::Float(0.45));
    assert_eq!(config.source("quota.ratio").unwrap(), Source::File);
    // A variable of the environment, which wins over the file, reads as what was written, for this configuration only
    // and whenever the environment has the value it had then.
    environment.set("APP_QUOTA_RATIO", "0.9");
    config.persist("quota.ratio", 0.3, None).unwrap();
    assert_eq!(environment.value("APP_QUOTA_RATIO").unwrap(), "0.9");
    assert_eq!(get(&config, "quota.ratio"), Value::Float(0.3));
    assert_eq!(config.source("quota.ratio").unwrap(), Source::Env);
    environment.set("APP_QUOTA_RATIO", "0.7");
    assert_eq!(get(&config, "quota.ratio"), Value::Float(0.7));
    // Only the settings to persist; a bad value writes nothing.
    assert!(config.persist("server.port", "1", None).is_err());
    assert!(config.persist("quota.ratio", "soon", None).is_err());
    assert_eq!(Scratch::read(&file), "# ntfy\nAPP_NOTIFY_TOPIC=secret\nAPP_QUOTA_RATIO=0.3\n");
    // A new file is made with 0600.
    let other = scratch.dir.join("new.env");
    config.persist("quota.ratio", "2", Some(&other)).unwrap();
    assert_eq!(Scratch::read(&other), "APP_QUOTA_RATIO=2.0\n");
    assert_eq!(mode(&other), 0o600);
}

#[test]
fn a_file_is_found_from_a_directory_upwards() {
    let scratch = Scratch::new();
    let file = scratch.write(".env.local", "A=1\n");
    fs::create_dir_all(scratch.dir.join("a").join("b")).unwrap();
    assert_eq!(find_upward(".env.local", scratch.dir.join("a").join("b")), Some(file));
    assert_eq!(find_upward(".no-such-file-anywhere", scratch.dir.join("a")), None);
}

#[test]
fn persist_into_a_file_that_a_later_file_outranks_does_not_claim_the_value() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let low = scratch.write("low.env", "APP_QUOTA_RATIO=0.1\n");
    let high = scratch.write("high.env", "APP_QUOTA_RATIO=0.9\n");
    let o = Options { write_file: Some(low.clone()), ..options(&environment, &[&low, &high]) };
    let mut config = parse(DEFAULTS, o);
    config.persist("quota.ratio", 0.5, None).unwrap();
    assert_eq!(get(&config, "quota.ratio"), Value::Float(0.9));
    config.reload_files().unwrap();
    assert_eq!(get(&config, "quota.ratio"), Value::Float(0.9));
    assert_eq!(Scratch::read(&low), "APP_QUOTA_RATIO=0.5\n");
}

#[test]
fn persist_writes_a_link_target_and_keeps_the_link() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let real = scratch.write("real.env", "APP_QUOTA_RATIO=0.1\n");
    let link = scratch.dir.join(".env.local");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut config = parse(DEFAULTS, options(&environment, &[&link]));
    config.persist("quota.ratio", 0.4, None).unwrap();
    assert!(link.is_symlink());
    assert_eq!(Scratch::read(&real), "APP_QUOTA_RATIO=0.4\n");
}

#[test]
fn writers_at_the_same_time_leave_a_whole_file_and_no_temporary_ones() {
    let scratch = Scratch::new();
    let file = scratch.write(".env.local", "# kept\n");
    let write = |value: f64| {
        let environment = FakeEnvironment::default();
        let mut config = parse(DEFAULTS, options(&environment, &[&file]));
        for _ in 0..200 {
            config.persist("quota.ratio", value, None).unwrap();
        }
    };
    std::thread::scope(|scope| {
        scope.spawn(|| write(0.25));
        scope.spawn(|| write(0.75));
    });
    let text = Scratch::read(&file);
    assert!(text == "# kept\nAPP_QUOTA_RATIO=0.25\n" || text == "# kept\nAPP_QUOTA_RATIO=0.75\n", "{text:?}");
    assert_eq!(fs::read_dir(&scratch.dir).unwrap().count(), 1);
}

#[test]
fn a_snapshot_keeps_which_file_each_value_came_from() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let a = scratch.write("a.env", "APP_SERVER_PORT=x\n");
    let b = scratch.write("b.env", "APP_QUOTA_RATIO=0.2\n");
    let mut config = parse(DEFAULTS, options(&environment, &[&a, &b]));
    let saved = config.state();
    config.set_file_values(BTreeMap::new(), None);
    config.set("server.port", 1i64).unwrap();
    config.restore(&saved);
    assert!(config.overrides().is_empty());
    assert_eq!(config.problems().len(), 1);
    assert_eq!(config.problems()[0].file.as_deref(), Some(a.as_path()));
}

#[test]
fn text_not_utf8_is_no_value_the_defaults_file_must_be_a_file_a_float_of_minus_2_pow_63_is_an_integer() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let file = scratch.write(".env.local", b"APP_SERVER_HOST=caf\xE9\n");
    let mut config = parse(DEFAULTS, options(&environment, &[&file]));
    assert!(config.get("server.host").is_err());
    assert_eq!(config.problems().len(), 1);
    assert_eq!(config.problems()[0].text.as_bytes(), b"caf\xE9");
    assert!(Config::load(&scratch.dir, options(&environment, &[])).is_err());
    config.set("server.port", -9_223_372_036_854_775_808.0).unwrap();
    assert_eq!(get(&config, "server.port"), Value::Int(i64::MIN));
}

#[test]
fn found_upwards_from_where_a_path_really_is() {
    let scratch = Scratch::new();
    let file = scratch.write("a/.env.local", "A=1\n");
    fs::create_dir_all(scratch.dir.join("b")).unwrap();
    assert_eq!(find_upward(".env.local", scratch.dir.join("a").join("..").join("b")), None);
    assert_eq!(find_upward(".env.local", scratch.dir.join("b").join("..").join("a")), Some(file));
}

#[test]
fn keys_in_tomllib_order() {
    let environment = FakeEnvironment::default();
    let config = parse("[x.y]\nz = 1\n[a]\nq = 2\n[x]\nw = 3\n", plain(&environment));
    assert_eq!(config.keys(), ["x.y.z", "x.w", "a.q"]);
}

#[test]
fn persist_refuses_a_line_a_quote_never_closed_would_take_in() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let source = "APP_NOTIFY_TOPIC='never closed\nAPP_SERVER_PORT=9001\n";
    let file = scratch.write(".env.local", source);
    let mut config =
        parse(DEFAULTS, Options { persisted: ["server.host".to_owned()].into(), ..options(&environment, &[&file]) });
    let error = config.persist("server.host", "it's", None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!(Scratch::read(&file), source);
    assert_eq!(get(&config, "server.port"), Value::Int(9001));
    // A value without the quote is written.
    config.persist("server.host", "localhost", None).unwrap();
    assert_eq!(Scratch::read(&file), format!("{source}APP_SERVER_HOST=localhost\n"));
}

#[test]
fn a_key_written_twice_is_the_setting_its_first_one_says() {
    let environment = FakeEnvironment::default();
    let config = parse("\"a.b\" = 1\n[a]\nb = \"x\"\n", plain(&environment));
    assert_eq!(config.type_of("a.b").unwrap(), Type::Int);
    assert_eq!(config.default_value("a.b").unwrap(), &Value::Int(1));
}

#[test]
fn persist_refuses_a_name_no_env_style_file_can_have() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let file = scratch.write(".env.local", "");
    let options = Options { files: vec![file.clone()], persisted: ["a b".to_owned()].into(), ..plain(&environment) };
    let mut config = parse("\"a b\" = \"x\"\n", options);
    let error = config.persist("a b", "y", None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert!(error.message().contains("A B is no name an env-style file can have"), "{}", error.message());
    assert_eq!(Scratch::read(&file), "");
}

#[test]
fn persist_writes_the_target_of_a_link_whose_target_is_not_there_yet() {
    let scratch = Scratch::new();
    let environment = FakeEnvironment::default();
    let link = scratch.dir.join(".env.local");
    std::os::unix::fs::symlink("real.env", &link).unwrap();
    let options = Options { write_file: Some(link.clone()), ..options(&environment, &[]) };
    let mut config = parse(DEFAULTS, options);
    config.persist("quota.ratio", "0.5", None).unwrap();
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(Scratch::read(&scratch.dir.join("real.env")), "APP_QUOTA_RATIO=0.5\n");
}
