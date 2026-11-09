# layeredcfg
<p align="right">
    <a href="../README.md">English</a> | <strong>中文</strong>
</p>

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Release](https://img.shields.io/github/v/release/lutianyu2001/layeredcfg)](https://github.com/lutianyu2001/layeredcfg/releases/latest)
[![Crates.io Version](https://img.shields.io/crates/v/layeredcfg)](https://crates.io/crates/layeredcfg)
[![PyPI Version](https://img.shields.io/pypi/v/layeredcfg)](https://pypi.org/project/layeredcfg/)<br>
[![CI](https://github.com/lutianyu2001/layeredcfg/actions/workflows/ci.yml/badge.svg)](https://github.com/lutianyu2001/layeredcfg/actions/workflows/ci.yml)
[![audit](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Faudit.json)](../SECURITY.md#ci)
[![pins](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fpins.json)](../xtask/src/pins.rs)

一个轻量的分层配置库，核心是 Rust，带 Python 绑定：每个配置项的值取自 TOML 默认值文件、用户的 TOML 文件、环境变量写法的文件、环境变量或命令行，并转换成默认值的类型。

## 目录

- [概述](#概述)
- [功能特性](#功能特性)
- [安全审计](#安全审计)
- [前置要求](#前置要求)
- [安装](#安装)
- [用法](#用法)
- [各层的写法](#各层的写法)
- [用户的 TOML 文件](#用户的-toml-文件)
- [环境变量写法的文件](#环境变量写法的文件)
- [关掉某一层](#关掉某一层)
- [测试](#测试)
- [兼容性](#兼容性)
- [许可证](#许可证)
- [作者](#作者)

## 概述

一个 TOML 默认值文件说明有哪些配置项，每一项是什么类型：就是它默认值的类型。一个配置项的值取自有它的最高一层，

```text
默认值（TOML 文件）  <  用户的 TOML 文件  <  环境变量写法的文件  <  环境变量  <  覆盖（命令行）
```

并转换成默认值的类型。转换不了就是错，绝不猜：布尔值写成 `ture` 是写错了，不是假。用户的 TOML 文件用的是默认值文件里的键。配置项在环境变量和环境变量写法的文件里的名字是前缀加上键名，大写、点换成下划线：前缀是 `APP_` 时，`server.port` 是 `APP_SERVER_PORT`。程序用不到的层可以关掉。

## 功能特性

- 不用在代码里另写一份定义：加一个配置项，就是在 TOML 文件里加一行，带上注释
- 五层：默认值、用户的 TOML 文件、环境变量写法的文件、环境变量、覆盖；除了第一层和最后一层都可以关掉
- 严格转换成默认值的类型，文字照 Python 的方式转换
- 环境变量写法的文件大体照 python-dotenv 的方式读
- 把值原地写回文件，别的字节原样保留（`persist`）
- 密钥显示时打码；可以规定某些配置项只能由文件设置
- Rust 库，自己的 unsafe 代码只有在密码数据库里查家目录这一处；加上没有 unsafe 的 Python 绑定，出错时是 Python 惯用的异常

## 安全审计

[![review](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Freview.json)](../audit/review)
[![deny](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fdeny.json)](https://github.com/lutianyu2001/layeredcfg/actions/workflows/advisories.yml)
[![vet](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fvet.json)](../audit/vet)
[![geiger](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fgeiger.json)](../audit/geiger)
[![miri](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fmiri.json)](../audit/miri)
[![asan](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fasan.json)](../audit/asan)
[![coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fcoverage.json)](../audit/coverage)
[![kani](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Fkani.json)](../audit/kani)
[![fuzz](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Flutianyu2001%2Flayeredcfg%2Fbadges%2Ffuzz.json)](../audit/fuzz)

这是项目维护者自己对代码做的安全审计，不是第三方的审计：它说明查了什么、怎么查的，不保证没有问题。每项检查最近一次的结果和代码一起提交在 [`audit/`](../audit)，记着是在哪个提交上跑的；上面的徽章显示每项结果对 `main` 上现在的代码是否仍然有效。耗时开跑前就能知道的检查（评审记录、deny、vet、geiger、AddressSanitizer、覆盖率和模糊测试），以及只要几分钟的 Miri，在每次发版时由 GitHub 重跑；Kani 手动跑。怎么跑、每项覆盖什么，见 [SECURITY.md](../SECURITY.md)；依赖见 [SUPPLY_CHAIN.md](../SUPPLY_CHAIN.md)。

- **按威胁模型审查。** 每一层里的文字，任何字节都算；密钥会不会出现在显示的值、错误信息和 Python 的异常里；`persist` 写的文件同一台机器上的其他用户能不能碰到（权限、符号链接、写了一半的文件），几个进程同时写；`${NAME}` 能不能把一个小文件的值撑得很大。发现的问题都修了，没修的写明留下了什么风险，见 SECURITY.md。
- **`unsafe` 代码。** 绑定层没有（`#![forbid(unsafe_code)]`）。库只在 `src/passwd.rs` 这一个模块里有：为 `~` 和 `~name` 在密码数据库里查家目录（C 库的 `getpwuid_r` 和 `getpwnam_r`，Rust 标准库没有按名字查的函数）。每个 unsafe 块只做一次调用或读一个指针，并写明为什么成立。调用、读取调用填好的结果（有写明的前提条件）和重试三部分分开：重试是安全代码，C 库可能给的每种回答都有测试，Kani 对任意回答序列都证明过；读取在 Miri 下用照 C 库的方式造出的结果测试；真实的调用在 glibc、musl 和 macOS 上和 `getent`（Linux）、`dscl`（macOS）对照，并在 AddressSanitizer 下跑。这个模块有两位评审读过，评审记录带着文件的哈希，文件一改就要重新评审（`cargo xtask review`）。依赖里的 `unsafe` 由 cargo geiger 计数，大部分在调用 Python C API 的 PyO3 里。
- **依赖。** 25 个 crate，只来自 crates.io，由 cargo deny 检查已知漏洞、无人维护或被撤回的 crate、许可证和来源。cargo vet 里，2 个有其他组织的审计记录（导入的），其余 23 个由维护者自己批准（certify），没有豁免。依赖没有本地化（vendor）：这是一个库，`Cargo.lock` 固定了每个版本和校验和。
- **Miri。** Rust 库的 45 个测试（各层、persist、环境变量写法的文件、读文字的函数）在 Miri 下跑过，没有未定义行为；绑定层调用 Python 的 C API，不在其内。
- **模糊测试。** 三个 target，在工作站上每个跑一小时，发版时每个跑四小时：`envfile`（读出的语句按顺序落在原文以内，`persist` 写进去的值读回来不变）、`numbers`（整数和浮点数写出来再读回来不变，ASCII 以外的数字也算，`strip` 幂等，列表的每项不含逗号、首尾没有空白）、`config`（读默认值，以及对每个配置项 `get`、`shown`、`path`、`set` 都不 panic）。
- **Kani。** 13 个证明：在 6 字节以内的任何输入下，不论 ASCII 以外的字符被当作什么，UTF-8 的解码和标准库一致，空白只在字符之间切，整数和布尔值读取不 panic、不溢出，列表只在逗号处切开；在 6 字节以内的任何 ASCII 输入下，空白一次去干净，整数读取结果和 `str::parse` 一致；另外 21 字节的整数在 64 位边界处读得准确。每个证明单独跑、事先编译好，2026-11-05 在 1466eab 上跑，机器是 AMD Ryzen 9 5950X、32 GB 内存：

  | 证明 | 耗时 | 峰值内存 |
  |---|---|---|
  | `decode_past_the_end_is_no_character` | 1 秒 | 95 MiB |
  | `decode_reads_utf8_as_the_standard_library_does` | 56 秒 | 1.3 GiB |
  | `parse_bool_does_not_panic` | 19 秒 | 614 MiB |
  | `parse_int_does_not_panic_or_overflow` | 43 秒 | 1.3 GiB |
  | `parse_int_is_exact_at_the_largest_i64` | 19 分 0 秒 | 9.6 GiB |
  | `parse_int_is_exact_at_the_smallest_i64` | 16 分 13 秒 | 9.6 GiB |
  | `parse_int_is_exact_past_u64` | 14 分 28 秒 | 9.6 GiB |
  | `parse_int_is_exact_past_u64_negative` | 14 分 19 秒 | 9.6 GiB |
  | `parse_int_reads_plain_numbers_as_the_standard_library_does` | 59 秒 | 2.2 GiB |
  | `space_at_stays_in_the_text` | 1 秒 | 103 MiB |
  | `split_list_cuts_at_the_commas_and_nowhere_else` | 53 秒 | 2.1 GiB |
  | `strip_str_cuts_at_the_boundaries_of_characters` | 23 秒 | 1.1 GiB |
  | `strip_takes_whitespace_off_once_and_for_all` | 19 秒 | 1.0 GiB |
- **分支覆盖率。** Rust 测试走到了 Rust 代码 310 个分支中的 85.81%，其中库的 294 个里是 90.48%（`audit/coverage/summary.txt`）；绑定层的 16 个留给 Python 测试。
- **两路评审。** 每次改动都经过代码评审；最敏感的代码（环境变量写法的文件的读取器、`persist`、密钥打码）另由几位评审从头到尾读过，给他们的只有威胁模型，没有已有的结论。

这些说明了什么：本项目自己的代码是 Rust，`unsafe` 只有调用密码数据库的那两处，没有内存错误（除非编译器、C 库或依赖里的 `unsafe` 代码有错），上面的检查也没有发现 panic、溢出或未定义行为。没有说明什么：Rust 不排除逻辑错误；依赖里的 `unsafe` 代码靠它们自己和它们的审计记录；Miri 和 Kani 都跟不进对 C 库的调用，那部分只靠测试；Kani 只证明界限以内的输入，模糊测试只是抽样。没有证明、只靠测试和模糊测试覆盖的：ASCII 以外哪些字符算空白或数字（取自正在运行的 Python，Python 测试对它的每个数字和空白字符逐一对照过）、`parse_float`、环境变量写法的文件的读取器，以及读文字的函数之上的部分（各层、`persist`）。

## 前置要求

- Python 包要求 Python 3.10 或更新，crate 要求 Rust 1.88 或更新
- 从源码构建和跑测试时：
  - [rustup](https://rustup.rs)：第一次用到时会装上 `rust-toolchain.toml` 固定的 Rust 版本（1.99.1）
  - [pixi](https://pixi.sh)，其余的都由它装进这个目录的环境：
    - [maturin](https://www.maturin.rs) ≥1.9
    - Python 3.12：给绑定层和它的测试用

## 安装

### 从 PyPI 安装（推荐）

```bash
pip install layeredcfg
```

wheel 只有 Linux 的（x86_64 和 aarch64，glibc 和 musl），每个平台一个，适用于 Python 3.10 及以后的所有版本。其他系统（macOS）上 pip 会从源码包构建，需要 Rust 1.88 或更新。

### 从 crates.io 安装

```bash
cargo add layeredcfg
```

### 从源码构建

1. 克隆仓库：

   ```bash
   git clone https://github.com/lutianyu2001/layeredcfg.git
   cd layeredcfg
   ```

2. 打出 wheel，放在 `dist/`：

   ```bash
   pixi run wheel
   ```

3. 装进你的 Python：

   ```bash
   pip install dist/layeredcfg-*.whl
   ```

Rust 项目不从 crates.io 而从这个仓库取 crate 时，按路径或这个仓库的 git 地址依赖 `crates/layeredcfg` 即可。

发布到 PyPI 的文件（源码包和各平台的 wheel）用 `pixi run release-files <tag>` 从发布的 tag 构建，放在 `dist/`。它需要 [zig](https://ziglang.org)、各个 wheel 的 Rust target（`rustup target add`）和 [cargo-about](https://github.com/EmbarkStudios/cargo-about) 0.9.3，后者把编进 wheel 的 crate 的协议全文收集进每个 wheel。构建出的文件里只要出现构建它的机器上的路径，或者缺了这些协议全文，它就失败。

## 用法

### Python

```python
from pathlib import Path

from layeredcfg import Config, find_upward

config = Config(
    "defaults.toml",
    prefix="APP_",
    user_files=["/etc/app/config.toml", Path.home() / ".config/app/config.toml"],  # a later file wins
    files=[find_upward(".env.local"), "/srv/app/.env.local"],  # a later file wins
    root="/srv/app",                    # what a relative path is relative to
    file_only={"log.max_dir_size_mib"}, # not to be overridden on the command line
    persisted={"subagent.timeout_minutes"},
    secret={"notify.token"},
    disabled=set(),                     # layers turned off: "user", "file", "env"
)

config.server.port                   # 8420, an int, as the default is
config.get("server.port")
config.set("server.port", "8421")    # an override, converted at once
config.path("data.dir")              # ~ expanded, a relative path joined to the root
config.source("server.port")         # "override", "env", "file", "user" or "default"
config.shown("notify.token")         # "***" when it has a value
config.problems()                    # every value of the environment and the files that does not convert
config.persist("subagent.timeout_minutes", 45)  # written into the file for good, in force at once
```

出错时是 Python 惯用的异常：`UnknownKey` 是 `KeyError`，`InvalidValue` 和 `FileOnlyKey` 是 `ValueError`，`NotPersisted` 是 `KeyError`，`BadUserFile` 是 `ValueError`，它们都是 `ConfigError`；不存在的属性是 `AttributeError`。错误信息是模板，程序可以换成自己的话（`messages=`，见 `layeredcfg.MESSAGES`）。

环境变量就是 `os.environ`（或者 `environ=` 给的映射），取值时才读：Python 代码设的变量立即生效。文件里的值只读进配置，绝不放进进程的环境变量：那样的话，这个进程启动的每个程序都会继承它们。`persist` 也不改环境变量（见[环境变量写法的文件](#环境变量写法的文件)）。

### Rust

```rust,no_run
use layeredcfg::{Config, Options, Source, Value};

fn main() -> Result<(), layeredcfg::Error> {
    let options = Options {
        prefix: "APP_".to_owned(),
        user_files: vec!["/etc/app/config.toml".into()],
        files: vec!["/srv/app/.env.local".into()],
        disabled: [Source::Env].into(), // the environment's variables are no settings here
        ..Options::default()
    };
    let mut config = Config::load("defaults.toml", options)?;
    let Value::Int(port) = config.get("server.port")? else { unreachable!() };
    config.set("server.port", "8421")?;
    println!("{port}");
    Ok(())
}
```

值是 `layeredcfg::Value`（`Bool`、`Int`、`Float`、`Str`、`List`）；调用方给的值是 `Raw`，文字（`&str`、`String`、`OsString`）、其他类型和 `Value` 都能转成它。环境变量和文件里的文字是 `OsString`，不一定是 UTF-8。出错时返回 `layeredcfg::Error`，`kind()` 说明是哪种错。环境变量默认是进程的，`Options::environment` 可以换成别的（`Environment::new`），而且只读不写。TOML 用 [toml](https://crates.io/crates/toml) crate 读。

## 各层的写法

| 默认值的类型 | 写成文字时 |
|---|---|
| 布尔 | `1`、`true`、`yes`、`on`；`0`、`false`、`no`、`off` 或空（不分大小写） |
| 整数 | 和 Python 的 `int()` 一样：首尾空白、正负号、数字、数字之间的下划线 |
| 浮点数 | 和 Python 的 `float()` 一样：同上，再加小数点、指数、`inf`、`nan` |
| 字符串 | 原样 |
| 字符串列表 | 逗号分隔，每项去掉首尾空白，空项不要 |

文字必须是 UTF-8（不是的算写错）。数字和空白照 Python 认，ASCII 以外的也认：`８４２０` 和 `int()` 读出来一样是 8420，不换行空格、全角空格（U+3000）都算空白，值里和文件里都是。ASCII 以外的字符，绑定层去问正在运行的 Python（`str.isspace()`、`unicodedata.decimal()`），所以每个平台只有一个 wheel，读文字时用的是运行它的那个 Python 的 Unicode 版本。在 Rust 里，ASCII 以外的空白照 Unicode 认（`char::is_whitespace`，字符和 Python 的一样），ASCII 以外没有数字，和 `str::parse` 一样，除非用 `text::set_beyond_ascii` 另行指定。从 Python 给的不是文字的值，照 Python 的转换（`int()`、`float()`、`bool()`、`str()`、`list()`），`Decimal`、`None` 也一样。别的类型的 TOML 默认值（日期、数字的列表）在读文件时就拒绝：别的层写不出这种值。浮点数写回去的样子和 Python 一样（`45.0`、`1e+16`），所以换成这个库以后，Python 程序的行为不变。

## 用户的 TOML 文件

默认值是程序的；用户把要改的配置项写进同样键名的 TOML 文件，不改的可以不写：

```toml
[server]
port = 9000
```

文件不存在就跳过，后面的文件比前面的优先（先是系统的文件，再是用户自己的）。值照别的层转换文字的方式转换成默认值的类型（整数写成 `"9000"` 是 9000，布尔值写成 `"no"` 是假）。生成配置和调用 `reload_files` 时读这些文件，文件里写错了当时就报错，报 `BadUserFile`，错误信息里有文件、行和列：TOML 语法错误、不是配置项的键或表、转换不了的值（密钥的值显示成 `***`）。调用方自己读到的文字用 `set_user_text` 给；读到了什么用 `user_values` 看。

## 环境变量写法的文件

一行一个 `NAME=value`，大体和 python-dotenv 一样：注释、`export `、不加引号的值（空白后面的 `#` 开始注释，所以 `NAME= # 备注` 是空的）、`'单引号'` 里原样、`"双引号"` 里有转义、引号里可以跨行，不加引号和双引号的值里可以写 `${NAME}` 或 `${NAME:-默认}`（先找这个文件前面写过的，再找环境变量；单引号里原样不动，python-dotenv 会展开）。开头的 BOM、`\r\n` 和 `\r` 结尾的行也认。这里的空白也照 Python 认，和 python-dotenv 的 `\s` 一样：`#` 前面是全角空格，也和普通空格一样开始注释。

名字不要加引号。`'NAME'=value` 这种给名字加引号的写法不规范，这个库不支持，也不打算支持（python-dotenv 会把它读成 `NAME`）：在这里引号算名字的一部分，对不上任何配置项，这一行不起作用，也不会报错。

`persist` 换掉这个配置项的那条赋值（前面可以有 `export `，引号里跨行的值整个换掉），删掉后面同名的，别的字节原样保留，然后一次把文件换上去：权限照原文件（新文件是 0600），从写的那一刻起就是；符号链接留着，写的是它指向的文件。几个进程同时写不会把文件写坏，但留下的是最后一个写的。配置读到的值跟着变，除非排在这个文件后面的文件也设了同一个名字。环境变量里有这个变量时（它比文件优先），只要环境变量还是 `persist` 写入时的那个值，配置就改用写进去的值（把它设成别的值，读到的就是那个值）；环境变量本身不改，所以进程里别的代码和它启动的程序看到的还是原来的值。文件里有没闭合的引号时，读的时候会跳过它那一行；要写的值里如果正好有这种引号，它会让那个引号闭合，把中间的行都吞进去，所以这时 `persist` 报错，不动文件。

## 关掉某一层

`disabled`（Rust 里是 `Options::disabled`）列出程序用不到的层：`user`、`file`、`env`（`Source::User`、`Source::File`、`Source::Env`）。关掉的层一个值都不读，它们里面写错的值也不算问题。不是配置项的东西照样读环境变量：路径里的 `~` 用 `HOME`，环境变量写法的文件里的 `${NAME}` 也照样展开。`persist` 写的是环境变量写法的文件，这一层关掉时它报错。默认值和覆盖这两层不能关；不要覆盖的程序不调用 `set` 就行。

## 测试

| 命令 | 说明 |
|---|---|
| `pixi run test` | Rust 和 Python 的测试 |
| `pixi run test-rust` | Rust 的测试 |
| `pixi run test-python` | 打出并安装 wheel，再跑 Python 的测试 |
| `pixi run lint` | rustfmt 和 clippy |

## 兼容性

目前只支持 POSIX（Linux、macOS）。

测试环境：

- Ubuntu 24.04.4 LTS（x86_64）
  - Rust 1.99.1
  - Python 3.12.15，manylinux 的 wheel 另在 Python 3.15.0 上测过
  - [maturin](https://github.com/PyO3/maturin) 1.15.0
  - [PyO3](https://github.com/PyO3/pyo3) 0.30.1
  - [toml](https://crates.io/crates/toml) 1.1.8

## 许可证

Copyright 2026 Tianyu (Sky) Lu

The entire repository is licensed under the Apache License, Version 2.0 
(the "License"); you may not use any file in this repository except in 
compliance with the License. You may obtain a copy of the License at

```plaintext
http://www.apache.org/licenses/LICENSE-2.0
```

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

## 作者

Tianyu (Sky) Lu
