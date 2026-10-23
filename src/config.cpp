#include <fcntl.h>
#include <pwd.h>
#include <sys/stat.h>
#include <unistd.h>

#include <algorithm>
#include <cerrno>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <functional>
#include <sstream>
#include <tuple>

#include <toml++/toml.hpp>

#include "envfile.hpp"
#include "layeredconf/layeredconf.hpp"
#include "text.hpp"

namespace layeredconf {

namespace fs = std::filesystem;

namespace {

// A value as it appears in a message: text quoted, the rest as text.
std::string shown_in_message(const Value& value) {
  if (const auto* s = std::get_if<std::string>(&value)) {
    std::string out = "\"";
    for (char c : *s) {
      if (c == '"' || c == '\\') out += '\\';
      out += c;
    }
    return out + "\"";
  }
  if (const auto* list = std::get_if<List>(&value)) {
    std::string out = "[";
    for (std::size_t i = 0; i < list->size(); ++i) out += (i ? ", " : "") + shown_in_message(Value{(*list)[i]});
    return out + "]";
  }
  return to_text(value);
}

bool truthy(const Value& value) {
  return std::visit(
      [](const auto& v) -> bool {
        using T = std::decay_t<decltype(v)>;
        if constexpr (std::is_same_v<T, bool>) {
          return v;
        } else if constexpr (std::is_same_v<T, std::int64_t>) {
          return v != 0;
        } else if constexpr (std::is_same_v<T, double>) {
          return v != 0.0;
        } else {
          return !v.empty();
        }
      },
      value);
}

// A raw value converted to a type, or nothing when it does not convert (Python's conversions: bool(x) of a value
// that is not text, int(2.7) is 2, str(True) is "True").
std::optional<Value> convert_to(Type type, const Value& raw) {
  if (const auto* text = std::get_if<std::string>(&raw)) {
    if (!text::valid_utf8(*text)) return std::nullopt;  // every layer's text is UTF-8
    switch (type) {
      case Type::Bool:
        if (auto v = text::parse_bool(*text)) return Value{*v};
        return std::nullopt;
      case Type::Int:
        if (auto v = text::parse_int(*text)) return Value{*v};
        return std::nullopt;
      case Type::Float:
        if (auto v = text::parse_float(*text)) return Value{*v};
        return std::nullopt;
      case Type::String:
        return raw;
      case Type::List:
        return Value{text::split_list(*text)};
    }
  }
  if (const auto* list = std::get_if<List>(&raw)) {
    switch (type) {
      case Type::Bool:
        return Value{!list->empty()};
      case Type::List: {
        List items;
        for (const std::string& item : *list) {
          std::string_view stripped = text::strip(item);
          if (!stripped.empty()) items.emplace_back(stripped);
        }
        return Value{std::move(items)};
      }
      default:
        return std::nullopt;
    }
  }
  if (type == Type::List) return std::nullopt;
  if (type == Type::Bool) return Value{truthy(raw)};
  if (type == Type::String) return Value{to_text(raw)};
  if (const auto* b = std::get_if<bool>(&raw)) {
    return type == Type::Int ? Value{static_cast<std::int64_t>(*b)} : Value{*b ? 1.0 : 0.0};
  }
  if (const auto* i = std::get_if<std::int64_t>(&raw)) {
    return type == Type::Int ? Value{*i} : Value{static_cast<double>(*i)};
  }
  double d = std::get<double>(raw);
  if (type == Type::Float) return Value{d};
  if (!std::isfinite(d) || d < -9223372036854775808.0 || d >= 9223372036854775808.0) return std::nullopt;
  return Value{static_cast<std::int64_t>(std::trunc(d))};
}

std::string read_file(const fs::path& path) {
  std::error_code error;
  if (!fs::is_regular_file(path, error)) throw Error(ErrorKind::Io, "", "cannot read " + path.string() + ": not a file");
  std::ifstream in(path, std::ios::binary);
  std::ostringstream text;
  if (in) text << in.rdbuf();
  if (!in || in.bad()) throw Error(ErrorKind::Io, "", "cannot read " + path.string() + ": " + std::strerror(errno));
  return text.str();
}

// Written beside the file and put in its place at once, so that a write that fails halfway leaves the old file whole;
// with the old file's permissions, 0600 for a new one, from the moment it is created.
void replace_file(const fs::path& given, const std::string& text) {
  std::error_code error_code;
  fs::path path = fs::exists(given, error_code) ? fs::canonical(given, error_code) : given;  // a link's target
  if (error_code) path = given;
  struct stat old {};
  mode_t mode = ::stat(path.c_str(), &old) == 0 ? (old.st_mode & 07777) : 0600;
  // A name of its own (mkstemp makes it 0600), so that writers at the same time do not write into one another's.
  std::string pattern = path.string() + ".writing-XXXXXX";
  int fd = ::mkstemp(pattern.data());
  if (fd < 0) throw Error(ErrorKind::Io, "", "cannot write beside " + path.string() + ": " + std::strerror(errno));
  fs::path spare = pattern;
  const char* data = text.data();
  std::size_t left = text.size();
  bool ok = ::fchmod(fd, mode) == 0;
  while (ok && left > 0) {
    ssize_t written = ::write(fd, data, left);
    if (written < 0 && errno == EINTR) continue;
    ok = written > 0;
    if (ok) {
      data += written;
      left -= static_cast<std::size_t>(written);
    }
  }
  ok = ::fsync(fd) == 0 && ok;
  int error = errno;
  ok = ::close(fd) == 0 && ok;
  if (!ok || ::rename(spare.c_str(), path.c_str()) != 0) {
    error = ok ? errno : error;
    ::unlink(spare.c_str());
    throw Error(ErrorKind::Io, "", "cannot write " + path.string() + ": " + std::strerror(error));
  }
}

fs::path expand_user(const std::string& text, const Environment& environment) {
  if (text.empty() || text[0] != '~') return fs::path(text);
  std::size_t slash = text.find('/');
  std::string user = text.substr(1, slash == std::string::npos ? std::string::npos : slash - 1);
  std::string home;
  if (user.empty()) {
    if (auto from_env = environment.get ? environment.get("HOME") : std::nullopt) {
      home = *from_env;
    } else if (const passwd* entry = ::getpwuid(::getuid())) {
      home = entry->pw_dir;
    }
  } else if (const passwd* entry = ::getpwnam(user.c_str())) {
    home = entry->pw_dir;
  }
  if (home.empty()) return fs::path(text);
  while (home.size() > 1 && home.back() == '/') home.pop_back();
  return fs::path(home + (slash == std::string::npos ? std::string() : text.substr(slash)));
}

struct Flat {
  std::string key;
  Type type;
  Value value;
};

// A table's entries in the order the file has them (toml++ keeps a table sorted by key), each table's own entries
// together, as Python's tomllib reads them.
void flatten(const toml::table& table, const std::string& prefix, const std::string& origin, std::vector<Flat>& out,
             std::set<std::string, std::less<>>& sections) {
  std::vector<std::pair<std::string, const toml::node*>> entries;
  for (auto&& [name, node] : table) entries.emplace_back(std::string(name.str()), &node);
  // A table first appears where the first thing in it is written ([x.y] before [x] makes x appear at [x.y]), as
  // Python's tomllib inserts it.
  std::function<std::tuple<std::uint32_t, std::uint32_t>(const toml::node*)> place = [&](const toml::node* node) {
    const auto& begin = node->source().begin;
    auto first = begin.line ? std::make_tuple(begin.line, begin.column) : std::make_tuple(~0u, ~0u);
    if (const toml::table* inner = node->as_table()) {
      for (auto&& [name, child] : *inner) first = std::min(first, place(&child));
    }
    return first;
  };
  std::stable_sort(entries.begin(), entries.end(),
                   [&](const auto& a, const auto& b) { return place(a.second) < place(b.second); });
  for (const auto& entry : entries) {
    const toml::node* node = entry.second;  // not a structured binding: a lambda below captures it
    std::string key = prefix + entry.first;
    auto bad = [&](const std::string& what) {
      const auto& begin = node->source().begin;
      return Error(ErrorKind::BadDefaults, key,
                   origin + ":" + std::to_string(begin.line) + ":" + std::to_string(begin.column) + ": " + key + " " +
                       what);
    };
    if (const toml::table* inner = node->as_table()) {
      sections.insert(key);
      flatten(*inner, key + ".", origin, out, sections);
    } else if (const auto* b = node->as_boolean()) {
      out.push_back({key, Type::Bool, Value{b->get()}});
    } else if (const auto* i = node->as_integer()) {
      out.push_back({key, Type::Int, Value{static_cast<std::int64_t>(i->get())}});
    } else if (const auto* f = node->as_floating_point()) {
      out.push_back({key, Type::Float, Value{f->get()}});
    } else if (const auto* s = node->as_string()) {
      out.push_back({key, Type::String, Value{s->get()}});
    } else if (const toml::array* array = node->as_array()) {
      List items;
      for (const toml::node& item : *array) {
        const auto* s = item.as_string();
        if (!s) throw bad("is a list of something else than text, which no other layer could write");
        items.push_back(s->get());
      }
      out.push_back({key, Type::List, Value{std::move(items)}});
    } else {
      throw bad("has a type no other layer could write (a date or a time)");
    }
  }
}

}  // namespace

Error::Error(ErrorKind kind, std::string key, const std::string& message, std::optional<Value> raw,
             std::optional<Type> expected)
    : std::runtime_error(message), kind_(kind), key_(std::move(key)), raw_(std::move(raw)), expected_(expected) {}

Environment Environment::process() {
  Environment environment;
  environment.get = [](const std::string& name) -> std::optional<std::string> {
    const char* value = std::getenv(name.c_str());
    if (!value) return std::nullopt;
    return std::string(value);
  };
  environment.set = [](const std::string& name, const std::string& value) { ::setenv(name.c_str(), value.c_str(), 1); };
  return environment;
}

std::string type_name(Type type) {
  switch (type) {
    case Type::Bool:
      return "bool";
    case Type::Int:
      return "int";
    case Type::Float:
      return "float";
    case Type::String:
      return "string";
    case Type::List:
      return "list";
  }
  return "?";
}

std::string source_name(Source source) {
  switch (source) {
    case Source::Default:
      return "default";
    case Source::File:
      return "file";
    case Source::Env:
      return "env";
    case Source::Override:
      return "override";
  }
  return "?";
}

std::string to_text(const Value& value) {
  return std::visit(
      [](const auto& v) -> std::string {
        using T = std::decay_t<decltype(v)>;
        if constexpr (std::is_same_v<T, bool>) {
          return v ? "True" : "False";
        } else if constexpr (std::is_same_v<T, std::int64_t>) {
          return std::to_string(v);
        } else if constexpr (std::is_same_v<T, double>) {
          return text::format_float(v);
        } else if constexpr (std::is_same_v<T, std::string>) {
          return v;
        } else {
          std::string out;
          for (std::size_t i = 0; i < v.size(); ++i) out += (i ? "," : "") + v[i];
          return out;
        }
      },
      value);
}

std::optional<fs::path> find_upward(const std::string& name, const fs::path& start) {
  std::error_code error;
  fs::path directory = fs::absolute(start, error).lexically_normal();
  if (error) return std::nullopt;
  if (directory.has_relative_path() && directory.filename().empty()) directory = directory.parent_path();
  while (true) {
    fs::path candidate = directory / name;
    if (fs::is_regular_file(candidate, error)) return candidate;
    fs::path parent = directory.parent_path();
    if (parent == directory || parent.empty()) return std::nullopt;
    directory = parent;
  }
}

Config Config::load(const fs::path& defaults, Options options) {
  std::string text;
  try {
    text = read_file(defaults);
  } catch (const Error& error) {
    throw Error(ErrorKind::BadDefaults, "", error.what());
  }
  return parse(text, std::move(options), defaults.string());
}

Config Config::parse(std::string_view toml_text, Options options, const std::string& origin) {
  toml::table root;
  try {
    root = toml::parse(toml_text, origin);
  } catch (const toml::parse_error& error) {
    const auto& begin = error.source().begin;
    throw Error(ErrorKind::BadDefaults, "",
                origin + ":" + std::to_string(begin.line) + ":" + std::to_string(begin.column) + ": " +
                    std::string(error.description()));
  }
  Config config;
  config.options_ = std::move(options);
  std::vector<Flat> flat;
  flatten(root, "", origin, flat, config.sections_);
  for (Flat& entry : flat) {
    config.keys_.push_back(entry.key);
    config.settings_.emplace(entry.key, Setting{entry.type, std::move(entry.value)});
  }
  for (const auto* group : {&config.options_.file_only, &config.options_.persisted, &config.options_.secret}) {
    for (const std::string& key : *group) config.setting(key);  // each must be a setting
  }
  config.reload_files();
  return config;
}

bool Config::has(std::string_view key) const { return settings_.find(key) != settings_.end(); }

bool Config::is_section(std::string_view name) const { return sections_.find(name) != sections_.end(); }

const Config::Setting& Config::setting(std::string_view key) const {
  auto found = settings_.find(key);
  if (found == settings_.end()) {
    throw Error(ErrorKind::UnknownKey, std::string(key), "unknown setting: " + std::string(key));
  }
  return found->second;
}

Type Config::type(std::string_view key) const { return setting(key).type; }

const Value& Config::default_value(std::string_view key) const { return setting(key).fallback; }

std::string Config::variable(std::string_view key) const {
  std::string name = options_.prefix;
  for (char c : key) {
    if (c == '.') {
      name += '_';
    } else if (c >= 'a' && c <= 'z') {
      name += static_cast<char>(c - 'a' + 'A');
    } else {
      name += c;
    }
  }
  return name;
}

const Value* Config::override_of(std::string_view key) const {
  for (const auto& [name, value] : overrides_) {
    if (name == key) return &value;
  }
  return nullptr;
}

std::optional<std::string> Config::env(const std::string& name) const {
  if (!options_.environment.get) return std::nullopt;
  return options_.environment.get(name);
}

Value Config::convert(std::string_view key, const Setting& setting, const Value& raw) const {
  if (auto value = convert_to(setting.type, raw)) return std::move(*value);
  throw Error(ErrorKind::InvalidValue, std::string(key),
              "invalid value for " + std::string(key) + ": " + shown_in_message(raw) + " (expected " +
                  type_name(setting.type) + ")",
              raw, setting.type);
}

Value Config::cast(std::string_view key, const Value& raw) const { return convert(key, setting(key), raw); }

Value Config::get(std::string_view key) const {
  const Setting& s = setting(key);
  if (const Value* given = override_of(key)) return convert(key, s, *given);
  std::string name = variable(key);
  if (auto text = env(name)) return convert(key, s, Value{std::move(*text)});
  auto found = files_.find(name);
  if (found != files_.end()) return convert(key, s, Value{found->second.text});
  return s.fallback;
}

Source Config::source(std::string_view key) const {
  setting(key);
  if (override_of(key)) return Source::Override;
  std::string name = variable(key);
  if (env(name)) return Source::Env;
  if (files_.find(name) != files_.end()) return Source::File;
  return Source::Default;
}

Value Config::shown(std::string_view key) const {
  Value value = get(key);
  if (options_.secret.count(std::string(key)) && truthy(value)) return Value{std::string("***")};
  return value;
}

fs::path Config::path(std::string_view key) const {
  Value value = get(key);
  const auto* text = std::get_if<std::string>(&value);
  fs::path path = expand_user(text ? *text : to_text(value), options_.environment);
  if (path.is_relative() && !options_.root.empty()) path = options_.root / path;
  return path;
}

std::vector<Problem> Config::problems() const {
  std::vector<Problem> found;
  for (const std::string& key : keys_) {
    if (override_of(key)) continue;
    const Setting& s = settings_.find(key)->second;
    std::string name = variable(key);
    Problem problem{key, Source::Env, name, {}, {}, s.type};
    if (auto text = env(name)) {
      problem.text = std::move(*text);
    } else if (auto file = files_.find(name); file != files_.end()) {
      problem.source = Source::File;
      problem.file = file->second.file;
      problem.text = file->second.text;
    } else {
      continue;
    }
    if (!convert_to(s.type, Value{problem.text})) found.push_back(std::move(problem));
  }
  return found;
}

void Config::set(std::string_view key, const Value& raw) {
  const Setting& s = setting(key);
  if (options_.file_only.count(std::string(key))) {
    throw Error(ErrorKind::FileOnly, std::string(key),
                std::string(key) + " cannot be overridden: set " + variable(key) + " in a file or the environment");
  }
  convert(key, s, raw);
  for (auto& entry : overrides_) {
    if (entry.first == key) {
      entry.second = raw;
      return;
    }
  }
  overrides_.emplace_back(std::string(key), raw);
}

void Config::unset(std::string_view key) {
  setting(key);
  overrides_.erase(std::remove_if(overrides_.begin(), overrides_.end(), [&](const auto& e) { return e.first == key; }),
                   overrides_.end());
}

void Config::clear_overrides() { overrides_.clear(); }

std::vector<std::pair<std::string, std::string>> Config::overrides() const {
  std::vector<std::pair<std::string, std::string>> out;
  for (const auto& [key, value] : overrides_) {
    // Text as it was given (a path keeps its ~); anything else as its converted value, which the type reads back
    // (8421.0 for an integer is 8421).
    const auto* text = std::get_if<std::string>(&value);
    out.emplace_back(key, text ? *text : to_text(convert(key, settings_.find(key)->second, value)));
  }
  return out;
}

State Config::state() const {
  State state;
  state.overrides_ = overrides_;
  state.files_ = files_;
  return state;
}

void Config::restore(const State& state) {
  overrides_ = state.overrides_;
  files_ = state.files_;
}

std::map<std::string, std::string> Config::file_values() const {
  std::map<std::string, std::string> out;
  for (const auto& [name, value] : files_) out.emplace(name, value.text);
  return out;
}

void Config::set_file_values(const std::map<std::string, std::string>& values, const fs::path& file) {
  files_.clear();
  fs::path where = file.empty() ? write_target() : file;
  for (const auto& [name, text] : values) files_.emplace(name, FileValue{text, where});
}

fs::path Config::write_target() const {
  if (!options_.write_file.empty()) return options_.write_file;
  return options_.files.empty() ? fs::path() : options_.files.back();
}

void Config::reload_files() {
  std::map<std::string, FileValue, std::less<>> read;
  envfile::Lookup lookup = [this](const std::string& name) { return env(name); };
  for (const fs::path& file : options_.files) {
    std::error_code error;
    if (file.empty() || !fs::is_regular_file(file, error)) continue;
    for (envfile::Assignment& assignment : envfile::parse(read_file(file), lookup)) {
      read[assignment.name] = FileValue{std::move(assignment.value), file};
    }
  }
  files_ = std::move(read);
}

Value Config::persist(std::string_view key, const Value& raw, const fs::path& file) {
  const Setting& s = setting(key);
  if (!options_.persisted.count(std::string(key))) {
    throw Error(ErrorKind::NotPersisted, std::string(key), std::string(key) + " is not one of the settings to persist");
  }
  Value typed = convert(key, s, raw);
  fs::path target = file.empty() ? write_target() : file;
  if (target.empty()) throw Error(ErrorKind::Io, std::string(key), "no file to write " + std::string(key) + " into");
  std::error_code error;
  std::string old = fs::exists(target, error) ? read_file(target) : std::string();
  std::string name = variable(key);
  std::string text = to_text(typed);
  replace_file(target, envfile::replace(old, name, envfile::quote(text)));
  // Where the files stand now: a file read after the written one that sets the name still wins.
  auto rank = [&](const fs::path& file) {
    for (std::size_t i = 0; i < options_.files.size(); ++i) {
      std::error_code same;
      if (file == options_.files[i] || fs::equivalent(file, options_.files[i], same)) return i;
    }
    return options_.files.size();  // not one of the files: as if read last
  };
  auto current = files_.find(name);
  if (current == files_.end() || rank(current->second.file) <= rank(target)) files_[name] = FileValue{text, target};
  if (env(name) && options_.environment.set) options_.environment.set(name, text);
  return typed;
}

}  // namespace layeredconf
