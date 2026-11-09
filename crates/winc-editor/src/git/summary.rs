//! AI 提交说明的只读输入：只比较勾选文件的 HEAD 与实际待提交内容。

use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::io::Read;

use winc_core::diff::{LineHunk, line_hunks};
use winc_i18n::t;

use super::*;

/// 单个文件用于差分的读取上限（8 MiB），避免小请求先读入无界的大文件。
const MAX_FILE: usize = 8 << 20;
/// 差分输入最多允许的行数，限制逐行索引的内存。
const MAX_LINES: usize = 100_000;

/// AI 输入及本机内容指纹；指纹只用来判断结果是否过期，不发送给服务。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    /// 可发送给 AI 的文本差异，二进制内容已省略。
    pub diff: String,
    /// 包括二进制内容在内的本机指纹。
    pub fingerprint: u64,
}

impl Git {
    /// 有界读取部分提交所需的 HEAD 基准，使用与差异视图相同的工作区换行形式。
    ///
    /// 先拒绝自定义过滤器、编码转换和 ident，再仅允许 Git 内建的检出转换。
    ///
    /// # Errors
    ///
    /// 转换不受支持、文件过大、Git 失败或请求取消时返回说明。
    pub fn summary_partial_base(
        &self,
        root: &Path,
        path: &Path,
        limit: usize,
        cancel: &Cancel,
    ) -> Result<Option<Vec<u8>>, String> {
        let autocrlf = self.summary_autocrlf(root, cancel)?;
        self.summary_attributes(root, path, autocrlf, cancel)?;
        if self
            .summary_head_content(root, path, limit, cancel)?
            .is_none()
        {
            return Ok(None);
        }
        let relative = checked_path(root, path)?;
        let mut command = self.command(root);
        // 即使仓库已被信任，也不让此只读输入路径运行已配置的仓库过滤器。
        for name in &self.inspect(root)?.filters {
            for key in ["clean", "smudge", "process"] {
                command.arg("-c").arg(format!("filter.{name}.{key}="));
            }
        }
        command
            .args(["cat-file", "--filters", "--end-of-options"])
            .arg(joined_arg(&[OsStr::new("HEAD:"), &relative]));
        let content = self.run(&mut command, SHOW_TIMEOUT, cancel)?;
        if content.len() > limit.min(MAX_FILE) {
            return Err(too_large());
        }
        Ok(Some(content))
    }

    /// 有界读取准备提交的工作区内容，拒绝越界路径和目录中的符号链接。
    ///
    /// # Errors
    ///
    /// 路径不安全、内容过大或读取失败时返回说明。
    pub fn summary_working_content(
        &self,
        root: &Path,
        path: &Path,
    ) -> Result<Option<Vec<u8>>, String> {
        read_working(root, path)
    }

    /// 读取文件在 HEAD 中的原始内容，不执行过滤器，不写入索引或对象库。
    ///
    /// `limit` 是单文件读取字节上限；HEAD 或该文件不存在时返回 `None`。
    ///
    /// # Errors
    ///
    /// 文件超过上限、路径越界、Git 失败或请求被取消时返回说明。
    pub fn summary_head_content(
        &self,
        root: &Path,
        path: &Path,
        limit: usize,
        cancel: &Cancel,
    ) -> Result<Option<Vec<u8>>, String> {
        let relative = checked_path(root, path)?;
        let spec = joined_arg(&[OsStr::new("HEAD:"), &relative]);
        let mut size = self.command(root);
        size.args(["cat-file", "-s", "--end-of-options"])
            .arg(&spec)
            .env("LC_ALL", "C")
            .stderr(Stdio::piped());
        let output =
            process::run(&mut size, Vec::new(), SHOW_TIMEOUT, cancel).map_err(|e| describe(&e))?;
        if !output.status.success() {
            return if output.status.code() == Some(128) && absent(&output.stderr, "HEAD") {
                Ok(None)
            } else {
                Err(failure(&output.stderr))
            };
        }
        let bytes = std::str::from_utf8(&output.stdout)
            .ok()
            .and_then(|n| n.trim().parse::<usize>().ok())
            .ok_or_else(|| t!("git-ai-diff-unreadable").to_owned())?;
        if bytes > limit.min(MAX_FILE) {
            return Err(too_large());
        }
        let mut command = self.command(root);
        command
            .args(["cat-file", "--end-of-options", "blob"])
            .arg(spec);
        let bytes = self.run(&mut command, SHOW_TIMEOUT, cancel)?;
        if bytes.len() > limit.min(MAX_FILE) {
            return Err(too_large());
        }
        Ok(Some(bytes))
    }

