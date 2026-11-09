//! 第三方 `OpenAI` 兼容接口：只发送调用方准备好的差异，不访问仓库，不执行 Git 操作。

mod credentials;
mod models;
#[cfg(test)]
pub(crate) mod tests;
mod transport;

use std::collections::BTreeMap;

use winc_config::Json;
use winc_editor::shell::Cancel;
use winc_i18n::t;

pub(crate) use credentials::{load_api_key, save_api_key};
pub(crate) use models::{ModelCatalog, fetch_models};

/// 单次发送的差异上限（字节）；超出时由界面提示缩小选择范围。
pub(crate) const MAX_DIFF_BYTES: usize = 128 * 1024;
pub(crate) const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_MESSAGE_BYTES: usize = 16 * 1024;

/// 默认提交信息的格式与生成规则。
pub(crate) const DEFAULT_PROMPT: &str =
    "你是一个 Git 提交信息生成专家，遵守Git commit 信息规范（Conventional Commits 风格），请根据我提供的 git diff，生成 1 条提交信息。

输出格式必须严格如下：

    <type>[optional scope]: < 短标题 >
      - 如果改动用户可见，描述用户看到的效果
      - 如果改动不可见，按标准正文描述技术改动或改动目的
    ```
  * type 示例：feat、fix、docs、style、refactor、test、chore等
  * Header 简洁描述本次提交内容
  * 正文必须严格与本次提交范围一致
  * 禁止只写单行标题或模糊描述
  * 提交内容使用中文";

/// 请求所需的普通设置；凭据始终单独传入，不进入配置或调试输出。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) prompt: String,
}

/// 根据调用方提供的本次实际提交差异生成提交说明。
pub(crate) fn generate(
    settings: &Settings,
    key: &str,
    diff: &str,
    cancel: &Cancel,
) -> Result<String, String> {
    if diff.trim().is_empty() {
        return Err(t!("ai-client-empty-diff").to_owned());
    }
    if diff.len() > MAX_DIFF_BYTES {
        return Err(t!("ai-client-request-too-large").to_owned());
    }
    request(settings, key, diff, cancel).map(|(_, message)| message)
}

/// 用户手动测试连接；请求不包含任何项目文件内容。
pub(crate) fn test_connection(
    settings: &Settings,
    key: &str,
    cancel: &Cancel,
) -> Result<String, String> {
    let settings = Settings {
        prompt: "Reply with OK only.".to_owned(),
        ..settings.clone()
    };
    request(&settings, key, "Connection test. Reply OK.", cancel).map(|(endpoint, _)| endpoint)
}

fn request(
    settings: &Settings,
    key: &str,
    content: &str,
    cancel: &Cancel,
) -> Result<(String, String), String> {
    let endpoints = endpoint_candidates(&settings.base_url)?;
    if settings.model.trim().is_empty() {
        return Err(t!("ai-client-model-required").to_owned());
    }
    validate_key(key)?;
    let prompt = if settings.prompt.trim().is_empty() {
        DEFAULT_PROMPT
    } else {
        &settings.prompt
    };
    if prompt.len() > MAX_PROMPT_BYTES || settings.model.len() > 1024 {
        return Err(t!("ai-client-request-too-large").to_owned());
    }
    if content.contains(key) || prompt.contains(key) || settings.model.contains(key) {
        return Err(t!("ai-client-key-in-content").to_owned());
    }
    let body = payload(settings.model.trim(), prompt, content).compact();
    let id = request_id();
    let started = std::time::Instant::now();
    let result = (|| {
        for (index, endpoint) in endpoints.iter().enumerate() {
            let response = transport::exchange(endpoint, key, Some(&body), cancel)?;
            if matches!(response.status, 404 | 405) && index + 1 < endpoints.len() {
                continue;
            }
            let message = response_message(&response.success()?, key)?;
            return Ok((endpoint.clone(), message));
        }
        Err(t!("ai-client-invalid-url").to_owned())
    })();
    // 只记录编号、长度、耗时和结果，不记录地址、请求正文、密钥或服务返回的原始错误。
    eprintln!(
        "winc: AI request id={id} input_bytes={} elapsed_ms={} success={}",
        content.len(),
        started.elapsed().as_millis(),
        result.is_ok()
    );
    result
}

fn request_id() -> u64 {
    static REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn object(fields: impl IntoIterator<Item = (&'static str, Json)>) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn payload(model: &str, prompt: &str, content: &str) -> Json {
    let message = |role: &str, text: &str| {
        object([
            ("role", Json::String(role.to_owned())),
            ("content", Json::String(text.to_owned())),
        ])
    };
    object([
        ("model", Json::String(model.to_owned())),
        ("stream", Json::Bool(false)),
        (
            "messages",
            Json::Array(vec![message("system", prompt), message("user", content)]),
        ),
    ])
}

/// 规范化基础地址和完整地址，并拒绝凭据、控制字符及不安全的明文远程连接。
pub(crate) fn endpoint(base_url: &str) -> Result<String, String> {
    let invalid = || t!("ai-client-invalid-url").to_owned();
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty()
        || base.chars().any(|c| c.is_control() || c.is_whitespace())
        || base.contains(['?', '#', '\\'])
    {
        return Err(invalid());
    }
    let (scheme, rest) = base.split_once("://").ok_or_else(invalid)?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "https" | "http") {
        return Err(invalid());
    }
    let (authority, path) = rest
        .split_once('/')
        .map_or((rest, ""), |(host, path)| (host, path));
    if authority.is_empty() || authority.contains(['@', '%']) || !authority.is_ascii() {
        return Err(invalid());
    }
    let (host, port) = if let Some(ip) = authority.strip_prefix('[') {
        let (ip, tail) = ip.split_once(']').ok_or_else(invalid)?;
        if ip.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(invalid());
        }
        let port = if tail.is_empty() {
            None
        } else {
            Some(tail.strip_prefix(':').ok_or_else(invalid)?)
        };
        (ip, port)
    } else {
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, None), |(h, p)| (h, Some(p)));
        if host.is_empty()
            || !host
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.'))
        {
            return Err(invalid());
        }
        (host, port)
    };
    if port.is_some_and(|port| port.parse::<u16>().map_or(true, |port| port == 0)) {
        return Err(invalid());
    }
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if scheme == "http" && !loopback {
        return Err(invalid());
    }
    let suffix = path.trim_end_matches('/');
    let suffix = if suffix.is_empty() {
        "chat/completions".to_owned()
    } else if suffix == "chat/completions" || suffix.ends_with("/chat/completions") {
        suffix.to_owned()
    } else {
        format!("{suffix}/chat/completions")
    };
    Ok(format!(
        "{scheme}://{}/{suffix}",
        authority.to_ascii_lowercase()
    ))
}

/// 裸域名自动识别 /v1；显式 API 路径不擅自改写。凭据账号仍用 endpoint 保持兼容。
pub(crate) fn endpoint_candidates(base_url: &str) -> Result<Vec<String>, String> {
    let normalized = endpoint(base_url)?;
    let (_, rest) = base_url
        .trim()
        .trim_end_matches('/')
        .split_once("://")
        .ok_or_else(|| t!("ai-client-invalid-url").to_owned())?;
    if !rest.contains('/') {
        let origin = normalized.strip_suffix("/chat/completions").unwrap();
        Ok(vec![format!("{origin}/v1/chat/completions"), normalized])
    } else {
        Ok(vec![normalized])
    }
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty()
        || key.len() > 8192
        || key
            .bytes()
            .any(|c| c.is_ascii_control() || c.is_ascii_whitespace())
    {
        return Err(t!("ai-client-key-required").to_owned());
    }
    Ok(())
}

fn response_message(response: &[u8], key: &str) -> Result<String, String> {
    let invalid = || t!("ai-client-invalid-response").to_owned();
    let text = std::str::from_utf8(response).map_err(|_| invalid())?;
    // 配置解析器是递归的；不让不可信服务用深层 JSON 耗尽线程栈。
    if !bounded_json_depth(text, 64) {
        return Err(invalid());
    }
    let json = Json::parse(text).ok_or_else(invalid)?;
    let choice = json
        .as_object()
        .and_then(|j| j.get("choices"))
        .and_then(Json::as_array)
        .and_then(|a| a.first())
        .and_then(Json::as_object)
        .ok_or_else(invalid)?;
    if choice
        .get("finish_reason")
        .and_then(Json::as_str)
        .is_some_and(|v| matches!(v, "length" | "content_filter"))
    {
        return Err(t!("ai-client-incomplete-response").to_owned());
    }
    let message = choice
        .get("message")
        .and_then(Json::as_object)
        .and_then(|j| j.get("content"))
        .and_then(Json::as_str)
        .ok_or_else(invalid)?;
    let mut message = message.trim();
    if message.starts_with("```") && message.ends_with("```") {
        message = message
            .split_once('\n')
            .and_then(|(_, text)| text.strip_suffix("```"))
            .ok_or_else(invalid)?
            .trim();
    }
    if message.is_empty()
        || message.len() > MAX_MESSAGE_BYTES
        || message.contains(key)
        || message
            .bytes()
            .any(|c| c.is_ascii_control() && !matches!(c, b'\n' | b'\r' | b'\t'))
    {
        return Err(invalid());
    }
    Ok(message.replace("\r\n", "\n"))
}

fn bounded_json_depth(text: &str, limit: usize) -> bool {
    let (mut depth, mut string, mut escaped) = (0usize, false, false);
    for c in text.bytes() {
        if string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                string = false;
            }
        } else {
            match c {
                b'"' => string = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > limit {
                        return false;
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    true
}
