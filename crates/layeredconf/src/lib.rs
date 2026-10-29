//! layeredconf: layered configuration.
//!
//! A TOML file of defaults says which settings there are and the type of each: that of its default. A setting's value
//! is taken from the highest layer that has one:
//!
//! ```text
//! defaults (the TOML file) < env-style files < the environment < overrides (the command line)
//! ```
//!
//! and converted to its default's type; a value that does not convert is an error, not a guess. A setting's name in the
//! environment and in the files is the prefix and its key upper-cased, dots made underscores: `server.port` is
//! `APP_SERVER_PORT` with the prefix `APP_`.
//!
//! Text from the environment and the files is bytes ([`OsString`]), which need not be UTF-8: a value that is not is an
//! invalid value when it is converted. POSIX only.
#![forbid(unsafe_code)]

mod config;
mod files;

// Not part of the API: public for the fuzz targets.
#[doc(hidden)]
pub mod envfile;
#[doc(hidden)]
pub mod text;

#[cfg(kani)]
mod proofs;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

pub use config::Config;
pub use files::find_upward;

/// The types a setting can have, those a TOML default can have that the other layers can write as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Bool,
    Int,
    Float,
    String,
    List,
}

impl Type {
    /// "bool", "int", "float", "string", "list".
    pub fn name(self) -> &'static str {
        match self {
            Type::Bool => "bool",
            Type::Int => "int",
            Type::Float => "float",
            Type::String => "string",
            Type::List => "list",
        }
    }
}

/// A setting's value, of its type.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<String>),
}

impl Value {
    pub fn type_of(&self) -> Type {
        match self {
            Value::Bool(_) => Type::Bool,
            Value::Int(_) => Type::Int,
            Value::Float(_) => Type::Float,
            Value::Str(_) => Type::String,
            Value::List(_) => Type::List,
        }
    }
}

/// The value as text: true and false as True and False, floats as their shortest round-tripping form with a ".0" when
/// that has no point or exponent (as Python writes them), lists comma-separated.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => f.write_str(if *b { "True" } else { "False" }),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(d) => f.write_str(&text::format_float(*d)),
            Value::Str(s) => f.write_str(s),
            Value::List(items) => f.write_str(&items.join(",")),
        }
    }
}

/// A value as a caller gives it: text to convert, or a value converted from its type.
#[derive(Clone, Debug, PartialEq)]
pub enum Raw {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(OsString),
    List(Vec<String>),
}

impl From<Value> for Raw {
    fn from(value: Value) -> Self {
        match value {
            Value::Bool(b) => Raw::Bool(b),
            Value::Int(i) => Raw::Int(i),
            Value::Float(d) => Raw::Float(d),
            Value::Str(s) => Raw::Text(s.into()),
            Value::List(items) => Raw::List(items),
        }
    }
}

impl From<bool> for Raw {
    fn from(value: bool) -> Self {
        Raw::Bool(value)
    }
}

impl From<i64> for Raw {
    fn from(value: i64) -> Self {
        Raw::Int(value)
    }
}

impl From<f64> for Raw {
    fn from(value: f64) -> Self {
        Raw::Float(value)
    }
}

impl From<&str> for Raw {
    fn from(value: &str) -> Self {
        Raw::Text(value.into())
    }
}

impl From<String> for Raw {
    fn from(value: String) -> Self {
        Raw::Text(value.into())
    }
}

impl From<OsString> for Raw {
    fn from(value: OsString) -> Self {
        Raw::Text(value)
    }
}

impl From<Vec<String>> for Raw {
    fn from(value: Vec<String>) -> Self {
        Raw::List(value)
    }
}

/// Where a setting's value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Default,
    File,
    Env,
    Override,
}

