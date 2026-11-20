# 功能矩阵

本页记录 Rust Support 插件自己的跨平台功能状态。Lithe 主仓库的功能矩阵（`shared/platform-feature-matrix/`）
只记录宿主能力，不收录插件能力（[1lck/Lithe-IDEA#1915](https://github.com/1lck/Lithe-IDEA/pull/1915)）。
插件能力的状态、证据和验证方法在这里维护，改动插件行为的 PR 同时更新本页。

## 状态取值

沿用主仓库功能矩阵的取值：

| 字段 | 取值 |
| --- | --- |
| 实现 | `implemented` 已实现；`partial` 部分实现；`missing` 未实现；`not-applicable` 该平台不适用 |
| 验证 | `verified` 已按“验证方法”完整验收；`pending` 尚未完整验收；`not-applicable` 无需验收 |

只有按“验证方法”完整走过一遍，并写明日期、插件版本和宿主版本，才能标 `verified`。
自动化测试的结果写在“最近验证”里，不能代替验收。

## 总览

| ID | 能力 | macOS | Windows | 最近验证 |
| --- | --- | --- | --- | --- |
| `rust-support-plugin` | rust-analyzer 代码智能，Cargo 构建、运行和依赖，cargo / rustup / rust-analyzer 工具定义 | implemented / pending | implemented / pending | 2026-11-20 macOS workbench 集成测试通过；Windows 后端只在 CI 构建，没有在 Windows 版 Lithe 里运行过 |
| `rust-debug-codelldb` | 用按需下载的 CodeLLDB 调试二进制和示例 | implemented / pending | partial / pending | 2026-11-21 macOS workbench 集成测试通过；Windows 没有运行过；Windows ARM64 没有 CodeLLDB 包 |
| `rust-project-generator` | 「新建项目」里的 Rust（Cargo 软件包），Cargo 选择 | implemented / pending | implemented / pending | 2026-11-20 macOS 宿主集成测试通过；Windows 没有运行过 |

## rust-support-plugin

**能力**：用 rustup 工具链里的 `rust-analyzer` 提供代码智能，启动前由插件后端按项目目录选出工具链（`rustAnalyzer.launch`），
工具链缺少 rust-analyzer 时可以先用 `rustup component add` 装上；用 Cargo 构建提供者识别含 `Cargo.toml` 的最外层目录，
`cargo metadata --no-deps` 组成软件包和目标；在构建窗口里运行 Build、Check、Clippy、Doc、Clean，运行二进制和示例，
`cargo metadata` 的依赖图按 `cargo tree` 的样子列出；`Cargo.toml`、`Cargo.lock`、`.cargo/config(.toml)`、
`rust-toolchain(.toml)` 变化后重新读取；构建选项 Release Profile、Features、All Features、No Default Features、Offline；
插件设置 Check on save、Check command、Install rust-analyzer with rustup。`toolDefinitions` 声明 `cargo`、`rustup`、
`rust-analyzer`，在 `~/.cargo`（`CARGO_HOME`）和 Homebrew 里查找，列在「构建、执行、部署 › 工具链」里，项目可以固定一个 Cargo。

| 平台 | 实现 | 验证 | 说明 |
| --- | --- | --- | --- |
| macOS | implemented | pending | 发布包的后端是 Apple Silicon 和 Intel 的通用二进制 |
| Windows | implemented | pending | `-windows-x64.zip` 和 `-windows-arm64.zip` 只有后端程序不同。rustup 在 Windows 上的代理是硬链接，cargo 的解析不依赖 Lithe-core-rust#135。见下文“Windows 安装包” |

**证据**：

- `io.github.lithe-idea.rust-support/plugin.json`：`languages`、`languageServers`（rust-analyzer 的 `backendResolved` 启动）、
  `buildSystems` 里的 `cargo`（`provider` 指向插件后端，`options`、`run`、`watch`）、`settings`、`toolDefinitions`。
- `backend/src/`：`analyzer.rs`（rust-analyzer 启动方式）、`metadata.rs`、`model.rs`、`dependencies.rs`、`execute.rs`、
  `launch.rs`，以及 `cargo test --locked` 的单元测试。
- `e2e/host/desktop/internal/projectgen/rust_integration_test.go`：`TestRustPluginToolDefinitionFindsTheLocalCargo`
  确认工具定义找到 `PATH` 里那个 cargo 和它的版本。
- `e2e/host/desktop/internal/workbench/rust_support_plugin_e2e_test.go`：`TestRustSupportPluginBuildsAndServesAPackage`。

**依赖的宿主能力**：`languageServers`（`backendResolved` 启动，宿主在每次启动前问插件后端）、`buildProviders` 1
（Cargo 构建提供者的模型、依赖、执行、运行、调试准备和运行入口）、`settings`、`toolDefinitions` 1、`backend`。
rustup 安装的 cargo 是指向 `rustup` 的符号链接，按启动时的名字决定运行哪个工具；macOS 和 Linux 上，工具定义要找到它，
需要固定了 Lithe-core-rust#135 的桌面端（1lck/Lithe-IDEA#2242），之前的 Core 把它解析成 `rustup`，版本探针失败。

**限制**：

- 不提供测试：构建窗口里没有测试树，也不能单独运行一个测试；`cargo test` 可以在终端里运行。
- 只读取打开的文件夹里的软件包；工作区成员在文件夹之外时不列出，记一条 WARN（`cargo.membersOutsideFolder`）。
- 依赖树超过 Core 的上限时截断（`cargo.dependencyTreeTruncated`）。
- 和 `lithe-plugin-cargo` 都声明了 `cargo` 构建系统，不要同时启用。
- 构建选项、插件设置的名称和说明有中文；工具定义、调试适配器和新建项目字段的名称插件语言文件还不能翻译。

**最近验证**：2026-11-20，macOS（Apple Silicon），插件 0.1.0 本地包，宿主 `035f3c5`，Core 为 Lithe-core-rust#135 的分支提交 `8291f3c`，rustup 默认工具链 1.100.0。
`TestRustSupportPluginBuildsAndServesAPackage` 通过：构建窗口的模型列出软件包 `hello`，Build 在 `target/debug` 留下程序，
运行二进制和示例输出正确；Features 选项列出 `hello/loud`，保存后运行带上 `--features hello/loud`；
rust-analyzer 经 `rustAnalyzer.launch` 从 rustup 默认工具链启动，对不存在的函数报出诊断。没有做下面的手工验收。

**验证方法**（macOS）：

1. 用「设置 → 插件 → 从文件安装…」安装 `dist/rust-support-<版本>-darwin.zip`，核对权限后确认，启用后不需要重启。
2. 打开含 `Cargo.toml` 的项目：构建窗口出现 Cargo，列出软件包和二进制、示例、库目标；二进制和示例有运行按钮。
3. 运行 Build、Check、Clippy、Doc、Clean（工作区、单个软件包、单个目标），输出显示在构建输出里；
   `文件:行:列: error: 消息` 形式的编译错误显示为问题。
4. 运行二进制和示例，程序输出显示在 Run 窗口里；在构建选项里勾选 Release Profile、选一个 Feature 后再运行，命令里带上
   `--release` 和 `--features`。
5. 打开 `.rs` 文件：rust-analyzer 来自项目的 rustup 工具链，补全、跳转定义、悬停、诊断和格式化可用；保存时运行
   `cargo check`，把 Check command 改成 clippy 后保存，诊断里出现 Clippy 的 lint，不需要重启语言服务器。
6. 在一个缺少 rust-analyzer 组件的工具链上打开项目（例如用 `rust-toolchain.toml` 指定另一个工具链）：Lithe 先运行
   `rustup component add rust-analyzer rust-src` 再启动；关闭 Install rust-analyzer with rustup 后重新打开，显示安装提示。
7. 查看依赖列表：层次和 `cargo tree` 一致，重复出现的软件包不再展开；修改 `Cargo.toml` 后模型重新读取。
8. 「构建、执行、部署 › 工具链」列出 cargo、rustup、rust-analyzer 和它们的版本；为项目固定另一个 Cargo 后，
   构建输出里的 cargo 随之改变。
9. 禁用插件后 rust-analyzer 和后端退出；插件安装目录在整个过程中不被写入。

## rust-debug-codelldb

**能力**：「调试」窗口和构建窗口里二进制、示例的「调试」：插件后端先用 `cargo build`（带上构建选项）构建目标，
再把程序路径、参数、环境和工作目录交给 Lithe，Lithe 用 stdio 启动 CodeLLDB（`sourceLanguages: ["rust"]`）。
支持断点、单步、调用栈、变量，程序输出显示在调试控制台，程序结束后会话自动结束。
CodeLLDB 不在插件包里：缺少 `codelldb` 时，插件详情的调试能力卡片显示安装按钮，Lithe 按 `commandInstallers` 的
`download` 声明下载 CodeLLDB 1.12.3 的 VSIX，核对写死的 SHA-256，解压到插件状态目录，之后从
`extension/adapter/codelldb` 启动；也可以用 `PATH` 里的 `codelldb`。

| 平台 | 实现 | 验证 | 说明 |
| --- | --- | --- | --- |
| macOS | implemented | pending | Apple Silicon 和 Intel 各有 VSIX。需要支持下载型命令安装器的桌面端（1lck/Lithe-IDEA#2241，`commandInstallers` 1），桌面端 0.8.11 及更早版本拒绝安装这个插件 |
| Windows | partial | pending | x64 下载 `codelldb-win32-x64.vsix`。ARM64：CodeLLDB 1.12.3 没有发布 Windows ARM64 的包，Core 不为这个平台提供安装，调试不可用，其他功能不受影响。见下文“Windows 安装包” |

**证据**：

- `io.github.lithe-idea.rust-support/plugin.json`：`debugAdapters` 里的 `codelldb`，`commandInstallers` 里的 `codelldb`
  （五个平台的 VSIX 地址和 SHA-256），`debug.launch`、`commands.install` 权限和 `onDebug:codelldb` 激活事件。
- `backend/src/launch.rs`：`lithe/build/prepareDebug` 先构建再返回程序。
- `e2e/host/desktop/internal/workbench/rust_support_plugin_e2e_test.go`：`TestRustSupportPluginDownloadsCodeLLDBAndDebugsAProgram`。

**依赖的宿主能力**：`debugAdapters`（stdio 传输、`targetBindings`）、`buildProviders` 1 的 `prepareDebug`、
`commandInstallers` 1（Lithe-core-rust#133，宿主 1lck/Lithe-IDEA#2241 下载、校验、解压到插件状态目录）。

**限制**：只支持 launch，不支持 attach，也不能调试测试。从编辑器里的运行入口只能运行，不能调试。
从经过符号链接的路径打开的项目里断点不会停下：rustc 记录解析后的真实路径，CodeLLDB 按完整路径匹配断点，
插件契约现在没有把这种路径对应关系交给 launch 的绑定（`target.pathMappings` 只用于 attach）。
适配器名称 Rust (CodeLLDB) 目前不能翻译：插件语言文件还不覆盖调试适配器。

**最近验证**：2026-11-21，macOS（Apple Silicon），插件 0.1.0 本地包，宿主 `035f3c5`，Core 为 Lithe-core-rust#135 的分支提交 `8291f3c`：
`TestRustSupportPluginDownloadsCodeLLDBAndDebugsAProgram` 通过（582 秒，大部分是下载）。安装前「功能」只缺 `codelldb`，
安装器给出固定的 CodeLLDB 1.12.3 地址和 SHA-256，下载、校验后装进插件状态目录，不在插件包里，装好后不再缺少；
在 `main` 里的一行设断点，构建窗口的「调试」先构建，程序停在 `src/main.rs:3`，调用栈顶是 `main`，变量里有局部变量 `name`；
「继续」后程序结束，调试控制台有程序的输出；插件包的文件和哈希不变。测试从解析掉符号链接的临时目录打开软件包（见“限制”）。
这是自动化测试，不能代替上面的手工验收。

**验证方法**（macOS）：

1. 安装插件包，确认安装对话框列出启动调试器和安装命令行工具的权限；`PATH` 里没有 `codelldb`。
2. 插件详情「功能」页的调试器卡片显示“未找到 codelldb”和安装按钮；点击后对话框列出下载地址和 SHA-256，
   装好后卡片不再提示，插件安装目录没有新增文件。
3. 打开一个二进制软件包，在 `main` 里的一行上设断点，在构建窗口里点这个二进制的「调试」：先构建，程序停在断点，
   调用栈顶是 `main` 和断点所在行，变量里能看到局部变量的值，按 Rust 的类型显示；单步可用。
4. 点「继续」：程序运行到结束，输出显示在调试控制台，会话自动结束。
5. 勾选 Release Profile 后调试，构建的是 release 程序；调试一个示例。
6. 整个过程中插件安装目录不被写入。

## rust-project-generator

**能力**：「新建项目」对话框里的 Rust（Cargo）。Cargo 下拉列出 `cargo` 工具定义在电脑上找到的 Cargo，默认是版本最新的一个，
创建后记为项目固定（`.lithe/toolchains/local.json`）；没有找到时留空，不影响创建。可选 Binary（`src/main.rs`）或
Library（带一个测试的 `src/lib.rs`），Package name 默认是小写的项目名，只能用字母、数字、`-` 和 `_` 并以字母开头；
Edition 2024（默认）或 2021。由模板写出 `Cargo.toml` 和忽略 `/target` 的 `.gitignore`，不运行 `cargo new`，不需要联网。

| 平台 | 实现 | 验证 | 说明 |
| --- | --- | --- | --- |
| macOS | implemented | pending | — |
| Windows | implemented | pending | 模板与平台无关；还没有在 Windows 版 Lithe 里跑过 |

**证据**：

- `io.github.lithe-idea.rust-support/plugin.json`：`projectGenerators` 里的 `cargo-package`；`templates/rust/`。
- `e2e/host/desktop/internal/projectgen/rust_integration_test.go`：`TestRustPluginGeneratesABinaryPackageThatRuns`、
  `TestRustPluginGeneratesALibraryWithATest`、`TestRustPluginFormRejectsAPackageNameWithSpaces`。

**依赖的宿主能力**：`projectGenerators`（Core 的表单、计划和宿主的模板展开）、`toolDefinitions` 1（Cargo 字段的安装列表和项目固定）。

**限制**：字段名称只有英文，插件语言文件还不能翻译新建项目的字段；不生成工作区（多个软件包）。

**最近验证**：2026-11-20，macOS（Apple Silicon），插件 0.1.0 本地包，宿主 `035f3c5`，Core 为 Lithe-core-rust#135 的分支提交 `8291f3c`。
四个 projectgen 测试全部通过：生成的二进制软件包能 `cargo run` 并打印 `Hello, Rustacean!`，库软件包的 `cargo test` 通过，
项目固定的 Cargo 来自工具定义找到的安装，带空格的软件包名被表单拒绝。没有做下面的手工验收。

**验证方法**（macOS）：

1. 安装插件包，打开「新建项目」：左侧有 Rust，Cargo 下拉列出电脑上的 Cargo。
2. 用 Binary 创建：生成 `Cargo.toml`、`.gitignore`、`src/main.rs`，打开 `src/main.rs`；构建窗口出现 Cargo，运行后打印 `Hello, Rustacean!`。
3. 用 Library 创建：生成 `src/lib.rs`，`cargo test` 通过。
4. 项目名含空格时 Package name 显示错误、不能创建；改成合法名字后可以创建。
5. 选 Edition 2021 创建，`Cargo.toml` 里是 `edition = "2021"`。
6. 「工具链」里这个项目固定的是创建时选的 Cargo。

## Windows 安装包

`-windows-x64.zip` 和 `-windows-arm64.zip` 与 darwin 包只有后端程序不同（`bin/backend.exe`，包里的 `plugin.json`
相应改写启动路径）。CI 在 Windows x64 和 ARM64 上构建后端并跑单元测试，没有在 Windows 版 Lithe 里安装和运行过插件。
Windows 上 rustup 的代理是硬链接，cargo、rust-analyzer 的版本探针不受上面的 Core 修复影响。
