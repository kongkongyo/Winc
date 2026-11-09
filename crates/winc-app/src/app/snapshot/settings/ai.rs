//! 个人 AI 设置：字段编辑、凭据遮罩、取消，以及分类与搜索中的可发现性。

use super::*;
use crate::layout::ToolLanguages;

#[test]
fn ai_settings_page_has_editable_fields_and_connection_test() {
    let mut app = blank_app(false);
    app.config_path = Some(temp_config("ai-settings"));
    app.open_settings();
    app.settings_show_page(SettingsPage::Ai);
    paint(&mut app);
    let layout = app.settings_layout();
    assert_eq!(layout.page, SettingsPage::Ai);
    let settings: Vec<_> = layout.rows.iter().map(|row| row.setting).collect();
    for setting in [
        Setting::AiBaseUrl,
        Setting::AiModel,
        Setting::AiApiKey,
        Setting::AiPrompt,
        Setting::AiTest,
    ] {
        assert!(settings.contains(&setting));
    }
    assert_eq!(Setting::AiTest.control(), SettingControl::Button);
    // 搜索词取自设置项文案本身，跨语言稳定；厂商词（openai）不在文案中，是否支持由文案决定。
    let term: String = Setting::AiBaseUrl.label().chars().take(2).collect();
    assert!(Setting::found(&term, &ToolLanguages::default(), false).contains(&Setting::AiBaseUrl));

    // 与真实点击相同的分派入口，不能落入字体选择或格式化工具菜单。
    app.settings_open_menu(Setting::AiBaseUrl);
    app.settings
        .editing
        .as_mut()
        .unwrap()
        .1
        .set_text("https://relay.example/v1");
    app.settings_commit_field();
    assert_eq!(
        app.config.ai_base_url.as_deref(),
        Some("https://relay.example/v1")
    );
    assert_eq!(app.config.editor_font, None);
    app.settings_open_menu(Setting::AiModel);
    app.settings
        .editing
        .as_mut()
        .unwrap()
        .1
        .set_text("custom-model");
    app.settings_leave_field();
    assert_eq!(app.config.ai_model.as_deref(), Some("custom-model"));
    app.settings_start_editing(Setting::AiPrompt);
    app.settings
        .editing
        .as_mut()
        .unwrap()
        .1
        .set_text("仅总结选中的变更");
    app.settings_commit_field();
    assert_eq!(app.config.ai_prompt.as_deref(), Some("仅总结选中的变更"));
    paint(&mut app);
}

#[test]
fn api_key_editing_is_masked_and_leaving_never_saves_or_clears_it() {
    let mut app = blank_app(false);
    app.open_settings();
    app.settings_show_page(SettingsPage::Ai);
    app.settings_start_editing(Setting::AiApiKey);
    let field = &mut app.settings.editing.as_mut().unwrap().1;
    assert!(field.is_masked());
    assert!(field.text().is_empty(), "不从系统凭据自动读回密钥");
    field.set_text("test-only-not-a-real-key");
    app.settings_leave_field();
    assert!(app.settings.editing.is_none());
    assert!(!app.config.to_json().contains("test-only-not-a-real-key"));
    assert!(app.settings.error.is_none());
    app.settings_start_editing(Setting::AiApiKey);
    app.settings_show_page(SettingsPage::Appearance);
    assert!(app.settings.editing.is_none());
    assert!(app.settings.error.is_none(), "离开初始空框不能尝试清除凭据");
}

#[test]
fn canceling_ai_setting_edits_preserves_saved_values() {
    let mut app = blank_app(false);
    app.config.ai_model = Some("saved-model".to_owned());
    app.open_settings();
    app.settings_start_editing(Setting::AiModel);
    app.settings
        .editing
        .as_mut()
        .unwrap()
        .1
        .set_text("unsaved-model");
    app.settings_cancel_field();
    assert_eq!(app.config.ai_model.as_deref(), Some("saved-model"));
    assert_eq!(app.config.editor_font, None);
    app.settings_start_editing(Setting::AiPrompt);
    app.settings
        .editing
        .as_mut()
        .unwrap()
        .1
        .set_text("unsaved prompt");
    app.close_settings();
    assert_eq!(app.config.ai_prompt, None);
}
