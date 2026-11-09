//! AI 设置使用宽输入区、可滚动模型列表及多行提示词。

use super::*;
use crate::layout::AiRowParts;

pub(super) fn draw_row(
    app: &mut App,
    frame: &mut Frame<'_>,
    row: &SettingRow,
    clip: Rect,
) -> Option<Rect> {
    let p = app.palette;
    let parts = AiRowParts::new(row);
    let x = parts.field.x;
    let title = Rect::new(x, row.row.y + 7.0, parts.status.w, 24.0);
    if row.setting != Setting::AiModels {
        ui_text(frame, app, x, title, row.setting.label(), p.text, clip);
    }
    if row.setting != Setting::AiModels
        && let Some(text) = row.setting.description()
    {
        let width = parts.status.w;
        let lines = crate::layout::wrap_text(&text, width, &mut |s: &str| {
            frame.measure(FontKind::Ui, app.fonts.ui_small, s)
        });
        for (i, line) in lines.iter().enumerate() {
            let at = Rect::new(x, title.bottom() + i as f32 * 18.0, width, 18.0);
            ui_text_small(frame, app, x, at, line, p.text_muted, clip);
        }
    }
    let editing = matches!(&app.settings.editing, Some((s, _)) if *s == row.setting);
    let key_configured = if row.setting == Setting::AiApiKey && !editing {
        match app.ai_key_configured() {
            Ok(configured) => configured,
            Err(error) => {
                app.settings.error = Some((Setting::AiApiKey, error));
                false
            }
        }
    } else {
        false
    };
    let focused = editing && app.focus == Focus::Setting;
    let error = app
        .settings
        .error
        .as_ref()
        .filter(|(s, _)| *s == row.setting)
        .map(|(_, e)| e.clone());
    let ring = if error.is_some() {
        p.error
    } else if focused {
        p.accent
    } else {
        p.border
    };
    frame.rect_clipped(parts.field.inset(-1.0, -1.0), ring, RADIUS_ROW + 1.0, clip);
    frame.rect_clipped(parts.field, p.editor_bg, RADIUS_ROW, clip);

    let mut field = if editing {
        app.settings.editing.take().unwrap().1
    } else {
        match row.setting {
            Setting::AiPrompt => {
                std::mem::replace(&mut app.settings.ai.prompt, TextField::multiline())
            }
            Setting::AiModels => std::mem::take(&mut app.settings.ai.query),
            _ => {
                let mut field = TextField::default();
                field.set_text(if key_configured {
                    "••••••••"
                } else {
                    app.ai_setting_value(row.setting).unwrap_or("")
                });
                field.set_caret(0, false);
                field
            }
        }
    };
    let placeholder = match row.setting {
        Setting::AiBaseUrl => t!("settings-ai-base-url-placeholder"),
        Setting::AiModel => t!("settings-ai-model-placeholder"),
        Setting::AiModels => t!("settings-ai-model-search"),
        Setting::AiApiKey => t!("settings-ai-api-key-placeholder"),
        _ => "",
    };
    let caret = if row.setting == Setting::AiPrompt {
        let area = parts.field.inset(10.0, 8.0);
        let wrap = crate::view::wrap_area_field(app, frame, area.w, &mut field, focused);
        let caret = frame.within(parts.field.intersect(clip), |frame| {
            crate::view::draw_area_field(app, frame, area, &mut field, wrap, "", focused)
        });
        let total = field
            .view
            .rows
            .last()
            .map_or(0.0, |row| (*row + 1) as f32 * field.view.line_height);
        if let Some(thumb) = crate::layout::scrollbar_thumb(area, field.view.scroll, total) {
            frame.rect_clipped(
                Rect::new(parts.field.right() - 5.0, thumb.y, 3.0, thumb.h),
                p.scrollbar,
                1.5,
                clip.intersect(parts.field),
            );
        }
        caret
    } else {
        crate::view::draw_bar_field(
            app,
            frame,
            parts.field.inset(9.0, 0.0),
            &mut field,
            placeholder,
            focused,
        )
    };
    if editing {
        app.settings.editing = Some((row.setting, field));
    } else if row.setting == Setting::AiPrompt {
        app.settings.ai.prompt = field;
    } else if row.setting == Setting::AiModels {
        app.settings.ai.query = field;
    }

    match row.setting {
        Setting::AiModel => {
            draw_button(
                frame,
                p,
                parts.action,
                clip,
                app.settings.hover == Some(SettingsPart::AiToggleModels),
                false,
            );
            icons::chevron_turned(
                frame,
                center(parts.action),
                7.0,
                if app.settings.ai.models_open {
                    -std::f32::consts::FRAC_PI_2
                } else {
                    std::f32::consts::FRAC_PI_2
                },
                p.text_muted,
                clip,
            );
        }
        Setting::AiApiKey => {
            let hovered = app.settings.hover == Some(SettingsPart::AiKeyVisibility);
            draw_button(frame, p, parts.action, clip, hovered, false);
            let visible =
                matches!(&app.settings.editing, Some((Setting::AiApiKey, f)) if !f.is_masked());
            icons::key_visibility(frame, center(parts.action), visible, p.text_muted, clip);
        }
        Setting::AiBaseUrl => {
            let raw = app.config.ai_base_url.as_deref().unwrap_or("");
            let resolved = app
                .settings
                .ai
                .resolved
                .as_ref()
                .filter(|(base, _)| base == raw);
            let status = if let Some(error) = &app.settings.ai.endpoint_error {
                error.clone()
            } else {
                match resolved {
                    Some((_, endpoint)) => t!("settings-ai-endpoint-resolved", endpoint = endpoint),
                    None if !raw.is_empty() => match crate::ai::endpoint_candidates(raw) {
                        Ok(candidates) => {
                            t!("settings-ai-endpoint-pending", endpoint = &candidates[0])
                        }
                        Err(_) => String::new(),
                    },
                    _ => t!("settings-ai-endpoint-hint").to_owned(),
                }
            };
            status_text(
                app,
                frame,
                parts.status,
                &status,
                if app.settings.ai.endpoint_error.is_some() {
                    p.error
                } else {
                    p.text_muted
                },
                clip,
            );
        }
        Setting::AiModels => {
            button(
                app,
                frame,
                parts.action,
                SettingsPart::AiFetchModels,
                if app.ai_summary.is_busy() {
                    t!("settings-ai-models-loading")
                } else {
                    t!("settings-ai-models-fetch")
                },
                clip,
            );
            draw_models(app, frame, &parts, clip);
        }
        Setting::AiPrompt => {
            button(
                app,
                frame,
                parts.action,
                SettingsPart::AiPromptSave,
                t!("settings-ai-prompt-save"),
                clip,
            );
            button(
                app,
                frame,
                parts.secondary,
                SettingsPart::AiPromptReset,
                t!("settings-ai-prompt-reset"),
                clip,
            );
            let saved = app
                .config
                .ai_prompt
                .as_deref()
                .unwrap_or(crate::ai::DEFAULT_PROMPT);
            let status = if let Some(error) = &error {
                error.as_str()
            } else if app.ai_prompt_text() != saved {
                t!("settings-ai-prompt-unsaved")
            } else if app.settings.ai.prompt_saved {
                t!("settings-ai-prompt-saved")
            } else if app.config.ai_prompt.is_some() {
                t!("settings-ai-prompt-custom")
            } else {
                t!("settings-ai-prompt-default")
            };
            let room = Rect::new(
                parts.status.x,
                parts.status.y,
                (parts.secondary.x - parts.status.x - 8.0).max(0.0),
                parts.status.h,
            );
            let color = if error.is_some() {
                p.error
            } else if app.settings.ai.prompt_saved {
                p.accent
            } else {
                p.text_muted
            };
            status_text(app, frame, room, status, color, clip);
        }
        _ => {}
    }
    if row.setting != Setting::AiPrompt
        && let Some(error) = error
    {
        // 错误替换说明第一行；行高与说明的 18px 对齐，避免裁切第二行。
        let status = Rect::new(x, row.row.y + 31.0, parts.status.w, 18.0);
        frame.rect_clipped(status, p.group_bg, 0.0, clip);
        status_text(app, frame, status, &error, p.error, clip);
    }
    caret
}

