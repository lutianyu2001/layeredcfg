#include "envfile.hpp"

#include <map>

#include "text.hpp"

namespace layeredconf::envfile {

namespace {

bool is_newline(char c) { return c == '\n' || c == '\r'; }

class Reader {
 public:
  Reader(std::string_view text, const Lookup& lookup) : s_(text), lookup_(lookup) {
    if (s_.substr(0, 3) == "\xEF\xBB\xBF") at_ = 3;  // a byte order mark
  }

  std::vector<Assignment> read() {
    std::vector<Assignment> out;
    while (at_ < s_.size()) {
      std::size_t begin = at_;
      skip_blanks();
      if (at_ >= s_.size()) break;
      char c = s_[at_];
      if (is_newline(c)) {
        end_line();
        continue;
      }
      if (c == '#') {
        skip_line();
        continue;
      }
      std::optional<Assignment> assignment = statement();
      if (assignment) {
        assignment->begin = begin;
        assignment->end = at_;
        known_[assignment->name] = assignment->value;
        out.push_back(std::move(*assignment));
      }
    }
    return out;
  }

 private:
  // A statement, from its name to past the end of its last line.
  std::optional<Assignment> statement() {
    if (s_.compare(at_, 6, "export") == 0 && blank_at(at_ + 6)) {
      at_ += 6;
      skip_blanks();
    }
    std::string name;
    while (at_ < s_.size() && s_[at_] != '=' && s_[at_] != '#' && !text::space_at(s_, at_)) name += s_[at_++];
    skip_blanks();
    if (name.empty() || at_ >= s_.size() || s_[at_] != '=') {
      skip_line();  // no name, or a name without a value
      return std::nullopt;
    }
    std::size_t value_start = ++at_;
    skip_blanks();
    std::optional<std::string> value;
    if (at_ < s_.size() && s_[at_] == '\'') {
      value = quoted('\'');
    } else if (at_ < s_.size() && s_[at_] == '"') {
      value = quoted('"');
      if (value) value = expand(*value);
    } else {
      at_ = value_start;  // the blanks before the value count for a comment that begins it
      value = expand(unquoted());
      skip_line();
    }
    if (!value) return std::nullopt;
    Assignment assignment;
    assignment.name = std::move(name);
    assignment.value = std::move(*value);
    return assignment;
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
    bool clean = at_ >= s_.size() || is_newline(s_[at_]) || s_[at_] == '#';
    skip_line();
    if (!clean) return std::nullopt;  // something after the closing quote
    return value;
  }

  // To the line's end, whitespace around taken off; a `#` after whitespace begins a comment.
  std::string unquoted() {
    std::size_t start = at_;
    while (at_ < s_.size() && !is_newline(s_[at_])) ++at_;
    std::string_view line = s_.substr(start, at_ - start);
    bool after_space = false;
    for (std::size_t i = 0; i < line.size();) {
      if (line[i] == '#' && after_space) {
        line = line.substr(0, i);
        break;
      }
      std::size_t length = text::space_at(line, i);
      after_space = length > 0;
      i += after_space ? length : 1;
    }
    return std::string(text::strip(line));
  }

  // ${NAME} and ${NAME:-default}, as python-dotenv takes them: a name of anything but `}` and `:`.
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
      std::size_t colon = inner.find(':');
      if (colon != std::string::npos && inner.compare(colon, 2, ":-") != 0) {
        out += value.substr(open, close + 1 - open);  // not a variable: as it is
        i = close + 1;
        continue;
      }
      std::string name = colon == std::string::npos ? inner : inner.substr(0, colon);
      std::string fallback = colon == std::string::npos ? std::string() : inner.substr(colon + 2);
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

  // The length of the whitespace character at `at` that does not end a line (python-dotenv's [^\S\r\n]), 0 if
  // there is none there.
  std::size_t blank_at(std::size_t at) const {
    return at < s_.size() && is_newline(s_[at]) ? 0 : text::space_at(s_, at);
  }

  void skip_blanks() {
    while (std::size_t length = blank_at(at_)) at_ += length;
  }

  void skip_line() {
    while (at_ < s_.size() && !is_newline(s_[at_])) ++at_;
    end_line();
  }

  // Past one line end: \r\n, \r or \n.
  void end_line() {
    if (at_ < s_.size() && s_[at_] == '\r') ++at_;
    if (at_ < s_.size() && s_[at_] == '\n') ++at_;
  }

  std::string_view s_;
  const Lookup& lookup_;
  std::size_t at_ = 0;
  std::map<std::string, std::string> known_;
};

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
  std::string line = name + "=" + written + "\n";
  std::string out;
  std::size_t at = 0;
  bool done = false;
  for (const Assignment& assignment : parse(text, Lookup())) {
    if (assignment.name != name) continue;
    out += text.substr(at, assignment.begin - at);
    if (!done) out += line;  // the first assignment is replaced, a later one dropped
    done = true;
    at = assignment.end;
  }
  out += text.substr(at);
  if (!done) {
    if (!out.empty() && !is_newline(out.back())) out += '\n';
    out += line;
  }
  return out;
}

}  // namespace layeredconf::envfile
