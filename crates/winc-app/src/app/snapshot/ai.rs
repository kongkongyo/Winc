//! AI 设置的真实离屏绘制与鼠标输入，密钥使用隔离的内存凭据。

use super::*;

fn show_ai_setting(app: &mut App, setting: Setting) {
    if setting == Setting::AiModels {
        app.settings.ai.models_open = true;
    }
    let layout = app.settings_layout();
    let row = layout.row(setting).unwrap();
    app.settings.scroll =
        (app.settings.scroll + row.row.y - layout.body.y).clamp(0.0, layout.max_scroll());
    paint(app);
}

fn ai_shot(app: &App, name: &str) {
    write_png(
        &std::env::temp_dir().join(name),
        app.renderer.as_ref().unwrap().image().unwrap(),
    )
    .unwrap();
}

#[test]
fn ai_connection_detects_v1_and_displays_the_actual_endpoint_for_a_bare_domain() {
    let (url, handle) = crate::ai::tests::server(
        200,
        crate::ai::tests::response("OK"),
        std::time::Duration::ZERO,
    );
    let base = url.strip_suffix("/v1").unwrap().to_owned();
    let mut app = blank_app(false);
    app.config.ai_base_url = Some(base.clone());
    app.config.ai_model = Some("fixture-model".into());
    crate::ai::save_api_key(&base, "mock-address-key").unwrap();
    app.open_settings();
    app.settings_show_page(SettingsPage::Ai);
    app.settings.motion = None;
    app.test_ai_connection();
    assert!(
        handle
            .join()
            .unwrap()
            .starts_with("POST /v1/chat/completions ")
    );
    assert_eq!(
        app.settings.ai.resolved,
        Some((base.clone(), format!("{base}/v1/chat/completions")))
    );
    assert_eq!(app.config.ai_base_url.as_deref(), Some(base.as_str()));
    assert!(!app.ai_summary.is_busy());
    paint(&mut app);
    let layout = app.settings_layout();
    for setting in [
        Setting::AiBaseUrl,
        Setting::AiModel,
        Setting::AiApiKey,
        Setting::AiPrompt,
    ] {
        let row = layout.row(setting).unwrap();
        let parts = crate::layout::AiRowParts::new(row);
        assert!(parts.field.bottom() < row.row.bottom());
        assert!(parts.field.y > row.row.y + 48.0);
    }
    ai_shot(&app, "winc-ai-address-verified.png");
}

#[test]
fn ai_saved_and_entered_keys_can_be_revealed_and_hide_when_leaving() {
    let mut app = blank_app(false);
    let base = "https://visibility-fixture.example";
    app.config.ai_base_url = Some(base.into());
    crate::ai::save_api_key(base, "mock-visible-key").unwrap();
    app.open_settings();
    app.settings_show_page(SettingsPage::Ai);
    app.settings.motion = None;
    paint(&mut app);
    show_ai_setting(&mut app, Setting::AiApiKey);
    click_part(&mut app, SettingsPart::AiKeyVisibility);
    let (_, field) = app.settings.editing.as_ref().unwrap();
    assert_eq!(field.text(), "mock-visible-key");
    assert!(!field.is_masked());
    click_part(&mut app, SettingsPart::AiKeyVisibility);
    assert!(app.settings.editing.as_ref().unwrap().1.is_masked());
    app.edit_field(|f| {
        f.set_text("mock-entered-key");
        true
    });
    click_part(&mut app, SettingsPart::AiKeyVisibility);
    assert_eq!(
        app.settings.editing.as_ref().unwrap().1.text(),
        "mock-entered-key"
    );
    assert!(!app.settings.editing.as_ref().unwrap().1.is_masked());
    app.settings_show_page(SettingsPage::Appearance);
    assert!(app.settings.editing.is_none());
    assert_eq!(
        crate::ai::load_api_key(base).unwrap().as_deref(),
        Some("mock-visible-key")
    );
    app.settings_show_page(SettingsPage::Ai);
    app.settings_start_editing(Setting::AiApiKey);
    assert!(app.settings.editing.as_ref().unwrap().1.is_masked());
    let mut changed = app.config.clone();
    changed.ai_base_url = Some("https://new-visibility-fixture.example".into());
    app.adopt_config(changed);
    assert!(app.settings.editing.is_none());
    assert!(
        crate::ai::load_api_key("https://new-visibility-fixture.example")
            .unwrap()
            .is_none()
    );
    app.close_settings();
    assert!(app.settings.editing.is_none());
    assert!(!app.config.to_json().contains("mock-visible-key"));
}

