//! 模型发现不包含仓库内容；仅在用户明确点击后读取同站点的模型列表。

use super::*;

pub(crate) struct ModelCatalog {
    /// 同前缀的完整聊天接口地址，供设置页显示识别结果。
    pub endpoint: String,
    pub models: Vec<String>,
}

pub(crate) fn fetch_models(
    base_url: &str,
    key: &str,
    cancel: &Cancel,
) -> Result<ModelCatalog, String> {
    validate_key(key)?;
    let endpoints = endpoint_candidates(base_url)?;
    let id = request_id();
    let started = std::time::Instant::now();
    let result = (|| {
        for (index, endpoint) in endpoints.iter().enumerate() {
            let prefix = endpoint.strip_suffix("/chat/completions").unwrap();
            let url = format!("{prefix}/models");
            let response = transport::exchange(&url, key, None, cancel)?;
            if matches!(response.status, 404 | 405) && index + 1 < endpoints.len() {
                continue;
            }
            if matches!(response.status, 404 | 405) {
                return Err(t!("ai-models-unsupported").to_owned());
            }
            let models = parse_models(&response.success()?, key)?;
            return Ok(ModelCatalog {
                endpoint: endpoint.clone(),
                models,
            });
        }
        Err(t!("ai-models-unsupported").to_owned())
    })();
    eprintln!(
        "winc: AI models id={id} elapsed_ms={} success={}",
        started.elapsed().as_millis(),
        result.is_ok()
    );
    result
}

fn parse_models(bytes: &[u8], key: &str) -> Result<Vec<String>, String> {
    let invalid = || t!("ai-models-invalid-response").to_owned();
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    if !bounded_json_depth(text, 32) {
        return Err(invalid());
    }
    let json = Json::parse(text).ok_or_else(invalid)?;
    let data = json
        .as_object()
        .and_then(|j| j.get("data"))
        .and_then(Json::as_array)
        .ok_or_else(invalid)?;
    if data.len() > 4096 {
        return Err(invalid());
    }
    let mut models = Vec::new();
    for item in data {
        let id = item
            .as_object()
            .and_then(|j| j.get("id"))
            .and_then(Json::as_str)
            .ok_or_else(invalid)?
            .trim();
        if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) || id.contains(key)
        {
            return Err(invalid());
        }
        models.push(id.to_owned());
    }
    models.sort();
    models.dedup();
    Ok(models)
}
