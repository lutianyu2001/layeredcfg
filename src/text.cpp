#include "text.hpp"

#include <charconv>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <system_error>

namespace layeredconf::text {

namespace {

bool is_space(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f' || c == '\v'; }
bool is_digit(char c) { return c >= '0' && c <= '9'; }

// Digits with single underscores between them, from `at`, appended to `out` without the underscores. False if there is
// no digit at `at`.
bool digit_run(std::string_view s, std::size_t& at, std::string& out) {
  if (at >= s.size() || !is_digit(s[at])) return false;
  out += s[at++];
  while (at < s.size()) {
    if (is_digit(s[at])) {
      out += s[at++];
    } else if (s[at] == '_' && at + 1 < s.size() && is_digit(s[at + 1])) {
      ++at;
    } else {
      break;
    }
  }
  return true;
}

// The power of ten of the first significant digit of a decimal number written as digits, an optional point and an
// optional exponent ("0.00012e3" is 0.12: -1); 0 for a number that is zero.
long decimal_exponent(const std::string& number) {
  std::size_t e = number.find('e');
  std::string mantissa = number.substr(0, e);
  long exponent = e == std::string::npos ? 0 : std::strtol(number.c_str() + e + 1, nullptr, 10);
  std::size_t point = mantissa.find('.');
  if (point == std::string::npos) point = mantissa.size();
  for (std::size_t i = 0; i < mantissa.size(); ++i) {
    if (mantissa[i] == '.' || mantissa[i] == '0') continue;
    long place = i < point ? static_cast<long>(point - i) - 1 : -static_cast<long>(i - point);
    return place + exponent;
  }
  return 0;
}

}  // namespace

std::string_view strip(std::string_view s) {
  std::size_t begin = 0, end = s.size();
  while (begin < end && is_space(s[begin])) ++begin;
  while (end > begin && is_space(s[end - 1])) --end;
  return s.substr(begin, end - begin);
}

std::string lower(std::string_view s) {
  std::string out(s);
  for (char& c : out) {
    if (c >= 'A' && c <= 'Z') c = static_cast<char>(c - 'A' + 'a');
  }
  return out;
}

std::optional<std::int64_t> parse_int(std::string_view raw) {
  std::string_view s = strip(raw);
  std::size_t at = 0;
  bool negative = false;
  if (at < s.size() && (s[at] == '+' || s[at] == '-')) negative = s[at++] == '-';
  std::string digits;
  if (!digit_run(s, at, digits) || at != s.size()) return std::nullopt;
  // Accumulated as an unsigned magnitude, which also holds the most negative number.
  constexpr std::uint64_t limit = static_cast<std::uint64_t>(std::numeric_limits<std::int64_t>::max());
  std::uint64_t magnitude = 0;
  for (char c : digits) {
    std::uint64_t digit = static_cast<std::uint64_t>(c - '0');
    if (magnitude > (limit + 1 - digit) / 10) return std::nullopt;
    magnitude = magnitude * 10 + digit;
  }
  if (!negative && magnitude > limit) return std::nullopt;
  if (negative) {
    return magnitude == limit + 1 ? std::numeric_limits<std::int64_t>::min() : -static_cast<std::int64_t>(magnitude);
  }
  return static_cast<std::int64_t>(magnitude);
}

std::optional<double> parse_float(std::string_view raw) {
  std::string_view s = strip(raw);
  std::size_t at = 0;
  bool negative = false;
  if (at < s.size() && (s[at] == '+' || s[at] == '-')) negative = s[at++] == '-';
  std::string word = lower(s.substr(at));
  if (word == "inf" || word == "infinity") {
    return negative ? -std::numeric_limits<double>::infinity() : std::numeric_limits<double>::infinity();
  }
  if (word == "nan") return std::nan("");
  std::string number;
  bool whole = digit_run(s, at, number);
  bool fraction = false;
  if (at < s.size() && s[at] == '.') {
    number += '.';
    ++at;
    fraction = digit_run(s, at, number);
  }
  if (!whole && !fraction) return std::nullopt;
  if (at < s.size() && (s[at] == 'e' || s[at] == 'E')) {
    number += 'e';
    ++at;
    if (at < s.size() && (s[at] == '+' || s[at] == '-')) number += s[at++];
    if (!digit_run(s, at, number)) return std::nullopt;
  }
  if (at != s.size()) return std::nullopt;
  double value = 0.0;
  auto [end, error] = std::from_chars(number.data(), number.data() + number.size(), value);
  if (error == std::errc::result_out_of_range) {
    value = decimal_exponent(number) > 0 ? std::numeric_limits<double>::infinity() : 0.0;
  } else if (error != std::errc() || end != number.data() + number.size()) {
    return std::nullopt;
  }
  return negative ? -value : value;
}

std::optional<bool> parse_bool(std::string_view raw) {
  std::string word = lower(strip(raw));
  if (word == "1" || word == "true" || word == "yes" || word == "on") return true;
  if (word.empty() || word == "0" || word == "false" || word == "no" || word == "off") return false;
  return std::nullopt;
}

std::vector<std::string> split_list(std::string_view s) {
  std::vector<std::string> items;
  std::size_t begin = 0;
  while (begin <= s.size()) {
    std::size_t comma = s.find(',', begin);
    if (comma == std::string_view::npos) comma = s.size();
    std::string_view item = strip(s.substr(begin, comma - begin));
    if (!item.empty()) items.emplace_back(item);
    begin = comma + 1;
  }
  return items;
}

std::string format_float(double value) {
  if (std::isnan(value)) return "nan";
  if (std::isinf(value)) return value < 0 ? "-inf" : "inf";
  char buffer[64];
  auto result = std::to_chars(buffer, buffer + sizeof buffer, std::fabs(value), std::chars_format::scientific);
  std::string scientific(buffer, result.ptr);  // "4.5e+01", "1e+16"
  std::size_t e = scientific.find('e');
  std::string digits;
  for (std::size_t i = 0; i < e; ++i) {
    if (scientific[i] != '.') digits += scientific[i];
  }
  int exponent = std::atoi(scientific.c_str() + e + 1);
  int point = exponent + 1;  // where the point goes, counted in digits from the first
  std::string out = std::signbit(value) ? "-" : "";
  if (point > -4 && point <= 16) {
    int count = static_cast<int>(digits.size());
    if (point <= 0) {
      out += "0." + std::string(static_cast<std::size_t>(-point), '0') + digits;
    } else if (point >= count) {
      out += digits + std::string(static_cast<std::size_t>(point - count), '0') + ".0";
    } else {
      out += digits.substr(0, static_cast<std::size_t>(point)) + "." + digits.substr(static_cast<std::size_t>(point));
    }
  } else {
    out += digits.substr(0, 1);
    if (digits.size() > 1) out += "." + digits.substr(1);
    char tail[16];
    std::snprintf(tail, sizeof tail, "e%c%02d", exponent < 0 ? '-' : '+', std::abs(exponent));
    out += tail;
  }
  return out;
}

}  // namespace layeredconf::text
