// Text to values and back, held to what Python's int(), float() and repr() give.
#include <doctest/doctest.h>

#include <cmath>
#include <limits>

#include "text.hpp"

using namespace layeredconf::text;

TEST_CASE("integers are read as Python's int() reads them") {
  CHECK(parse_int("42") == 42);
  CHECK(parse_int("  -7\n") == -7);
  CHECK(parse_int("+0") == 0);
  CHECK(parse_int("007") == 7);
  CHECK(parse_int("1_000_000") == 1000000);
  CHECK(parse_int("9223372036854775807") == std::numeric_limits<std::int64_t>::max());
  CHECK(parse_int("-9223372036854775808") == std::numeric_limits<std::int64_t>::min());
  for (const char* bad : {"", " ", "4x", "84x0", "1.5", "_1", "1_", "1__0", "--1", "0x10", "1 2", "9223372036854775808"}) {
    CAPTURE(bad);
    CHECK_FALSE(parse_int(bad).has_value());
  }
}

TEST_CASE("floats are read as Python's float() reads them") {
  CHECK(parse_float("0.6") == 0.6);
  CHECK(parse_float(" 10 ") == 10.0);
  CHECK(parse_float("-.5") == -0.5);
  CHECK(parse_float("5.") == 5.0);
  CHECK(parse_float("1e3") == 1000.0);
  CHECK(parse_float("1_000.5") == 1000.5);
  CHECK(parse_float("2.5E-1") == 0.25);
  CHECK(std::isinf(*parse_float("Infinity")));
  CHECK(*parse_float("-inf") < 0);
  CHECK(std::isnan(*parse_float("NaN")));
  // Past the range: infinite or zero, as Python has them.
  CHECK(std::isinf(*parse_float("1e400")));
  CHECK(*parse_float("1e-400") == 0.0);
  CHECK(std::signbit(*parse_float("-1e-400")));
  CHECK(*parse_float("1e-310") > 0.0);  // a subnormal number is a number
  // Exponents past any range, without an overflow of their own.
  CHECK(std::isinf(*parse_float("10e99999999999999999999")));
  CHECK(*parse_float("-10e99999999999999999999") < 0);
  CHECK(std::isinf(*parse_float("123e9223372036854775807")));
  CHECK(*parse_float("0.1e-99999999999999999999") == 0.0);
  CHECK(std::signbit(*parse_float("-nan")));
  for (const char* bad : {"", ".", "e5", "1e", "1e+", "soon", "1.5.2", "0x1p3", "1_.5", "1._5", "in f", "--1"}) {
    CAPTURE(bad);
    CHECK_FALSE(parse_float(bad).has_value());
  }
}

TEST_CASE("a boolean is one of its words") {
  for (const char* yes : {"1", "true", "TRUE", " yes ", "On"}) CHECK(parse_bool(yes) == true);
  for (const char* no : {"", "0", "false", "No", "OFF"}) CHECK(parse_bool(no) == false);
  for (const char* bad : {"ture", "maybe", "2", "y"}) CHECK_FALSE(parse_bool(bad).has_value());
}

TEST_CASE("a list is comma-separated, each item stripped and the empty ones left out") {
  CHECK(split_list("a.lan, b.lan,") == std::vector<std::string>{"a.lan", "b.lan"});
  CHECK(split_list("").empty());
  CHECK(split_list(" , ,").empty());
}

TEST_CASE("floats are written as Python's repr() writes them") {
  CHECK(format_float(45.0) == "45.0");
  CHECK(format_float(0.0) == "0.0");
  CHECK(format_float(-0.0) == "-0.0");
  CHECK(format_float(0.1) == "0.1");
  CHECK(format_float(0.6) == "0.6");
  CHECK(format_float(123.456) == "123.456");
  CHECK(format_float(0.0001) == "0.0001");
  CHECK(format_float(0.00001) == "1e-05");
  CHECK(format_float(1e15) == "1000000000000000.0");
  CHECK(format_float(1e16) == "1e+16");
  CHECK(format_float(1.5e300) == "1.5e+300");
  CHECK(format_float(-2.5e-7) == "-2.5e-07");
  CHECK(format_float(1.0 / 3) == "0.3333333333333333");
  CHECK(format_float(std::numeric_limits<double>::infinity()) == "inf");
  CHECK(format_float(-std::numeric_limits<double>::infinity()) == "-inf");
  CHECK(format_float(std::nan("")) == "nan");
}

TEST_CASE("UTF-8 is checked") {
  CHECK(valid_utf8("plain"));
  CHECK(valid_utf8("\xE4\xB8\xAD\xE6\x96\x87"));      // 中文
  CHECK(valid_utf8("\xF0\x9F\x98\x80"));              // an emoji
  CHECK_FALSE(valid_utf8("caf\xE9"));                    // Latin-1
  CHECK_FALSE(valid_utf8("\xC0\xAF"));                  // overlong
  CHECK_FALSE(valid_utf8("\xED\xA0\x80"));             // a surrogate
  CHECK_FALSE(valid_utf8("\xE4\xB8"));                  // cut short
}
