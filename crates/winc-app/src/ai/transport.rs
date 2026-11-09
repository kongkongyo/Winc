//! 通过系统 curl 请求接口；凭据和请求正文只经过标准输入，不出现在命令行或临时文件中。

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use winc_editor::shell::Cancel;
use winc_i18n::t;

const MAX_RESPONSE_BYTES: usize = 128 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn success(self) -> Result<Vec<u8>, String> {
        if !(200..300).contains(&self.status) {
            return Err(t!("ai-client-http-failed", status = self.status));
        }
        Ok(self.body)
    }
}

pub(super) fn exchange(
    url: &str,
    key: &str,
    body: Option<&str>,
    cancel: &Cancel,
) -> Result<HttpResponse, String> {
    exchange_with_timeout(url, key, body, cancel, TIMEOUT)
}

fn curl_config(url: &str, key: &str, body: Option<&str>) -> String {
    fn quote(value: &str) -> String {
        let mut result = String::from("\"");
        for c in value.chars() {
            match c {
                '\\' => result.push_str("\\\\"),
                '"' => result.push_str("\\\""),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                c => result.push(c),
            }
        }
        result.push('"');
        result
    }
    let mut config = format!(
        "url = {}\nheader = {}\nheader = \"Content-Type: application/json\"\n",
        quote(url),
        quote(&format!("Authorization: Bearer {key}")),
    );
    if let Some(body) = body {
        config.push_str(&format!("data-binary = {}\n", quote(body)));
    }
    config
}

fn curl_command(timeout: Duration, local: bool, post: bool) -> Command {
    #[cfg(windows)]
    let mut command = {
        // Windows 按名称启动会先搜索当前目录，不能让仓库中的 curl.exe 收到用户密钥。
        let system = std::env::var_os("SystemRoot")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| std::path::PathBuf::from("C:/Windows"));
        Command::new(system.join("System32/curl.exe"))
    };
    #[cfg(not(windows))]
    let mut command = Command::new("curl");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    // --disable 必须是第一个参数，禁止读取用户 curlrc 中额外的 URL、重定向或日志选项。
    command
        .args([
            "--disable",
            "--silent",
            "--request",
            if post { "POST" } else { "GET" },
            "--globoff",
            "--proto",
            "=http,https",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "10",
            "--max-time",
        ])
        .arg(timeout.as_secs_f64().to_string())
        .args([
            "--max-filesize",
            &MAX_RESPONSE_BYTES.to_string(),
            "--write-out",
            "\n%{http_code}",
            "--config",
            "-",
        ]);
    if local {
        command.args(["--noproxy", "*"]);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
pub(super) fn post_with_timeout(
    url: &str,
    key: &str,
    body: &str,
    cancel: &Cancel,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    exchange_with_timeout(url, key, Some(body), cancel, timeout)?.success()
}

fn exchange_with_timeout(
    url: &str,
    key: &str,
    body: Option<&str>,
    cancel: &Cancel,
    timeout: Duration,
) -> Result<HttpResponse, String> {
    if cancel.is_cancelled() {
        return Err(t!("ai-client-cancelled").to_owned());
    }
    let config = curl_config(url, key, body);
    let mut child = curl_command(timeout, url.starts_with("http://"), body.is_some())
        .spawn()
        .map_err(|_| t!("ai-client-curl-unavailable").to_owned())?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| t!("ai-client-network-failed").to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| t!("ai-client-network-failed").to_owned())?;
    let too_large = AtomicBool::new(false);
    let started = Instant::now();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            let mut stdin = stdin;
            stdin.write_all(config.as_bytes())
        });
        let reader = scope.spawn(|| {
            let mut bytes = Vec::new();
            let result = stdout
                .take((MAX_RESPONSE_BYTES + 1) as u64)
                .read_to_end(&mut bytes);
            if bytes.len() > MAX_RESPONSE_BYTES {
                too_large.store(true, Ordering::Relaxed);
            }
            result.map(|_| bytes)
        });
        let status = loop {
            let error = if cancel.is_cancelled() {
                Some(t!("ai-client-cancelled").to_owned())
            } else if too_large.load(Ordering::Relaxed) {
                Some(t!("ai-client-response-too-large").to_owned())
            } else if started.elapsed() >= timeout {
                Some(t!("ai-client-timeout").to_owned())
            } else {
                None
            };
            if let Some(error) = error {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(t!("ai-client-network-failed").to_owned());
                }
            }
        };
        let written = writer
            .join()
            .map_err(|_| t!("ai-client-network-failed").to_owned());
        let read = reader
            .join()
            .map_err(|_| t!("ai-client-network-failed").to_owned());
        let status = status?;
        if too_large.load(Ordering::Relaxed) || status.code() == Some(63) {
            return Err(t!("ai-client-response-too-large").to_owned());
        }
        if status.code() == Some(28) {
            return Err(t!("ai-client-timeout").to_owned());
        }
        if !status.success() {
            return Err(t!("ai-client-network-failed").to_owned());
        }
        written?.map_err(|_| t!("ai-client-network-failed").to_owned())?;
        let bytes = read?.map_err(|_| t!("ai-client-network-failed").to_owned())?;
        let at = bytes
            .iter()
            .rposition(|b| *b == b'\n')
            .ok_or_else(|| t!("ai-client-invalid-response").to_owned())?;
        let code = std::str::from_utf8(&bytes[at + 1..])
            .ok()
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| t!("ai-client-invalid-response").to_owned())?;
        if (300..400).contains(&code) {
            return Err(t!("ai-client-redirect-rejected").to_owned());
        }
        Ok(HttpResponse {
            status: code,
            body: bytes[..at].to_vec(),
        })
    })
}
