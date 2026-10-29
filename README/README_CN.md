# layeredconf
<p align="right">
    <a href="../README.md">English</a> | <strong>中文</strong>
</p>

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

一个轻量的分层配置库，核心是 Rust，带 Python 绑定：每个配置项的值取自 TOML 默认值文件、环境变量写法的文件、环境变量或命令行，并转换成默认值的类型。

## 目录

- [概述](#概述)
- [功能特性](#功能特性)
- [前置要求](#前置要求)
- [安装](#安装)
- [用法](#用法)
- [各层的写法](#各层的写法)
- [环境变量写法的文件](#环境变量写法的文件)
- [测试](#测试)
- [兼容性](#兼容性)
- [许可证](#许可证)
- [作者](#作者)

## 概述

一个 TOML 默认值文件说明有哪些配置项，每一项是什么类型：就是它默认值的类型。一个配置项的值取自有它的最高一层，

```
默认值（TOML 文件）  <  环境变量写法的文件  <  环境变量  <  覆盖（命令行）
```

并转换成默认值的类型。转换不了就是错，绝不猜：布尔值写成 `ture` 是写错了，不是假。配置项在环境变量和文件里的名字是前缀加上键名，大写、点换成下划线：前缀是 `APP_` 时，`server.port` 是 `APP_SERVER_PORT`。

## 功能特性

- 不用在代码里另写一份定义：加一个配置项，就是在 TOML 文件里加一行，带上注释
- 四层：默认值、环境变量写法的文件、环境变量、覆盖
- 严格转换成默认值的类型，文字照 Python 的方式转换
- 环境变量写法的文件大体照 python-dotenv 的方式读
- 把值原地写回文件，别的字节原样保留（`persist`）
- 密钥显示时打码；可以规定某些配置项只能由文件设置
- Rust 库，自己的代码里没有 unsafe，加上 Python 绑定，出错时是 Python 惯用的异常

## 前置要求

- [rustup](https://rustup.rs)：第一次用到时会装上 `rust-toolchain.toml` 固定的 Rust 版本（1.99.1）
- [pixi](https://pixi.sh)，其余的都由它装进这个目录的环境：
  - [maturin](https://www.maturin.rs) ≥1.9
  - Python 3.12：只构建 Rust 库也要用它，Unicode 的数字和空白表由 `tools/unicode_table.py` 从它的 `unicodedata` 生成（用
    `PYO3_PYTHON` 指定的 Python，没有就用 `PYTHON` 指定的，再没有就用 `python3`）

## 安装

layeredconf 还没有发布，需要从源码构建。

1. 克隆仓库：

   ```bash
   git clone https://github.com/lutianyu2001/layeredconf.git
   cd layeredconf
   ```

2. 打出 wheel，放在 `dist/`：

   ```bash
   pixi run wheel
   ```

3. 装进你的 Python：

   ```bash
   pip install dist/layeredconf-*.whl
   ```

Rust 项目按路径或这个仓库的 git 地址依赖 `crates/layeredconf` 里的 crate 即可；构建它同样要有 Python 3，见上。

## 用法

### Python

```python
from layeredconf import Config, find_upward

config = Config(
    "defaults.toml",
    prefix="APP_",
    files=[find_upward(".env.local"), "/srv/app/.env.local"],  # a later file wins
    root="/srv/app",                    # what a relative path is relative to
    file_only={"log.max_dir_size_mib"}, # not to be overridden on the command line
    persisted={"subagent.timeout_minutes"},
    secret={"notify.token"},
)

config.server.port                   # 8420, an int, as the default is
config.get("server.port")
config.set("server.port", "8421")    # an override, converted at once
config.path("data.dir")              # ~ expanded, a relative path joined to the root
config.source("server.port")         # "override", "env", "file" or "default"
config.shown("notify.token")         # "***" when it has a value
config.problems()                    # every value of the environment and the files that does not convert
config.persist("subagent.timeout_minutes", 45)  # written into the file for good, in force at once
```

出错时是 Python 惯用的异常：`UnknownKey` 是 `KeyError`，`InvalidValue` 和 `FileOnlyKey` 是 `ValueError`，`NotPersisted` 是 `KeyError`，它们都是 `ConfigError`；不存在的属性是 `AttributeError`。错误信息是模板，程序可以换成自己的话（`messages=`，见 `layeredconf.MESSAGES`）。

环境变量就是 `os.environ`（或者 `environ=` 给的映射），取值时才读：Python 代码设的变量立即生效。文件里的值只读进配置，绝不放进进程的环境变量：那样的话，这个进程启动的每个程序都会继承它们。`persist` 也不改环境变量（见[环境变量写法的文件](#环境变量写法的文件)）。

### Rust

```rust
use layeredconf::{Config, Options, Value};

let options = Options {
    prefix: "APP_".to_owned(),
    files: vec!["/srv/app/.env.local".into()],
    ..Options::default()
};
let mut config = Config::load("defaults.toml", options)?;
let Value::Int(port) = config.get("server.port")? else { unreachable!() };
config.set("server.port", "8421")?;
```

值是 `layeredconf::Value`（`Bool`、`Int`、`Float`、`Str`、`List`）；调用方给的值是 `Raw`，文字（`&str`、`String`、`OsString`）、其他类型和 `Value` 都能转成它。环境变量和文件里的文字是 `OsString`，不一定是 UTF-8。出错时返回 `layeredconf::Error`，`kind()` 说明是哪种错。环境变量默认是进程的，`Options::environment` 可以换成别的（`Environment::new`），而且只读不写。TOML 用 [toml](https://crates.io/crates/toml) crate 读。

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

名字不要加引号。`'NAME'=value` 这种给名字加引号的写法不规范，这个库不支持，也不打算支持（python-dotenv 会把它读成 `NAME`）：在这里引号算名字的一部分，对不上任何配置项，这一行不起作用，也不会报错。

`persist` 换掉这个配置项的那条赋值（前面可以有 `export `，引号里跨行的值整个换掉），删掉后面同名的，别的字节原样保留，然后一次把文件换上去：权限照原文件（新文件是 0600），从写的那一刻起就是；符号链接留着，写的是它指向的文件。几个进程同时写不会把文件写坏，但留下的是最后一个写的。配置读到的值跟着变，除非排在这个文件后面的文件也设了同一个名字。环境变量里有这个变量时（它比文件优先），只要环境变量还是 `persist` 写入时的那个值，配置就改用写进去的值（把它设成别的值，读到的就是那个值）；环境变量本身不改，所以进程里别的代码和它启动的程序看到的还是原来的值。文件里有没闭合的引号时，读的时候会跳过它那一行；要写的值里如果正好有这种引号，它会让那个引号闭合，把中间的行都吞进去，所以这时 `persist` 报错，不动文件。

## 测试

| 命令 | 说明 |
|---|---|
| `pixi run test` | Rust 和 Python 的测试 |
| `pixi run test-rust` | Rust 的测试 |
| `pixi run test-python` | 打出并安装 wheel，再跑 Python 的测试 |
| `pixi run lint` | rustfmt 和 clippy |

测试之外的检查（要装表里的工具，模糊测试和 Miri 还要 nightly 版的 Rust，Kani 自带它要的版本；依赖的检查见 [SUPPLY_CHAIN.md](../SUPPLY_CHAIN.md)）：

| 命令 | 说明 |
|---|---|
| `cargo kani -p layeredconf` | 用 [Kani](https://model-checking.github.io/kani/) 证明读文字的函数（空白、整数、布尔值、列表）在 6 字节以内的任何输入下都不 panic、不溢出，读法和标准库一致，并单独证明 64 位整数边界处的读法（`crates/layeredconf/src/proofs.rs`）；env 文件的读取器靠它的测试和模糊测试检查 |
| `cd fuzz && cargo +nightly fuzz run <target>` | 用 [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) 做模糊测试：`envfile`、`numbers`、`config` |
| `cargo +nightly miri test -p layeredconf` | 在 [Miri](https://github.com/rust-lang/miri) 下跑 Rust 的测试，检查未定义行为 |

Kani 全部跑完约一小时：64 位边界的四个证明各要 13 分钟左右、11 GB 内存，其余的几秒到一分钟。只跑一个证明用 `--harness <名字> --exact`。

## 兼容性

目前只支持 POSIX（Linux、macOS）。

测试环境：

- Ubuntu 24.04.4 LTS（x86_64）
  - Rust 1.99.1
  - Python 3.12.15
  - [maturin](https://github.com/PyO3/maturin) 1.15.0
  - [PyO3](https://github.com/PyO3/pyo3) 0.30.1
  - [toml](https://crates.io/crates/toml) 1.1.8

## 许可证

Copyright 2026 Tianyu (Sky) Lu

The entire repository is licensed under the Apache License, Version 2.0 
(the "License"); you may not use any file in this repository except in 
compliance with the License. You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

## 作者

Tianyu (Sky) Lu
