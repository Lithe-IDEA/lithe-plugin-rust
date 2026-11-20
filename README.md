# Rust Support for Lithe

为 [Lithe](https://github.com/1lck/Lithe-IDEA) 提供 Rust 支持：用 rustup 装的 `rust-analyzer` 提供代码智能，
用 Cargo 识别软件包和工作区，在构建窗口里列出软件包和目标，运行 Build、Check、Clippy、Doc 和 Clean，
运行二进制和示例，查看依赖树；用 CodeLLDB 调试，CodeLLDB 在第一次需要时由 Lithe 下载；在「新建项目」里创建 Cargo 软件包。
这是一个**社区插件**，按插件契约 v0 编写。插件包含一份 `plugin.json`、几份项目模板，和一个用 Rust SDK 写的后端；
cargo、rustup 和 rust-analyzer 用你电脑上的。

> English: Rust support for Lithe — code intelligence through the rust-analyzer of your rustup toolchain, Cargo
> packages and workspaces in the build window (model, Build, Check, Clippy, Doc, Clean, running binaries and
> examples, the dependency tree), debugging with CodeLLDB, which Lithe downloads the first time you debug, and a
> Rust package entry in New Project. The plugin is a `plugin.json`, a few project templates and a backend written
> with the Rust plugin SDK; cargo, rustup and rust-analyzer stay your own.

> **兼容性**：按插件契约 v0（`schemaVersion: 0`、`pluginAPIVersion: 0`）编写，只能装进新版 Lithe 桌面端
> （`desktop-v*` 预发布），旧版 macOS Lithe 无法安装。下载 CodeLLDB 用的是下载型命令安装器
> （[1lck/Lithe-IDEA#2241](https://github.com/1lck/Lithe-IDEA/pull/2241)，固定了
> [Lithe-core-rust#133](https://github.com/Lithe-IDEA/Lithe-core-rust/pull/133) 的 Core），桌面端 0.8.11 还没有它，
> 会拒绝安装这个插件；0.1.0 因此作为预发布版本发布。rustup 装的 `cargo` 和 `rust-analyzer` 是指向 `rustup` 的链接，
> 工具定义要在 macOS 上找到它们，需要固定了 [Lithe-core-rust#135](https://github.com/Lithe-IDEA/Lithe-core-rust/pull/135)
> 的桌面端（[1lck/Lithe-IDEA#2242](https://github.com/1lck/Lithe-IDEA/pull/2242)）。

本插件取代 `lithe-plugin-cargo`：Cargo 构建系统并入这里，那个插件不再维护。两个插件都声明了 `cargo` 构建系统，
不要同时启用。

## 功能

| 功能 | 说明 |
| --- | --- |
| 代码智能 | 补全、跳转定义、悬停、诊断、格式化，由 `rust-analyzer` 提供；保存时运行 `cargo check`（或 `cargo clippy`），结果显示为诊断 |
| 识别软件包 | 含 `Cargo.toml` 的最外层目录是构建根；`cargo metadata --no-deps` 的结果组成软件包和目标，只读清单，不联网 |
| 构建 | Build（`cargo build`）、Check、Clippy 作用于工作区、单个软件包或单个目标；Doc（`cargo doc --no-deps`）、Clean 作用于工作区或软件包 |
| 运行 | 每个二进制和示例在构建窗口里有运行按钮，执行 `cargo run --bin` 或 `cargo run --example`；打开 `src/main.rs`、`src/bin/*.rs`、`examples/*.rs` 时编辑器里也有运行入口 |
| 诊断 | 构建任务用 `--message-format=short`，`文件:行:列: error|warning: 消息` 的输出显示为问题 |
| 依赖 | `cargo metadata` 解析出的依赖图按 `cargo tree` 的样子列出；同一个软件包第二次出现时不再展开 |
| 同步 | `Cargo.toml`、`Cargo.lock`、`.cargo/config(.toml)`、`rust-toolchain(.toml)` 变化后重新读取 |
| 调试 | 构建窗口里二进制和示例的「调试」：先构建，再用 CodeLLDB 启动程序，支持断点、单步、调用栈、变量，程序输出显示在调试控制台 |
| 新建项目 | 「新建项目」里的 Rust：选择 Cargo，选二进制或库，生成 `Cargo.toml`、`.gitignore` 和 `src/main.rs` 或 `src/lib.rs`，不运行 `cargo new` |
| 工具链 | 在常见位置找到 cargo、rustup 和 rust-analyzer；项目可以固定用其中一个 Cargo 构建和运行 |

## 设置和构建选项

**插件设置**（设置 → 插件 → Rust Support →「设置」，也显示在「语言与框架 › Rust」页）：

| 设置 | 生效方式 | 说明 |
| --- | --- | --- |
| Check on save | 立即 | 保存 Rust 文件时运行 Cargo 检查，把结果显示为诊断 |
| Check command | 立即 | 保存时检查用 `cargo check` 还是 `cargo clippy`；Clippy 多报告它的 lint，耗时更长 |
| Install rust-analyzer with rustup | 下次启动语言服务器 | 项目的 rustup 工具链缺少 rust-analyzer 时，先运行 `rustup component add rust-analyzer rust-src` 再启动它；需要联网 |

前两项通过 `workspace/configuration` 发给正在运行的 rust-analyzer，不需要重启语言服务器。

**构建选项**（构建窗口 →「构建选项」），只对当前工作区生效：

| 选项 | 传给 cargo 的内容 |
| --- | --- |
| Release Profile | `--release`，程序输出到 `target/release` |
| Features | 选中的特性，按软件包列出；命令只传入它所构建的软件包的特性（`--features`） |
| All Features | `--all-features`，忽略上面选择的特性 |
| No Default Features | `--no-default-features` |
| Offline | `--offline` |

选项会加到构建、检查、Clippy、Doc、运行和调试的命令里，Clean 只看 Release Profile。

**工具链**（设置 →「构建、执行、部署」→「工具链」）：插件声明了 `cargo`、`rustup` 和 `rust-analyzer` 三个工具，
Lithe 在 `~/.cargo`、Homebrew（`/opt/homebrew/Cellar`、`/usr/local/Cellar`，Linux 上是 Linuxbrew）里查找，
用 `--version` 读出版本，列在这里。构建用的 Cargo 按顺序取第一个可用的：项目固定的 Cargo、全局默认的 Cargo、
`CARGO_HOME` 里的 `bin/cargo`、`PATH` 里的 `cargo`、找到的安装。选中的 Cargo 所在的 `bin` 目录加到命令的 `PATH` 前面。

## rust-analyzer

rust-analyzer 每次启动前，Lithe 都会问插件后端该怎样启动它（`rustAnalyzer.launch`），所以它跟着项目的工具链走：

| Lithe 找到的 | 怎样启动 |
| --- | --- |
| 不是 rustup 代理的 rust-analyzer（在设置里选的，或单独安装的） | 直接启动它，`CARGO` 设为构建系统用的 Cargo |
| rustup | `rustup run <工具链> rust-analyzer`，工具链是 rustup 在项目目录里选出的那一个。工具链缺少 rust-analyzer 组件时，先用 `rustup component add` 装上它和 rust-src（可在设置里关闭） |
| 只有 rust-analyzer 的 rustup 代理 | 启动这个代理 |
| 都没有 | 报错，提示怎样安装 rust-analyzer |

rustup 运行时带 `RUSTUP_AUTO_INSTALL=0`：只查询项目用哪个工具链时，不会顺便安装工具链。项目要求的工具链没有安装时，
错误信息里给出安装它的命令。工具链放在项目目录里时，它是项目代码，插件不启动它的 rust-analyzer。

## 调试

在构建窗口里，二进制或示例的运行按钮旁边有「调试」。点击后插件先用 `cargo build` 构建这个目标（带上构建选项），
再让 CodeLLDB 启动构建出的程序，在断点处停下。

- 程序的工作目录是软件包所在的目录，运行参数和环境变量和运行时相同。
- CodeLLDB 以 `sourceLanguages: ["rust"]` 启动，变量按 Rust 的类型显示。
- 程序结束后会话自动结束，输出留在调试控制台里。
- 目前只支持启动调试（launch），不支持附加到运行中的进程，也不支持调试测试。
- 从经过符号链接的路径打开项目时（例如 macOS 上从 `/tmp` 打开，`/tmp` 是 `/private/tmp` 的链接），断点不会停下：
  rustc 在调试信息里记下的是解析掉链接后的真实路径，CodeLLDB 按完整路径匹配断点。请从真实路径打开项目。

**CodeLLDB 不在插件包里。** 第一次调试前，打开 **设置 → 插件 → 已安装**，选中 Rust Support，切到 **功能**：
调试能力卡片上显示“未找到 codelldb”，旁边有 **安装** 按钮。点击后 Lithe 从
[vadimcn/codelldb](https://github.com/vadimcn/codelldb/releases/tag/v1.12.3) 的 1.12.3 版下载对应平台的 VSIX（Apple Silicon 的约 45 MB），
核对插件清单里写死的 SHA-256，解压到 Lithe 给插件的状态目录，之后启动调试时直接用 `extension/adapter/codelldb`。
卸载插件时一起删除。

| 平台 | CodeLLDB 包 |
| --- | --- |
| macOS Apple Silicon | `codelldb-darwin-arm64.vsix` |
| macOS Intel | `codelldb-darwin-x64.vsix` |
| Windows x64 | `codelldb-win32-x64.vsix` |
| Linux x64、ARM64 | `codelldb-linux-x64.vsix`、`codelldb-linux-arm64.vsix` |

CodeLLDB 1.12.3 没有发布 Windows ARM64 的包，那里没有调试，其他功能可用。也可以自己把 `codelldb` 放进 Lithe
能看到的 `PATH`，Lithe 会直接用它。

## 新建项目

「新建项目」对话框左侧有 **Rust**。可以填写：

| 字段 | 说明 |
| --- | --- |
| Cargo | 项目构建和运行用的 Cargo，从「工具链」找到的安装里选。创建后项目固定使用它（记在项目的 `.lithe/toolchains/local.json`），以后可以在「工具链」里修改。没有找到 Cargo 时留空，不影响创建 |
| Crate | Binary（默认）生成打印 `Hello, Rustacean!` 的 `src/main.rs`；Library 生成带一个测试的 `src/lib.rs`。创建后打开这个文件 |
| Package name | 写进 `Cargo.toml` 的 `name`，也是程序的名字。默认是小写的项目名；只能用字母、数字、`-` 和 `_`，以字母开头，项目名里有空格时需要改掉 |
| Edition（高级） | 2024（默认）或 2021。Edition 2024 需要 Rust 1.85 或更新版本 |

`Cargo.toml` 由模板直接写出，不运行 `cargo new` 或 `cargo init`，所以创建项目不需要联网，也不需要先装好 cargo。
生成的 `.gitignore` 忽略 `/target`。这些字段目前只有英文名称：插件语言文件还不能翻译新建项目的字段。

## 依赖的程序

| 程序 | 用途 | 查找方式 |
| --- | --- | --- |
| `cargo` | 读取软件包、构建、检查、运行、依赖、调试前构建 | 「工具链」选出的 Cargo，最后找 `PATH` 里的 `cargo` |
| `rustup` | 选项目的工具链、启动和安装 rust-analyzer | 「工具链」选出的 rustup，最后找 `PATH` 里的 `rustup` |
| `rust-analyzer` | 代码智能 | 见上文“rust-analyzer” |
| `codelldb`（CodeLLDB 1.12.3） | 调试 | 先找在 Lithe 里安装时记下的目录，再找 `PATH` 里的 `codelldb` |

安装插件时 Lithe 会列出这些程序和插件需要的权限（启动插件后端、启动语言服务器、运行构建命令、启动调试器、
创建项目、读取工作区元数据、安装插件所需的命令行工具），确认后才会安装。cargo 和 rustup 需要自己安装，
推荐用 [rustup](https://rustup.rs)。

## 安装

1. 从 [Releases](https://github.com/Lithe-IDEA/lithe-plugin-rust/releases) 下载对应平台的包：macOS 用 `rust-support-<版本>-darwin.zip`，
   Windows x64 用 `rust-support-<版本>-windows-x64.zip`，Windows ARM64 用 `rust-support-<版本>-windows-arm64.zip`。
2. 在 Lithe 中打开 **设置 → 插件**，点击页面标题栏右侧的 **从文件安装…**，选中这个 zip，核对 SHA-256 和权限后确认。
3. 打开含 `Cargo.toml` 的项目。构建窗口会出现 Cargo，打开 `.rs` 文件时启动 rust-analyzer。

darwin 包同时支持 Apple Silicon 和 Intel Mac。三个包只有后端程序不同：macOS 是通用二进制 `bin/backend`，
Windows 是对应架构的 `bin/backend.exe`。

## 工作方式

- 插件后端是一个用 [Rust 插件 SDK](https://github.com/Lithe-IDEA/Lithe-core-rust/tree/main/shared/plugins/sdk/rust)
  写的程序，Lithe 按两种身份分别启动它：插件后端回答 `rustAnalyzer.launch`；Cargo 构建提供者回答
  `lithe/build/model`、`dependencies`、`execute`、`prepareRun`、`prepareDebug` 和 `runEntrypoints`。
- 后端不保存状态，也不写文件：每次请求都重新问 Cargo 或 rustup。Cargo 只写项目的 target 目录，rustup 只写它自己的工具链。
- 运行由后端给出 `cargo run` 的命令行，Lithe 运行它；调试由后端先构建，再把程序路径交给 Lithe，Lithe 按
  `debugAdapters` 的声明启动 CodeLLDB（`stdio` 传输）。
- rust-analyzer 由 Lithe 按 `languageServers` 的声明启动，命令行由后端给出；插件设置通过 `workspaceConfiguration`
  里的 `${setting:<id>}` 传给它。
- CodeLLDB 由 Lithe 按 `commandInstallers` 的 `download` 声明下载、校验和解压到插件状态目录。
- 新建项目由 `projectGenerators` 声明，Lithe 把 `templates/rust/` 里的模板展开到新目录；模板名以 `dot-` 开头的文件生成为点文件。
- 插件目录在运行时保持只读。

## 诊断日志

后端用 Rust SDK 接入 Lithe 的日志协议，SDK 自动记录每次请求的成功或失败；编辑器每次打开 Rust 文件都会问运行入口，
这一项的成功不记录。插件只在真的降级时上报一次 WARN：

| 类别 | 什么时候 |
| --- | --- |
| `cargo.membersOutsideFolder` | 工作区有成员在打开的文件夹之外，模型里不列出它们 |
| `cargo.dependencyTreeTruncated` | 依赖树超过 Core 的上限，后面的依赖没有列出 |
| `cargo.outputTruncated` | 构建任务的输出超过 Core 保留的 32 MiB，最终结果被截断（构建输出窗口里仍是完整的） |
| `rust.noWorkspaceFolder` | 启动 rust-analyzer 时 Lithe 没有给出工作区文件夹，按 rustup 的默认工具链启动 |
| `rust.rustSrcMissing` | rust-analyzer 装上了，但 rust-src 没装上，标准库的跳转和补全不可用 |

rust-analyzer 和 CodeLLDB 由 Lithe 启动，它们的启动结果由 Lithe 记在插件的日志通道里；rust-analyzer 的
`window/logMessage` 和有界的 stderr 进语言服务日志，CodeLLDB 和被调试程序的输出只显示在调试控制台里。
cargo 的输出显示在构建输出里，不复制进诊断日志。

## 本地打包

需要 Rust 工具链和对应的 target（`rustup target add`）：macOS 上构建 darwin（Apple Silicon 和 Intel 合成一个通用二进制并做
ad-hoc 签名），Windows 上构建 windows-x64 和 windows-arm64。再用 Node.js 22+ 和 `zip`（macOS 或 Linux）打包：

```bash
(cd backend && cargo test --locked)            # 后端单元测试
node scripts/build-backend.mjs darwin          # 写到 .artifacts/backend-darwin/
node scripts/package.mjs --platforms darwin    # CI 和发布时三个平台都打
node --test scripts/package/*.test.mjs         # 核对 dist/ 里的每个包
```

输出 `dist/rust-support-<版本>-<平台>.zip` 和它们的 SHA-256。每个包带 `bin/backend`（Windows 是 `bin/backend.exe`，
包里的 `plugin.json` 相应改写启动路径）和列出后端所用 crate 许可证的 `THIRD_PARTY_LICENSES.txt`，不带 CodeLLDB。
本地开发时，用 **设置 → 插件 → 从文件安装…** 选择这个 zip 安装。

`plugin.json` 的完整格式见 Rust Core 仓库里的
[插件契约](https://github.com/Lithe-IDEA/Lithe-core-rust/blob/main/shared/plugins/contract/v1/README.md)、
[构建工具接入指南](https://github.com/Lithe-IDEA/Lithe-core-rust/blob/main/shared/plugins/guides/build-tool-integration.md)
和[语言服务器插件生命周期](https://github.com/Lithe-IDEA/Lithe-core-rust/blob/main/shared/plugins/guides/lsp-plugin-lifecycle.md)。

## 宿主端到端测试

`e2e/host/` 里的测试通过 Lithe 桌面端自己的测试工具（宿主的 `desktop/internal/workbench` 和 `desktop/internal/projectgen` 包）
安装真实插件包，像用户一样操作构建窗口、编辑器和调试窗口。主仓库不读取本仓库，`e2e/host/lithe-host.json`
记录测试最后一次通过时的宿主版本。

需要 Node.js 22+、Go、Rust 工具链（构建宿主固定的 Rust Core）、`git`，以及测试要用的 `cargo`、`rustup` 和
rust-analyzer 组件（`rustup component add rust-analyzer`；测试不会替你安装）。调试测试会联网下载 CodeLLDB：

```bash
node scripts/build-backend.mjs darwin
node scripts/package.mjs --platforms darwin
LITHE_TEST_RUST_PLUGIN_ARCHIVE="$PWD/dist/rust-support-0.1.0-darwin.zip" node scripts/e2e-host.mjs
```

插件包路径要写绝对路径。脚本把固定版本的宿主（只取 `desktop/` 和 `scripts/`）检出到 `.artifacts/lithe-host`，
复制 `e2e/host/desktop/` 下的测试，再在那里运行宿主的 `scripts/desktop/desktop-go.sh test`；第一次运行会克隆并构建
Rust Core，`LITHE_CORE_RUST_DIR` 可以指向本地的 Core 检出。`--source <本地 Lithe 检出>` 从本地仓库检出宿主，
`--ref <分支或 commit>` 换一个宿主版本试跑，`--run <正则>` 只跑部分测试。测试用的是宿主内部的测试工具，
升级 `lithe-host.json` 的 `revision` 时可能要同步修改测试。

## 功能矩阵

插件各项能力在 macOS 和 Windows 上的实现与验收状态见 [docs/feature-matrix.md](docs/feature-matrix.md)。
Lithe 主仓库的功能矩阵不收录插件能力，改动插件行为时在同一个 PR 里更新这一页。

## 发布新版本

1. 同时修改 `io.github.lithe-idea.rust-support/plugin.json` 和 `backend/Cargo.toml` 的 `version`：修复升修订号（0.1.0 → 0.1.1），
   加功能升次版本号（0.1.0 → 0.2.0）；只有特别大的革新才升主版本号，由维护者决定。新功能要求还没有发布的
   宿主能力或 Core 版本时，照样升次版本号，并作为预发布版本发布（见第 3 步）。合入 `main`。
2. 推送同名标签：

   ```bash
   git tag v0.1.0 && git push origin v0.1.0
   ```

3. `Release` 工作流会核对标签与版本一致、打包并发布 Release（说明里附 SHA-256 和
   `docs/releases/v<版本>.md`）。`release.yml` 里 `PRERELEASE_TAGS` 列出的版本会标为预发布版本：
   目前是依赖还没发布的下载型命令安装器（1lck/Lithe-IDEA#2241）和 rustup 链接修复（1lck/Lithe-IDEA#2242）的 `v0.1.0`。
4. Lithe Store 的自动追踪会发现稳定版本并开合并请求，合并后用户即可在 Lithe 中更新。自动追踪会跳过预发布版本；
   需要收录时在 Store 中手工提交条目，并使用已发布 ZIP 的实际 SHA-256。

## 许可

[Apache-2.0](LICENSE)。CodeLLDB 以 MIT 许可发布，由 Lithe 从它的发布页下载，不随插件分发。
