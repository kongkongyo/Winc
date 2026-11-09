//! AI 生成与结果归属：后台读取和请求，回填前核对仓库、勾选及实际差异。

use winc_editor::git::CommitSummary;
use winc_editor::shell::Cancel;

use super::*;

#[derive(Clone, PartialEq, Eq)]
struct Scope {
    project: Option<PathBuf>,
    root: PathBuf,
    oid: Option<String>,
    entries: Vec<Entry>,
    skips: HashMap<PathBuf, Vec<Skip>>,
    checks: u64,
    amend: bool,
}

struct Running {
    id: u64,
    cancel: Cancel,
    generating: bool,
}

struct Preview {
    id: u64,
    scope: Scope,
    input: CommitSummary,
    message: String,
}

enum ResultEvent {
    Generated(u64, Scope, String, Result<(CommitSummary, String), String>),
    Verified(Preview, String, Result<(), String>),
    Tested(u64, String, Result<String, String>),
    Models(u64, String, Result<crate::ai::ModelCatalog, String>),
}

/// 一个窗口同时只运行一个 AI 操作，密钥不保存在界面状态中。
pub(crate) struct AiSummary {
    sequence: u64,
    running: Option<Running>,
    preview: Option<Preview>,
    results: (Sender<ResultEvent>, Receiver<ResultEvent>),
}

impl Default for AiSummary {
    fn default() -> Self {
        Self {
            sequence: 0,
            running: None,
            preview: None,
            results: channel(),
        }
    }
}

impl AiSummary {
    /// 请求正在准备、发送或复核。
    pub(crate) fn is_busy(&self) -> bool {
        self.running.is_some()
    }

    pub(crate) fn is_generating(&self) -> bool {
        self.running.as_ref().is_some_and(|r| r.generating)
    }

    /// 还有待用户明确采用的预览。
    pub(crate) fn preview_id(&self) -> Option<u64> {
        self.preview.as_ref().map(|p| p.id)
    }

    /// 切换仓库时取消后台请求，使已弹出的预览也失效。
    pub(crate) fn cancel(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.cancel();
        }
        self.preview = None;
    }

    fn begin(&mut self) -> (u64, Cancel) {
        self.begin_operation(true)
    }

    fn begin_operation(&mut self, generating: bool) -> (u64, Cancel) {
        self.preview = None;
        self.sequence += 1;
        let cancel = Cancel::default();
        self.running = Some(Running {
            id: self.sequence,
            cancel: cancel.clone(),
            generating,
        });
        (self.sequence, cancel)
    }

    fn finish(&mut self, id: u64) -> bool {
        if !self.running.as_ref().is_some_and(|r| r.id == id) {
            return false;
        }
        self.running = None;
        true
    }
}

impl Drop for AiSummary {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl App {
    pub(in crate::app) fn stop_ai_summary(&mut self) {
        if !self.ai_summary.is_generating() {
            return;
        }
        self.ai_summary.cancel();
        self.flash(t!("ai-summary-stopped"));
        self.request_redraw();
    }

    pub(in crate::app) fn clear_commit_message(&mut self) {
        if self.ai_summary.is_busy() || self.commit.busy || self.commit.message.text().is_empty() {
            return;
        }
        self.ai_summary.cancel();
        self.commit.message.set_text("");
        self.restore_commit_message_height();
        self.field_preedit.clear();
        self.focus = Focus::CommitMessage;
        self.reset_blink();
        self.request_redraw();
    }

    fn ai_settings(&self) -> Result<crate::ai::Settings, String> {
        let settings = crate::ai::Settings {
            base_url: self.config.ai_base_url.clone().unwrap_or_default(),
            model: self.config.ai_model.clone().unwrap_or_default(),
            prompt: self.config.ai_prompt.clone().unwrap_or_default(),
        };
        crate::ai::endpoint(&settings.base_url)?;
        if settings.model.trim().is_empty() {
            return Err(t!("ai-client-model-required").to_owned());
        }
        Ok(settings)
    }

    fn ai_scope(&self) -> Option<Scope> {
        let status = self.git.status.as_ref()?;
        Some(Scope {
            project: self.editor.root().map(Path::to_path_buf),
            root: self.git.root.clone()?,
            oid: status.oid.clone(),
            entries: self.commit.checked_entries(status).cloned().collect(),
            skips: self.commit.skipped.clone(),
            checks: self.commit.checks(),
            amend: self.commit.amend,
        })
    }

    fn ai_dirty(&self, scope: &Scope) -> bool {
        scope.entries.iter().any(|entry| {
            let path = winc_editor::canonical_path(&entry.path);
            self.editor
                .documents
                .iter()
                .any(|d| d.path.as_deref() == Some(path.as_path()) && d.is_dirty())
        })
    }

    fn ai_scope_current(&self, scope: &Scope) -> bool {
        !self.commit.busy && self.ai_scope().as_ref() == Some(scope) && !self.ai_dirty(scope)
    }

