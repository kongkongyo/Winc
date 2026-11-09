//! 编译时嵌入的语言包。
//!
//! 每种语言由一组 `.ftl` 文件组成，文件按界面领域划分，文件名见 [`FILES`]。

use crate::Locale;

/// 列出各领域的文件名，并为每种语言嵌入同名的一组文件。
macro_rules! catalog {
    ($($file:literal),+ $(,)?) => {
        /// 各领域的文件名（不含扩展名），每种语言的目录下都有这一组文件。
        pub(crate) const FILES: &[&str] = &[$($file),+];
        /// 简体中文的各文件内容，顺序与 [`FILES`] 一致。
        const ZH_CN: &[&str] = &[$(include_str!(concat!("../locales/zh-CN/", $file, ".ftl"))),+];
        /// 英文的各文件内容，顺序与 [`FILES`] 一致。
        const EN_US: &[&str] = &[$(include_str!(concat!("../locales/en-US/", $file, ".ftl"))),+];
    };
}

catalog![
    "common",
    "commands",
    "editor",
    "workspace",
    "git",
    "ai",
    "terminal",
    "preview",
    "settings",
    "config",
    "format",
    "menu",
];

/// 返回 `locale` 的各文件名及其内容。
pub(crate) fn sources(locale: Locale) -> impl Iterator<Item = (&'static str, &'static str)> {
    let sources = match locale {
        Locale::ZhCn => ZH_CN,
        Locale::EnUs => EN_US,
    };
    FILES.iter().copied().zip(sources.iter().copied())
}
