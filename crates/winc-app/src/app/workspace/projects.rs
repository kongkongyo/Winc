//! 项目标签只保存目录快捷入口；切换仍关闭原项目并按原有现场重新打开。

use super::*;

/// 顶部项目标签的命中区域。
pub(crate) struct ProjectTabHit {
    pub(crate) index: usize,
    pub(crate) rect: Rect,
    pub(crate) close: Rect,
}

/// 当前窗口的项目快捷入口及其标签栏状态。
#[derive(Default)]
pub(crate) struct ProjectTabs {
    pub(crate) folders: Vec<PathBuf>,
    pub(crate) hits: Vec<ProjectTabHit>,
    pub(crate) view: Rect,
    pub(crate) new_tab: Rect,
    pub(crate) scroll: f32,
    pub(crate) content: f32,
    pub(crate) revealed: Option<PathBuf>,
    /// 文件夹选择器此次由项目栏的「＋」打开。
    pub(crate) picking: bool,
}

impl App {
    /// 启动时恢复该窗口的项目标签，只记录路径，不加载其他项目。
    pub(crate) fn restore_project_tabs(&mut self) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        self.restore_project_tabs_for(&root);
        if let Some(window) = self
            .state
            .project_windows
            .first()
            .filter(|window| window.folder == root)
        {
            self.projects.folders.clone_from(&window.project_tabs);
        }
        self.note_project_opened();
    }

    /// 恢复目录对应的标签列表，供首次打开目录前使用，避免覆盖已保存的列表。
    pub(crate) fn restore_project_tabs_for(&mut self, root: &Path) {
        self.projects.folders = self
            .state
            .session_for(root)
            .map(|session| session.project_tabs.clone())
            .unwrap_or_default();
    }

    /// 仅在成功打开目录后加入标签，取消切换不改变标签和当前项。
    pub(in crate::app) fn note_project_opened(&mut self) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        if !self.projects.folders.contains(&root) {
            self.projects.folders.push(root);
        }
        self.remember_project_tabs();
    }

    /// 保存当前标签列表，保留已有的文件现场。
    pub(in crate::app) fn remember_project_tabs(&mut self) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        let folders = self.projects.folders.clone();
        self.update_state(|state| {
            let mut session = state
                .session_for(&root)
                .cloned()
                .unwrap_or_else(|| Session {
                    folder: root,
                    ..Session::default()
                });
            session.project_tabs = folders;
            state.set_session(session);
        });
    }

    /// 标签栏中的项目和关闭按钮；命中区域已裁剪到可见部分。
    pub(in crate::app) fn project_tab_at(&self, x: f32, y: f32) -> Option<(usize, bool)> {
        self.projects.hits.iter().find_map(|hit| {
            hit.rect
                .contains(x, y)
                .then(|| (hit.index, hit.close.contains(x, y)))
        })
    }

    /// 通过项目标签或添加菜单在当前窗口打开项目。
    pub(in crate::app) fn open_project_path(&mut self, path: &Path) {
        if self.closing_pending() {
            return;
        }
        if !path.is_dir() {
            self.notify(winc_i18n::t!(
                "workspace-recent-missing",
                path = winc_editor::display_path(path, None)
            ));
            return;
        }
        self.switch_folder_here(path);
    }

    /// 点击已存在的项目标签。
    pub(in crate::app) fn activate_project_tab(&mut self, index: usize) {
        if let Some(folder) = self.projects.folders.get(index).cloned() {
            self.open_project_path(&folder);
        }
    }

    /// 从当前窗口移除项目快捷入口；关闭当前项目时沿用关闭确认。
    pub(in crate::app) fn close_project_tab(&mut self, index: usize) {
        if self.closing_pending() {
            return;
        }
        let Some(folder) = self.projects.folders.get(index).cloned() else {
            return;
        };
        if self.editor.root() != Some(folder.as_path()) {
            self.projects.folders.remove(index);
            self.projects.hits.clear();
            self.projects.revealed = None;
            self.remember_project_tabs();
            self.request_redraw();
            return;
        }
        let next = self
            .projects
            .folders
            .iter()
            .skip(index + 1)
            .chain(self.projects.folders[..index].iter().rev())
            .find(|folder| folder.is_dir())
            .cloned();
        self.remember_session();
        let request = CloseRequest::CloseProject { folder, next };
        if !self.confirm_terminal_quit(request.clone()) {
            self.request_close(request);
        }
    }

    /// 成功关闭项目后更新列表；项目文件和最近记录不受影响。
    pub(in crate::app) fn forget_project_tab(&mut self, folder: &Path) {
        self.projects.folders.retain(|p| p != folder);
        self.projects.hits.clear();
        self.projects.revealed = None;
        self.remember_project_tabs();
    }

    /// 顶部「＋」显示最近项目和选择文件夹入口。
    pub(in crate::app) fn open_project_add_menu(&mut self) {
        let rect = self.projects.new_tab;
        self.open_context_menu(
            crate::app::input::context_menu::MenuTarget::ProjectAdd,
            (rect.x, rect.bottom() + 4.0),
        );
    }
}
