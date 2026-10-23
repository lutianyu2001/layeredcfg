// Env-style files: reading them, and replacing one name's line.
#include <doctest/doctest.h>

#include <map>

#include "envfile.hpp"

using namespace layeredconf::envfile;

namespace {

std::map<std::string, std::string> read(std::string_view text, std::map<std::string, std::string> environment = {}) {
  Lookup lookup = [&](const std::string& name) -> std::optional<std::string> {
    auto found = environment.find(name);
    if (found == environment.end()) return std::nullopt;
    return found->second;
  };
  std::map<std::string, std::string> out;
  for (const Assignment& a : parse(text, lookup)) out[a.name] = a.value;
  return out;
}

}  // namespace

TEST_CASE("names, values, comments and export") {
  auto values = read(
      "# ntfy\n"
      "APP_TOPIC=secret\n"
      "export APP_PORT = 8421\n"
      "\n"
      "   APP_HOSTS=a.lan,b.lan   # the hosts\n"
      "APP_HASH=a#b\n"
      "APP_EMPTY=\n"
      "APP_BARE\n"
      "=nameless\n");
  CHECK(values == std::map<std::string, std::string>{{"APP_TOPIC", "secret"},
                                                     {"APP_PORT", "8421"},
                                                     {"APP_HOSTS", "a.lan,b.lan"},
                                                     {"APP_HASH", "a#b"},
                                                     {"APP_EMPTY", ""}});
}

TEST_CASE("a later assignment wins, CRLF line ends are read") {
  CHECK(read("A=1\r\nA=2\r\n") == std::map<std::string, std::string>{{"A", "2"}});
}

TEST_CASE("quoted values") {
  auto values = read(
      "S='it''s'\n"
      "Q='a \\'b\\' # c' # a comment\n"
      "D=\"tab\\there\\n\\\"q\\\" \\x\"\n"
      "M=\"two\nlines\"\n"
      "BAD='open\n"
      "AFTER=\"x\" y\n"
      "OK=1\n");
  CHECK(values.count("S") == 0);  // something after the closing quote
  CHECK(values["Q"] == "a 'b' # c");
  CHECK(values["D"] == "tab\there\n\"q\" \\x");
  CHECK(values["M"] == "two\nlines");
  CHECK(values.count("BAD") == 0);
  CHECK(values.count("AFTER") == 0);
  CHECK(values["OK"] == "1");
}

TEST_CASE("${NAME} takes the file's own earlier value, then the environment's, then the default") {
  auto values = read(
      "HOME_DIR=/home/u\n"
      "DATA=${HOME_DIR}/data\n"
      "FROM_ENV=${SHELL}\n"
      "MISSING=[${NOPE}]\n"
      "DEFAULTED=${NOPE:-x}\n"
      "SINGLE='${HOME_DIR}'\n"
      "DOUBLE=\"${HOME_DIR}!\"\n",
      {{"SHELL", "/bin/bash"}, {"HOME_DIR", "/elsewhere"}});
  CHECK(values["DATA"] == "/home/u/data");
  CHECK(values["FROM_ENV"] == "/bin/bash");
  CHECK(values["MISSING"] == "[]");
  CHECK(values["DEFAULTED"] == "x");
  CHECK(values["SINGLE"] == "${HOME_DIR}");
  CHECK(values["DOUBLE"] == "/home/u!");
}

TEST_CASE("a value is written so that it reads back as it is") {
  CHECK(quote("45.0") == "45.0");
  CHECK(quote("a.lan,b.lan") == "a.lan,b.lan");
  CHECK(quote("") == "");
  for (std::string value : {"two words", "a#b", "it's", "back\\slash", "${X}", "two\nlines", "\"q\""}) {
    CAPTURE(value);
    CHECK(read("V=" + quote(value) + "\n")["V"] == value);
  }
}

TEST_CASE("a name's line is replaced, later ones dropped, every other line kept") {
  std::string text = "# ntfy\nAPP_TOPIC=secret\nexport APP_LIMIT=10\nAPP_LIMITS=x\nAPP_LIMIT = 12\n";
  CHECK(replace(text, "APP_LIMIT", "45.0") == "# ntfy\nAPP_TOPIC=secret\nAPP_LIMIT=45.0\nAPP_LIMITS=x\n");
  CHECK(replace("A=1", "B", "2") == "A=1\nB=2\n");
  CHECK(replace("", "B", "2") == "B=2\n");
  CHECK(replace("A=1\r\n", "A", "3") == "A=3\n");
}
