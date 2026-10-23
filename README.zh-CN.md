# layeredconf

分层配置库：核心是 C++，带 Python 绑定。[English](README.md)

一个 TOML 默认值文件说明有哪些配置项，每一项是什么类型：就是它默认值的类型。一个配置项的值取自有它的最高一层，

```
默认值（TOML 文件）  <  环境变量写法的文件  <  环境变量  <  覆盖（命令行）
```

并转换成默认值的类型。转换不了就是错，绝不猜：布尔值写成 `ture` 是写错了，不是假。配置项在环境变量和文件里的名字是前缀加上键名，大写、点换成下划线：前缀是 `APP_` 时，`server.port` 是 `APP_SERVER_PORT`。

不用在代码里另写一份定义：加一个配置项，就是在 TOML 文件里加一行，带上注释。

## Python

```python
from layeredconf import Config, find_upward

config = Config(
    "defaults.toml",
    prefix="APP_",
    files=[find_upward(".env.local"), "/srv/app/.env.local"],  # 后面的文件优先
    root="/srv/app",                    # 相对路径相对于它
    file_only={"log.max_dir_size_mib"}, # 不能用命令行覆盖
    persisted={"subagent.timeout_minutes"},
    secret={"notify.token"},
)

config.server.port                   # 8420，和默认值一样是整数
config.get("server.port")
config.set("server.port", "8421")    # 覆盖，当场转换
config.path("data.dir")              # 展开 ~，相对路径接在 root 后面
config.source("server.port")         # "override"、"env"、"file" 或 "default"
config.shown("notify.token")         # 有值时是 "***"
config.problems()                    # 环境变量和文件里所有转换不了的值
config.persist("subagent.timeout_minutes", 45)  # 写进文件，立即生效
```

出错时是 Python 惯用的异常：`UnknownKey` 是 `KeyError`，`InvalidValue` 和 `FileOnlyKey` 是 `ValueError`，`NotPersisted` 是 `KeyError`，它们都是 `ConfigError`；不存在的属性是 `AttributeError`。错误信息是模板，程序可以换成自己的话（`messages=`，见 `layeredconf.MESSAGES`）。

环境变量就是 `os.environ`（或者 `environ=` 给的映射），取值时才读：Python 代码设的变量立即生效。文件里的值只读进配置，绝不放进进程的环境变量：那样的话，这个进程启动的每个程序都会继承它们。

## C++

```cpp
#include <layeredconf/layeredconf.hpp>

layeredconf::Options options;
options.prefix = "APP_";
options.files = {"/srv/app/.env.local"};
auto config = layeredconf::Config::load("defaults.toml", options);
auto port = std::get<std::int64_t>(config.get("server.port"));
```

库是 C++17，用 [toml++](https://marzer.github.io/tomlplusplus/) 读 TOML。值是 `std::variant<bool, std::int64_t, double, std::string, std::vector<std::string>>`；出错时抛出 `layeredconf::Error`，`kind()` 说明是哪种错。

## 各层的写法

| 默认值的类型 | 写成文字时 |
|---|---|
| 布尔 | `1`、`true`、`yes`、`on`；`0`、`false`、`no`、`off` 或空（不分大小写） |
| 整数 | 和 Python 的 `int()` 一样：首尾空白、正负号、数字、数字之间的下划线 |
| 浮点数 | 和 Python 的 `float()` 一样：同上，再加小数点、指数、`inf`、`nan` |
| 字符串 | 原样 |
| 字符串列表 | 逗号分隔，每项去掉首尾空白，空项不要 |

文字必须是 UTF-8（不是的算写错）。数字和空白照 Python 认，ASCII 以外的也认：`８４２０` 和 `int()` 读出来一样是 8420，不换行空格、全角空格（U+3000）都算空白，值里和文件里都是。哪些字符算，取自构建这个库的那个 Python 的 `unicodedata`，所以每个 wheel 的 Unicode 版本和它对应的 Python 一样。从 Python 给的不是文字的值，照 Python 的转换（`int()`、`float()`、`bool()`、`str()`、`list()`），`Decimal`、`None` 也一样。别的类型的 TOML 默认值（日期、数字的列表）在读文件时就拒绝：别的层写不出这种值。浮点数写回去的样子和 Python 一样（`45.0`、`1e+16`），所以换成这个库以后，Python 程序的行为不变。

## 环境变量写法的文件

一行一个 `NAME=value`，大体和 python-dotenv 一样：注释、`export `、不加引号的值（空白后面的 `#` 开始注释，所以 `NAME= # 备注` 是空的）、`'单引号'` 里原样、`"双引号"` 里有转义、引号里可以跨行，不加引号和双引号的值里可以写 `${NAME}` 或 `${NAME:-默认}`（先找这个文件前面写过的，再找环境变量；单引号里原样不动，python-dotenv 会展开）。开头的 BOM、`\r\n` 和 `\r` 结尾的行也认。这里的空白也照 Python 认，和 python-dotenv 的 `\s` 一样：`#` 前面是全角空格，也和普通空格一样开始注释。

`persist` 换掉这个配置项的那条赋值（前面可以有 `export `，引号里跨行的值整个换掉），删掉后面同名的，别的字节原样保留，然后一次把文件换上去：权限照原文件（新文件是 0600），从写的那一刻起就是；符号链接留着，写的是它指向的文件。几个进程同时写不会把文件写坏，但留下的是最后一个写的。配置读到的值跟着变，除非排在这个文件后面的文件也设了同一个名字。

## 构建

都在这个目录的 [pixi](https://pixi.sh) 环境里（编译器、CMake、toml++、pybind11，还有 Python：只构建 C++ 库也要用它，Unicode 的数字和空白表由 `tools/unicode_table.py` 从它的 `unicodedata` 生成）：

```bash
pixi run test          # C++ 和 Python 的测试
pixi run wheel         # 打出 wheel，放在 dist/
```

wheel 静态链接了 C++ 运行库，不依赖构建环境里的那一份。

## 状态

还没有发布。目前只支持 POSIX（Linux、macOS）。
