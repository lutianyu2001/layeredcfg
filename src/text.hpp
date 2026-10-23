// Text to values and back, the way Python reads and writes them (so that a binding behaves as Python code did).
#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace layeredconf::text {

// ASCII whitespace off both ends; ASCII letters made lower case.
std::string_view strip(std::string_view s);
std::string lower(std::string_view s);

// As Python's int(s) in base 10: whitespace around, a sign, digits with single underscores between them. Past the
// range of a 64-bit integer is no value; so are digits and whitespace beyond ASCII, which Python would take.
std::optional<std::int64_t> parse_int(std::string_view s);

// As Python's float(s): whitespace around, a sign, digits with an optional point and exponent (underscores between
// digits), or inf, infinity, nan in any case. A number too large is infinite and one too small zero, as in Python.
std::optional<double> parse_float(std::string_view s);

// Whether the bytes are UTF-8 (the text of every layer must be).
bool valid_utf8(std::string_view s);

// 1, true, yes, on and 0, false, no, off or nothing, in any case; anything else is no boolean.
std::optional<bool> parse_bool(std::string_view s);

// Comma-separated items, each stripped, empty ones left out.
std::vector<std::string> split_list(std::string_view s);

// As Python's repr(float): the shortest digits that read back as the same number, written without an exponent from
// 1e-4 up to 1e16 (with a ".0" when there is no point), with one (e+16, e-05) otherwise; inf, -inf, nan.
std::string format_float(double value);

}  // namespace layeredconf::text
