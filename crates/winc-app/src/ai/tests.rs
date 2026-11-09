//! 只访问本机模拟 HTTP 服务；不读取宿主凭据，不调用真实中转站。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use super::*;

const KEY: &str = "mock-test-credential";

/// 连续应答同一站点的候选路径，超时即失败，避免失败测试永久等待。
fn sequence_server(replies: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in replies {
            let started = std::time::Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(30) =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    other => panic!("mock server accept failed: {other:?}"),
                }
            };
            // Windows 的 accept 会继承 listener 的非阻塞模式，后续按完整请求读取需恢复阻塞。
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut block = [0; 4096];
            loop {
                let n = stream.read(&mut block).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&block[..n]);
                if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..at]);
                    let len = header
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= at + 4 + len {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let response = format!(
                "HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        requests
    });
    (url, handle)
}

#[test]
fn bare_domains_detect_v1_and_only_fall_back_on_unsupported_routes() {
    assert_eq!(
        endpoint_candidates("https://relay.example/").unwrap(),
        [
            "https://relay.example/v1/chat/completions",
            "https://relay.example/chat/completions"
        ]
    );
    for base in [
        "https://relay.example/v1",
        "https://relay.example/api/v2",
        "https://relay.example/chat/completions",
    ] {
        assert_eq!(
            endpoint_candidates(base).unwrap(),
            [endpoint(base).unwrap()]
        );
    }
    let (url, handle) = sequence_server(vec![
        (404, "missing".into()),
        (200, response("fix: result")),
    ]);
    let resolved = test_connection(&settings(&url), KEY, &Cancel::default()).unwrap();
    assert_eq!(resolved, format!("{url}/chat/completions"));
    let requests = handle.join().unwrap();
    assert!(requests[0].starts_with("POST /v1/chat/completions "));
    assert!(requests[1].starts_with("POST /chat/completions "));

    let (url, handle) = sequence_server(vec![(401, KEY.into())]);
    let error = test_connection(&settings(&url), KEY, &Cancel::default()).unwrap_err();
    assert!(error.contains("401") && !error.contains(KEY), "{error}");
    assert_eq!(handle.join().unwrap().len(), 1);
}

#[test]
fn model_discovery_uses_get_preserves_explicit_paths_and_sorts_unique_ids() {
    let body = r#"{"data":[{"id":"z-model"},{"id":"a-model"},{"id":"a-model"}]}"#;
    let (url, handle) = sequence_server(vec![(200, body.into())]);
    let base = format!("{url}/custom/api/chat/completions");
    let catalog = fetch_models(&base, KEY, &Cancel::default()).unwrap();
    assert_eq!(catalog.endpoint, base);
    assert_eq!(catalog.models, ["a-model", "z-model"]);
    let requests = handle.join().unwrap();
    assert!(requests[0].starts_with("GET /custom/api/models "));
    assert!(requests[0].contains(&format!("Authorization: Bearer {KEY}")));
    assert!(!requests[0].contains("Connection test") && !requests[0].contains("messages"));
}

#[test]
fn models_fall_back_from_v1_and_reject_invalid_or_secret_content() {
    let (url, handle) = sequence_server(vec![(405, String::new()), (200, r#"{"data":[]}"#.into())]);
    let catalog = fetch_models(&url, KEY, &Cancel::default()).unwrap();
    assert!(catalog.models.is_empty());
    assert_eq!(catalog.endpoint, format!("{url}/chat/completions"));
    let requests = handle.join().unwrap();
    assert!(requests[0].starts_with("GET /v1/models "));
    assert!(requests[1].starts_with("GET /models "));
    for body in [
        "{}".to_owned(),
        format!(r#"{{"data":[{{"id":"{KEY}"}}]}}"#),
        r#"{"data":[{"id":""}]}"#.into(),
    ] {
        let (url, handle) = server(200, body, Duration::ZERO);
        let error = fetch_models(&url, KEY, &Cancel::default()).err().unwrap();
        assert!(!error.contains(KEY));
        handle.join().unwrap();
    }
}

fn settings(url: &str) -> Settings {
    Settings {
        base_url: url.into(),
        model: "test-model".into(),
        prompt: "custom instruction".into(),
    }
}

pub(crate) fn server(
    status: u16,
    body: String,
    delay: Duration,
) -> (String, std::thread::JoinHandle<String>) {
    server_with(status, body, delay, || {})
}

pub(crate) fn server_with(
    status: u16,
    body: String,
    delay: Duration,
    before_response: impl FnOnce() + Send + 'static,
) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    // 与 sequence_server 相同的 accept 超时：客户端在连接前失败时测试报错而不是挂死。
    // 完整套件并行时本机连接可能明显变慢，所以给足 30 秒。
    listener.set_nonblocking(true).unwrap();
    let handle = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && started.elapsed() < Duration::from_secs(30) =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                other => panic!("mock server accept failed: {other:?}"),
            }
        };
        // Windows 的 accept 会继承 listener 的非阻塞模式，后续按完整请求读取需恢复阻塞。
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut block = [0; 4096];
        loop {
            let n = stream.read(&mut block).unwrap();
            assert!(n > 0);
            request.extend_from_slice(&block[..n]);
            if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..at]);
                let size = header
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= at + 4 + size {
                    break;
                }
            }
        }
        std::thread::sleep(delay);
        before_response();
        let reply = format!(
            "HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(reply.as_bytes());
        String::from_utf8(request).unwrap()
    });
    (url, handle)
}

pub(crate) fn response(message: &str) -> String {
    object([(
        "choices",
        Json::Array(vec![object([
            ("finish_reason", Json::String("stop".into())),
            (
                "message",
                object([("content", Json::String(message.into()))]),
            ),
        ])]),
    )])
    .compact()
}

