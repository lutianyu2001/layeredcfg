// Env-style files: `NAME=value` lines, as python-dotenv reads them for the most part.
#pragma once

#include <cstddef>
#include <functional>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace layeredconf::envfile {

struct Assignment {
  std::string name;
  std::string value;
  // Where the statement is in the text: from the start of its first line to past the end of its last (a quoted value
  // may go on over several lines).
  std::size_t begin = 0;
  std::size_t end = 0;
};

using Lookup = std::function<std::optional<std::string>(const std::string&)>;

// The assignments of a file, in order (a later one of a name wins). What it reads:
//   - a UTF-8 byte order mark at the start; lines ended by \n, \r\n or \r; whitespace as Python's str.isspace()
//     takes it (python-dotenv's \s), U+3000 and the others beyond ASCII too;
//   - blank lines and lines that begin with `#`; an `export ` before a name;
//   - unquoted values, ending at the line's end or at a `#` after whitespace, stripped (`NAME= # note` is empty);
//   - 'single-quoted' values, taken as they are but for \' and \\;
//   - "double-quoted" values with the escapes \\ \' \" \a \b \f \n \r \t \v;
//   - quoted values that go on over several lines;
//   - ${NAME} and ${NAME:-default} in unquoted and double-quoted values: the file's own earlier value of NAME, else
//     `lookup` (the environment), else the default, else nothing. Anything else after a `$` stays as it is.
// A line it cannot read is left out.
std::vector<Assignment> parse(std::string_view text, const Lookup& lookup);

// `value` written so that `parse` reads it back as it is: bare when it has nothing a reader would take apart, else
// single-quoted.
std::string quote(const std::string& value);

// The text with NAME's first assignment (all its lines) made the line `NAME=<written>`, its later ones dropped, every
// other byte kept; a line added at the end when there was none.
std::string replace(std::string_view text, const std::string& name, const std::string& written);

}  // namespace layeredconf::envfile