#[test]
fn ai_prompt_shows_default_and_only_saved_multiline_edits_affect_generation() {
    let mut app = blank_app(false);
    app.config_path = Some(temp_config("ai-prompt"));
    app.open_settings();
    app.settings_show_page(SettingsPage::Ai);
    app.settings.motion = None;
    show_ai_setting(&mut app, Setting::AiPrompt);
    assert_eq!(app.ai_prompt_text(), crate::ai::DEFAULT_PROMPT);
    assert!(
        app.settings_layout()
            .row(Setting::AiPrompt)
            .unwrap()
            .control
            .h
            >= 220.0
    );
    ai_shot(&app, "winc-ai-prompt-default.png");
    click_part(&mut app, SettingsPart::Field(Setting::AiPrompt));
    app.edit_field(|f| {
        f.set_text("中文标题\n\n- 只总结实际改动");
        true
    });
    app.settings_leave_field();
    assert!(app.config.ai_prompt.is_none());
    assert!(app.ai_prompt_text().contains('\n'));
    click_part(&mut app, SettingsPart::AiPromptSave);
    assert_eq!(
        app.config.ai_prompt.as_deref(),
        Some("中文标题\n\n- 只总结实际改动")
    );
    click_part(&mut app, SettingsPart::Field(Setting::AiPrompt));
    app.edit_field(|f| {
        f.set_text(&"long prompt line\n".repeat(60));
        true
    });
    paint(&mut app);
    let area = app
        .settings_layout()
        .row(Setting::AiPrompt)
        .unwrap()
        .control;
    let page_scroll = app.settings.scroll;
    assert!(app.ai_settings_wheel(area.x + 20.0, area.y + 20.0, 50.0));
    assert_eq!(app.settings.scroll, page_scroll);
    click_part(&mut app, SettingsPart::AiPromptReset);
    assert!(app.config.ai_prompt.is_none());
    assert_eq!(app.ai_prompt_text(), crate::ai::DEFAULT_PROMPT);
    paint(&mut app);
    ai_shot(&app, "winc-ai-prompt-verified.png");
}

#[test]
fn ai_models_are_fetched_searched_added_and_saved_without_overwriting_manual_input() {
    let body = r#"{"data":[{"id":"Alpha-chat"},{"id":"Beta-code"},{"id":"Beta-fast"}]}"#;
    let (url, handle) = crate::ai::tests::server(200, body.into(), std::time::Duration::ZERO);
    let mut app = blank_app(false);
    app.config.ai_base_url = Some(url.clone());
    crate::ai::save_api_key(&url, "mock-model-ui-key").unwrap();
    app.open_settings();
    app.settings_show_page(SettingsPage::Ai);
    app.settings.motion = None;
    show_ai_setting(&mut app, Setting::AiModels);
    click_part(&mut app, SettingsPart::AiFetchModels);
    let request = handle.join().unwrap();
    assert!(request.starts_with("GET /v1/models "));
    assert_eq!(app.settings.ai.models.len(), 3);
    assert!(!app.ai_summary.is_busy());
    assert_eq!(
        app.settings.ai.resolved.as_ref().unwrap().1,
        format!("{url}/chat/completions")
    );
    click_part(&mut app, SettingsPart::Field(Setting::AiModels));
    app.edit_field(|f| f.insert("BETA"));
    assert_eq!(app.ai_model_options(), ["Beta-code", "Beta-fast"]);
    paint(&mut app);
    click_part(&mut app, SettingsPart::AiChooseModel(1));
    assert_eq!(app.config.ai_model.as_deref(), Some("Beta-fast"));
    assert_eq!(app.config.ai_models, ["Beta-fast"]);
    let (restored, errors) = winc_config::Config::parse(&app.config.to_json());
    assert!(errors.is_empty());
    assert_eq!(restored.ai_models, ["Beta-fast"]);
    assert_eq!(app.ai_model_options()[0], "Beta-fast");
    paint(&mut app);
    ai_shot(&app, "winc-ai-models-verified.png");
    app.settings
        .ai
        .models
        .extend((0..40).map(|i| format!("Beta-{i:02}")));
    // 选完模型会收起面板（ai_choose_model）；滚动断言针对展开态，这里重新展开。
    app.settings.ai.models_open = true;
    let layout = app.settings_layout();
    let parts = crate::layout::AiRowParts::new(layout.row(Setting::AiModels).unwrap());
    let old_scroll = app.settings.scroll;
    assert!(app.ai_settings_wheel(parts.list.x + 10.0, parts.list.y + 10.0, -100.0));
    assert!(app.settings.ai.model_scroll > 0.0);
    assert_eq!(app.settings.scroll, old_scroll);
}