    /// 为勾选文件生成 AI 提交说明输入，默认每段差异保留上下各三行。
    ///
    /// 普通文件比较 HEAD 和工作区；`partial` 使用实际部分提交内容。新增文件发送新增全文，
    /// 二进制仅发送路径与类型。此操作不更改文件、索引或对象库，也不执行仓库过滤器。
    ///
    /// # Errors
    ///
    /// 内容超过 `limit`、读取失败、路径越界、特殊操作或不支持的转换时返回明确说明。
    pub fn commit_summary_diff(
        &self,
        root: &Path,
        entries: &[Entry],
        partial: &[Partial],
        limit: usize,
        cancel: &Cancel,
    ) -> Result<CommitSummary, String> {
        if git_dir(root)
            .and_then(|dir| Operation::detect(&dir))
            .is_some()
        {
            return Err(t!("git-ai-diff-operation").to_owned());
        }
        let mut out = Limited::new(limit);
        let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
        let mut seen = std::collections::HashSet::new();
        let autocrlf = self.summary_autocrlf(root, cancel)?;
        let filemode = self
            .config(root, "core.filemode", cancel)
            .is_some_and(|v| v.trim() == "true");
        for entry in entries {
            if cancel.is_cancelled() {
                return Err(describe(&RunError::Cancelled));
            }
            if !seen.insert(entry.path.clone()) {
                continue;
            }
            let relative = checked_path(root, &entry.path)?;
            if entry.is_dir || entry.conflicted() {
                return Err(t!("git-ai-diff-unsupported", path = path_label(&relative)).to_owned());
            }
            let old_path = entry.from.as_ref().unwrap_or(&entry.path);
            let old_relative = checked_path(root, old_path)?;
            let normalize = self.summary_attributes(root, &entry.path, autocrlf, cancel)?;
            let old = self.summary_head_content(root, old_path, MAX_FILE, cancel)?;
            let new = match partial.iter().find(|part| part.path == entry.path) {
                Some(part) if part.content.len() > MAX_FILE => return Err(too_large()),
                Some(part) => Some(part.content.clone()),
                None => read_working(root, &entry.path)?,
            };
            let renamed = entry.from.is_some() && old_path != &entry.path;
            if renamed && std::fs::symlink_metadata(old_path).is_ok() {
                // 旧路径被重新创建时现有 --only 提交也会包含它，不能把它误写为单纯重命名。
                return Err(
                    t!("git-ai-diff-unsupported", path = path_label(&old_relative)).to_owned(),
                );
            }
            let old_mode = if old.is_some() {
                self.summary_head_mode(root, old_path, cancel)?
            } else {
                None
            };
            let new_mode = if new.is_some() {
                working_mode(&entry.path, old_mode.as_deref(), filemode)?
            } else {
                None
            };
            // 新增/删除文件两侧必有一侧为空，不当作 mode 变化输出多余行；只在两个已有 blob 间比较。
            let mode_changed = old_mode.is_some() && new_mode.is_some() && old_mode != new_mode;
            relative.hash(&mut fingerprint);
            old_relative.hash(&mut fingerprint);
            old_mode.hash(&mut fingerprint);
            new_mode.hash(&mut fingerprint);
            old.hash(&mut fingerprint);
            new.hash(&mut fingerprint);
            if old == new && !renamed && !mode_changed {
                continue;
            }
            let is_binary =
                old.as_ref().is_some_and(|v| binary(v)) || new.as_ref().is_some_and(|v| binary(v));
            let (old_text, new_text) = if is_binary {
                (String::new(), String::new())
            } else {
                (
                    text(old.as_deref().unwrap_or_default(), normalize)?,
                    text(new.as_deref().unwrap_or_default(), normalize)?,
                )
            };
            if !is_binary
                && old_text == new_text
                && old.is_some() == new.is_some()
                && !renamed
                && !mode_changed
            {
                continue;
            }
            let old_label = path_label(&old_relative);
            let new_label = path_label(&relative);
            out.push(&format!("diff --git a/{old_label} b/{new_label}\n"))?;
            match (&old, &new) {
                (None, Some(_)) => out.push("new file\n")?,
                (Some(_), None) => out.push("deleted file\n")?,
                _ if renamed => {
                    out.push(&format!("rename from {old_label}\nrename to {new_label}\n"))?;
                }
                _ => out.push("modified file\n")?,
            }
            if mode_changed {
                if let Some(mode) = &old_mode {
                    out.push(&format!("old mode {mode}\n"))?;
                }
                if let Some(mode) = &new_mode {
                    out.push(&format!("new mode {mode}\n"))?;
                }
            }
            if is_binary {
                out.push("Binary file changed (contents omitted)\n\n")?;
                continue;
            }
            let old_name = if old.is_some() {
                format!("a/{old_label}")
            } else {
                "/dev/null".into()
            };
            let new_name = if new.is_some() {
                format!("b/{new_label}")
            } else {
                "/dev/null".into()
            };
            let hunks = line_hunks(&old_text, &new_text);
            if !hunks.is_empty() {
                out.push(&format!("--- {old_name}\n+++ {new_name}\n"))?;
                unified(&mut out, &old_text, &new_text, &hunks)?;
            }
            out.push("\n")?;
        }
        Ok(CommitSummary {
            diff: out.text,
            fingerprint: fingerprint.finish(),
        })
    }