fn button(
    app: &App,
    frame: &mut Frame<'_>,
    area: Rect,
    part: SettingsPart,
    text: &str,
    clip: Rect,
) {
    let p = app.palette;
    frame.rect_clipped(
        area,
        if app.settings.hover == Some(part) {
            p.hover
        } else {
            p.frame_bg
        },
        RADIUS_ROW,
        clip,
    );
    let text = ellipsize(frame, text, app.fonts.ui_small, area.w - 12.0);
    let width = frame.measure(FontKind::Ui, app.fonts.ui_small, &text);
    ui_text_small(
        frame,
        app,
        area.x + (area.w - width) / 2.0,
        area,
        &text,
        p.text_muted,
        clip,
    );
}

fn status_text(app: &App, frame: &mut Frame<'_>, area: Rect, text: &str, color: Color, clip: Rect) {
    let text = ellipsize(frame, text, app.fonts.ui_small, area.w);
    ui_text_small(frame, app, area.x, area, &text, color, clip.intersect(area));
}

fn draw_models(app: &mut App, frame: &mut Frame<'_>, parts: &AiRowParts, clip: Rect) {
    let p = app.palette;
    let models = app.ai_model_options();
    let list = parts.list;
    frame.rect_clipped(list, p.editor_bg, RADIUS_ROW, clip);
    let max = (models.len() as f32 * 32.0 - list.h).max(0.0);
    app.settings.ai.model_scroll = app.settings.ai.model_scroll.clamp(0.0, max);
    for (index, model) in models.iter().enumerate() {
        let row = Rect::new(
            list.x + 4.0,
            list.y + index as f32 * 32.0 - app.settings.ai.model_scroll,
            list.w - 8.0,
            32.0,
        );
        if row.bottom() <= list.y || row.y >= list.bottom() {
            continue;
        }
        let current = app.config.ai_model.as_ref() == Some(model);
        let favorite = app.config.ai_models.contains(model);
        if current {
            frame.rect_clipped(row, p.selected_inactive, RADIUS_ROW, clip.intersect(list));
        }
        let action = Rect::new(row.right() - 108.0, row.y + 2.0, 108.0, 28.0);
        let label = if current {
            t!("settings-ai-model-current")
        } else if favorite {
            t!("settings-ai-model-use")
        } else {
            t!("settings-ai-model-add")
        };
        button(
            app,
            frame,
            action,
            SettingsPart::AiChooseModel(index),
            label,
            clip.intersect(list),
        );
        let text = ellipsize(
            frame,
            model,
            app.fonts.ui,
            (action.x - row.x - 14.0).max(0.0),
        );
        ui_text(
            frame,
            app,
            row.x + 8.0,
            row,
            &text,
            p.text,
            clip.intersect(list),
        );
        app.settings
            .ai
            .model_hits
            .push((row.intersect(list).intersect(clip), index));
    }
    if models.is_empty() {
        status_text(
            app,
            frame,
            list.inset(12.0, 8.0),
            t!("settings-ai-models-empty"),
            p.text_dim,
            clip,
        );
    }
    if let Some(thumb) = crate::layout::scrollbar_thumb(
        list,
        app.settings.ai.model_scroll,
        models.len() as f32 * 32.0,
    ) {
        frame.rect_clipped(
            Rect::new(list.right() - 4.0, thumb.y, 3.0, thumb.h),
            p.scrollbar,
            1.5,
            clip.intersect(list),
        );
    }
    let status = app
        .settings
        .ai
        .model_status
        .as_deref()
        .unwrap_or(t!("settings-ai-models-manual"));
    status_text(
        app,
        frame,
        parts.status,
        status,
        if app.settings.ai.model_error {
            p.error
        } else {
            p.text_muted
        },
        clip,
    );
}
