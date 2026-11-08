use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::files::{self, read_file, replace_file};
use crate::{
    Error, ErrorKind, FileValue, Options, Problem, Raw, Result, Source, State, Type, UserValue, Value, Written,
    envfile, text,
};

/// A raw value as it appears in a message: text quoted, the rest as text.
fn shown_in_message(raw: &Raw) -> String {
    fn quoted(text: &str) -> String {
        let mut out = String::from("\"");
        for c in text.chars() {
            if c == '"' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out + "\""
    }
    match raw {
        Raw::Text(text) => quoted(&text.to_string_lossy()),
        Raw::List(items) => format!("[{}]", items.iter().map(|item| quoted(item)).collect::<Vec<_>>().join(", ")),
        Raw::Bool(b) => Value::Bool(*b).to_string(),
        Raw::Int(i) => Value::Int(*i).to_string(),
        Raw::Float(d) => Value::Float(*d).to_string(),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Float(d) => *d != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::List(items) => !items.is_empty(),
    }
}

/// A raw value converted to a type, or nothing when it does not convert (Python's conversions: bool(x) of a value
/// that is not text, int(2.7) is 2, str(True) is "True").
fn convert_to(ty: Type, raw: &Raw) -> Option<Value> {
    match raw {
        Raw::Text(text) => {
            let text = std::str::from_utf8(text.as_bytes()).ok()?; // every layer's text is UTF-8
            match ty {
                Type::Bool => text::parse_bool(text.as_bytes()).map(Value::Bool),
                Type::Int => text::parse_int(text.as_bytes()).map(Value::Int),
                Type::Float => text::parse_float(text.as_bytes()).map(Value::Float),
                Type::String => Some(Value::Str(text.to_owned())),
                Type::List => Some(Value::List(text::split_list(text))),
            }
        }
        Raw::List(items) => match ty {
            Type::Bool => Some(Value::Bool(!items.is_empty())),
            Type::List => Some(Value::List(
                items
                    .iter()
                    .map(|item| text::strip_str(item))
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned)
                    .collect(),
            )),
            _ => None,
        },
        _ if ty == Type::List => None,
        Raw::Bool(b) => Some(match ty {
            Type::Int => Value::Int(i64::from(*b)),
            Type::Float => Value::Float(if *b { 1.0 } else { 0.0 }),
            Type::String => Value::Str(Value::Bool(*b).to_string()),
            _ => Value::Bool(*b),
        }),
        Raw::Int(i) => Some(match ty {
            Type::Bool => Value::Bool(*i != 0),
            // To the nearest float, as Python's float() of an int.
            #[allow(clippy::cast_precision_loss)]
            Type::Float => Value::Float(*i as f64),
            Type::String => Value::Str(i.to_string()),
            _ => Value::Int(*i),
        }),
        Raw::Float(d) => match ty {
            Type::Bool => Some(Value::Bool(*d != 0.0)),
            Type::String => Some(Value::Str(Value::Float(*d).to_string())),
            Type::Int => {
                // -2**63 <= d < 2**63, truncated toward zero.
                let fits = d.is_finite() && *d >= -9_223_372_036_854_775_808.0 && *d < 9_223_372_036_854_775_808.0;
                #[allow(clippy::cast_possible_truncation)] // in range, as checked above
                fits.then(|| Value::Int(d.trunc() as i64))
            }
            _ => Some(Value::Float(*d)),
        },
    }
}

/// Line and column (from 1, the column in characters) of a byte offset into the text.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = text.get(..offset).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

struct Setting {
    ty: Type,
    fallback: Value,
}

const INT_PAST_RANGE: &str = "is an integer past the range of a 64-bit integer";
const FLOAT_PAST_RANGE: &str = "is a float past the range of a double";

fn toml_int(i: &toml::de::DeInteger<'_>) -> Option<i64> {
    i64::from_str_radix(i.as_str(), i.radix()).ok()
}

fn toml_float(f: &toml::de::DeFloat<'_>) -> Option<f64> {
    let text = f.as_str();
    text.parse::<f64>().ok().filter(|d| d.is_finite() || text.contains("inf") || text.contains("nan"))
}

/// Where a value is first written in the file: for a table, where the first thing in it is.
fn place(node: &Spanned<DeValue<'_>>) -> usize {
    let own = node.span().start;
    match node.get_ref() {
        DeValue::Table(inner) => inner.values().map(place).fold(own, usize::min),
        _ => own,
    }
}

