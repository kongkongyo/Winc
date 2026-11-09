//! 顶部项目标签：选择少量项目、切换与取消、关闭、恢复，以及窄窗口和横向滚动。

use super::*;
use crate::keymap::Action;

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_top_bar_alignment() {
    let base = project_folder("top-bar-alignment");
    let mut app = app_with(&base.join("a.txt"), false);
    for parent in ["first", "second", "third"] {
        let folder = base.join(parent).join("same-name");
        std::fs::create_dir_all(&folder).unwrap();
        app.projects.folders.push(folder);
    }
    app.window_controls = true;
    app.activate_project_tab(3);
    for width in [1280.0, 600.0] {
        app.renderer = Some(
            Renderer::offscreen(width, 600.0, SCALE, app.palette.frame_bg, None, None).unwrap(),
        );
        shoot(&mut app, &format!("top-bar-alignment-{width}"));
    }
    app.renderer =
        Some(Renderer::offscreen(1280.0, 600.0, SCALE, app.palette.frame_bg, None, None).unwrap());
    app.projects.folders.truncate(1);
    app.activate_project_tab(0);
    for view in [SidebarView::Files, SidebarView::Search, SidebarView::Commit] {
        app.sidebar_view = view;
        shoot(&mut app, &format!("top-bar-alignment-{view:?}"));
    }
    let mut blank = blank_app(false);
    blank.set_sidebar(true);
    blank.sidebar_motion = None;
    shoot(&mut blank, "top-bar-alignment-empty");
    std::fs::remove_dir_all(base).unwrap();
}

fn project_folder(name: &str) -> PathBuf {
    winc_config::files::canonicalize(sample_folder(name)).unwrap()
}

fn project_point(app: &App, index: usize) -> (f32, f32) {
    let rect = app
        .projects
        .hits
        .iter()
        .find(|h| h.index == index)
        .unwrap()
        .rect;
    (rect.x + 20.0_f32.min(rect.w / 2.0), rect.y + rect.h / 2.0)
}

