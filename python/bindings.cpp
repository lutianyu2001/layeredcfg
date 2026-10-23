// The module layeredconf._core: the C++ library as it is. layeredconf/__init__.py makes the Python API of it (attribute
// access, Python's exceptions and messages).
#include <pybind11/functional.h>
#include <pybind11/pybind11.h>
#include <pybind11/stl.h>
#include <pybind11/stl/filesystem.h>

#include "layeredconf/layeredconf.hpp"

namespace py = pybind11;
namespace fs = std::filesystem;
using namespace layeredconf;

namespace {

std::string kind_name(ErrorKind kind) {
  switch (kind) {
    case ErrorKind::UnknownKey:
      return "unknown_key";
    case ErrorKind::InvalidValue:
      return "invalid_value";
    case ErrorKind::FileOnly:
      return "file_only";
    case ErrorKind::NotPersisted:
      return "not_persisted";
    case ErrorKind::BadDefaults:
      return "bad_defaults";
    case ErrorKind::Io:
      return "io";
  }
  return "?";
}

// Text to Python and back as os.environ has it: bytes that are not UTF-8 become surrogates and back (surrogateescape),
// so that such a value is reported as an invalid value instead of failing to convert.
py::str to_python(const std::string& text) {
  PyObject* decoded = PyUnicode_DecodeUTF8(text.data(), static_cast<Py_ssize_t>(text.size()), "surrogateescape");
  if (!decoded) throw py::error_already_set();
  return py::reinterpret_steal<py::str>(decoded);
}

std::string from_python(const py::handle& text) {
  PyObject* encoded = PyUnicode_AsEncodedString(text.ptr(), "utf-8", "surrogateescape");
  if (!encoded) throw py::error_already_set();
  py::bytes bytes = py::reinterpret_steal<py::bytes>(encoded);
  return std::string(bytes);
}

py::object raw_to_python(const Value& value) {
  if (const auto* text = std::get_if<std::string>(&value)) return to_python(*text);
  return py::cast(value);
}

Config make(const fs::path& defaults, const std::string& prefix, const std::vector<fs::path>& files,
            const fs::path& write_file, const fs::path& root, const std::set<std::string>& file_only,
            const std::set<std::string>& persisted, const std::set<std::string>& secret, const py::object& get,
            const py::object& set) {
  Options options;
  options.prefix = prefix;
  options.files = files;
  options.write_file = write_file;
  options.root = root;
  options.file_only = file_only;
  options.persisted = persisted;
  options.secret = secret;
  // The environment as the caller's language has it (Python's os.environ), when given.
  if (!get.is_none()) {
    options.environment.get = [get](const std::string& name) -> std::optional<std::string> {
      py::object value = get(name);
      if (value.is_none()) return std::nullopt;
      return from_python(value);
    };
  }
  if (!set.is_none()) {
    options.environment.set = [set](const std::string& name, const std::string& value) { set(name, value); };
  }
  return Config::load(defaults, std::move(options));
}

}  // namespace

// A path argument's default is "": pybind11 keeps a default as a Python object, and an empty std::filesystem::path made
// into one is Path("."), which comes back as "." and is no longer empty.
PYBIND11_MODULE(_core, m) {
  m.doc() = "The C++ core of layeredconf.";
  m.attr("__version__") = LAYEREDCONF_VERSION;

  // A failure comes out as _core.Error(kind, key, raw, expected, message), which the Python layer turns into its own
  // exceptions with their messages.
  PYBIND11_CONSTINIT static py::gil_safe_call_once_and_store<py::object> error_type;
  error_type.call_once_and_store_result([&]() { return py::object(py::exception<Error>(m, "Error")); });
  py::register_exception_translator([](std::exception_ptr pointer) {
    try {
      if (pointer) std::rethrow_exception(pointer);
    } catch (const Error& error) {
      py::object raw = error.raw() ? raw_to_python(*error.raw()) : py::none();
      py::object expected = error.expected() ? py::object(py::str(type_name(*error.expected()))) : py::none();
      py::tuple args = py::make_tuple(kind_name(error.kind()), to_python(error.key()), raw, expected, to_python(error.what()));
      PyErr_SetObject(error_type.get_stored().ptr(), args.ptr());
    }
  });

  py::class_<State>(m, "State");

  py::class_<Config>(m, "Config")
      .def(py::init(&make), py::arg("defaults"), py::kw_only(), py::arg("prefix") = "",
           py::arg("files") = std::vector<fs::path>{}, py::arg("write_file") = "", py::arg("root") = "",
           py::arg("file_only") = std::set<std::string>{}, py::arg("persisted") = std::set<std::string>{},
           py::arg("secret") = std::set<std::string>{}, py::arg("get") = py::none(), py::arg("set") = py::none())
      .def("keys", &Config::keys)
      .def("has", &Config::has)
      .def("is_section", &Config::is_section)
      .def("type", [](const Config& config, const std::string& key) { return type_name(config.type(key)); })
      .def("default", &Config::default_value)
      .def("variable", &Config::variable)
      .def("get", &Config::get)
      .def("source", [](const Config& config, const std::string& key) { return source_name(config.source(key)); })
      .def("shown", &Config::shown)
      .def("path", &Config::path)
      .def("cast", &Config::cast)
      .def("problems",
           [](const Config& config) {
             py::list out;
             for (const Problem& problem : config.problems()) {
               py::object file = problem.file.empty() ? py::none() : py::cast(problem.file);
               out.append(py::make_tuple(problem.key, source_name(problem.source), problem.variable, file,
                                         to_python(problem.text), type_name(problem.expected)));
             }
             return out;
           })
      .def("set", &Config::set)
      .def("unset", &Config::unset)
      .def("clear_overrides", &Config::clear_overrides)
      .def("override_values", &Config::override_values)
      .def("overrides", &Config::overrides)
      .def("file_values",
           [](const Config& config) {
             py::dict out;
             for (const auto& [name, text] : config.file_values()) out[to_python(name)] = to_python(text);
             return out;
           })
      .def("set_file_values", &Config::set_file_values, py::arg("values"), py::arg("file") = "")
      .def("reload_files", &Config::reload_files)
      .def("state", &Config::state)
      .def("restore", &Config::restore)
      .def("persist", &Config::persist, py::arg("key"), py::arg("raw"), py::arg("file") = "");

  m.def("to_text", &to_text);
  m.def("find_upward", &find_upward, py::arg("name"), py::arg("start"));
}
