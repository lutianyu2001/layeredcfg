//! The module layeredconf._core: the Rust library as it is. layeredconf/__init__.py makes the Python API of it
//! (attribute access, Python's exceptions and messages).
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use layeredconf::{Environment, ErrorKind, Options, Raw, Value};
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString};
use pyo3::{IntoPyObjectExt, create_exception};

// A failure comes out as _core.Error(kind, key, raw, expected, message), which the Python layer turns into its own
// exceptions with their messages.
create_exception!(_core, Error, PyException);

fn kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::UnknownKey => "unknown_key",
        ErrorKind::InvalidValue => "invalid_value",
        ErrorKind::FileOnly => "file_only",
        ErrorKind::NotPersisted => "not_persisted",
        ErrorKind::BadDefaults => "bad_defaults",
        ErrorKind::Io => "io",
    }
}

// Text to Python and back as os.environ has it: bytes that are not UTF-8 become surrogates and back (surrogateescape),
// so that such a value is reported as an invalid value instead of failing to convert.

fn to_python<'py>(py: Python<'py>, text: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    PyBytes::new(py, text).call_method1("decode", ("utf-8", "surrogateescape"))
}

fn from_python(text: &Bound<'_, PyAny>) -> PyResult<OsString> {
    if text.is_instance_of::<PyBytes>() {
        return Ok(OsString::from_vec(text.extract()?)); // a path as bytes
    }
    let bytes = text.call_method1("encode", ("utf-8", "surrogateescape"))?;
    Ok(OsString::from_vec(bytes.extract()?))
}

fn path_to_python<'py>(py: Python<'py>, path: &Path) -> PyResult<Bound<'py, PyAny>> {
    py.import("pathlib")?.getattr("Path")?.call1((to_python(py, path.as_os_str().as_bytes())?,))
}

/// A path argument; "" is none.
fn path_from_python(text: &Bound<'_, PyAny>) -> PyResult<Option<PathBuf>> {
    let text = from_python(text)?;
    Ok((!text.is_empty()).then(|| PathBuf::from(text)))
}

fn value_to_python<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    match value {
        Value::Bool(b) => b.into_bound_py_any(py),
        Value::Int(i) => i.into_bound_py_any(py),
        Value::Float(d) => d.into_bound_py_any(py),
        Value::Str(s) => s.into_bound_py_any(py),
        Value::List(items) => items.into_bound_py_any(py),
    }
}

fn raw_to_python<'py>(py: Python<'py>, raw: &Raw) -> PyResult<Bound<'py, PyAny>> {
    match raw {
        Raw::Bool(b) => b.into_bound_py_any(py),
        Raw::Int(i) => i.into_bound_py_any(py),
        Raw::Float(d) => d.into_bound_py_any(py),
        Raw::Text(text) => to_python(py, text.as_bytes()),
        Raw::List(items) => items.into_bound_py_any(py),
    }
}

/// A value as the Python layer gives it: bool, int, float, str, or a list of str.
fn raw_from_python(value: &Bound<'_, PyAny>) -> PyResult<Raw> {
    Ok(if value.is_instance_of::<PyBool>() {
        Raw::Bool(value.extract()?)
    } else if value.is_instance_of::<PyInt>() {
        Raw::Int(value.extract()?)
    } else if value.is_instance_of::<PyFloat>() {
        Raw::Float(value.extract()?)
    } else if value.is_instance_of::<PyString>() {
        Raw::Text(from_python(value)?)
    } else {
        Raw::List(value.extract()?)
    })
}

fn error_to_python(py: Python<'_>, error: &layeredconf::Error) -> PyErr {
    let raw = match error.raw() {
        Some(raw) => match raw_to_python(py, raw) {
            Ok(raw) => raw.unbind(),
            Err(failure) => return failure,
        },
        None => py.None(),
    };
    let expected = error.expected().map(|ty| ty.name());
    Error::new_err((kind_name(error.kind()), error.key().to_owned(), raw, expected, error.message().to_owned()))
}