#[test]
fn the_add_button_starts_at_the_left_and_follows_a_single_project() {
    let mut app = blank_app(false);
    paint(&mut app);
    assert_eq!(app.projects.new_tab.x, app.layout.projects.x);
    let folder = project_folder("projects-add-follow");
    app.open_project_path(&folder);
    paint(&mut app);
    let tab = app.projects.hits[0].rect;
    assert_eq!(app.projects.new_tab.x, tab.right() + TAB_GAP);
    assert_eq!(app.projects.scroll, 0.0);
    let button = app.projects.new_tab;
    let point = (button.x + button.w / 2.0, button.y + button.h / 2.0);
    app.on_mouse_move(point.0, point.1);
    assert_eq!(app.hover, Hover::NewProject);
    click(&mut app, point);
    assert_eq!(app.popup().unwrap().at, (button.x, button.bottom() + 4.0));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn resizing_and_toggling_the_sidebar_repositions_the_add_button_without_overlap() {
    let base = project_folder("projects-add-resize");
    let mut app = app_with(&base, false);
    for parent in ["first", "second", "third"] {
        let folder = base.join(parent).join("same-name");
        std::fs::create_dir_all(&folder).unwrap();
        app.projects.folders.push(folder);
    }
    app.activate_project_tab(3);
    app.window_controls = true;
    let mut fits_seen = false;
    let mut overflow_seen = false;
    for width in [1280.0, 480.0, 1000.0, 600.0, 1280.0] {
        app.renderer = Some(
            Renderer::offscreen(width, 600.0, SCALE, app.palette.frame_bg, None, None).unwrap(),
        );
        for _ in 0..2 {
            app.run_action(Action::ToggleSidebar);
            app.sidebar_motion = None;
            paint(&mut app);
            let view = app.projects.view;
            let button = app.projects.new_tab;
            let active = app.projects.hits.iter().find(|hit| hit.index == 3).unwrap();
            assert!(active.rect.w > 0.0);
            assert!(view.right() <= button.x);
            assert!(button.right() <= app.layout.controls.x);
            assert!(button.x >= app.layout.sidebar_button.right());
            assert!(app.layout.projects.bottom() <= app.layout.tabs.y);
            if app.projects.content <= view.w {
                fits_seen = true;
                assert_eq!(app.projects.scroll, 0.0);
                assert_eq!(
                    button.x,
                    app.layout.projects.x + app.projects.content + TAB_GAP
                );
            } else {
                overflow_seen = true;
                assert_eq!(button.right(), app.layout.projects.right());
            }
            let point = (button.x + button.w / 2.0, button.y + button.h / 2.0);
            app.on_mouse_move(point.0, point.1);
            assert_eq!(app.hover, Hover::NewProject);
            assert!(app.project_tab_at(point.0, point.1).is_none());
        }
    }
    assert!(fits_seen && overflow_seen);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn project_folder_picker_results_stay_in_this_window_and_cancellation_clears_origin() {
    let a = project_folder("projects-picker-a");
    let b = project_folder("projects-picker-b");
    let mut app = app_with(&a, false);
    app.config.open_folder_in_new_window = Some(true);
    for kind in [Pick::OpenFolder, Pick::WslFolder("test-distro".into())] {
        app.open_project_path(&a);
        app.projects.picking = true;
        app.on_picked(kind.clone(), None);
        assert!(!app.projects.picking);
        assert_eq!(app.editor.root(), Some(a.as_path()));
        app.projects.picking = true;
        app.on_picked(kind, Some(b.clone()));
        assert!(!app.projects.picking);
        assert_eq!(app.editor.root(), Some(b.as_path()));
    }
    for folder in [a, b] {
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[test]
fn project_tabs_only_include_projects_selected_for_this_window() {
    let a = project_folder("projects-selected-a");
    let b = project_folder("projects-selected-b");
    let other = project_folder("projects-selected-history");
    let mut app = blank_app(false);
    app.state.remember(&other);
    app.open_project_path(&a);
    app.open_project_path(&b);
    paint(&mut app);

    assert_eq!(app.projects.folders, [a.clone(), b.clone()]);
    assert_eq!(app.editor.root(), Some(b.as_path()));
    assert!(app.state.recent.contains(&other));
    assert_eq!(app.projects.hits.len(), 2);
    assert!(app.layout.projects.bottom() <= app.layout.tabs.y);
    for dir in [a, b, other] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn clicking_project_tabs_switches_in_place_and_restores_file_tabs() {
    let a = project_folder("projects-click-a");
    let b = project_folder("projects-click-b");
    let mut app = app_with(&a.join("a.txt"), false);
    let original = app.editor.active().unwrap().id;
    // 项目标签始终在当前窗口切换，不采用普通「打开目录」的新窗口偏好。
    app.config.open_folder_in_new_window = Some(true);
    app.open_project_path(&b);
    assert!(app.editor.documents.is_empty());
    app.open_path(&b.join("short.rs"));
    paint(&mut app);
    let point = project_point(&app, 0);
    app.on_mouse_move(point.0, point.1);
    assert_eq!(app.hover, Hover::ProjectTab(0, false));
    click(&mut app, point);

    assert_eq!(app.editor.root(), Some(a.as_path()));
    assert_eq!(app.editor.documents.len(), 1);
    assert_eq!(app.editor.active().unwrap().name(), "a.txt");
    assert_ne!(
        app.editor.active().unwrap().id,
        original,
        "文件按现有流程重新打开"
    );
    assert_eq!(app.projects.folders, [a.clone(), b.clone()]);
    paint(&mut app);
    let point = project_point(&app, 1);
    click(&mut app, point);
    assert_eq!(app.editor.root(), Some(b.as_path()));
    assert_eq!(app.editor.active().unwrap().name(), "short.rs");
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn opening_an_existing_project_reuses_its_tab() {
    let a = project_folder("projects-duplicate-a");
    let b = project_folder("projects-duplicate-b");
    let mut app = app_with(&a.join("a.txt"), false);
    let document = app.editor.active().unwrap().id;
    app.open_project_path(&a.join("."));
    assert_eq!(app.editor.active().unwrap().id, document);
    app.open_project_path(&b);
    app.open_project_path(&a);
    app.open_project_path(&b);
    assert_eq!(app.projects.folders, [a.clone(), b.clone()]);
    assert_eq!(app.editor.root(), Some(b.as_path()));
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn the_project_add_button_offers_recent_projects_and_folder_selection() {
    let a = project_folder("projects-add-a");
    let b = project_folder("projects-add-b");
    let mut app = app_with(&a, false);
    app.state.remember(&b);
    paint(&mut app);
    let button = app.projects.new_tab;
    click(
        &mut app,
        (button.x + button.w / 2.0, button.y + button.h / 2.0),
    );
    assert!(matches!(
        app.context_menu.as_ref().unwrap().target,
        MenuTarget::ProjectAdd
    ));
    let recent = app.state.recent.iter().position(|p| p == &b).unwrap();
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::OpenFolder, true)));
    assert!(commands.contains(&(MenuCommand::OpenRecent(recent), true)));
    app.config.open_folder_in_new_window = Some(true);
    app.run_menu_command(MenuCommand::OpenRecent(recent));
    assert_eq!(app.editor.root(), Some(b.as_path()));
    assert_eq!(app.projects.folders, [a.clone(), b.clone()]);
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn closing_a_background_project_keeps_the_current_document() {
    let a = project_folder("projects-close-background-a");
    let b = project_folder("projects-close-background-b");
    let mut app = app_with(&a, false);
    app.open_project_path(&b);
    app.open_path(&b.join("a.txt"));
    app.run(Command::Insert("未保存".into()));
    let document = app.editor.active().unwrap().id;
    paint(&mut app);
    let close = app
        .projects
        .hits
        .iter()
        .find(|h| h.index == 0)
        .unwrap()
        .close;
    click(&mut app, (close.x + close.w / 2.0, close.y + close.h / 2.0));

    assert_eq!(app.projects.folders, std::slice::from_ref(&b));
    assert_eq!(app.editor.root(), Some(b.as_path()));
    assert_eq!(app.editor.active().unwrap().id, document);
    assert!(app.editor.active().unwrap().is_dirty());
    assert!(app.save_prompt.is_none());
    assert_eq!(
        app.state.session_for(&b).unwrap().project_tabs,
        std::slice::from_ref(&b)
    );
    assert!(a.join("a.txt").is_file());
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn closing_the_current_project_selects_a_neighbor_and_the_last_returns_to_start() {
    let a = project_folder("projects-close-current-a");
    let b = project_folder("projects-close-current-b");
    let mut app = app_with(&a, false);
    app.open_project_path(&b);
    app.activate_project_tab(0);
    app.close_project_tab(0);
    assert_eq!(app.editor.root(), Some(b.as_path()));
    assert_eq!(app.projects.folders, std::slice::from_ref(&b));
    assert!(app.state.recent.contains(&a));
    app.close_project_tab(0);
    paint(&mut app);
    assert!(app.projects.folders.is_empty());
    assert!(app.editor.workspace.is_none());
    assert!(app.editor.documents.is_empty());
    assert!(app.state.last.is_none());
    assert!(!app.start_hits.is_empty());
    for dir in [a, b] {
        assert!(dir.join("a.txt").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn canceling_a_project_switch_keeps_the_source_and_does_not_add_a_tab() {
    let a = project_folder("projects-cancel-switch-a");
    let b = project_folder("projects-cancel-switch-b");
    let mut app = app_with(&a.join("a.txt"), false);
    app.run(Command::Insert("未保存".into()));
    let document = app.editor.active().unwrap().id;
    app.open_project_path(&b);
    assert!(app.save_prompt.is_some());
    assert_eq!(app.editor.root(), Some(a.as_path()));
    assert_eq!(app.projects.folders, std::slice::from_ref(&a));
    app.answer_save_prompt(SaveChoice::Cancel);
    assert_eq!(app.editor.root(), Some(a.as_path()));
    assert_eq!(app.projects.folders, std::slice::from_ref(&a));
    assert_eq!(app.editor.active().unwrap().id, document);
    assert!(app.editor.active().unwrap().is_dirty());
    app.open_project_path(&b);
    app.answer_save_prompt(SaveChoice::Discard);
    assert_eq!(app.editor.root(), Some(b.as_path()));
    assert_eq!(app.projects.folders, [a.clone(), b.clone()]);
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn canceling_a_project_close_preserves_its_tab_and_document() {
    let a = project_folder("projects-cancel-close-a");
    let b = project_folder("projects-cancel-close-b");
    let mut app = app_with(&a, false);
    app.open_project_path(&b);
    app.open_path(&b.join("a.txt"));
    app.run(Command::Insert("未保存".into()));
    let document = app.editor.active().unwrap().id;
    app.close_project_tab(1);
    assert!(matches!(
        app.save_prompt.as_ref().unwrap().request,
        CloseRequest::CloseProject { .. }
    ));
    app.answer_save_prompt(SaveChoice::Cancel);
    assert_eq!(app.editor.root(), Some(b.as_path()));
    assert_eq!(app.projects.folders, [a.clone(), b.clone()]);
    assert_eq!(app.editor.active().unwrap().id, document);
    assert!(app.editor.active().unwrap().is_dirty());
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn restarting_restores_project_order_without_loading_the_other_projects() {
    let a = project_folder("projects-restart-a");
    let b = project_folder("projects-restart-b");
    let mut app = app_with(&a.join("a.txt"), false);
    app.open_project_path(&b);
    app.open_path(&b.join("short.rs"));
    app.activate_project_tab(0);
    app.save_session();
    let state = State::parse(&app.state.to_json());
    let session = state.last_session().unwrap();
    assert_eq!(session.folder, a);
    assert_eq!(session.project_tabs, [a.clone(), b.clone()]);

    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, session));
    crate::app::restore_at_launch(&mut editor, Some(session), None);
    let mut restored = blank_app(false);
    restored.editor = editor;
    restored.state = state;
    restored.restore_project_tabs();
    assert_eq!(restored.projects.folders, [a.clone(), b.clone()]);
    assert_eq!(restored.editor.root(), Some(a.as_path()));
    assert_eq!(restored.editor.documents.len(), 1);
    assert_eq!(restored.editor.active().unwrap().name(), "a.txt");
    assert!(
        restored
            .editor
            .documents
            .iter()
            .all(|d| d.path.as_ref().unwrap().starts_with(&a))
    );
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn overflowing_project_tabs_scroll_and_keep_controls_separate_in_a_narrow_window() {
    let base = project_folder("projects-overflow");
    let mut app = app_with(&base, false);
    for i in 0..18 {
        let path = base.join(format!("project-{i:02}-with-a-long-name"));
        std::fs::create_dir_all(&path).unwrap();
        app.projects.folders.push(path);
    }
    app.renderer =
        Some(Renderer::offscreen(480.0, 600.0, SCALE, app.palette.frame_bg, None, None).unwrap());
    app.window_controls = true;
    paint(&mut app);
    assert!(app.projects.content > app.projects.view.w);
    assert_eq!(app.projects.scroll, 0.0);
    let point = project_point(&app, 0);
    app.on_mouse_move(point.0, point.1);
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -1.0));
    let vertical = app.projects.scroll;
    assert!(vertical > 0.0);
    app.on_wheel(MouseScrollDelta::LineDelta(-1.0, 0.0));
    assert!(app.projects.scroll > vertical);

    app.activate_project_tab(18);
    paint(&mut app);
    assert!(app.projects.hits.iter().any(|h| h.index == 18));
    assert!(app.projects.view.right() <= app.projects.new_tab.x);
    assert!(app.projects.new_tab.right() <= app.layout.controls.x);
    assert!(app.layout.projects.bottom() <= app.layout.tabs.y);
    for hit in &app.projects.hits {
        assert!(hit.rect.x >= app.projects.view.x);
        assert!(hit.rect.right() <= app.projects.view.right());
    }
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn project_tabs_stay_in_this_window_when_another_window_has_the_project() {
    let a = project_folder("projects-same-window-a");
    let b = project_folder("projects-same-window-b");
    let mut first = app_with(&a, false);
    first.open_project_path(&b);
    first.activate_project_tab(0);
    let mut winc = crate::winc::Winc::new(None, None);
    let first = winc.add_detached(first);
    let second = winc.add_detached(app_with(&b, false));
    winc.app(first).activate_project_tab(1);
    winc.settle_for_test();
    assert_eq!(winc.app(first).editor.root(), Some(b.as_path()));
    assert_eq!(winc.app(second).editor.root(), Some(b.as_path()));
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn a_missing_neighbor_does_not_prevent_closing_the_current_project() {
    let a = project_folder("projects-missing-neighbor-a");
    let b = project_folder("projects-missing-neighbor-b");
    let c = project_folder("projects-missing-neighbor-c");
    let mut app = app_with(&a, false);
    app.open_project_path(&b);
    app.open_project_path(&c);
    app.activate_project_tab(0);
    std::fs::remove_dir_all(&b).unwrap();
    app.close_project_tab(0);
    assert_eq!(app.editor.root(), Some(c.as_path()));
    assert!(!app.projects.folders.contains(&a));
    for dir in [a, c] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn closing_a_project_rechecks_neighbors_after_the_save_confirmation() {
    let a = project_folder("projects-close-confirm-neighbor-a");
    let b = project_folder("projects-close-confirm-neighbor-b");
    let c = project_folder("projects-close-confirm-neighbor-c");
    let mut app = app_with(&a.join("a.txt"), false);
    app.open_project_path(&b);
    app.open_project_path(&c);
    app.open_path(&c.join("short.rs"));
    app.activate_project_tab(0);
    app.run(Command::Insert("未保存".into()));
    app.close_project_tab(0);
    assert!(app.save_prompt.is_some());

    std::fs::remove_dir_all(&b).unwrap();
    app.answer_save_prompt(SaveChoice::Discard);

    assert_eq!(app.editor.root(), Some(c.as_path()));
    assert_eq!(app.editor.active().unwrap().name(), "short.rs");
    assert_eq!(app.projects.folders, [b, c.clone()]);
    assert!(a.join("a.txt").is_file());
    for dir in [a, c] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn closing_a_project_returns_to_start_when_all_neighbors_disappear_during_confirmation() {
    let a = project_folder("projects-close-confirm-missing-a");
    let b = project_folder("projects-close-confirm-missing-b");
    let c = project_folder("projects-close-confirm-missing-c");
    let mut app = app_with(&a.join("a.txt"), false);
    app.open_project_path(&b);
    app.open_project_path(&c);
    app.activate_project_tab(0);
    app.run(Command::Insert("未保存".into()));
    app.close_project_tab(0);
    assert!(app.save_prompt.is_some());

    for dir in [&b, &c] {
        std::fs::remove_dir_all(dir).unwrap();
    }
    app.answer_save_prompt(SaveChoice::Discard);
    paint(&mut app);

    assert!(app.editor.root().is_none());
    assert!(app.editor.documents.is_empty());
    assert!(!app.projects.folders.contains(&a));
    assert!(!app.start_hits.is_empty());
    assert!(app.state.last.is_none());
    assert!(a.join("a.txt").is_file());
    std::fs::remove_dir_all(a).unwrap();
}

#[test]
fn canceling_a_project_close_keeps_the_source_when_neighbors_disappear() {
    let a = project_folder("projects-close-confirm-cancel-a");
    let b = project_folder("projects-close-confirm-cancel-b");
    let mut app = app_with(&a.join("a.txt"), false);
    app.open_project_path(&b);
    app.activate_project_tab(0);
    app.run(Command::Insert("未保存".into()));
    let document = app.editor.active().unwrap().id;
    app.close_project_tab(0);
    assert!(app.save_prompt.is_some());

    std::fs::remove_dir_all(&b).unwrap();
    app.answer_save_prompt(SaveChoice::Cancel);

    assert_eq!(app.editor.root(), Some(a.as_path()));
    assert_eq!(app.projects.folders, [a.clone(), b]);
    assert_eq!(app.editor.active().unwrap().id, document);
    assert!(app.editor.active().unwrap().is_dirty());
    std::fs::remove_dir_all(a).unwrap();
}

#[test]
fn closing_a_background_tab_keeps_the_current_project_visible() {
    let base = project_folder("projects-close-reveal");
    let mut app = app_with(&base, false);
    for name in [
        "a-project-with-a-very-long-folder-name",
        "x",
        "another-long-project-name",
        "one-more-long-project-name",
    ] {
        let folder = base.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        app.projects.folders.push(folder);
    }
    app.activate_project_tab(2);
    app.renderer =
        Some(Renderer::offscreen(480.0, 600.0, SCALE, app.palette.frame_bg, None, None).unwrap());
    paint(&mut app);
    let active = app
        .projects
        .hits
        .iter()
        .find(|hit| hit.index == 2)
        .unwrap()
        .rect;
    // 将当前短标签滚到视口左缘附近，前面的长标签只留下末尾。
    app.projects.scroll += active.x - app.projects.view.x - 8.0;
    paint(&mut app);
    app.close_project_tab(1);
    paint(&mut app);
    assert!(app.projects.hits.iter().any(|hit| hit.index == 1));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn restoring_other_windows_into_a_blank_first_window_keeps_project_tabs() {
    let a = project_folder("projects-restore-empty-a");
    let b = project_folder("projects-restore-empty-b");
    let mut saved = app_with(&a, false);
    saved.open_project_path(&b);
    saved.activate_project_tab(0);
    saved.save_session();
    let mut blank = blank_app(false);
    blank.state = State::parse(&saved.state.to_json());
    blank.state.last = None;
    let mut winc = crate::winc::Winc::new(None, None);
    let first = winc.add_detached(blank);
    winc.restore_windows(std::slice::from_ref(&a));
    assert_eq!(winc.app(first).projects.folders, [a.clone(), b.clone()]);
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn the_existing_close_folder_action_also_removes_the_current_project_tab() {
    let a = project_folder("projects-close-menu-a");
    let b = project_folder("projects-close-menu-b");
    let mut app = app_with(&a, false);
    app.open_project_path(&b);
    app.close_folder();
    assert_eq!(app.editor.root(), Some(a.as_path()));
    assert_eq!(app.projects.folders, std::slice::from_ref(&a));
    app.close_folder();
    assert!(app.editor.root().is_none());
    assert!(app.projects.folders.is_empty());
    for dir in [a, b] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}