/// A table's entries in the order the file has them, each table's own entries together, as Python's tomllib reads
/// them: a table appears where the first thing in it is written ([x.y] before [x] makes x appear at [x.y]), which the
/// toml crate's order is not when [x] comes after [x.y].
fn flatten(
    table: &DeTable<'_>,
    prefix: &str,
    origin: &str,
    source: &str,
    out: &mut Vec<(String, Setting)>,
    sections: &mut BTreeSet<String>,
) -> Result<()> {
    let mut entries: Vec<_> = table.iter().collect();
    entries.sort_by_key(|(_, node)| place(node));
    for (name, node) in entries {
        let key = format!("{prefix}{}", name.get_ref());
        let bad = |what: &str| {
            let (line, column) = position(source, node.span().start);
            Error::new(ErrorKind::BadDefaults, key.clone(), format!("{origin}:{line}:{column}: {key} {what}"))
        };
        let (ty, fallback) = match node.get_ref() {
            DeValue::Table(inner) => {
                sections.insert(key.clone());
                flatten(inner, &format!("{key}."), origin, source, out, sections)?;
                continue;
            }
            DeValue::Boolean(b) => (Type::Bool, Value::Bool(*b)),
            DeValue::Integer(i) => match toml_int(i) {
                Some(i) => (Type::Int, Value::Int(i)),
                None => return Err(bad(INT_PAST_RANGE)),
            },
            DeValue::Float(f) => match toml_float(f) {
                Some(d) => (Type::Float, Value::Float(d)),
                None => return Err(bad(FLOAT_PAST_RANGE)),
            },
            DeValue::String(s) => (Type::String, Value::Str(s.to_string())),
            DeValue::Array(array) => {
                let items = array.iter().map(|item| item.get_ref().as_str().map(str::to_owned)).collect();
                match items {
                    Some(items) => (Type::List, Value::List(items)),
                    None => {
                        return Err(bad("is a list of something else than text, which no other layer could write"));
                    }
                }
            }
            DeValue::Datetime(_) => {
                return Err(bad("has a type no other layer could write (a date or a time)"));
            }
        };
        out.push((key, Setting { ty, fallback }));
    }
    Ok(())
}

/// The settings of a program, from their defaults and the layers above them.
#[derive(Clone)]
pub struct Config {
    options: Options,
    keys: Vec<String>,
    settings: BTreeMap<String, Setting>,
    sections: BTreeSet<String>,
    overrides: Vec<(String, Raw)>,
    user: BTreeMap<String, UserValue>,
    files: BTreeMap<OsString, FileValue>,
    written: BTreeMap<OsString, Written>,
}

impl Clone for Setting {
    fn clone(&self) -> Self {
        Setting { ty: self.ty, fallback: self.fallback.clone() }
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config").field("keys", &self.keys).field("options", &self.options).finish_non_exhaustive()
    }
}

impl Config {
    /// The defaults from a TOML file. The files are read at once.
    ///
    /// # Errors
    ///
    /// `BadDefaults` when the TOML file cannot be read, and as `parse`.
    pub fn load(defaults: impl AsRef<Path>, options: Options) -> Result<Config> {
        let defaults = defaults.as_ref();
        let text = read_file(defaults).map_err(|error| Error::new(ErrorKind::BadDefaults, "", error.message()))?;
        let origin = defaults.display().to_string();
        match std::str::from_utf8(&text) {
            Ok(text) => Config::parse(text, options, &origin),
            Err(error) => {
                let valid = std::str::from_utf8(&text[..error.valid_up_to()]).unwrap_or("");
                let (line, column) = position(valid, valid.len());
                Err(Error::new(ErrorKind::BadDefaults, "", format!("{origin}:{line}:{column}: invalid UTF-8")))
            }
        }
    }