    /// 只在用户点击后调用，不自动保存、暂存或修改文件。
    pub(in crate::app) fn generate_ai_summary(&mut self) {
        if self.ai_summary.is_busy() || self.commit.busy {
            return;
        }
        let Some(scope) = self.ai_scope() else {
            return;
        };
        if scope.entries.is_empty() {
            self.flash(t!("git-commit-nothing-checked"));
            return;
        }
        if self.ai_dirty(&scope) {
            self.flash(t!("ai-summary-save-first"));
            return;
        }
        let settings = match self.ai_settings() {
            Ok(settings) => settings,
            Err(error) => {
                self.flash(error);
                return;
            }
        };
        let original = self.commit.message.text().to_owned();
        let (id, cancel) = self.ai_summary.begin();
        self.restore_commit_message_height();
        let sender = self.ai_summary.results.0.clone();
        self.request_redraw();
        self.spawn_git(UserEvent::AiSummary, move |tool| {
            let result = tool.as_ref().map_err(unusable).and_then(|git| {
                let input = prepare(git, &scope, &cancel)?;
                let key = crate::ai::load_api_key(&settings.base_url)?
                    .ok_or_else(|| t!("ai-client-key-required").to_owned())?;
                let message = crate::ai::generate(&settings, &key, &input.diff, &cancel)?;
                if prepare(git, &scope, &cancel)? != input {
                    return Err(t!("ai-summary-stale").to_owned());
                }
                Ok((input, message))
            });
            let _ = sender.send(ResultEvent::Generated(id, scope, original, result));
        });
    }

    /// 不带仓库内容测试中转站；也受同一个并发限制。
    pub(in crate::app) fn test_ai_connection(&mut self) {
        if self.ai_summary.is_busy() {
            return;
        }
        // 点击测试时先收下正在编辑的普通设置，密钥仍须由回车显式保存。
        self.settings_leave_field();
        self.settings.ai.endpoint_error = None;
        let settings = match self.ai_settings() {
            Ok(settings) => settings,
            Err(error) => {
                self.settings.ai.endpoint_error = Some(error.clone());
                self.flash(error);
                return;
            }
        };
        let (id, cancel) = self.ai_summary.begin_operation(false);
        let base = settings.base_url.clone();
        let sender = self.ai_summary.results.0.clone();
        self.flash(t!("ai-summary-testing"));
        self.spawn_blocking_then(UserEvent::AiSummary, move || {
            let result = crate::ai::load_api_key(&settings.base_url).and_then(|key| {
                let key = key.ok_or_else(|| t!("ai-client-key-required").to_owned())?;
                crate::ai::test_connection(&settings, &key, &cancel)
            });
            let _ = sender.send(ResultEvent::Tested(id, base, result));
        });
    }

    /// 模型列表通过后台 GET 获取，不需要先选模型，也不发送提示词或仓库内容。
    pub(in crate::app) fn fetch_ai_models(&mut self) {
        if self.ai_summary.is_busy() {
            return;
        }
        self.settings_leave_field();
        let base = self.config.ai_base_url.clone().unwrap_or_default();
        if let Err(error) = crate::ai::endpoint(&base) {
            self.settings.ai.model_status = Some(error);
            self.settings.ai.model_error = true;
            self.request_redraw();
            return;
        }
        let (id, cancel) = self.ai_summary.begin_operation(false);
        self.settings.ai.model_status = Some(t!("settings-ai-models-loading").to_owned());
        self.settings.ai.model_error = false;
        let sender = self.ai_summary.results.0.clone();
        self.spawn_blocking_then(UserEvent::AiSummary, move || {
            let result = crate::ai::load_api_key(&base).and_then(|key| {
                let key = key.ok_or_else(|| t!("ai-client-key-required").to_owned())?;
                crate::ai::fetch_models(&base, &key, &cancel)
            });
            let _ = sender.send(ResultEvent::Models(id, base, result));
        });
        self.request_redraw();
    }

