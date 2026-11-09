//! 提交与回滚的测试：在指定仓库提交完成之后收下结果。

use super::*;

#[test]
fn sidebar_head_requires_a_directory_repository_and_existing_head() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut app = App::new(
        winc_editor::Editor::default(),
        None,
        crate::bench::Bench::from_env(),
        winc_config::Config::default(),
    );
    assert!(app.sidebar_head_label().is_none());
    app.editor.open_folder(root).unwrap();
    app.git.root = Some(root.to_path_buf());
    assert!(app.sidebar_head_label().is_none());
    let mut status = Status {
        head: Some(Head::Branch("main".into())),
        oid: Some("abcdef0123456789".into()),
        ..Status::default()
    };
    app.git.status = Some(status.clone());
    assert_eq!(app.sidebar_head_label().as_deref(), Some("main"));
    status.head = Some(Head::Detached("abcdef0".into()));
    app.git.status = Some(status.clone());
    assert_eq!(app.sidebar_head_label().as_deref(), Some("(abcdef0)"));
    status.oid = None;
    status.head = Some(Head::Branch("unborn-main".into()));
    app.git.status = Some(status.clone());
    assert!(app.sidebar_head_label().is_none());
    status.oid = Some("abcdef0123456789".into());
    status.head = None;
    app.git.status = Some(status.clone());
    assert!(app.sidebar_head_label().is_none());
    status.head = Some(Head::Branch("main".into()));
    app.git.status = Some(status);
    app.git.root = None;
    assert!(app.sidebar_head_label().is_none());
    app.git.root = Some(root.to_path_buf());
    app.editor.workspace = None;
    assert!(app.sidebar_head_label().is_none());
}

#[test]
fn sync_status_text_distinguishes_counts_and_remote_states() {
    for (state, expected) in [
        (SyncState::Push(2), "↑2"),
        (SyncState::Pull(10), "↓10"),
        (
            SyncState::Both {
                ahead: 3,
                behind: 10,
            },
            "↓10 ↑3",
        ),
        (SyncState::InSync, "已同步"),
        (SyncState::Publish, "未发布"),
        (SyncState::NoRemote, "无远程"),
        (SyncState::Hidden, ""),
    ] {
        assert_eq!(state.label(), expected);
    }
}

impl App {
    /// 测试用：在 `root` 里提交好了，收下结果。
    pub(in crate::app) fn committed_in(&mut self, root: &Path) {
        let outcome = Outcome::Committed(Ok("abc1234".into()), "msg".into(), root.into());
        let _ = self.commit.results.0.send(outcome);
        self.poll_commit();
    }
}