impl Source {
    /// "default", "file", "env", "override".
    pub fn name(self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::File => "file",
            Source::Env => "env",
            Source::Override => "override",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// No such setting.
    UnknownKey,
    /// A value that does not convert to the setting's type.
    InvalidValue,
    /// An override of a setting that only the files and the environment may change.
    FileOnly,
    /// persist for a setting that is not to be written.
    NotPersisted,
    /// The TOML file cannot be read, or holds what is no setting.
    BadDefaults,
    /// A file that cannot be read or written.
    Io,
}

#[derive(Clone, Debug)]
pub struct Error {
    kind: ErrorKind,
    key: String,
    message: String,
    raw: Option<Raw>,
    expected: Option<Type>,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, key: impl Into<String>, message: impl Into<String>) -> Self {
        Error { kind, key: key.into(), message: message.into(), raw: None, expected: None }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The setting, empty when the error is no setting's.
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// InvalidValue: the value that did not convert.
    pub fn raw(&self) -> Option<&Raw> {
        self.raw.as_ref()
    }

    /// InvalidValue: the type it should have converted to.
    pub fn expected(&self) -> Option<Type> {
        self.expected
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// How the environment is read: the process's own unless a binding gives its language's, so that the two always agree
/// (Python's os.environ is a copy that the C library's environment does not follow). It is only ever read: persist
/// keeps what it writes in the configuration, and the process's environment stays as it is.
#[derive(Clone)]
pub struct Environment {
    get: Arc<Lookup>,
}

type Lookup = dyn Fn(&OsStr) -> Option<OsString> + Send + Sync;

impl Environment {
    /// The process's environment.
    pub fn process() -> Self {
        Environment::new(|name| {
            // A name std::env would refuse is no variable.
            let bytes = std::os::unix::ffi::OsStrExt::as_bytes(name);
            if bytes.is_empty() || bytes.contains(&b'=') || bytes.contains(&0) {
                return None;
            }
            std::env::var_os(name)
        })
    }

    pub fn new(get: impl Fn(&OsStr) -> Option<OsString> + Send + Sync + 'static) -> Self {
        Environment { get: Arc::new(get) }
    }

    pub fn get(&self, name: &OsStr) -> Option<OsString> {
        (self.get)(name)
    }
}

impl Default for Environment {
    fn default() -> Self {
        Environment::process()
    }
}

impl fmt::Debug for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Environment")
    }
}

/// A value of the environment or of a file that does not convert to its setting's type.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub key: String,
    /// Source::Env or Source::File.
    pub source: Source,
    /// Its name there.
    pub variable: String,
    /// The file it is written in; nothing for the environment.
    pub file: Option<PathBuf>,
    /// What is written.
    pub text: OsString,
    pub expected: Type,
}

/// A value read from a file, and the file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FileValue {
    pub(crate) text: OsString,
    pub(crate) file: Option<PathBuf>,
}

/// A variable persist wrote while the environment had it: its value then, and what was written. It is the variable's
/// value for as long as the environment still has that value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Written {
    pub(crate) seen: OsString,
    pub(crate) text: OsString,
}

/// The overrides, what was read from the files and what persist wrote, to be put back with [`Config::restore`]
/// (tests).
#[derive(Clone, Debug, Default)]
pub struct State {
    overrides: Vec<(String, Raw)>,
    files: BTreeMap<OsString, FileValue>,
    written: BTreeMap<OsString, Written>,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// What the names of the environment and the files begin with.
    pub prefix: String,
    /// Env-style files (KEY=value lines), read in this order: a later file wins over an earlier one.
    pub files: Vec<PathBuf>,
    /// Where persist writes when it is given no file: this one, or else the last of `files`.
    pub write_file: Option<PathBuf>,
    /// What a relative path ([`Config::path`]) is relative to; nothing leaves it relative.
    pub root: Option<PathBuf>,
    /// Settings that only the files and the environment may change: an override of one is refused.
    pub file_only: BTreeSet<String>,
    /// Settings persist may write into a file.
    pub persisted: BTreeSet<String>,
    /// Settings [`Config::shown`] masks.
    pub secret: BTreeSet<String>,
    pub environment: Environment,
}
