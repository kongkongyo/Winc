//! 把 ICO 文件转换成 Windows 资源文件（`.res`）：链接器直接接受这种格式，不需要资源编译器。

/// 资源类型：单个图标。
const RT_ICON: u16 = 3;
/// 资源类型：图标组，列出同一个图标的各个尺寸。
const RT_GROUP_ICON: u16 = 14;
/// 资源的内存标志：可移动、用时才加载、可丢弃。
const MEMORY_FLAGS: u16 = 0x1030;
/// 资源的语言：与语言无关。
const NEUTRAL_LANGUAGE: u16 = 0;
/// ICO 文件头的长度。
const ICO_HEADER: usize = 6;
/// ICO 目录项的长度。
const ICO_ENTRY: usize = 16;

/// 把 ICO 文件的内容 `ico` 转换成 `.res` 文件的内容；图标组的编号是 `group`。
///
/// 各尺寸的编号从 1 起。文件头不合法或目录项超出文件范围时返回 `None`。
pub fn icon_resources(ico: &[u8], group: u16) -> Option<Vec<u8>> {
    let word = |at: usize| Some(u16::from_le_bytes(ico.get(at..at + 2)?.try_into().ok()?));
    let long = |at: usize| Some(u32::from_le_bytes(ico.get(at..at + 4)?.try_into().ok()?));
    if word(0)? != 0 || word(2)? != 1 {
        return None;
    }
    let count = word(4)?;

    // `.res` 文件以一条类型和名称都是 0 的空记录开头，用来和 16 位格式区分。
    let mut res = Vec::new();
    push_resource(&mut res, 0, 0, 0, 0, &[]);

    let mut directory = vec![0, 0, 1, 0];
    directory.extend(count.to_le_bytes());
    for id in 1..=count {
        let entry = ICO_HEADER + ICO_ENTRY * usize::from(id - 1);
        let size = long(entry + 8)?;
        let offset = long(entry + 12)? as usize;
        let image = ico.get(offset..offset.checked_add(size as usize)?)?;
        push_resource(&mut res, RT_ICON, id, MEMORY_FLAGS, NEUTRAL_LANGUAGE, image);
        // 前 12 字节（尺寸、颜色数、位面、位深、数据长度）两种目录相同，最后的文件偏移换成编号。
        directory.extend(ico.get(entry..entry + 12)?);
        directory.extend(id.to_le_bytes());
    }
    push_resource(
        &mut res,
        RT_GROUP_ICON,
        group,
        MEMORY_FLAGS,
        NEUTRAL_LANGUAGE,
        &directory,
    );
    Some(res)
}

/// 向 `res` 追加一条资源记录；类型和名称都是编号，记录按 4 字节对齐。
fn push_resource(res: &mut Vec<u8>, kind: u16, name: u16, flags: u16, language: u16, data: &[u8]) {
    res.extend((data.len() as u32).to_le_bytes());
    res.extend(32u32.to_le_bytes());
    res.extend([0xff, 0xff]);
    res.extend(kind.to_le_bytes());
    res.extend([0xff, 0xff]);
    res.extend(name.to_le_bytes());
    // 数据版本。
    res.extend(0u32.to_le_bytes());
    res.extend(flags.to_le_bytes());
    res.extend(language.to_le_bytes());
    // 版本和特征。
    res.extend([0; 8]);
    res.extend(data);
    res.resize(res.len().next_multiple_of(4), 0);
}