    pub(in crate::app) fn poll_ai_summary(&mut self) {
        while let Ok(event) = self.ai_summary.results.1.try_recv() {
            match event {
                ResultEvent::Tested(id, base, result) => {
                    if self.ai_summary.finish(id)
                        && self.config.ai_base_url.as_deref() == Some(&base)
                    {
                        match result {
                            Ok(endpoint) => {
                                self.settings.ai.resolved = Some((base, endpoint));
                                self.settings.ai.endpoint_error = None;
                                self.flash(t!("ai-summary-test-ok"));
                            }
                            Err(error) => {
                                self.settings.ai.resolved = None;
                                self.settings.ai.endpoint_error = Some(error.clone());
                                self.flash(error);
                            }
                        }
                    }
                }
                ResultEvent::Models(id, base, result) => {
                    if !self.ai_summary.finish(id)
                        || self.config.ai_base_url.as_deref() != Some(&base)
                    {
                        continue;
                    }
                    match result {
                        Ok(catalog) => {
                            self.settings.ai.model_status =
                                Some(t!("settings-ai-models-count", count = catalog.models.len()));
                            self.settings.ai.model_error = false;
                            self.settings.ai.models = catalog.models;
                            self.settings.ai.model_scroll = 0.0;
                            self.settings.ai.resolved = Some((base, catalog.endpoint));
                        }
                        Err(error) => {
                            self.settings.ai.model_status = Some(error);
                            self.settings.ai.model_error = true;
                        }
                    }
                }
                ResultEvent::Generated(id, scope, original, result) => {
                    if !self.ai_summary.finish(id) {
                        continue;
                    }
                    if !self.ai_scope_current(&scope) {
                        self.flash(t!("ai-summary-stale"));
                        continue;
                    }
                    match result {
                        Err(error) => self.flash(error),
                        Ok((input, message)) => {
                            let preview = Preview {
                                id,
                                scope,
                                input,
                                message,
                            };
                            if original.is_empty() && self.commit.message.text().is_empty() {
                                // 后台复核与事件处理之间磁盘仍可能变化；与采用路径相同，
                                // 回填前再复核一次。验证期内用户有输入则转为预览，不覆盖。
                                let confirmed = String::new();
                                let cancel = Cancel::default();
                                self.ai_summary.running = Some(Running {
                                    id,
                                    cancel: cancel.clone(),
                                    generating: true,
                                });
                                let sender = self.ai_summary.results.0.clone();
                                self.spawn_git(UserEvent::AiSummary, move |tool| {
                                    let result = tool.as_ref().map_err(unusable).and_then(|git| {
                                        if prepare(git, &preview.scope, &cancel)? != preview.input {
                                            return Err(t!("ai-summary-stale").to_owned());
                                        }
                                        Ok(())
                                    });
                                    let _ = sender
                                        .send(ResultEvent::Verified(preview, confirmed, result));
                                });
                            } else {
                                self.preview_ai_summary(preview);
                            }
                        }
                    }
                }
                ResultEvent::Verified(preview, confirmed, result) => {
                    if !self.ai_summary.finish(preview.id) {
                        continue;
                    }
                    if !self.ai_scope_current(&preview.scope) {
                        self.flash(t!("ai-summary-stale"));
                        continue;
                    }
                    match result {
                        Err(error) => self.flash(error),
                        Ok(()) if self.commit.message.text() == confirmed => {
                            self.fill_ai_summary(&preview.message)
                        }
                        Ok(()) => self.preview_ai_summary(preview),
                    }
                }
            }
        }
        self.request_redraw();
    }

    fn preview_ai_summary(&mut self, preview: Preview) {
        // 现有提示框不支持长文滚动；复用只读输出标签完整展示，避免说明和按钮超出窗口。
        self.editor
            .show_output(t!("ai-summary-preview"), &preview.message);
        self.ai_summary.preview = Some(preview);
        self.focus = Focus::Editor;
        self.relayout();
        self.ensure_caret_visible();
        self.flash(t!("ai-summary-preview-hint"));
    }

    /// 用户点击替换后，再次只读复核磁盘；预览期间文件也可能改变。
    pub(in crate::app) fn use_ai_summary(&mut self, id: u64) {
        if self.ai_summary.is_busy() {
            return;
        }
        let Some(preview) = self.ai_summary.preview.take().filter(|p| p.id == id) else {
            return;
        };
        if !self.ai_scope_current(&preview.scope) {
            self.flash(t!("ai-summary-stale"));
            return;
        }
        let confirmed = self.commit.message.text().to_owned();
        let cancel = Cancel::default();
        self.ai_summary.running = Some(Running {
            id,
            cancel: cancel.clone(),
            generating: true,
        });
        let sender = self.ai_summary.results.0.clone();
        self.spawn_git(UserEvent::AiSummary, move |tool| {
            let result = tool.as_ref().map_err(unusable).and_then(|git| {
                if prepare(git, &preview.scope, &cancel)? != preview.input {
                    return Err(t!("ai-summary-stale").to_owned());
                }
                Ok(())
            });
            let _ = sender.send(ResultEvent::Verified(preview, confirmed, result));
        });
    }

    fn fill_ai_summary(&mut self, message: &str) {
        self.commit.message.set_text(message);
        self.focus = Focus::CommitMessage;
        self.flash(t!("ai-summary-ready"));
    }
}

fn prepare(git: &Git, scope: &Scope, cancel: &Cancel) -> Result<CommitSummary, String> {
    let mut partial = Vec::new();
    for entry in &scope.entries {
        let Some(skipped) = scope.skips.get(&entry.path).filter(|s| !s.is_empty()) else {
            continue;
        };
        let head = git
            .summary_head_content(&scope.root, &entry.path, 8 << 20, cancel)?
            .unwrap_or_default();
        // 有界读取与完整提交共用按行选择的函数，且不执行 Git 过滤器。
        let disk = git
            .summary_working_content(&scope.root, &entry.path)?
            .ok_or_else(|| t!("git-ai-diff-unreadable").to_owned())?;
        partial.push(actions::partial_from_bytes(
            &entry.path,
            head,
            disk,
            skipped,
        )?);
    }
    git.commit_summary_diff(
        &scope.root,
        &scope.entries,
        &partial,
        crate::ai::MAX_DIFF_BYTES,
        cancel,
    )
}

#[cfg(test)]
mod tests;
