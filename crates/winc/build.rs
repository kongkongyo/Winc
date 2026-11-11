//! 构建脚本：把应用图标嵌入 Windows 的 `winc.exe`，文件管理器、文件关联和任务栏都从这里取图标。

#[path = "build_icon.rs"]
mod build_icon;

use std::env::var;
use std::path::Path;

/// 图标组的资源编号，运行时按此编号加载窗口图标（见 `App::resumed`）。
const ICON_GROUP: u16 = 1;
/// 应用图标的路径，相对于本 crate 的目录。
const ICO: &str = "../winc-app/assets/winc.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_icon.rs");
    println!("cargo:rerun-if-changed={ICO}");

    let msvc = var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows")
        && var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !msvc {
        return;
    }
    let ico = std::fs::read(ICO).expect("读取应用图标");
    let res = build_icon::icon_resources(&ico, ICON_GROUP).expect("解析应用图标");
    let out = Path::new(&var("OUT_DIR").expect("Cargo 总是设置 OUT_DIR")).join("winc.res");
    std::fs::write(&out, res).expect("写入资源文件");
    println!("cargo:rustc-link-arg-bin=winc={}", out.display());
}
