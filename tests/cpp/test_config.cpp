// The layers, the conversions, the checks, and persist, with an environment of the test's own.
#include <doctest/doctest.h>

#include <sys/stat.h>

#include <fstream>
#include <map>
#include <random>
#include <limits>
#include <sstream>
#include <thread>

#include "layeredconf/layeredconf.hpp"

using namespace layeredconf;
namespace fs = std::filesystem;

namespace {

const char* const DEFAULTS = R"(# The test's defaults.
[server]
port = 8420
host = "0.0.0.0"

[notify]
enabled = true
topic = ""
token = ""

[quota]
ratio = 1.0
hosts = []

[log]
max_mib = 10.0
dir = "data/logs"
)";

struct Scratch {
  fs::path dir;
  Scratch() {
    std::random_device random;
    dir = fs::temp_directory_path() / ("layeredconf-test-" + std::to_string(random()));
    fs::create_directories(dir);
  }
  ~Scratch() {
    std::error_code error;
    fs::remove_all(dir, error);
  }
  fs::path write(const std::string& name, const std::string& text) const {
    fs::path path = dir / name;
    fs::create_directories(path.parent_path());
    std::ofstream(path) << text;
    return path;
  }
  static std::string read(const fs::path& path) {
    std::ifstream in(path);
    std::ostringstream text;
    text << in.rdbuf();
    return text.str();
  }
};

// An environment the test sets as it likes.
struct FakeEnvironment {
  std::map<std::string, std::string> values;
  Environment get() {
    Environment environment;
    environment.get = [this](const std::string& name) -> std::optional<std::string> {
      auto found = values.find(name);
      if (found == values.end()) return std::nullopt;
      return found->second;
    };
    environment.set = [this](const std::string& name, const std::string& value) { values[name] = value; };
    return environment;
  }
};

Options options(FakeEnvironment& environment, std::vector<fs::path> files = {}) {
  Options o;
  o.prefix = "APP_";
  o.files = std::move(files);
  o.root = "/srv/app";
  o.file_only = {"log.max_mib"};
  o.persisted = {"quota.ratio"};
  o.secret = {"notify.topic", "notify.token"};
  o.environment = environment.get();
  return o;
}

}  // namespace

TEST_CASE("the defaults say which settings there are, in the file's order, and their types") {
  FakeEnvironment environment;
  Config config = Config::parse(DEFAULTS, options(environment));
  CHECK(config.keys() == std::vector<std::string>{"server.port", "server.host", "notify.enabled", "notify.topic",
                                                  "notify.token", "quota.ratio", "quota.hosts", "log.max_mib",
                                                  "log.dir"});
  CHECK(config.type("server.port") == Type::Int);
  CHECK(config.type("quota.hosts") == Type::List);
  CHECK(config.is_section("server"));
  CHECK_FALSE(config.is_section("server.port"));
  CHECK(std::get<std::int64_t>(config.get("server.port")) == 8420);
  CHECK(config.source("server.port") == Source::Default);
  CHECK(config.variable("server.port") == "APP_SERVER_PORT");
  CHECK_THROWS_AS(config.get("server.nope"), Error);
  // A table's entries together, in the file's order, as Python's tomllib has them.
  Options plain;
  plain.environment = environment.get();
  Config reopened = Config::parse("[b]\nx = 1\n[a]\ny = 2\n[b.c]\nz = 3\n", plain);
  CHECK(reopened.keys() == std::vector<std::string>{"b.x", "b.c.z", "a.y"});
}

TEST_CASE("defaults that no other layer could write are refused") {
  FakeEnvironment environment;
  Options plain;
  plain.environment = environment.get();
  CHECK_THROWS_AS(Config::parse("when = 1979-05-27\n", plain), Error);
  CHECK_THROWS_AS(Config::parse("ports = [1, 2]\n", plain), Error);
  CHECK_THROWS_AS(Config::parse("broken = \n", plain), Error);
  // Settings named in the options must exist.
  Options named = plain;
  named.secret = {"no.such"};
  CHECK_THROWS_AS(Config::parse(DEFAULTS, named), Error);
}