#[test]
fn endpoint_accepts_base_and_full_urls_and_rejects_unsafe_inputs() {
    assert_eq!(
        endpoint(" https://Relay.Example/v1/ ").unwrap(),
        "https://relay.example/v1/chat/completions"
    );
    assert_eq!(
        endpoint("https://relay.example/v1/chat/completions").unwrap(),
        "https://relay.example/v1/chat/completions"
    );
    assert!(endpoint("http://[::1]:1234/v1").is_ok());
    for url in [
        "",
        "http://relay.example/v1",
        "https://user:pass@relay.example",
        "https://relay.example/?key=x",
        "https://relay.example/#x",
        "https://relay.example/\nheader=x",
        "file:///tmp/x",
        "https://relay.example:0",
        "https://relay.example\\x",
    ] {
        assert!(endpoint(url).is_err(), "{url:?}");
    }
}

#[test]
fn generation_sends_selected_diff_model_and_prompt_as_json() {
    let (url, handle) = server(200, response("feat: 测试\n\n- 仅生成说明"), Duration::ZERO);
    let diff = "diff --git a/a.rs b/a.rs\n- old\n+ \"new\"\n";
    assert_eq!(
        generate(&settings(&url), KEY, diff, &Cancel::default()).unwrap(),
        "feat: 测试\n\n- 仅生成说明"
    );
    let request = handle.join().unwrap();
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(request.contains(&format!("Authorization: Bearer {KEY}")));
    let body = Json::parse(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    let body = body.as_object().unwrap();
    assert_eq!(body["model"].as_str(), Some("test-model"));
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].as_object().unwrap()["content"].as_str(),
        Some("custom instruction")
    );
    assert_eq!(
        messages[1].as_object().unwrap()["content"].as_str(),
        Some(diff)
    );
    assert!(!request.split_once("\r\n\r\n").unwrap().1.contains(KEY));
}

#[test]
fn test_connection_sends_no_repository_content() {
    let (url, handle) = server(200, response("OK"), Duration::ZERO);
    test_connection(&settings(&url), KEY, &Cancel::default()).unwrap();
    let request = handle.join().unwrap();
    assert!(request.contains("Connection test. Reply OK."));
    assert!(!request.contains("custom instruction"));
}

#[test]
fn rejects_empty_oversized_invalid_or_incomplete_content() {
    let config = settings("http://127.0.0.1:1/v1");
    assert!(generate(&config, KEY, "  ", &Cancel::default()).is_err());
    assert!(
        generate(
            &config,
            KEY,
            &format!("diff contains {KEY}"),
            &Cancel::default()
        )
        .is_err()
    );
    assert!(
        generate(
            &config,
            KEY,
            &"x".repeat(MAX_DIFF_BYTES + 1),
            &Cancel::default()
        )
        .is_err()
    );
    assert!(response_message(b"not json", KEY).is_err());
    assert!(response_message(response(KEY).as_bytes(), KEY).is_err());
    assert!(response_message(response("").as_bytes(), KEY).is_err());
    assert!(
        response_message(response(&"x".repeat(MAX_MESSAGE_BYTES + 1)).as_bytes(), KEY).is_err()
    );
    assert!(
        response_message(
            response("partial")
                .replace("\"stop\"", "\"length\"")
                .as_bytes(),
            KEY
        )
        .is_err()
    );
    assert!(
        response_message(
            format!("{}0{}", "[".repeat(65), "]".repeat(65)).as_bytes(),
            KEY
        )
        .is_err()
    );
    assert_eq!(
        response_message(response("```text\nfeat: yes\n```").as_bytes(), KEY).unwrap(),
        "feat: yes"
    );
}

#[test]
fn http_errors_redirects_response_limit_and_timeout_are_safe() {
    for status in [401, 404, 429, 500, 302] {
        let (url, handle) = server(status, format!("server echoes {KEY}"), Duration::ZERO);
        let error = generate(&settings(&url), KEY, "diff", &Cancel::default()).unwrap_err();
        assert!(!error.contains(KEY));
        assert!(!error.contains("server echoes"));
        handle.join().unwrap();
    }
    let (url, handle) = server(200, "x".repeat(130 * 1024), Duration::ZERO);
    assert!(generate(&settings(&url), KEY, "diff", &Cancel::default()).is_err());
    handle.join().unwrap();
    let (url, handle) = server(200, response("OK"), Duration::from_millis(150));
    assert!(
        transport::post_with_timeout(
            &endpoint(&url).unwrap(),
            KEY,
            "{}",
            &Cancel::default(),
            Duration::from_millis(80)
        )
        .is_err()
    );
    // 80ms 客户端超时可能赶在 curl 建立连接之前就杀掉进程，mock 等不到连接是正常情况；
    // 分离线程不等它的收场，避免把环境慢误判为测试失败。
    drop(handle);
    let cancel = Cancel::default();
    cancel.cancel();
    assert!(generate(&settings("http://127.0.0.1:1"), KEY, "diff", &cancel).is_err());
}

#[test]
fn credentials_are_scoped_to_the_endpoint_without_real_keychain_access() {
    let a = "https://credential-test-a.example/v1";
    let b = "https://credential-test-b.example/v1";
    save_api_key(a, KEY).unwrap();
    assert_eq!(load_api_key(a).unwrap().as_deref(), Some(KEY));
    assert_eq!(load_api_key(b).unwrap(), None);
    assert!(save_api_key(a, "bad\nkey").is_err());
    save_api_key(a, "").unwrap();
    assert_eq!(load_api_key(a).unwrap(), None);
}