#[test]
fn ai_settings_edit_and_mask_credentials_without_storing_them_in_config() {
    let mut app = blank_app(false);
    app.open_settings();
    app.settings_show_page(SettingsPage::Ai);
    app.settings.motion = None;
    paint(&mut app);
    let layout = app.settings_layout();
    for setting in [
        Setting::AiBaseUrl,
        Setting::AiModel,
        Setting::AiApiKey,
        Setting::AiPrompt,
        Setting::AiTest,
    ] {
        assert!(layout.row(setting).is_some());
    }
    click_part(&mut app, SettingsPart::Field(Setting::AiBaseUrl));
    app.edit_field(|f| f.insert("https://settings-fixture.example/v1"));
    app.settings_commit_field();
    assert_eq!(
        app.config.ai_base_url.as_deref(),
        Some("https://settings-fixture.example/v1")
    );
    click_part(&mut app, SettingsPart::Field(Setting::AiModel));
    app.edit_field(|f| f.insert("custom-model"));
    app.settings_commit_field();
    assert_eq!(app.config.ai_model.as_deref(), Some("custom-model"));
    show_ai_setting(&mut app, Setting::AiApiKey);
    click_part(&mut app, SettingsPart::Field(Setting::AiApiKey));
    app.edit_field(|f| f.insert("mock-ui-credential"));
    paint(&mut app);
    assert!(app.settings.editing.as_ref().unwrap().1.is_masked());
    app.settings_commit_field();
    assert_eq!(
        crate::ai::load_api_key("https://settings-fixture.example/v1")
            .unwrap()
            .as_deref(),
        Some("mock-ui-credential")
    );
    assert!(!app.config.to_json().contains("mock-ui-credential"));
    // 已保存的密钥可重新查看，失焦和 Esc 不改变原凭据。
    app.settings_start_editing(Setting::AiApiKey);
    assert_eq!(
        app.settings.editing.as_ref().unwrap().1.text(),
        "mock-ui-credential"
    );
    app.settings_leave_field();
    assert!(
        crate::ai::load_api_key("https://settings-fixture.example/v1")
            .unwrap()
            .is_some()
    );
    app.settings_start_editing(Setting::AiApiKey);
    app.edit_field(|f| {
        f.set_text("");
        true
    });
    app.settings_commit_field();
    assert!(
        crate::ai::load_api_key("https://settings-fixture.example/v1")
            .unwrap()
            .is_none()
    );
    paint(&mut app);
    let shot = std::env::temp_dir().join("winc-ai-settings-verified.png");
    write_png(&shot, app.renderer.as_ref().unwrap().image().unwrap()).unwrap();
}

#[test]
fn ai_generation_button_is_above_the_message_without_overlapping_other_controls() {
    let mut app = blank_app(false);
    app.git.root = Some(PathBuf::from("C:/fixture"));
    app.git.status = Some(winc_editor::git::Status {
        entries: vec![winc_editor::git::Entry {
            path: PathBuf::from("C:/fixture/a.txt"),
            from: None,
            staged: None,
            unstaged: Some(winc_editor::git::Change::Modified),
            is_dir: false,
        }],
        ..Default::default()
    });
    app.sidebar_view = SidebarView::Commit;
    app.panel_visible = true;
    paint(&mut app);
    let parts = app.commit_parts();
    assert!(parts.ai.bottom() <= parts.message.y);
    assert!(parts.message.bottom() <= parts.commit.y);
    assert!(parts.list.bottom() <= parts.card.unwrap().y);
    assert!(
        app.commit_hits
            .iter()
            .any(|(_, target)| *target == CommitTarget::AiGenerate)
    );
    let shot = std::env::temp_dir().join("winc-ai-commit-verified.png");
    write_png(&shot, app.renderer.as_ref().unwrap().image().unwrap()).unwrap();
}
