// layeredconf: layered configuration.
//
// A TOML file of defaults says which settings there are and the type of each: that of its default. A setting's value is
// taken from the highest layer that has one:
//
//   defaults (the TOML file) < env-style files < the environment < overrides (the command line)
//
// and converted to its default's type; a value that does not convert is an error, not a guess. A setting's name in the
// environment and in the files is the prefix and its key upper-cased, dots made underscores: `server.port` is
// `APP_SERVER_PORT` with the prefix `APP_`.
#pragma once

#include <cstdint>
#include <filesystem>
#include <functional>
#include <map>
#include <optional>
#include <set>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <variant>
#include <vector>

namespace layeredconf {

// The types a setting can have, those a TOML default can have that the other layers can write as text.
enum class Type { Bool, Int, Float, String, List };

using List = std::vector<std::string>;

// A typed value; given by a caller as a raw value, a string is text to convert and anything else is converted from
// that type.
using Value = std::variant<bool, std::int64_t, double, std::string, List>;

// Where a setting's value comes from.
enum class Source { Default, File, Env, Override };

enum class ErrorKind {
  UnknownKey,    // no such setting
  InvalidValue,  // a value that does not convert to the setting's type
  FileOnly,      // an override of a setting that only the files and the environment may change
  NotPersisted,  // persist for a setting that is not to be written
  BadDefaults,   // the TOML file cannot be read, or holds what is no setting
  Io,            // a file that cannot be read or written
};

class Error : public std::runtime_error {
 public:
  Error(ErrorKind kind, std::string key, const std::string& message, std::optional<Value> raw = std::nullopt,
        std::optional<Type> expected = std::nullopt);

  ErrorKind kind() const noexcept { return kind_; }
  const std::string& key() const noexcept { return key_; }
  // InvalidValue: the value that did not convert, and the type it should have converted to.
  const std::optional<Value>& raw() const noexcept { return raw_; }
  const std::optional<Type>& expected() const noexcept { return expected_; }

 private:
  ErrorKind kind_;
  std::string key_;
  std::optional<Value> raw_;
  std::optional<Type> expected_;
};

// How the environment is read and changed: the process's own (getenv, setenv) unless a binding gives its language's,
// so that the two always agree (Python's os.environ is a copy that the C library's setenv does not update).
struct Environment {
  std::function<std::optional<std::string>(const std::string&)> get;
  std::function<void(const std::string&, const std::string&)> set;

  static Environment process();
};

// A value of the environment or of a file that does not convert to its setting's type.
struct Problem {
  std::string key;
  Source source;               // Source::Env or Source::File
  std::string variable;        // its name there
  std::filesystem::path file;  // the file it is written in; empty for the environment
  std::string text;            // what is written
  Type expected;
};

struct Options {
  // What the names of the environment and the files begin with.
  std::string prefix;
  // Env-style files (KEY=value lines), read in this order: a later file wins over an earlier one.
  std::vector<std::filesystem::path> files;
  // Where persist writes when it is given no file: this one, or else the last of `files`.
  std::filesystem::path write_file;
  // What a relative path (`path`) is relative to; empty leaves it relative.
  std::filesystem::path root;
  // Settings that only the files and the environment may change: an override of one is refused.
  std::set<std::string> file_only;
  // Settings persist may write into a file.
  std::set<std::string> persisted;
  // Settings `shown` masks.
  std::set<std::string> secret;
  Environment environment = Environment::process();
};

class Config {
 public:
  // The defaults from a TOML file, or from TOML text (`origin` names it in errors). The files are read at once.
  static Config load(const std::filesystem::path& defaults, Options options);
  static Config parse(std::string_view toml, Options options, const std::string& origin = "defaults");

  // The settings, in the order the TOML file has them; the tables they are in (`server` for `server.port`).
  const std::vector<std::string>& keys() const noexcept { return keys_; }
  bool has(std::string_view key) const;
  bool is_section(std::string_view name) const;
  Type type(std::string_view key) const;
  const Value& default_value(std::string_view key) const;
  std::string variable(std::string_view key) const;
  const Options& options() const noexcept { return options_; }

  // A setting's value, from the highest layer that has one, converted to its type.
  Value get(std::string_view key) const;
  Source source(std::string_view key) const;
  // The value as it is shown: "***" for a secret setting that has a value.
  Value shown(std::string_view key) const;
  // A path setting: `~` expanded, a relative one joined to the root.
  std::filesystem::path path(std::string_view key) const;
  // Every value of the environment and the files that does not convert; an override is checked when it is set.
  std::vector<Problem> problems() const;
  // A raw value converted to a setting's type, as the layers would.
  Value cast(std::string_view key, const Value& raw) const;

  // The overrides, the highest layer. `set` converts the value at once, so that a mistake is reported where it is made.
  void set(std::string_view key, const Value& raw);
  void unset(std::string_view key);
  void clear_overrides();
  // As given, and as text that `set` would take again (lists comma-separated), both in the order first set.
  const std::vector<std::pair<std::string, Value>>& override_values() const noexcept { return overrides_; }
  std::vector<std::pair<std::string, std::string>> overrides() const;

  // What was read from the files: variable -> text.
  std::map<std::string, std::string> file_values() const;
  // Replace it (tests, or values a caller read itself); `file` is said to be where they are written, by default the
  // file persist writes into.
  void set_file_values(const std::map<std::string, std::string>& values, const std::filesystem::path& file = {});
  void reload_files();

  // Write a setting of `persisted` into a file for good: its line replaced (an `export ` before it allowed), later
  // lines of the same name dropped, every other line kept as it was, the file replaced at once with its permissions
  // (0600 for a new one). The value read from the files changes with it, and the environment's too where it has the
  // variable, so that it holds at once; an override still wins. Returns the value as the setting's type.
  Value persist(std::string_view key, const Value& raw, const std::filesystem::path& file = {});

 private:
  struct Setting {
    Type type;
    Value fallback;
  };
  struct FileValue {
    std::string text;
    std::filesystem::path file;
  };

  Config() = default;
  const Setting& setting(std::string_view key) const;
  std::filesystem::path write_target() const;
  const Value* override_of(std::string_view key) const;
  std::optional<std::string> env(const std::string& name) const;
  Value convert(std::string_view key, const Setting& setting, const Value& raw) const;

  Options options_;
  std::vector<std::string> keys_;
  std::map<std::string, Setting, std::less<>> settings_;
  std::set<std::string, std::less<>> sections_;
  std::vector<std::pair<std::string, Value>> overrides_;
  std::map<std::string, FileValue, std::less<>> files_;
};

// Names for messages and bindings: "bool", "int", "float", "string", "list"; "default", "file", "env", "override".
std::string type_name(Type type);
std::string source_name(Source source);

// A value as text: true and false as True and False, floats as their shortest round-tripping form with a ".0" when
// that has no point or exponent (as Python writes them), lists comma-separated.
std::string to_text(const Value& value);

// The first directory from `start` upwards that has a regular file `name`, and that file.
std::optional<std::filesystem::path> find_upward(const std::string& name, const std::filesystem::path& start);

}  // namespace layeredconf
