//! 直接交回后台结果，验证窗口状态变化和手写说明保护。

use super::*;

#[test]
fn model_results_from_an_old_address_are_ignored_and_manual_models_survive_errors() {
    let mut app = app();
    app.config.ai_base_url = Some("https://current.example/v1".into());
    app.config.ai_model = Some("manual-model".into());
    let (id, _) = app.ai_summary.begin();
    app.ai_summary
        .results
        .0
        .send(ResultEvent::Models(
            id,
            "https://old.example/v1".into(),
            Ok(crate::ai::ModelCatalog {
                endpoint: "https://old.example/v1/chat/completions".into(),
                models: vec!["wrong-model".into()],
            }),
        ))
        .unwrap();
    app.poll_ai_summary();
    assert!(app.settings.ai.models.is_empty());
    assert!(app.settings.ai.resolved.is_none());
    let (id, _) = app.ai_summary.begin();
    app.ai_summary
        .results
        .0
        .send(ResultEvent::Models(
            id,
            "https://current.example/v1".into(),
            Err("mock error".into()),
        ))
        .unwrap();
    app.poll_ai_summary();
    assert_eq!(app.config.ai_model.as_deref(), Some("manual-model"));
    assert!(app.settings.ai.model_error);
    assert_eq!(app.settings.ai.model_status.as_deref(), Some("mock error"));
}

fn app() -> App {
    let mut app = App::new(
        winc_editor::Editor::default(),
        None,
        crate::bench::Bench::from_env(),
        winc_config::Config::default(),
    );
    app.git.root = Some(PathBuf::from("C:/test-repo"));
    app.git.status = Some(Status {
        entries: vec![Entry {
            path: PathBuf::from("C:/test-repo/a.txt"),
            from: None,
            staged: Some(winc_editor::git::Change::Modified),
            unstaged: None,
            is_dir: false,
        }],
        ..Status::default()
    });
    app
}

fn completed(app: &mut App, original: &str) -> u64 {
    let scope = app.ai_scope().unwrap();
    let (id, _) = app.ai_summary.begin();
    app.ai_summary
        .results
        .0
        .send(ResultEvent::Generated(
            id,
            scope,
            original.into(),
            Ok((
                CommitSummary {
                    diff: "diff".into(),
                    fingerprint: 1,
                },
                "feat: generated".into(),
            )),
        ))
        .unwrap();
    id
}

/// 真实仓库的注入：自动回填在回填前会复核磁盘，注入的输入必须与复核结果一致。
fn real_completed(app: &mut App, original: &str, message: &str) -> u64 {
    let scope = app.ai_scope().unwrap();
    let prepared = std::sync::Arc::new(std::sync::Mutex::new(None));
    let capture = prepared.clone();
    let scope_for_prepare = scope.clone();
    app.spawn_git(UserEvent::AiSummary, move |tool| {
        let input = tool
            .as_ref()
            .map_err(unusable)
            .and_then(|git| prepare(git, &scope_for_prepare, &Cancel::default()));
        *capture.lock().unwrap() = Some(input);
    });
    let input = prepared.lock().unwrap().take().unwrap().unwrap();
    let (id, _) = app.ai_summary.begin();
    app.ai_summary
        .results
        .0
        .send(ResultEvent::Generated(
            id,
            scope,
            original.into(),
            Ok((input, message.into())),
        ))
        .unwrap();
    id
}

