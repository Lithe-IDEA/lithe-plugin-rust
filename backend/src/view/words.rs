//! The words the previews show, in the languages the plugin translates its
//! settings to. Setting names match the titles of the declared rows under the
//! region (`plugin.json` and `l10n/zh-Hans.json`), and the toolchain layers
//! match the host's Toolchains page, so a card names what changes it.
//!
//! Rust code and the tools' own output, such as diagnostics, stay as the
//! tools write them.

pub struct Words {
    pub preview_title: &'static str,
    pub preview_label: &'static str,
    pub follows_options: &'static str,
    pub includes_draft: &'static str,
    pub on: &'static str,
    pub off: &'static str,

    pub toolchain_title: &'static str,
    pub toolchain_info: &'static str,

    pub macro_title: &'static str,
    pub macro_on: &'static str,
    pub macro_off: &'static str,
    /// The caption on the generated code, and in its place when it is not.
    pub macro_expanded: &'static str,
    pub macro_not_expanded: &'static str,

    pub build_title: &'static str,
    pub build_on: &'static str,
    pub build_off: &'static str,
    /// Build scripts switched off while procedural macros still run them.
    pub build_implied: &'static str,

    pub doc_title: &'static str,
    pub doc_on: &'static str,
    pub doc_off: &'static str,

    pub check_title: &'static str,
    pub check_off: &'static str,
    pub check_check: &'static str,
    pub check_clippy: &'static str,

    pub resolution_title: &'static str,
    pub resolution_label: &'static str,
    pub step_project: &'static str,
    pub step_project_detail: [&'static str; 2],
    pub step_global: &'static str,
    pub step_global_detail: [&'static str; 2],
    pub step_automatic: &'static str,
    pub step_automatic_detail: [&'static str; 2],
    pub rustup_title: &'static str,
    pub rustup_detail: [&'static str; 2],
    pub first: &'static str,
    pub then: &'static str,
    pub last: &'static str,
    pub used_for: &'static str,
    pub uses: [&'static str; 5],
}

const ZH: Words = Words {
    preview_title: "效果预览",
    preview_label: "rust-analyzer 选项的效果预览",
    follows_options: "示例代码，随下方选项即时变化",
    includes_draft: "含未应用的修改",
    on: "开",
    off: "关",

    toolchain_title: "工具链",
    toolchain_info:
        "Lithe 用这些程序读取项目、构建和提供语言服务。点击一行，到「工具链」页选择其他版本或位置。",

    macro_title: "展开过程宏",
    macro_on: "展开 derive 和属性宏，生成的代码参与补全和跳转",
    macro_off: "不展开，过程宏生成的代码不可见",
    macro_expanded: "derive 生成",
    macro_not_expanded: "未展开",

    build_title: "运行构建脚本",
    build_on: "运行 build.rs，OUT_DIR 里生成的代码可以分析",
    build_off: "不运行 build.rs，OUT_DIR 里的代码无法解析",
    build_implied: "已关闭，但「展开过程宏」开启，仍会运行",

    doc_title: "高亮文档注释中的代码",
    doc_on: "文档示例里的 Rust 代码和链接按源码着色",
    doc_off: "文档注释整段显示为注释颜色",

    check_title: "保存时检查",
    check_off: "保存时不检查，只显示 rust-analyzer 的诊断",
    check_check: "保存时运行 cargo check，报告编译器警告",
    check_clippy: "保存时运行 cargo clippy，另外报告 lint",

    resolution_title: "Lithe 按这个顺序找 cargo",
    resolution_label: "Lithe 查找 cargo 程序的顺序",
    step_project: "项目环境",
    step_project_detail: [
        "为项目固定的 cargo，其次项目选的版本",
        "在「工具链」页的项目范围选择",
    ],
    step_global: "全局默认",
    step_global_detail: [
        "全局固定的 cargo，其次默认版本",
        "在「工具链」页的全局范围选择",
    ],
    step_automatic: "自动发现",
    step_automatic_detail: [
        "CARGO_HOME/bin → PATH 中的 cargo",
        "→ ~/.cargo、Homebrew 的安装",
    ],
    rustup_title: "rustup 选 Rust 版本",
    rustup_detail: [
        "cargo 是 rustup 的代理时，按目录覆盖",
        "→ rust-toolchain.toml → 默认工具链",
    ],
    first: "先找这里",
    then: "其次",
    last: "最后",
    used_for: "用于",
    uses: ["读取项目", "构建", "检查", "运行", "调试"],
};

const EN: Words = Words {
    preview_title: "Preview",
    preview_label: "Preview of the rust-analyzer options",
    follows_options: "Sample code; follows the options below",
    includes_draft: "Includes changes not applied yet",
    on: "On",
    off: "Off",

    toolchain_title: "Toolchain",
    toolchain_info: "Lithe runs these programs to read the project, build it and analyze the code. Choose a row to pick another version or location on the Toolchains page.",

    macro_title: "Expand procedural macros",
    macro_on: "Generated code feeds completion and navigation",
    macro_off: "Not expanded; the generated code is unseen",
    macro_expanded: "from derive",
    macro_not_expanded: "Not expanded",

    build_title: "Run build scripts",
    build_on: "Runs build.rs, so code in OUT_DIR resolves",
    build_off: "build.rs does not run; OUT_DIR is unset",
    build_implied: "Off, but procedural macros still run them",

    doc_title: "Highlight code in doc comments",
    doc_on: "Code and links in docs look like source",
    doc_off: "Doc comments take one comment color",

    check_title: "Check on save",
    check_off: "Only rust-analyzer's own diagnostics",
    check_check: "Runs cargo check on save: compiler warnings",
    check_clippy: "Runs cargo clippy on save: also its lints",

    resolution_title: "How Lithe finds cargo",
    resolution_label: "The order in which Lithe looks for the cargo program",
    step_project: "Project",
    step_project_detail: ["Pinned for the project, then its version", "Set on the Toolchains page, for the project"],
    step_global: "Global default",
    step_global_detail: ["Pinned globally, then the default version", "Set on the Toolchains page, for all"],
    step_automatic: "Automatic",
    step_automatic_detail: ["CARGO_HOME/bin → cargo on PATH", "→ ~/.cargo and Homebrew installs"],
    rustup_title: "rustup picks the Rust version",
    rustup_detail: ["When cargo is rustup's proxy: override", "→ rust-toolchain.toml → default toolchain"],
    first: "Tried first",
    then: "Then",
    last: "Tried last",
    used_for: "Used to",
    uses: ["read projects", "build", "check", "run", "debug"],
};

/// The words for the host's interface language: Chinese for a `zh` tag,
/// English otherwise.
pub fn for_language(tag: &str) -> &'static Words {
    if tag == "zh" || tag.starts_with("zh-") {
        &ZH
    } else {
        &EN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chinese_tag_of_any_script_or_region_gets_chinese() {
        for tag in ["zh", "zh-Hans", "zh-CN", "zh-Hant-TW"] {
            assert_eq!(for_language(tag).on, ZH.on, "{tag}");
        }
        for tag in ["en", "en-GB", "fr", "zhx", ""] {
            assert_eq!(for_language(tag).on, EN.on, "{tag}");
        }
    }
}
