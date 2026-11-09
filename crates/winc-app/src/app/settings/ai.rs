//! AI 设置交互：显式保存提示词、密钥显隐以及可搜索的常用模型。

use super::*;

#[derive(Debug)]
pub(crate) struct AiSettings {
    pub prompt: TextField,
    pub prompt_saved: bool,
    pub query: TextField,
    pub models_open: bool,
    pub models: Vec<String>,
    pub model_scroll: f32,
    pub model_hits: Vec<(Rect, usize)>,
    pub model_status: Option<String>,
    pub model_error: bool,
    /// 仅缓存当前地址的密钥配置状态，不保留密钥内容。
    pub key_status: Option<(String, Result<bool, String>)>,
    /// 原始设置地址与实际通过测试的聊天地址。
    pub resolved: Option<(String, String)>,
    pub endpoint_error: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            prompt: TextField::multiline(),
            prompt_saved: false,
            query: TextField::default(),
            models_open: false,
            models: Vec::new(),
            model_scroll: 0.0,
            model_hits: Vec::new(),
            model_status: None,
            model_error: false,
            key_status: None,
            resolved: None,
            endpoint_error: None,
        }
    }
}

impl App {
    pub(crate) fn ai_key_configured(&mut self) -> Result<bool, String> {
        let Some(base) = self.config.ai_base_url.as_deref().filter(|s| !s.is_empty()) else {
            return Ok(false);
        };
        if let Some((address, status)) = &self.settings.ai.key_status
            && address == base
        {
            return status.clone();
        }
        let status = crate::ai::load_api_key(base).map(|key| key.is_some());
        self.settings.ai.key_status = Some((base.to_owned(), status.clone()));
        status
    }

    pub(in crate::app) fn reset_ai_endpoint(&mut self) {
        self.ai_summary.cancel();
        if matches!(self.settings.editing, Some((Setting::AiApiKey, _))) {
            self.settings_cancel_field();
        }
        self.settings.ai.resolved = None;
        self.settings.ai.key_status = None;
        self.settings.ai.endpoint_error = None;
        self.settings.ai.models.clear();
        self.settings.ai.model_hits.clear();
        self.settings.ai.model_status = None;
        self.settings.ai.model_scroll = 0.0;
    }

    pub(in crate::app) fn reset_ai_prompt(&mut self) {
        self.settings.ai.prompt_saved = false;
        let mut field = TextField::multiline();
        field.set_text(
            self.config
                .ai_prompt
                .as_deref()
                .unwrap_or(crate::ai::DEFAULT_PROMPT),
        );
        field.set_caret(0, false);
        self.settings.ai.prompt = field;
    }

    pub(crate) fn ai_prompt_text(&self) -> &str {
        match &self.settings.editing {
            Some((Setting::AiPrompt, field)) => field.text(),
            _ => self.settings.ai.prompt.text(),
        }
    }

    pub(crate) fn ai_model_options(&self) -> Vec<String> {
        let query = match &self.settings.editing {
            Some((Setting::AiModels, field)) => field.text(),
            _ => self.settings.ai.query.text(),
        }
        .trim()
        .to_lowercase();
        let mut models = self.config.ai_models.clone();
        if let Some(current) = &self.config.ai_model {
            models.push(current.clone());
        }
        models.extend(self.settings.ai.models.iter().cloned());
        models.sort();
        models.dedup();
        models.retain(|m| m.to_lowercase().contains(&query));
        // 常用模型优先；同组内保持名称排序。
        models.sort_by_key(|m| !self.config.ai_models.contains(m));
        models
    }

    pub(in crate::app) fn ai_choose_model(&mut self, index: usize) {
        let Some(model) = self.ai_model_options().get(index).cloned() else {
            return;
        };
        self.settings_leave_field();
        if !self.config.ai_models.contains(&model) {
            self.config.ai_models.push(model.clone());
        }
        self.config.ai_model = Some(model);
        self.settings.ai.models_open = false;
        self.settings.ai.query.set_text("");
        self.settings.selected = Setting::AiModel;
        self.settings.scroll = self
            .settings
            .scroll
            .min(self.settings_layout().max_scroll());
        self.save_config();
        self.request_redraw();
    }