TEST_CASE("each layer wins over the one below, and each value is converted to its default's type") {
  Scratch scratch;
  FakeEnvironment environment;
  fs::path near = scratch.write("work/.env.local", "APP_SERVER_PORT=9001\nAPP_QUOTA_HOSTS=x.lan\n");
  fs::path mine = scratch.write("repo/.env.local", "APP_SERVER_PORT=9002\nAPP_QUOTA_RATIO=0.5\n");
  Config config = Config::parse(DEFAULTS, options(environment, {near, mine}));
  CHECK(std::get<std::int64_t>(config.get("server.port")) == 9002);  // the later file
  CHECK(std::get<List>(config.get("quota.hosts")) == List{"x.lan"});
  CHECK(std::get<double>(config.get("quota.ratio")) == 0.5);
  CHECK(config.source("server.port") == Source::File);
  environment.values["APP_SERVER_PORT"] = "9003";
  CHECK(std::get<std::int64_t>(config.get("server.port")) == 9003);
  CHECK(config.source("server.port") == Source::Env);
  config.set("server.port", Value{std::string("9004")});
  CHECK(std::get<std::int64_t>(config.get("server.port")) == 9004);
  CHECK(config.source("server.port") == Source::Override);
  config.unset("server.port");
  CHECK(std::get<std::int64_t>(config.get("server.port")) == 9003);
  // Values that are not text are converted as Python converts them.
  config.set("server.port", Value{2.7});
  CHECK(std::get<std::int64_t>(config.get("server.port")) == 2);
  config.set("notify.enabled", Value{std::int64_t{0}});
  CHECK(std::get<bool>(config.get("notify.enabled")) == false);
  config.set("server.host", Value{true});
  CHECK(std::get<std::string>(config.get("server.host")) == "True");
  config.set("quota.ratio", Value{std::int64_t{3}});
  CHECK(std::get<double>(config.get("quota.ratio")) == 3.0);
}

TEST_CASE("an override is checked as it is set, and kept in the order first set, as text that sets it again") {
  FakeEnvironment environment;
  Config config = Config::parse(DEFAULTS, options(environment));
  CHECK_THROWS_AS(config.set("server.port", Value{std::string("84x0")}), Error);
  CHECK_THROWS_AS(config.set("no.such", Value{std::string("1")}), Error);
  try {
    config.set("log.max_mib", Value{std::string("1")});
    FAIL("a file-only setting was overridden");
  } catch (const Error& error) {
    CHECK(error.kind() == ErrorKind::FileOnly);
  }
  try {
    config.set("notify.enabled", Value{std::string("maybe")});
    FAIL("a word that is no boolean was taken");
  } catch (const Error& error) {
    CHECK(error.kind() == ErrorKind::InvalidValue);
    CHECK(error.key() == "notify.enabled");
    CHECK(std::get<std::string>(*error.raw()) == "maybe");
    CHECK(*error.expected() == Type::Bool);
  }
  config.set("server.port", Value{std::int64_t{8421}});
  config.set("quota.hosts", Value{List{" a ", "", "b"}});
  config.set("quota.ratio", Value{0.6});
  config.set("server.port", Value{std::string("8422")});
  CHECK(config.overrides() == std::vector<std::pair<std::string, std::string>>{
                                  {"server.port", "8422"}, {"quota.hosts", "a,b"}, {"quota.ratio", "0.6"}});
  // As text that set takes again: what is not text is written as its converted value.
  config.set("server.port", Value{8421.0});
  config.set("notify.enabled", Value{std::int64_t{2}});
  CHECK(config.overrides()[0].second == "8421");
  CHECK(config.overrides()[3].second == "True");
  for (const auto& [key, text] : config.overrides()) CHECK_NOTHROW(config.cast(key, Value{text}));
  config.clear_overrides();
  CHECK(config.overrides().empty());
}

TEST_CASE("problems are the values of the environment and the files that do not convert, with where they are") {
  Scratch scratch;
  FakeEnvironment environment;
  fs::path file = scratch.write(".env.local", "APP_NOTIFY_ENABLED=maybe\nAPP_SERVER_HOST=anything\n");
  Config config = Config::parse(DEFAULTS, options(environment, {file}));
  environment.values["APP_SERVER_PORT"] = "84x0";
  std::vector<Problem> found = config.problems();
  REQUIRE(found.size() == 2);
  CHECK(found[0].key == "server.port");
  CHECK(found[0].source == Source::Env);
  CHECK(found[0].text == "84x0");
  CHECK(found[1].key == "notify.enabled");
  CHECK(found[1].source == Source::File);
  CHECK(found[1].file == file);
  CHECK(found[1].variable == "APP_NOTIFY_ENABLED");
  // An override hides what is under it.
  config.set("server.port", Value{std::int64_t{1}});
  CHECK(config.problems().size() == 1);
  // Values given by a caller are said to be in the file persist writes into.
  config.set_file_values({{"APP_QUOTA_RATIO", "x"}});
  REQUIRE(config.problems().size() == 1);
  CHECK(config.problems()[0].file == file);
}