    /// The defaults from TOML text (`origin` names it in errors). The files are read at once.
    ///
    /// # Errors
    ///
    /// `BadDefaults` when the text is no TOML, holds what is no setting, or the options name a key that is
    /// none or turn off the defaults or the overrides; `BadUserFile` as [`Config::reload_files`]; `Io` when one of the
    /// files cannot be read.
    pub fn parse(toml: &str, options: Options, origin: &str) -> Result<Config> {
        let root = DeTable::parse(toml).map_err(|error| {
            let message = error.message();
            let at = match error.span() {
                Some(span) => {
                    let (line, column) = position(toml, span.start);
                    format!(":{line}:{column}")
                }
                None => String::new(),
            };
            Error::new(ErrorKind::BadDefaults, "", format!("{origin}{at}: {message}"))
        })?;
        let mut flat = Vec::new();
        let mut sections = BTreeSet::new();
        flatten(root.get_ref(), "", origin, toml, &mut flat, &mut sections)?;
        let mut config = Config {
            options,
            keys: flat.iter().map(|(key, _)| key.clone()).collect(),
            settings: BTreeMap::new(),
            sections,
            overrides: Vec::new(),
            user: BTreeMap::new(),
            files: BTreeMap::new(),
            written: BTreeMap::new(),
        };
        // A key written twice ("a.b" and b in [a]) is the setting its first one says.
        for (key, setting) in flat {
            config.settings.entry(key).or_insert(setting);
        }
        let groups = [&config.options.file_only, &config.options.persisted, &config.options.secret];
        for key in groups.into_iter().flatten() {
            config.setting(key)?; // each must be a setting
        }
        if let Some(layer) =
            config.options.disabled.iter().find(|layer| matches!(layer, Source::Default | Source::Override))
        {
            let message = format!("the layer of the {}s cannot be turned off", layer.name());
            return Err(Error::new(ErrorKind::BadDefaults, "", message));
        }
        config.reload_files()?;
        Ok(config)
    }

