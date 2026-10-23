#include "envfile.hpp"

#include <map>

namespace layeredconf::envfile {

namespace {

bool is_blank(char c) { return c == ' ' || c == '\t'; }
bool is_space(char c) { return is_blank(c) || c == '\n' || c == '\r' || c == '\f' || c == '\v'; }

class Reader {
 public:
  Reader(std::string_view text, const Lookup& lookup) : s_(text), lookup_(lookup) {}

  std::vector<Assignment> read() {
    std::vector<Assignment> out;
    while (at_ < s_.size()) {
      skip_blanks();
      if (at_ >= s_.size()) break;
      char c = s_[at_];
      if (c == '\n' || c == '\r') {
        ++at_;
        continue;
      }
      if (c == '#') {
        skip_line();
        continue;
      }
      std::optional<Assignment> assignment = statement();
      if (assignment) {
        known_[assignment->name] = assignment->value;
        out.push_back(std::move(*assignment));
      }
    }
    return out;
  }

 private:
  std::optional<Assignment> statement() {
    if (s_.compare(at_, 6, "export") == 0 && at_ + 6 < s_.size() && is_blank(s_[at_ + 6])) {
      at_ += 6;
      skip_blanks();
    }
    std::string name;
    while (at_ < s_.size() && s_[at_] != '=' && s_[at_] != '#' && !is_space(s_[at_])) name += s_[at_++];
    skip_blanks();
    if (name.empty() || at_ >= s_.size() || s_[at_] != '=') {
      skip_line();  // no name, or a name without a value
      return std::nullopt;
    }
    ++at_;
    skip_blanks();
    std::optional<std::string> value;
    if (at_ < s_.size() && s_[at_] == '\'') {
      value = quoted('\'');
    } else if (at_ < s_.size() && s_[at_] == '"') {
      value = quoted('"');
      if (value) value = expand(*value);
    } else {
      value = expand(unquoted());
    }
    if (!value) return std::nullopt;
    return Assignment{std::move(name), std::move(*value)};
  }

  // From the opening quote to the closing one, then only blanks and a comment may follow on the line.
  std::optional<std::string> quoted(char quote) {
    std::size_t start = at_++;
    std::string value;
    while (at_ < s_.size() && s_[at_] != quote) {
      char c = s_[at_++];
      if (c == '\\' && at_ < s_.size()) {
        char next = s_[at_];
        if (quote == '\'' && (next == '\\' || next == '\'')) {
          value += next;
          ++at_;
          continue;
        }
        if (quote == '"') {
          static const std::map<char, char> escapes = {{'\\', '\\'}, {'\'', '\''}, {'"', '"'}, {'a', '\a'},
                                                       {'b', '\b'},  {'f', '\f'},  {'n', '\n'}, {'r', '\r'},
                                                       {'t', '\t'},  {'v', '\v'}};
          auto found = escapes.find(next);
          if (found != escapes.end()) {
            value += found->second;
            ++at_;
            continue;
          }
        }
      }
      value += c;
    }
    if (at_ >= s_.size()) {  // never closed: the line is not read
      at_ = start;
      skip_line();
      return std::nullopt;
    }
    ++at_;
    skip_blanks();
    if (at_ < s_.size() && s_[at_] != '\n' && s_[at_] != '\r' && s_[at_] != '#') {
      skip_line();  // something after the closing quote
      return std::nullopt;
    }
    skip_line();
    return value;
  }

  // To the line's end; a `#` after whitespace begins a comment.
  std::string unquoted() {
    std::size_t start = at_;
    while (at_ < s_.size() && s_[at_] != '\n' && s_[at_] != '\r') ++at_;
    std::string_view line = s_.substr(start, at_ - start);
    for (std::size_t i = 1; i < line.size(); ++i) {
      if (line[i] == '#' && is_space(line[i - 1])) {
        line = line.substr(0, i);
        break;
      }
    }
    std::size_t end = line.size();
    while (end > 0 && is_space(line[end - 1])) --end;
    return std::string(line.substr(0, end));
  }

  // ${NAME} and ${NAME:-default}.
  std::string expand(const std::string& value) const {
    std::string out;
    std::size_t i = 0;
    while (i < value.size()) {
      std::size_t open = value.find("${", i);
      std::size_t close = open == std::string::npos ? open : value.find('}', open + 2);
      if (open == std::string::npos || close == std::string::npos) {
        out += value.substr(i);
        break;
      }
      out += value.substr(i, open - i);
      std::string inner = value.substr(open + 2, close - open - 2);
      std::string name = inner, fallback;
      std::size_t dash = inner.find(":-");
      if (dash != std::string::npos) {
        name = inner.substr(0, dash);
        fallback = inner.substr(dash + 2);
      }
      auto known = known_.find(name);
      if (known != known_.end()) {
        out += known->second;
      } else if (auto found = lookup_ ? lookup_(name) : std::nullopt) {
        out += *found;
      } else {
        out += fallback;
      }
      i = close + 1;
    }
    return out;
  }

  void skip_blanks() {
    while (at_ < s_.size() && is_blank(s_[at_])) ++at_;
  }

  void skip_line() {
    while (at_ < s_.size() && s_[at_] != '\n') ++at_;
    if (at_ < s_.size()) ++at_;
  }

  std::string_view s_;
  const Lookup& lookup_;
  std::size_t at_ = 0;
  std::map<std::string, std::string> known_;
};

// The start of a line that assigns NAME: blanks, an optional `export` and blanks, the name, blanks, `=`.
bool assigns(std::string_view line, const std::string& name) {
  std::size_t at = 0;
  while (at < line.size() && is_space(line[at])) ++at;
  if (line.compare(at, 6, "export") == 0 && at + 6 < line.size() && is_space(line[at + 6])) {
    at += 6;
    while (at < line.size() && is_space(line[at])) ++at;
  }
  if (line.compare(at, name.size(), name) != 0) return false;
  at += name.size();
  while (at < line.size() && is_space(line[at])) ++at;
  return at < line.size() && line[at] == '=';
}

}  // namespace

std::vector<Assignment> parse(std::string_view text, const Lookup& lookup) { return Reader(text, lookup).read(); }

std::string quote(const std::string& value) {
  bool bare = true;
  for (char c : value) {
    bool safe = (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' || c == '.' ||
                c == ',' || c == ':' || c == '/' || c == '@' || c == '+' || c == '=' || c == '-' || c == '%' ||
                c == '~';
    if (!safe) {
      bare = false;
      break;
    }
  }
  if (bare) return value;
  std::string out = "'";
  for (char c : value) {
    if (c == '\\' || c == '\'') out += '\\';
    out += c;
  }
  return out + "'";
}

std::string replace(std::string_view text, const std::string& name, const std::string& written) {
  std::vector<std::string_view> lines;
  std::size_t begin = 0;
  while (begin < text.size()) {
    std::size_t end = text.find('\n', begin);
    if (end == std::string_view::npos) end = text.size();
    std::string_view line = text.substr(begin, end - begin);
    if (!line.empty() && line.back() == '\r') line.remove_suffix(1);
    lines.push_back(line);
    begin = end + 1;
  }
  std::string out;
  bool done = false;
  for (std::string_view line : lines) {
    if (assigns(line, name)) {
      if (done) continue;  // the first assignment is replaced, a later one dropped
      out += name + "=" + written + "\n";
      done = true;
      continue;
    }
    out += std::string(line) + "\n";
  }
  if (!done) out += name + "=" + written + "\n";
  return out;
}

}  // namespace layeredconf::envfile