    pub(in crate::app) fn toggle_ai_models(&mut self) {
        self.settings_leave_field();
        self.settings.ai.models_open = !self.settings.ai.models_open;
        self.settings.ai.model_hits.clear();
        self.settings.ai.model_scroll = 0.0;
        if self.settings.ai.models_open {
            self.settings.ai.query.set_text("");
            self.settings_start_editing(Setting::AiModels);
        } else {
            self.settings.selected = Setting::AiModel;
            self.settings.scroll = self
                .settings
                .scroll
                .min(self.settings_layout().max_scroll());
        }
        self.request_redraw();
    }

    pub(in crate::app) fn toggle_ai_key_visibility(&mut self) {
        if !matches!(self.settings.editing, Some((Setting::AiApiKey, _))) {
            self.settings_start_editing(Setting::AiApiKey);
        }
        if let Some((Setting::AiApiKey, field)) = &mut self.settings.editing {
            field.set_masked(!field.is_masked());
            self.focus = Focus::Setting;
            self.reset_blink();
            self.request_redraw();
        }
    }

    pub(in crate::app) fn save_ai_prompt(&mut self) {
        self.settings.ai.prompt_saved = false;
        let text = self.ai_prompt_text().to_owned();
        if text.len() > crate::ai::MAX_PROMPT_BYTES {
            self.settings.error = Some((
                Setting::AiPrompt,
                t!("ai-client-request-too-large").to_owned(),
            ));
            self.request_redraw();
            return;
        }
        let value = (!text.trim().is_empty() && text != crate::ai::DEFAULT_PROMPT).then_some(text);
        let original = std::mem::replace(&mut self.config.ai_prompt, value);
        if let Err(error) = self.write_config(false) {
            self.config.ai_prompt = original;
            self.settings.error = Some((Setting::AiPrompt, error));
            self.request_redraw();
            return;
        }
        if matches!(self.settings.editing, Some((Setting::AiPrompt, _))) {
            self.settings.editing = None;
            self.settings.editing_original = None;
            self.focus = Focus::Settings;
        }
        self.settings.error = None;
        self.reset_ai_prompt();
        self.settings.ai.prompt_saved = true;
        self.request_redraw();
    }

    pub(in crate::app) fn restore_ai_prompt(&mut self) {
        if matches!(self.settings.editing, Some((Setting::AiPrompt, _))) {
            self.settings.editing = None;
            self.focus = Focus::Settings;
        }
        let original_config = self.config.ai_prompt.take();
        let original_draft =
            std::mem::replace(&mut self.settings.ai.prompt, TextField::multiline());
        self.reset_ai_prompt();
        if let Err(error) = self.write_config(true) {
            // 写盘失败时整体回到重置前：内存与磁盘必须一致，否则旧提示词会在下次启动复活。
            self.config.ai_prompt = original_config;
            self.settings.ai.prompt = original_draft;
            self.settings.error = Some((Setting::AiPrompt, error));
            self.request_redraw();
            return;
        }
        self.settings.error = None;
        self.request_redraw();
    }

    /// 输入区内优先滚动长提示词或模型列表，其他位置仍滚动整页。
    pub(in crate::app) fn ai_settings_wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        let layout = self.settings_layout();
        if let Some(row) = layout.row(Setting::AiPrompt)
            && row.control.contains(x, y)
        {
            let field = match &mut self.settings.editing {
                Some((Setting::AiPrompt, field)) => field,
                _ => &mut self.settings.ai.prompt,
            };
            let max = field.view.rows.last().map_or(0.0, |row| {
                (*row + 1) as f32 * field.view.line_height - field.view.area.h
            });
            if max > 0.0 {
                field.view.scroll = (field.view.scroll - dy).clamp(0.0, max);
                self.request_redraw();
                return true;
            }
        }
        if let Some(row) = layout.row(Setting::AiModels) {
            let list = crate::layout::AiRowParts::new(row).list;
            if list.contains(x, y) {
                let max = (self.ai_model_options().len() as f32 * 32.0 - list.h).max(0.0);
                if max > 0.0 {
                    self.settings.ai.model_scroll =
                        (self.settings.ai.model_scroll - dy).clamp(0.0, max);
                    self.request_redraw();
                    return true;
                }
            }
        }
        false
    }
}