/// The first exception the environment's `get` raised during a call into the core, which cannot carry it.
type Failure = Arc<Mutex<Option<PyErr>>>;

/// The environment as Python has it (os.environ.get).
fn python_environment(get: Py<PyAny>, failure: Failure) -> Environment {
    Environment::new(move |name| {
        Python::attach(|py| {
            let value = to_python(py, name.as_bytes()).and_then(|name| get.bind(py).call1((name,)));
            match value.and_then(|value| if value.is_none() { Ok(None) } else { from_python(&value).map(Some) }) {
                Ok(value) => value,
                Err(raised) => {
                    failure.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(raised);
                    None
                }
            }
        })
    })
}

#[pyclass(name = "State", module = "layeredconf._core", frozen)]
struct State(layeredconf::State);

#[pyclass(name = "Config", module = "layeredconf._core")]
struct Config {
    core: layeredconf::Config,
    failure: Failure,
}

/// A core call's result, or the exception the environment raised during it.
fn done<T>(py: Python<'_>, failure: &Failure, result: layeredconf::Result<T>) -> PyResult<T> {
    if let Some(raised) = failure.lock().unwrap_or_else(PoisonError::into_inner).take() {
        return Err(raised);
    }
    result.map_err(|error| error_to_python(py, &error))
}