TEST_CASE("secrets are shown masked when they have a value") {
  FakeEnvironment environment;
  Config config = Config::parse(DEFAULTS, options(environment));
  CHECK(std::get<std::string>(config.shown("notify.topic")).empty());
  environment.values["APP_NOTIFY_TOPIC"] = "my-topic";
  CHECK(std::get<std::string>(config.shown("notify.topic")) == "***");
  CHECK(std::get<std::string>(config.get("notify.topic")) == "my-topic");
  CHECK(std::get<std::int64_t>(config.shown("server.port")) == 8420);
}

TEST_CASE("a path setting has ~ expanded and a relative one joined to the root") {
  FakeEnvironment environment;
  environment.values["HOME"] = "/home/someone/";
  Config config = Config::parse(DEFAULTS, options(environment));
  CHECK(config.path("log.dir") == fs::path("/srv/app/data/logs"));
  config.set("log.dir", Value{std::string("~/logs")});
  CHECK(config.path("log.dir") == fs::path("/home/someone/logs"));
  config.set("log.dir", Value{std::string("/var/log/app")});
  CHECK(config.path("log.dir") == fs::path("/var/log/app"));
}

TEST_CASE("persist replaces the setting's line, keeps the file's other lines and permissions, and holds at once") {
  Scratch scratch;
  FakeEnvironment environment;
  fs::path file = scratch.write(".env.local", "# ntfy\nAPP_NOTIFY_TOPIC=secret\nexport APP_QUOTA_RATIO=0.1\nAPP_QUOTA_RATIO=0.2\n");
  ::chmod(file.c_str(), 0600);
  Config config = Config::parse(DEFAULTS, options(environment, {file}));
  CHECK(std::get<double>(config.persist("quota.ratio", Value{std::string("0.45")})) == 0.45);
  CHECK(Scratch::read(file) == "# ntfy\nAPP_NOTIFY_TOPIC=secret\nAPP_QUOTA_RATIO=0.45\n");
  struct stat info {};
  ::stat(file.c_str(), &info);
  CHECK((info.st_mode & 07777) == 0600);
  CHECK_FALSE(fs::exists(file.string() + ".writing"));
  CHECK(std::get<double>(config.get("quota.ratio")) == 0.45);
  CHECK(config.source("quota.ratio") == Source::File);
  // A variable of the environment, which wins over the file, is replaced too.
  environment.values["APP_QUOTA_RATIO"] = "0.9";
  config.persist("quota.ratio", Value{0.3});
  CHECK(environment.values["APP_QUOTA_RATIO"] == "0.3");
  // Only the settings to persist; a bad value writes nothing.
  CHECK_THROWS_AS(config.persist("server.port", Value{std::string("1")}), Error);
  CHECK_THROWS_AS(config.persist("quota.ratio", Value{std::string("soon")}), Error);
  CHECK(Scratch::read(file) == "# ntfy\nAPP_NOTIFY_TOPIC=secret\nAPP_QUOTA_RATIO=0.3\n");
  // A new file is made with 0600.
  fs::path other = scratch.dir / "new.env";
  config.persist("quota.ratio", Value{std::string("2")}, other);
  CHECK(Scratch::read(other) == "APP_QUOTA_RATIO=2.0\n");
  ::stat(other.c_str(), &info);
  CHECK((info.st_mode & 07777) == 0600);
}

TEST_CASE("a file is found from a directory upwards") {
  Scratch scratch;
  fs::path file = scratch.write(".env.local", "A=1\n");
  fs::create_directories(scratch.dir / "a" / "b");
  CHECK(find_upward(".env.local", scratch.dir / "a" / "b") == file);
  CHECK_FALSE(find_upward(".no-such-file-anywhere", scratch.dir / "a").has_value());
}

