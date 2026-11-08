//! 顶部项目标签：只展示选入当前窗口的目录，文件标签仍由下一行管理。

use super::*;

use crate::app::ProjectTabHit;
use crate::view::workspace::top_bar::{distinct_folder, draw_tab_close, draw_tab_fill, tab_close};

/// 项目标签中图标、关闭按钮和左右留白占据的宽度（逻辑像素）。
const CHROME: f32 = 10.0 + icons::FILE_WIDTH + 7.0 + 6.0 + 16.0 + 6.0;

/// 绘制项目标签和固定在右侧的添加按钮，记录这一帧的点击区域。
pub(in crate::view) fn draw_project_tabs(app: &mut App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let area = app.layout.projects;
    frame.rect(area, p.frame_bg);

    let button = Rect::new(
        (area.right() - BUTTON).max(area.x),
        area.y + (area.h - BUTTON) / 2.0,
        BUTTON.min(area.w),
        BUTTON,
    );
    let view = Rect::new(
        area.x,
        area.y,
        (button.x - TAB_GAP - area.x).max(0.0),
        area.h,
    );
    let shrunk = view.w < app.projects.view.w;
    let names: Vec<_> = app
        .projects
        .folders
        .iter()
        .map(|path| winc_editor::display_name(path).into_owned())
        .collect();
    let labels: Vec<_> = app
        .projects
        .folders
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let others: Vec<_> = app
                .projects
                .folders
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index && names[*i] == names[index])
                .map(|(_, path)| path.as_path())
                .collect();
            let parent = (!others.is_empty())
                .then(|| distinct_folder(path, &others))
                .flatten();
            let parent_width = parent.as_ref().map_or(0.0, |parent| {
                6.0 + frame.measure(FontKind::Ui, app.fonts.ui_small, parent)
            });
            let width =
                (CHROME + frame.measure(FontKind::Ui, app.fonts.ui, &names[index]) + parent_width)
                    .ceil()
                    .clamp(TAB_WIDTH_MIN, 240.0);
            (width, parent)
        })
        .collect();
    let content = labels.iter().map(|(width, _)| width + TAB_GAP).sum();
    let active = app.editor.root().map(std::path::Path::to_path_buf);
    let reveal = shrunk || app.projects.revealed != active;
    let tabs = &mut app.projects;
    tabs.view = view;
    tabs.new_tab = button;
    tabs.content = content;
    if reveal {
        let mut offset = 0.0;
        for (index, (width, _)) in labels.iter().enumerate() {
            if active.as_ref() == tabs.folders.get(index) {
                tabs.scroll = crate::layout::scroll_to_show(
                    tabs.scroll,
                    offset,
                    offset + width.min(view.w),
                    view.w,
                );
                break;
            }
            offset += width + TAB_GAP;
        }
        tabs.revealed = active.clone();
    }
    tabs.scroll = tabs.scroll.clamp(0.0, (content - view.w).max(0.0));
    let scroll = (tabs.scroll * frame.scale()).round() / frame.scale();
    let mut hits = std::mem::take(&mut tabs.hits);
    hits.clear();

    let mut left = view.x - scroll;
    for (index, (width, parent)) in labels.iter().enumerate() {
        let tab = Rect::new(
            left,
            area.y + (area.h - TAB_HEIGHT) / 2.0,
            *width,
            TAB_HEIGHT,
        );
        left += width + TAB_GAP;
        let visible = tab.intersect(view);
        if visible.w <= 0.0 {
            continue;
        }
        let selected = active.as_ref() == app.projects.folders.get(index);
        let hovered = matches!(app.hover, Hover::ProjectTab(i, _) if i == index);
        draw_tab_fill(frame, p, tab, selected, hovered, view);
        let icon = (tab.x + 10.0, icon_center_y(app, tab, app.fonts.ui) - 6.0);
        icons::folder(frame, icon, selected, p.folder, view);

        let close = tab_close(tab);
        let x = tab.x + 10.0 + icons::FILE_WIDTH + 7.0;
        let room = (close.x - 6.0 - x).max(0.0);
        let parent_width = parent.as_ref().map_or(0.0, |parent| {
            frame
                .measure(FontKind::Ui, app.fonts.ui_small, parent)
                .min(room * 0.4)
                + 6.0
        });
        let name = ellipsize(frame, &names[index], app.fonts.ui, room - parent_width);
        let color = if selected { p.text } else { p.text_muted };
        let text_clip = Rect::new(x, tab.y, room, tab.h).intersect(view);
        let end = ui_text(frame, app, x, tab, &name, color, text_clip);
        if let Some(parent) = parent {
            let parent = ellipsize(frame, parent, app.fonts.ui_small, close.x - end - 12.0);
            ui_text_small(frame, app, end + 6.0, tab, &parent, p.text_dim, text_clip);
        }
        draw_tab_close(
            frame,
            p,
            close,
            app.hover == Hover::ProjectTab(index, true),
            view,
        );
        hits.push(ProjectTabHit {
            index,
            rect: visible,
            close: close.intersect(view),
        });
    }
    app.projects.hits = hits;

    let hovered = app.hover == Hover::NewProject;
    draw_button(frame, p, button, area, hovered, false);
    let color = if hovered { p.text } else { p.text_muted };
    icons::plus(frame, center(button), 9.0, color, button.intersect(area));
}