#[pymethods]
impl Config {
    #[new]
    #[pyo3(signature = (
        defaults, *, prefix = String::new(), files = Vec::new(), write_file = None, root = None,
        file_only = BTreeSet::new(), persisted = BTreeSet::new(), secret = BTreeSet::new(), get = None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        defaults: &Bound<'_, PyAny>,
        prefix: String,
        files: Vec<Bound<'_, PyAny>>,
        write_file: Option<&Bound<'_, PyAny>>,
        root: Option<&Bound<'_, PyAny>>,
        file_only: BTreeSet<String>,
        persisted: BTreeSet<String>,
        secret: BTreeSet<String>,
        get: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let failure = Failure::default();
        let mut options = Options {
            prefix,
            files: files.iter().map(|file| from_python(file).map(PathBuf::from)).collect::<PyResult<_>>()?,
            write_file: write_file.map(path_from_python).transpose()?.flatten(),
            root: root.map(path_from_python).transpose()?.flatten(),
            file_only,
            persisted,
            secret,
            ..Options::default()
        };
        // The environment as the caller's language has it (Python's os.environ), when given.
        if let Some(get) = get.filter(|get| !get.is_none(py)) {
            options.environment = python_environment(get, failure.clone());
        }
        let defaults = PathBuf::from(from_python(defaults)?);
        let core = done(py, &failure, layeredconf::Config::load(defaults, options))?;
        Ok(Config { core, failure })
    }

    fn keys(&self) -> Vec<String> {
        self.core.keys().to_vec()
    }

    fn has(&self, key: &str) -> bool {
        self.core.has(key)
    }

    fn is_section(&self, name: &str) -> bool {
        self.core.is_section(name)
    }

    fn r#type(&self, py: Python<'_>, key: &str) -> PyResult<&'static str> {
        done(py, &self.failure, self.core.type_of(key)).map(|ty| ty.name())
    }

    fn default<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Bound<'py, PyAny>> {
        value_to_python(py, done(py, &self.failure, self.core.default_value(key))?)
    }

    fn variable(&self, key: &str) -> String {
        self.core.variable(key)
    }

    fn get<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Bound<'py, PyAny>> {
        value_to_python(py, &done(py, &self.failure, self.core.get(key))?)
    }

    fn source(&self, py: Python<'_>, key: &str) -> PyResult<&'static str> {
        done(py, &self.failure, self.core.source(key)).map(|source| source.name())
    }

    fn shown<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Bound<'py, PyAny>> {
        value_to_python(py, &done(py, &self.failure, self.core.shown(key))?)
    }

    fn path<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Bound<'py, PyAny>> {
        path_to_python(py, &done(py, &self.failure, self.core.path(key))?)
    }

    fn cast<'py>(&self, py: Python<'py>, key: &str, raw: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let raw = raw_from_python(raw)?;
        value_to_python(py, &done(py, &self.failure, self.core.cast(key, raw))?)
    }

    /// (key, source, variable, file or None, text, expected) for each problem.
    fn problems<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let problems = done(py, &self.failure, Ok(self.core.problems()))?;
        let out = PyList::empty(py);
        for problem in problems {
            let file = match &problem.file {
                Some(file) => path_to_python(py, file)?,
                None => py.None().into_bound(py),
            };
            out.append((
                problem.key,
                problem.source.name(),
                problem.variable,
                file,
                to_python(py, problem.text.as_bytes())?,
                problem.expected.name(),
            ))?;
        }
        Ok(out)
    }

    fn set(&mut self, py: Python<'_>, key: &str, raw: &Bound<'_, PyAny>) -> PyResult<()> {
        let raw = raw_from_python(raw)?;
        done(py, &self.failure, self.core.set(key, raw))
    }

    fn unset(&mut self, py: Python<'_>, key: &str) -> PyResult<()> {
        done(py, &self.failure, self.core.unset(key))
    }

    fn clear_overrides(&mut self) {
        self.core.clear_overrides();
    }

    fn override_values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        for (key, raw) in self.core.override_values() {
            out.append((key, raw_to_python(py, raw)?))?;
        }
        Ok(out)
    }

    fn overrides<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        for (key, text) in self.core.overrides() {
            out.append((key, to_python(py, text.as_bytes())?))?;
        }
        Ok(out)
    }

    fn file_values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        for (name, text) in self.core.file_values() {
            out.set_item(to_python(py, name.as_bytes())?, to_python(py, text.as_bytes())?)?;
        }
        Ok(out)
    }

    #[pyo3(signature = (values, file = None))]
    fn set_file_values(&mut self, values: &Bound<'_, PyDict>, file: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let mut read = BTreeMap::new();
        for (name, text) in values.iter() {
            read.insert(from_python(&name)?, from_python(&text)?);
        }
        let file = file.map(path_from_python).transpose()?.flatten();
        self.core.set_file_values(read, file.as_deref());
        Ok(())
    }

    fn reload_files(&mut self, py: Python<'_>) -> PyResult<()> {
        done(py, &self.failure, self.core.reload_files())
    }

    fn state(&self) -> State {
        State(self.core.state())
    }

    fn restore(&mut self, state: &State) {
        self.core.restore(&state.0);
    }

    #[pyo3(signature = (key, raw, file = None))]
    fn persist<'py>(
        &mut self,
        py: Python<'py>,
        key: &str,
        raw: &Bound<'py, PyAny>,
        file: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let raw = raw_from_python(raw)?;
        let file = file.map(path_from_python).transpose()?.flatten();
        let value = done(py, &self.failure, self.core.persist(key, raw, file.as_deref()))?;
        value_to_python(py, &value)
    }
}

/// A value as text, as the core writes it.
#[pyfunction]
fn to_text<'py>(py: Python<'py>, value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    match raw_from_python(value)? {
        Raw::Text(text) => to_python(py, text.as_bytes()),
        Raw::Bool(b) => Value::Bool(b).to_string().into_bound_py_any(py),
        Raw::Int(i) => Value::Int(i).to_string().into_bound_py_any(py),
        Raw::Float(d) => Value::Float(d).to_string().into_bound_py_any(py),
        Raw::List(items) => Value::List(items).to_string().into_bound_py_any(py),
    }
}

/// The first file `name` in `start` or a directory above it.
#[pyfunction]
fn find_upward<'py>(
    py: Python<'py>,
    name: &Bound<'py, PyAny>,
    start: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let found = layeredconf::find_upward(from_python(name)?, from_python(start)?);
    found.map(|path| path_to_python(py, &path)).transpose()
}

/// The Rust core of layeredconf.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("Error", m.py().get_type::<Error>())?;
    m.add_class::<Config>()?;
    m.add_class::<State>()?;
    m.add_function(wrap_pyfunction!(to_text, m)?)?;
    m.add_function(wrap_pyfunction!(find_upward, m)?)?;
    Ok(())
}