    fn summary_head_mode(
        &self,
        root: &Path,
        path: &Path,
        cancel: &Cancel,
    ) -> Result<Option<String>, String> {
        let mut command = self.command(root);
        command
            .args(["--literal-pathspecs", "ls-tree", "-z", "HEAD", "--"])
            .arg(checked_path(root, path)?);
        let bytes = self.run(&mut command, SHOW_TIMEOUT, cancel)?;
        let mode = bytes.split(|&b| b == b' ').next().filter(|v| !v.is_empty());
        Ok(mode.map(|mode| String::from_utf8_lossy(mode).into_owned()))
    }

    fn summary_autocrlf(&self, root: &Path, cancel: &Cancel) -> Result<bool, String> {
        let mut command = self.command(root);
        command
            .args(["config", "--get", "core.autocrlf"])
            .stderr(Stdio::piped());
        let output = process::run(&mut command, Vec::new(), SHOW_TIMEOUT, cancel)
            .map_err(|e| describe(&e))?;
        match output.status.code() {
            Some(0) => Ok(matches!(
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "true" | "yes" | "on" | "1" | "input"
            )),
            Some(1) => Ok(false),
            _ => Err(failure(&output.stderr)),
        }
    }

    fn summary_attributes(
        &self,
        root: &Path,
        path: &Path,
        autocrlf: bool,
        cancel: &Cancel,
    ) -> Result<bool, String> {
        let relative = checked_path(root, path)?;
        let mut command = self.command(root);
        command
            .args([
                "check-attr",
                "-z",
                "filter",
                "working-tree-encoding",
                "ident",
                "text",
                "eol",
                "--",
            ])
            .arg(&relative);
        let bytes = self.run(&mut command, SHOW_TIMEOUT, cancel)?;
        let fields: Vec<_> = bytes.split(|&b| b == 0).collect();
        let mut normalize = autocrlf;
        for fields in fields.chunks_exact(3) {
            let (name, value) = (fields[1], fields[2]);
            if matches!(name, b"filter" | b"working-tree-encoding" | b"ident")
                && !matches!(value, b"unspecified" | b"unset")
            {
                return Err(t!("git-ai-diff-unsupported", path = path_label(&relative)).to_owned());
            }
            if name == b"text" {
                normalize = match value {
                    b"unset" => false,
                    b"set" | b"auto" => true,
                    _ => normalize,
                };
            }
            if name == b"eol" && matches!(value, b"lf" | b"crlf") {
                normalize = true;
            }
        }
        Ok(normalize)
    }
}

fn checked_path(root: &Path, path: &Path) -> Result<OsString, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| t!("git-file-outside-repository").to_owned())?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(t!("git-file-outside-repository").to_owned());
    }
    slash_path(root, path)
}

fn path_label(relative: &OsStr) -> String {
    format!("{:?}", relative.to_string_lossy())
}

fn working_mode(path: &Path, old: Option<&str>, filemode: bool) -> Result<Option<String>, String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| t!("git-ai-diff-read-failed", error = e.to_string()))?;
    if meta.file_type().is_symlink() {
        return Ok(Some("120000".into()));
    }
    let mode = if matches!(old, Some("100755" | "100644")) && !filemode {
        old.unwrap_or("100644")
    } else {
        "100644"
    };
    #[cfg(unix)]
    let mode = if filemode {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 != 0 {
            "100755"
        } else {
            "100644"
        }
    } else {
        mode
    };
    Ok(Some(mode.to_owned()))
}