#[test]
fn empty_message_is_filled_but_existing_or_newly_typed_text_requires_preview() {
    let (mut app, root) = repository_app("https://unused.example/v1", "autofill");
    real_completed(&mut app, "", "feat: generated");
    app.poll_ai_summary();
    assert_eq!(app.commit.message.text(), "feat: generated");
    assert!(!app.ai_summary.is_busy());
    app.commit.message.set_text("handwritten");
    real_completed(&mut app, "handwritten", "feat: generated");
    app.poll_ai_summary();
    assert_eq!(app.commit.message.text(), "handwritten");
    assert!(app.editor.active().unwrap().readonly);
    assert!(app.ai_summary.preview.is_some());
    real_completed(&mut app, "", "feat: generated");
    app.commit.message.set_text("typed while waiting");
    app.poll_ai_summary();
    assert_eq!(app.commit.message.text(), "typed while waiting");
    assert!(app.ai_summary.preview.is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn switching_repository_or_changing_selection_rejects_old_results() {
    let mut app = app();
    completed(&mut app, "");
    app.git.root = Some(PathBuf::from("C:/another-repo"));
    app.poll_ai_summary();
    assert!(app.commit.message.text().is_empty());
    app.git.root = Some(PathBuf::from("C:/test-repo"));
    completed(&mut app, "");
    app.commit.checks += 1;
    app.poll_ai_summary();
    assert!(app.commit.message.text().is_empty());
    assert!(app.alert.is_none());
}

#[test]
fn cancelled_request_cannot_complete_new_request_or_write_a_message() {
    let mut app = app();
    let old = completed(&mut app, "");
    app.ai_summary.cancel();
    let (new, _) = app.ai_summary.begin();
    assert_ne!(old, new);
    app.poll_ai_summary();
    assert!(app.ai_summary.is_busy());
    assert!(app.commit.message.text().is_empty());
    let cancel = app.ai_summary.running.as_ref().unwrap().cancel.clone();
    app.ai_summary.cancel();
    assert!(cancel.is_cancelled());
}

#[test]
fn errors_preserve_manual_text_and_cancelled_preview_does_not_replace_it() {
    let mut app = app();
    app.commit.message.set_text("manual");
    let scope = app.ai_scope().unwrap();
    let (id, _) = app.ai_summary.begin();
    app.ai_summary
        .results
        .0
        .send(ResultEvent::Generated(
            id,
            scope,
            "manual".into(),
            Err("mock failure".into()),
        ))
        .unwrap();
    app.poll_ai_summary();
    assert_eq!(app.commit.message.text(), "manual");
    let id = completed(&mut app, "manual");
    app.poll_ai_summary();
    app.ai_summary.cancel();
    app.use_ai_summary(id);
    assert_eq!(app.commit.message.text(), "manual");
}

#[test]
fn duplicate_clicks_are_ignored_before_any_network_request() {
    let mut app = app();
    let (id, _) = app.ai_summary.begin();
    app.generate_ai_summary();
    app.test_ai_connection();
    assert_eq!(app.ai_summary.running.as_ref().unwrap().id, id);
    assert_eq!(app.ai_summary.sequence, id);
}

fn repository_app(url: &str, name: &str) -> (App, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "winc-ai-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["init", "-q"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let root = winc_editor::canonical_path(&root);
    std::fs::write(root.join("selected.txt"), "chosen content\n").unwrap();
    std::fs::write(root.join("excluded.txt"), "unselected secret context\n").unwrap();
    let tool = Git::find(&winc_editor::shell::Shell::with_vars(
        std::env::vars_os().collect(),
    ))
    .unwrap();
    let mut app = app();
    app.git.root = Some(root.clone());
    app.git.status = Some(tool.status(&root, &Cancel::default()).unwrap());
    app.git.tool.keep(tool);
    let selected = app
        .git
        .status
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|e| e.path.ends_with("selected.txt"))
        .unwrap()
        .clone();
    app.commit.set_checked(&selected, true);
    app.config.ai_base_url = Some(url.into());
    app.config.ai_model = Some("test-model".into());
    crate::ai::save_api_key(url, "mock-integration-key").unwrap();
    (app, root)
}

#[test]
fn full_generation_uploads_only_checked_files_and_preserves_git_state() {
    let (url, handle) = crate::ai::tests::server(
        200,
        crate::ai::tests::response("feat: selected files"),
        Duration::ZERO,
    );
    let (mut app, root) = repository_app(&url, "integration");
    let before = app.git.status.clone();
    app.generate_ai_summary();
    assert_eq!(app.commit.message.text(), "feat: selected files");
    let request = handle.join().unwrap();
    assert!(request.contains("chosen content"));
    assert!(!request.contains("unselected secret context") && !request.contains("excluded.txt"));
    assert_eq!(app.git.status, before);
    assert!(!root.join(".git/index").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("selected.txt")).unwrap(),
        "chosen content\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn disk_change_during_http_request_is_detected_before_filling_message() {
    let mutation = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let changed = mutation.clone();
    let (url, handle) = crate::ai::tests::server_with(
        200,
        crate::ai::tests::response("stale result"),
        Duration::ZERO,
        move || {
            std::fs::write(
                changed.lock().unwrap().as_ref().unwrap(),
                "changed while requesting\n",
            )
            .unwrap();
        },
    );
    let (mut app, root) = repository_app(&url, "mutation");
    *mutation.lock().unwrap() = Some(root.join("selected.txt"));
    app.commit.message.set_text("manual draft");
    app.generate_ai_summary();
    handle.join().unwrap();
    assert_eq!(app.commit.message.text(), "manual draft");
    assert!(app.ai_summary.preview.is_none());
    assert!(!app.ai_summary.is_busy());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn accepting_a_preview_rechecks_files_and_does_not_use_a_stale_result() {
    let (url, handle) = crate::ai::tests::server(
        200,
        crate::ai::tests::response("feat: generated"),
        Duration::ZERO,
    );
    let (mut app, root) = repository_app(&url, "preview-stale");
    app.commit.message.set_text("manual draft");
    app.generate_ai_summary();
    handle.join().unwrap();
    let id = app.ai_summary.preview_id().unwrap();
    std::fs::write(root.join("selected.txt"), "changed after preview\n").unwrap();
    app.use_ai_summary(id);
    assert_eq!(app.commit.message.text(), "manual draft");
    assert!(!app.ai_summary.is_busy());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ai_partial_input_uses_the_existing_commit_line_selection() {
    use winc_core::diff::{hunk_key, line_hunks, lines};
    let mut app = app();
    let old: String = (0..20).map(|n| format!("line {n}\n")).collect();
    let new = old
        .replace("line 2\n", "chosen\n")
        .replace("line 17\n", "excluded\n");
    let (a, b) = (lines(&old), lines(&new));
    let all: Vec<_> = line_hunks(&old, &new)
        .into_iter()
        .map(|h| {
            (
                hunk_key(a[h.old.clone()].iter().copied(), b[h.new].iter().copied()),
                h.old,
            )
        })
        .collect();
    assert_eq!(all.len(), 2);
    let entry = app.git.status.as_ref().unwrap().entries[0].clone();
    app.commit.toggle_hunk(&entry, 1, &all);
    let part = actions::partial_from_bytes(
        &entry.path,
        old.into_bytes(),
        new.into_bytes(),
        &app.commit.skipped[&entry.path],
    )
    .unwrap();
    let text = String::from_utf8(part.content).unwrap();
    assert!(text.contains("chosen\n"));
    assert!(!text.contains("excluded\n"));
    assert!(text.contains("line 17\n"));
}