TEST_CASE("persist into a file that a later file outranks does not claim the value") {
  Scratch scratch;
  FakeEnvironment environment;
  fs::path low = scratch.write("low.env", "APP_QUOTA_RATIO=0.1\n");
  fs::path high = scratch.write("high.env", "APP_QUOTA_RATIO=0.9\n");
  Options o = options(environment, {low, high});
  o.write_file = low;
  Config config = Config::parse(DEFAULTS, o);
  config.persist("quota.ratio", Value{0.5});
  CHECK(std::get<double>(config.get("quota.ratio")) == 0.9);
  config.reload_files();
  CHECK(std::get<double>(config.get("quota.ratio")) == 0.9);
  CHECK(Scratch::read(low) == "APP_QUOTA_RATIO=0.5\n");
}

TEST_CASE("persist writes a link's target and keeps the link") {
  Scratch scratch;
  FakeEnvironment environment;
  fs::path real = scratch.write("real.env", "APP_QUOTA_RATIO=0.1\n");
  fs::path link = scratch.dir / ".env.local";
  fs::create_symlink(real, link);
  Config config = Config::parse(DEFAULTS, options(environment, {link}));
  config.persist("quota.ratio", Value{0.4});
  CHECK(fs::is_symlink(link));
  CHECK(Scratch::read(real) == "APP_QUOTA_RATIO=0.4\n");
}

TEST_CASE("writers at the same time leave a whole file and no temporary ones") {
  Scratch scratch;
  fs::path file = scratch.write(".env.local", "# kept\n");
  auto write = [&](double value) {
    FakeEnvironment environment;
    Config config = Config::parse(DEFAULTS, options(environment, {file}));
    for (int i = 0; i < 200; ++i) config.persist("quota.ratio", Value{value});
  };
  std::thread one(write, 0.25), two(write, 0.75);
  one.join();
  two.join();
  std::string text = Scratch::read(file);
  CHECK((text == "# kept\nAPP_QUOTA_RATIO=0.25\n" || text == "# kept\nAPP_QUOTA_RATIO=0.75\n"));
  int entries = 0;
  for (const auto& entry : fs::directory_iterator(scratch.dir)) {
    (void)entry;
    ++entries;
  }
  CHECK(entries == 1);
}

TEST_CASE("a snapshot keeps which file each value came from") {
  Scratch scratch;
  FakeEnvironment environment;
  fs::path a = scratch.write("a.env", "APP_SERVER_PORT=x\n");
  fs::path b = scratch.write("b.env", "APP_QUOTA_RATIO=0.2\n");
  Config config = Config::parse(DEFAULTS, options(environment, {a, b}));
  State saved = config.state();
  config.set_file_values({});
  config.set("server.port", Value{std::int64_t{1}});
  config.restore(saved);
  CHECK(config.overrides().empty());
  REQUIRE(config.problems().size() == 1);
  CHECK(config.problems()[0].file == a);
}

TEST_CASE("text that is not UTF-8 is no value, the defaults file must be a file, a float of -2^63 is an integer") {
  Scratch scratch;
  FakeEnvironment environment;
  fs::path file = scratch.write(".env.local", "APP_SERVER_HOST=caf\xE9\n");
  Config config = Config::parse(DEFAULTS, options(environment, {file}));
  CHECK_THROWS_AS(config.get("server.host"), Error);
  CHECK(config.problems().size() == 1);
  CHECK_THROWS_AS(Config::load(scratch.dir, options(environment)), Error);
  config.set("server.port", Value{-9223372036854775808.0});
  CHECK(std::get<std::int64_t>(config.get("server.port")) == std::numeric_limits<std::int64_t>::min());
}

TEST_CASE("found upwards from where a path really is, and keys in tomllib's order") {
  Scratch scratch;
  fs::path file = scratch.write("a/.env.local", "A=1\n");
  fs::create_directories(scratch.dir / "b");
  CHECK_FALSE(find_upward(".env.local", scratch.dir / "a" / ".." / "b").has_value());
  CHECK(find_upward(".env.local", scratch.dir / "b" / ".." / "a") == file);
  FakeEnvironment environment;
  Options plain;
  plain.environment = environment.get();
  Config config = Config::parse("[x.y]\nz = 1\n[a]\nq = 2\n[x]\nw = 3\n", plain);
  CHECK(config.keys() == std::vector<std::string>{"x.y.z", "x.w", "a.q"});
}