fn read_working(root: &Path, path: &Path) -> Result<Option<Vec<u8>>, String> {
    checked_path(root, path)?;
    let mut parent = path.parent();
    while let Some(dir) = parent.filter(|p| *p != root) {
        if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(
                t!("git-ai-diff-unsupported", path = path.display().to_string()).to_owned(),
            );
        }
        parent = dir.parent();
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(t!("git-ai-diff-read-failed", error = e.to_string()).to_owned()),
    };
    if metadata.file_type().is_symlink() {
        return std::fs::read_link(path)
            .map(|target| Some(target.as_os_str().as_encoded_bytes().to_vec()))
            .map_err(|e| t!("git-ai-diff-read-failed", error = e.to_string()).to_owned());
    }
    if !metadata.is_file() {
        return Err(t!("git-ai-diff-unsupported", path = path.display().to_string()).to_owned());
    }
    if metadata.len() > MAX_FILE as u64 {
        return Err(too_large());
    }
    let mut content = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE as u64 + 1).read_to_end(&mut content))
        .map_err(|e| t!("git-ai-diff-read-failed", error = e.to_string()).to_owned())?;
    if content.len() > MAX_FILE {
        return Err(too_large());
    }
    Ok(Some(content))
}

fn binary(bytes: &[u8]) -> bool {
    bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
}

fn text(bytes: &[u8], normalize: bool) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| t!("git-ai-diff-unreadable").to_owned())?;
    if text.bytes().filter(|&b| matches!(b, b'\n' | b'\r')).count() > MAX_LINES {
        return Err(too_large());
    }
    Ok(if normalize {
        text.replace("\r\n", "\n")
    } else {
        text.to_owned()
    })
}

struct Limited {
    text: String,
    limit: usize,
}

impl Limited {
    fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            limit,
        }
    }
    fn push(&mut self, text: &str) -> Result<(), String> {
        if self.text.len().saturating_add(text.len()) > self.limit {
            return Err(too_large());
        }
        self.text.push_str(text);
        Ok(())
    }
}

fn too_large() -> String {
    t!("git-ai-diff-too-large").to_owned()
}

fn unified(out: &mut Limited, old: &str, new: &str, hunks: &[LineHunk]) -> Result<(), String> {
    // 必须与 line_hunks 使用同一种切行规则，单独 CR 也会被编辑器视为换行。
    let (a, b) = (winc_core::diff::lines(old), winc_core::diff::lines(new));
    let mut at = 0;
    while at < hunks.len() {
        let first = at;
        let old_start = hunks[at].old.start.saturating_sub(3);
        let new_start = hunks[at].new.start.saturating_sub(3);
        while at + 1 < hunks.len() && hunks[at + 1].old.start <= hunks[at].old.end.saturating_add(6)
        {
            at += 1;
        }
        let old_end = hunks[at].old.end.saturating_add(3).min(a.len());
        let new_end = hunks[at].new.end.saturating_add(3).min(b.len());
        let old_count = old_end - old_start;
        let new_count = new_end - new_start;
        let old_line = old_start + usize::from(old_count != 0);
        let new_line = new_start + usize::from(new_count != 0);
        out.push(&format!(
            "@@ -{old_line},{old_count} +{new_line},{new_count} @@\n"
        ))?;
        let mut old_at = old_start;
        for hunk in &hunks[first..=at] {
            for line in &a[old_at..hunk.old.start] {
                diff_line(out, ' ', line)?;
            }
            for line in &a[hunk.old.clone()] {
                diff_line(out, '-', line)?;
            }
            for line in &b[hunk.new.clone()] {
                diff_line(out, '+', line)?;
            }
            old_at = hunk.old.end;
        }
        for line in &a[old_at..old_end] {
            diff_line(out, ' ', line)?;
        }
        at += 1;
    }
    Ok(())
}

fn diff_line(out: &mut Limited, prefix: char, line: &str) -> Result<(), String> {
    let mut text = String::new();
    let _ = write!(text, "{prefix}{line}");
    out.push(&text)?;
    if !line.ends_with('\n') {
        out.push("\n\\ No newline at end of file\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