    /// The settings, in the order the TOML file has them.
    #[must_use]
    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.settings.contains_key(key)
    }

    /// Whether `name` is a table settings are in (`server` for `server.port`).
    #[must_use]
    pub fn is_section(&self, name: &str) -> bool {
        self.sections.contains(name)
    }

    /// # Errors
    ///
    /// `UnknownKey` for no such setting.
    pub fn type_of(&self, key: &str) -> Result<Type> {
        Ok(self.setting(key)?.ty)
    }

    /// # Errors
    ///
    /// `UnknownKey` for no such setting.
    pub fn default_value(&self, key: &str) -> Result<&Value> {
        Ok(&self.setting(key)?.fallback)
    }

    /// Whether a layer is on: not one of [`Options::disabled`].
    #[must_use]
    pub fn is_on(&self, layer: Source) -> bool {
        !self.options.disabled.contains(&layer)
    }

    /// The setting's name in the environment and the files: the prefix and the key upper-cased, dots made underscores.
    #[must_use]
    pub fn variable(&self, key: &str) -> String {
        let mut name = self.options.prefix.clone();
        name.extend(key.chars().map(|c| if c == '.' { '_' } else { c.to_ascii_uppercase() }));
        name
    }

    #[must_use]
    pub fn options(&self) -> &Options {
        &self.options
    }

    fn setting(&self, key: &str) -> Result<&Setting> {
        self.settings.get(key).ok_or_else(|| Error::new(ErrorKind::UnknownKey, key, format!("unknown setting: {key}")))
    }

    fn override_of(&self, key: &str) -> Option<&Raw> {
        self.overrides.iter().find(|(name, _)| name == key).map(|(_, raw)| raw)
    }

    /// The environment's variable as a setting's value: nothing while the layer is off.
    fn env_value(&self, name: &OsStr) -> Option<OsString> {
        if self.is_on(Source::Env) { self.env(name) } else { None }
    }

    /// What the env-style files set a name to: nothing while the layer is off.
    fn file_value(&self, name: &OsStr) -> Option<&FileValue> {
        if self.is_on(Source::File) { self.files.get(name) } else { None }
    }

    /// What the user's files set a setting to: nothing while the layer is off.
    fn user_value(&self, key: &str) -> Option<&UserValue> {
        if self.is_on(Source::User) { self.user.get(key) } else { None }
    }

    /// The environment's variable, or what persist wrote into it while the environment has the value it had then.
    fn env(&self, name: &OsStr) -> Option<OsString> {
        let current = self.options.environment.get(name)?;
        match self.written.get(name) {
            Some(written) if written.seen == current => Some(written.text.clone()),
            _ => Some(current),
        }
    }

    fn convert(&self, key: &str, setting: &Setting, raw: &Raw) -> Result<Value> {
        convert_to(setting.ty, raw).ok_or_else(|| Error {
            kind: ErrorKind::InvalidValue,
            key: key.to_owned(),
            // A secret's value is kept out of the message, which is apt to be logged.
            message: format!(
                "invalid value for {key}: {} (expected {})",
                if self.options.secret.contains(key) { "***".to_owned() } else { shown_in_message(raw) },
                setting.ty.name()
            ),
            raw: Some(raw.clone()),
            expected: Some(setting.ty),
        })
    }

    /// A raw value converted to a setting's type, as the layers would.
    ///
    /// # Errors
    ///
    /// `UnknownKey` for no such setting, `InvalidValue` for a value that does not convert.
    pub fn cast(&self, key: &str, raw: impl Into<Raw>) -> Result<Value> {
        self.convert(key, self.setting(key)?, &raw.into())
    }

    /// A setting's value, from the highest layer that has one, converted to its type.
    ///
    /// # Errors
    ///
    /// `UnknownKey` for no such setting, `InvalidValue` for a value that does not convert.
    pub fn get(&self, key: &str) -> Result<Value> {
        let setting = self.setting(key)?;
        if let Some(given) = self.override_of(key) {
            return self.convert(key, setting, given);
        }
        let name = OsString::from(self.variable(key));
        if let Some(text) = self.env_value(&name) {
            return self.convert(key, setting, &Raw::Text(text));
        }
        if let Some(found) = self.file_value(&name) {
            return self.convert(key, setting, &Raw::Text(found.text.clone()));
        }
        if let Some(found) = self.user_value(key) {
            return Ok(found.value.clone()); // converted as the file was read
        }
        Ok(setting.fallback.clone())
    }

    /// # Errors
    ///
    /// `UnknownKey` for no such setting.
    pub fn source(&self, key: &str) -> Result<Source> {
        self.setting(key)?;
        if self.override_of(key).is_some() {
            return Ok(Source::Override);
        }
        let name = OsString::from(self.variable(key));
        Ok(if self.env_value(&name).is_some() {
            Source::Env
        } else if self.file_value(&name).is_some() {
            Source::File
        } else if self.user_value(key).is_some() {
            Source::User
        } else {
            Source::Default
        })
    }

    /// The value as it is shown: "***" for a secret setting that has a value.
    ///
    /// # Errors
    ///
    /// `UnknownKey` for no such setting, `InvalidValue` for a value that does not convert.
    pub fn shown(&self, key: &str) -> Result<Value> {
        let value = self.get(key)?;
        if self.options.secret.contains(key) && truthy(&value) {
            return Ok(Value::Str("***".to_owned()));
        }
        Ok(value)
    }

    /// A path setting: `~` expanded, a relative one joined to the root.
    ///
    /// # Errors
    ///
    /// `UnknownKey` for no such setting, `InvalidValue` for a value that does not convert.
    pub fn path(&self, key: &str) -> Result<PathBuf> {
        let text = match self.get(key)? {
            Value::Str(text) => text,
            value => value.to_string(),
        };
        let path = files::expand_user(text.as_bytes(), &self.options.environment);
        Ok(match &self.options.root {
            Some(root) if path.is_relative() && !root.as_os_str().is_empty() => root.join(path),
            _ => path,
        })
    }

    /// Every value of the environment and the env-style files that does not convert; an override is checked when it is
    /// set, and the user's files when they are read.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        let mut found = Vec::new();
        for key in &self.keys {
            if self.override_of(key).is_some() {
                continue;
            }
            let setting = &self.settings[key];
            let variable = self.variable(key);
            let name = OsString::from(&variable);
            let (source, file, text) = if let Some(text) = self.env_value(&name) {
                (Source::Env, None, text)
            } else if let Some(value) = self.file_value(&name) {
                (Source::File, value.file.clone(), value.text.clone())
            } else {
                continue;
            };
            if convert_to(setting.ty, &Raw::Text(text.clone())).is_none() {
                found.push(Problem { key: key.clone(), source, variable, file, text, expected: setting.ty });
            }
        }
        found
    }

    /// Override a setting, the highest layer. The value is converted at once, so that a mistake is reported where it
    /// is made.
    ///
    /// # Errors
    ///
    /// `UnknownKey` for no such setting, `FileOnly` for a setting only the files and the environment may change,
    /// `InvalidValue` for a value that does not convert.
    pub fn set(&mut self, key: &str, raw: impl Into<Raw>) -> Result<()> {
        let setting = self.setting(key)?;
        if self.options.file_only.contains(key) {
            return Err(Error::new(ErrorKind::FileOnly, key, self.file_only_message(key)));
        }
        let raw = raw.into();
        self.convert(key, setting, &raw)?;
        match self.overrides.iter_mut().find(|(name, _)| name == key) {
            Some(entry) => entry.1 = raw,
            None => self.overrides.push((key.to_owned(), raw)),
        }
        Ok(())
    }

    /// Where a setting that cannot be overridden may be set instead, as the layers that are on have it.
    fn file_only_message(&self, key: &str) -> String {
        let variable = self.variable(key);
        let mut places = Vec::new();
        if self.is_on(Source::User) && !self.options.user_files.is_empty() {
            places.push(format!("{key} in a TOML file of settings"));
        }
        match (self.is_on(Source::File), self.is_on(Source::Env)) {
            (true, true) => places.push(format!("{variable} in a file or the environment")),
            (true, false) => places.push(format!("{variable} in a file")),
            (false, true) => places.push(format!("{variable} in the environment")),
            (false, false) => {}
        }
        if places.is_empty() {
            format!("{key} cannot be overridden")
        } else {
            format!("{key} cannot be overridden: set {}", places.join(" or "))
        }
    }

    /// # Errors
    ///
    /// `UnknownKey` for no such setting.
    pub fn unset(&mut self, key: &str) -> Result<()> {
        self.setting(key)?;
        self.overrides.retain(|(name, _)| name != key);
        Ok(())
    }

    pub fn clear_overrides(&mut self) {
        self.overrides.clear();
    }

    /// The overrides as given, in the order first set.
    #[must_use]
    pub fn override_values(&self) -> &[(String, Raw)] {
        &self.overrides
    }

    /// The overrides as text that `set` takes again, in the order first set: text as it was given (a path keeps its
    /// ~), anything else as its converted value, which the type reads back (8421.0 for an integer is 8421).
    #[must_use]
    pub fn overrides(&self) -> Vec<(String, OsString)> {
        self.overrides
            .iter()
            .map(|(key, raw)| {
                let text = match raw {
                    Raw::Text(text) => text.clone(),
                    // A key no setting has comes only from the snapshot of another configuration.
                    raw => match self.settings.get(key).and_then(|setting| convert_to(setting.ty, raw)) {
                        Some(value) => value.to_string().into(),
                        None => OsString::new(), // not set: `set` converted it
                    },
                };
                (key.clone(), text)
            })
            .collect()
    }

    /// What was read from the user's TOML files: setting -> value, converted to its type.
    #[must_use]
    pub fn user_values(&self) -> BTreeMap<String, Value> {
        self.user.iter().map(|(key, found)| (key.clone(), found.value.clone())).collect()
    }

    /// Replace what was read from the user's files with the settings of TOML text (tests, or text a caller read
    /// itself), said to be written in `file`.
    ///
    /// # Errors
    ///
    /// `BadUserFile` as [`Config::reload_files`]; what was read stays as it was.
    pub fn set_user_text(&mut self, toml: &str, file: &Path) -> Result<()> {
        let mut read = BTreeMap::new();
        self.read_user_text(toml, file, &mut read)?;
        self.user = read;
        Ok(())
    }

    /// What was read from the env-style files: variable -> text.
    #[must_use]
    pub fn file_values(&self) -> BTreeMap<OsString, OsString> {
        self.files.iter().map(|(name, value)| (name.clone(), value.text.clone())).collect()
    }

    /// Replace what was read from the files (tests, or values a caller read itself); `file` is said to be where they
    /// are written, by default the file persist writes into.
    pub fn set_file_values(&mut self, values: BTreeMap<OsString, OsString>, file: Option<&Path>) {
        let file = file.map(Path::to_path_buf).or_else(|| self.write_target());
        self.files = values.into_iter().map(|(name, text)| (name, FileValue { text, file: file.clone() })).collect();
    }

    fn write_target(&self) -> Option<PathBuf> {
        self.options.write_file.clone().or_else(|| self.options.files.last().cloned())
    }

    /// Read the files again: the user's TOML files and the env-style files, those of the layers that are on. Nothing
    /// changes when one of them fails.
    ///
    /// # Errors
    ///
    /// `BadUserFile` when one of the user's files is not UTF-8 or no TOML, or holds a key that is no setting or a value
    /// that does not convert to its setting's type (the message says the file, the line and the column); `Io` when one
    /// of the files cannot be read.
    pub fn reload_files(&mut self) -> Result<()> {
        let mut user = BTreeMap::new();
        if self.is_on(Source::User) {
            for file in &self.options.user_files {
                if file.as_os_str().is_empty() || !file.is_file() {
                    continue;
                }
                let text = read_file(file)?;
                let text = std::str::from_utf8(&text).map_err(|error| {
                    let valid = std::str::from_utf8(&text[..error.valid_up_to()]).unwrap_or("");
                    let (line, column) = position(valid, valid.len());
                    let message = format!("{}:{line}:{column}: invalid UTF-8", file.display());
                    Error::new(ErrorKind::BadUserFile, "", message)
                })?;
                self.read_user_text(text, file, &mut user)?;
            }
        }
        let mut read = BTreeMap::new();
        if self.is_on(Source::File) {
            let lookup = |name: &[u8]| self.env(OsStr::from_bytes(name)).map(OsString::into_vec);
            for file in &self.options.files {
                if file.as_os_str().is_empty() || !file.is_file() {
                    continue;
                }
                for assignment in envfile::parse(&read_file(file)?, &lookup) {
                    let value = FileValue { text: OsString::from_vec(assignment.value), file: Some(file.clone()) };
                    read.insert(OsString::from_vec(assignment.name), value);
                }
            }
        }
        self.user = user;
        self.files = read;
        Ok(())
    }

    /// The settings of one of the user's TOML files into `into`, over what an earlier file set.
    fn read_user_text(&self, toml: &str, file: &Path, into: &mut BTreeMap<String, UserValue>) -> Result<()> {
        let origin = file.display().to_string();
        let at = |offset: usize| {
            let (line, column) = position(toml, offset);
            format!("{origin}:{line}:{column}")
        };
        let root = DeTable::parse(toml).map_err(|error| {
            let place = error.span().map_or_else(|| origin.clone(), |span| at(span.start));
            Error::new(ErrorKind::BadUserFile, "", format!("{place}: {}", error.message()))
        })?;
        let mut tables = vec![(String::new(), root.get_ref())];
        while let Some((prefix, table)) = tables.pop() {
            for (name, node) in table {
                let key = format!("{prefix}{}", name.get_ref());
                // A mistake in the value is where the value is; one in the key, where the key is.
                let bad = |what: String| {
                    Error::new(ErrorKind::BadUserFile, key.clone(), format!("{}: {what}", at(node.span().start)))
                };
                let bad_key = |what: String| {
                    Error::new(ErrorKind::BadUserFile, key.clone(), format!("{}: {what}", at(name.span().start)))
                };
                let raw = match node.get_ref() {
                    DeValue::Table(inner) if self.sections.contains(&key) => {
                        tables.push((format!("{key}."), inner));
                        continue;
                    }
                    DeValue::Table(_) if self.settings.contains_key(&key) => {
                        return Err(bad_key(format!("{key} is a setting, not a table of settings")));
                    }
                    DeValue::Table(_) => return Err(bad_key(format!("unknown table of settings: {key}"))),
                    _ if self.sections.contains(&key) => {
                        return Err(bad_key(format!("{key} is a table of settings, not a setting")));
                    }
                    DeValue::Boolean(b) => Raw::Bool(*b),
                    DeValue::Integer(i) => Raw::Int(toml_int(i).ok_or_else(|| bad(format!("{key} {INT_PAST_RANGE}")))?),
                    DeValue::Float(f) => {
                        Raw::Float(toml_float(f).ok_or_else(|| bad(format!("{key} {FLOAT_PAST_RANGE}")))?)
                    }
                    DeValue::String(text) => Raw::Text(OsString::from(text.as_ref())),
                    DeValue::Array(array) => {
                        let items = array.iter().map(|item| item.get_ref().as_str().map(str::to_owned)).collect();
                        match items {
                            Some(items) => Raw::List(items),
                            None => return Err(bad(format!("{key} is a list of something else than text"))),
                        }
                    }
                    DeValue::Datetime(_) => return Err(bad(format!("{key} is a date or a time, which no setting is"))),
                };
                let setting = self.settings.get(&key).ok_or_else(|| bad_key(format!("unknown setting: {key}")))?;
                let value = self.convert(&key, setting, &raw).map_err(|error| {
                    let mut error = Error { kind: ErrorKind::BadUserFile, ..error };
                    error.message = format!("{}: {}", at(node.span().start), error.message);
                    error
                })?;
                into.insert(key, UserValue { value, file: file.to_path_buf() });
            }
        }
        Ok(())
    }

    /// The overrides, what was read from the files and what persist wrote, to be put back as they were (tests).
    #[must_use]
    pub fn state(&self) -> State {
        State {
            overrides: self.overrides.clone(),
            user: self.user.clone(),
            files: self.files.clone(),
            written: self.written.clone(),
        }
    }

    pub fn restore(&mut self, state: &State) {
        self.overrides.clone_from(&state.overrides);
        self.user.clone_from(&state.user);
        self.files.clone_from(&state.files);
        self.written.clone_from(&state.written);
    }

    /// Write a setting of `persisted` into a file for good: its assignment replaced (an `export ` before it allowed,
    /// all the lines of a quoted value), later ones of the same name dropped, everything else kept as it was, the file
    /// replaced at once with its permissions (0600 for a new one; a symbolic link stays and its target is written).
    /// It holds at once: the value read from the files changes with it, unless a file read after this one sets it too
    /// (that one still wins), and so does the environment's where it has the variable, for this configuration only
    /// (the process's environment is not changed), whenever the environment has the value it had then; an override
    /// still wins.
    /// Several processes writing at once do not corrupt the file, but the last one's file is what stays. Returns the
    /// value as the setting's type.
    ///
    /// # Errors
    ///
    /// `UnknownKey` for no such setting, `NotPersisted` for a setting that is not to be written, `InvalidValue` for a
    /// value that does not convert, `Io` when the env-style files are turned off, there is no file to write, or it
    /// cannot be read or written.
    pub fn persist(&mut self, key: &str, raw: impl Into<Raw>, file: Option<&Path>) -> Result<Value> {
        let setting = self.setting(key)?;
        if !self.is_on(Source::File) {
            return Err(Error::new(
                ErrorKind::Io,
                key,
                format!("cannot write {key}: the env-style files are turned off"),
            ));
        }
        if !self.options.persisted.contains(key) {
            return Err(Error::new(
                ErrorKind::NotPersisted,
                key,
                format!("{key} is not one of the settings to persist"),
            ));
        }
        let typed = self.convert(key, setting, &raw.into())?;
        let target = file
            .map(Path::to_path_buf)
            .or_else(|| self.write_target())
            .filter(|target| !target.as_os_str().is_empty())
            .ok_or_else(|| Error::new(ErrorKind::Io, key, format!("no file to write {key} into")))?;
        // Absent is new; one that cannot be looked at (a link the system will not follow, say) is not written.
        let old = match fs::metadata(&target) {
            Ok(_) => read_file(&target)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                let why = files::os_message(&error);
                return Err(Error::new(ErrorKind::Io, key, format!("cannot read {}: {why}", target.display())));
            }
        };
        let name = OsString::from(self.variable(key));
        let text = typed.to_string();
        let cannot =
            |why: &str| Error::new(ErrorKind::Io, key, format!("cannot write {key} into {}: {why}", target.display()));
        if !envfile::parse(&[name.as_bytes(), b"=x"].concat(), &|_| None).iter().any(|a| a.name == name.as_bytes()) {
            return Err(cannot(&format!("{} is no name an env-style file can have", name.display())));
        }
        let Some(replaced) = envfile::replace(&old, name.as_bytes(), &envfile::quote(text.as_bytes())) else {
            return Err(cannot("a quote in it that is never closed would take in the line"));
        };
        replace_file(&target, &replaced)?;
        // Where the files stand now: a file read after the written one that sets the name still wins.
        let files = &self.options.files;
        let rank = |file: Option<&Path>| {
            file.and_then(|file| files.iter().position(|f| f == file || files::same_file(file, f)))
                .unwrap_or(files.len()) // not one of the files: as if read last
        };
        let current = self.files.get(&name);
        if current.is_none_or(|current| rank(current.file.as_deref()) <= rank(Some(&target))) {
            self.files.insert(name.clone(), FileValue { text: text.clone().into(), file: Some(target) });
        }
        match self.options.environment.get(&name) {
            Some(seen) => self.written.insert(name, Written { seen, text: text.into() }),
            None => self.written.remove(&name),
        };
        Ok(typed)
    }
}
