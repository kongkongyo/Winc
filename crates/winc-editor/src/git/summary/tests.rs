use super::*;

struct Repo {
    root: PathBuf,
    git: Git,
}

impl Repo {
    fn new(name: &str) -> Option<Self> {
        let git = super::super::tests::real_git()?;
        let root = crate::test_util::scratch(name);
        let repo = Self { root, git };
        repo.sh(&["init", "-q", "-b", "main"]);
        repo.sh(&["config", "core.autocrlf", "false"]);
        Some(repo)
    }

    fn sh(&self, args: &[&str]) -> Vec<u8> {
        let mut command = self.git.command(&self.root);
        command.args(args).stderr(Stdio::piped());
        let output =
            process::run(&mut command, Vec::new(), SHOW_TIMEOUT, &Cancel::default()).unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn write(&self, name: &str, content: impl AsRef<[u8]>) -> PathBuf {
        let path = self.root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    fn seed(&self) {
        self.sh(&["add", "."]);
        self.sh(&["commit", "-q", "-m", "initial"]);
    }

    fn entries(&self) -> Vec<Entry> {
        self.git
            .status(&self.root, &Cancel::default())
            .unwrap()
            .entries
    }

    fn summary(
        &self,
        entries: &[Entry],
        partial: &[Partial],
        limit: usize,
    ) -> Result<CommitSummary, String> {
        self.git
            .commit_summary_diff(&self.root, entries, partial, limit, &Cancel::default())
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn summary_only_sends_selected_actual_commit_content_and_changes_no_files() {
    let Some(repo) = Repo::new("summary-selected") else {
        return;
    };
    repo.write("selected.txt", "initial\n");
    repo.write("excluded.txt", "private old\n");
    repo.seed();
    repo.write("selected.txt", "staged but no longer current\n");
    repo.sh(&["add", "selected.txt"]);
    repo.write("selected.txt", "actual worktree commit\n");
    repo.write("excluded.txt", "PRIVATE UNSELECTED CONTENT\n");
    let index = std::fs::read(repo.root.join(".git/index")).unwrap();
    let head = repo.sh(&["rev-parse", "HEAD"]);
    let entries: Vec<_> = repo
        .entries()
        .into_iter()
        .filter(|e| e.path.ends_with("selected.txt"))
        .collect();
    let output = repo.summary(&entries, &[], 20_000).unwrap();
    assert!(output.diff.contains("-initial\n+actual worktree commit\n"));
    assert!(!output.diff.contains("staged but no longer current"));
    assert!(!output.diff.contains("excluded.txt"));
    assert!(!output.diff.contains("PRIVATE"));
    assert_eq!(std::fs::read(repo.root.join(".git/index")).unwrap(), index);
    assert_eq!(repo.sh(&["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read_to_string(repo.root.join("selected.txt")).unwrap(),
        "actual worktree commit\n"
    );
}

#[test]
fn summary_uses_the_supplied_partial_commit_not_the_rest_of_the_worktree() {
    let Some(repo) = Repo::new("summary-partial") else {
        return;
    };
    let path = repo.write("partial.txt", "one\ntwo\nthree\n");
    repo.seed();
    repo.write("partial.txt", "SELECTED\ntwo\nUNSELECTED\n");
    let partial = Partial {
        path,
        content: b"SELECTED\ntwo\nthree\n".to_vec(),
    };
    let output = repo.summary(&repo.entries(), &[partial], 20_000).unwrap();
    assert!(output.diff.contains("+SELECTED\n"));
    assert!(!output.diff.contains("UNSELECTED"));
    assert!(output.diff.contains(" three\n"));
}

#[test]
fn summary_supports_new_deleted_renamed_and_binary_files() {
    let Some(repo) = Repo::new("summary-kinds") else {
        return;
    };
    repo.write("deleted.txt", "deleted text\n");
    repo.write("old.txt", "rename unchanged\n");
    repo.seed();
    std::fs::remove_file(repo.root.join("deleted.txt")).unwrap();
    repo.sh(&["mv", "old.txt", "new.txt"]);
    repo.write("added.txt", "added text\n");
    repo.write("image.bin", b"SECRET_BINARY\0bytes");
    let output = repo.summary(&repo.entries(), &[], 20_000).unwrap();
    assert!(output.diff.contains("new file\n"));
    assert!(output.diff.contains("+added text\n"));
    assert!(output.diff.contains("deleted file\n"));
    assert!(output.diff.contains("-deleted text\n"));
    assert!(
        output
            .diff
            .contains("rename from \"old.txt\"\nrename to \"new.txt\"")
    );
    assert!(output.diff.contains("image.bin"));
    assert!(output.diff.contains("Binary file changed"));
    assert!(!output.diff.contains("SECRET_BINARY"));
}

#[test]
fn summary_handles_an_unborn_repository_and_refuses_payload_truncation() {
    let Some(repo) = Repo::new("summary-unborn") else {
        return;
    };
    repo.write("first.txt", "first added line\n");
    let entries = repo.entries();
    assert!(
        repo.summary(&entries, &[], 20_000)
            .unwrap()
            .diff
            .contains("+first added line\n")
    );
    assert!(repo.summary(&entries, &[], 8).is_err());
    assert!(!repo.root.join(".git/index").exists());
}

#[test]
fn summary_has_three_lines_of_context_and_no_unrelated_file_body() {
    let Some(repo) = Repo::new("summary-context") else {
        return;
    };
    let old: String = (0..2000).map(|i| format!("line-{i}\n")).collect();
    repo.write("large.txt", &old);
    repo.seed();
    repo.write("large.txt", old.replace("line-1000\n", "changed-1000\n"));
    let output = repo.summary(&repo.entries(), &[], 20_000).unwrap();
    assert!(output.diff.contains(" line-997\n"));
    assert!(output.diff.contains(" line-1003\n"));
    assert!(!output.diff.contains("line-996\n"));
    assert!(!output.diff.contains("line-1004\n"));
    assert!(!output.diff.contains("line-0\n"));
    assert!(output.diff.contains("@@ -998,7 +998,7 @@"));
}

#[test]
fn summary_refuses_large_reads_and_a_cancelled_request() {
    let Some(repo) = Repo::new("summary-limits") else {
        return;
    };
    let file = std::fs::File::create(repo.root.join("large.txt")).unwrap();
    file.set_len(MAX_FILE as u64 + 1).unwrap();
    let entries = repo.entries();
    assert!(repo.summary(&entries, &[], 20_000).is_err());
    let cancel = Cancel::default();
    cancel.cancel();
    assert!(
        repo.git
            .commit_summary_diff(&repo.root, &entries, &[], 20_000, &cancel)
            .is_err()
    );
}

#[test]
fn summary_refuses_custom_filters_without_executing_them() {
    let Some(repo) = Repo::new("summary-filter") else {
        return;
    };
    repo.write("test.txt", "before\n");
    repo.seed();
    repo.write("test.txt", "after\n");
    repo.write(".gitattributes", "test.txt filter=dangerous\n");
    repo.sh(&[
        "config",
        "filter.dangerous.smudge",
        "echo FILTER_RAN > filter-ran.txt",
    ]);
    repo.sh(&[
        "config",
        "filter.dangerous.clean",
        "echo FILTER_RAN > filter-ran.txt",
    ]);
    let risks = repo.git.inspect(&repo.root).unwrap();
    repo.git.trust(&repo.root, risks.fingerprint);
    let entries = vec![Entry {
        path: repo.root.join("test.txt"),
        from: None,
        staged: None,
        unstaged: Some(Change::Modified),
        is_dir: false,
    }];
    assert!(repo.summary(&entries, &[], 20_000).is_err());
    assert!(
        repo.git
            .summary_partial_base(&repo.root, &entries[0].path, 20_000, &Cancel::default())
            .is_err()
    );
    assert!(!repo.root.join("filter-ran.txt").exists());
}

#[test]
fn partial_base_matches_existing_checkout_eol_and_is_bounded() {
    let Some(repo) = Repo::new("summary-eol") else {
        return;
    };
    let path = repo.write("eol.txt", "one\ntwo\n");
    repo.seed();
    repo.sh(&["config", "core.autocrlf", "true"]);
    let cancel = Cancel::default();
    let expected = repo.git.head_content(&repo.root, &path, &cancel).unwrap();
    assert_eq!(
        repo.git
            .summary_partial_base(&repo.root, &path, 20_000, &cancel)
            .unwrap(),
        expected
    );
    assert!(
        repo.git
            .summary_partial_base(&repo.root, &path, 2, &cancel)
            .is_err()
    );
    repo.write("eol.txt", b"changed\r\ntwo\r\n");
    let entries = repo.entries();
    let diff = repo.summary(&entries, &[], 20_000).unwrap().diff;
    assert!(diff.contains("-one\n+changed\n"));
    assert!(!diff.contains('\r'));
}

#[test]
fn summary_rejects_operation_state_and_outside_paths() {
    let Some(repo) = Repo::new("summary-operation") else {
        return;
    };
    repo.write("test.txt", "before\n");
    repo.seed();
    repo.write("test.txt", "after\n");
    let mut entries = repo.entries();
    std::fs::write(
        repo.root.join(".git/MERGE_HEAD"),
        b"operation in progress\n",
    )
    .unwrap();
    assert!(repo.summary(&entries, &[], 20_000).is_err());
    std::fs::remove_file(repo.root.join(".git/MERGE_HEAD")).unwrap();
    entries[0].path = repo.root.join("../outside.txt");
    assert!(repo.summary(&entries, &[], 20_000).is_err());
}

#[test]
fn summary_fingerprint_detects_binary_content_changes_without_uploading_them() {
    let Some(repo) = Repo::new("summary-binary-fingerprint") else {
        return;
    };
    repo.write("file.bin", b"one\0old");
    repo.seed();
    repo.write("file.bin", b"two\0new");
    let entries = repo.entries();
    let first = repo.summary(&entries, &[], 20_000).unwrap();
    repo.write("file.bin", b"three\0newer");
    let second = repo.summary(&entries, &[], 20_000).unwrap();
    assert_eq!(first.diff, second.diff);
    assert_ne!(first.fingerprint, second.fingerprint);
}

#[test]
fn summary_uses_matching_line_indices_for_bare_carriage_returns() {
    let Some(repo) = Repo::new("summary-cr-lines") else {
        return;
    };
    repo.write("cr.txt", "one\rtwo\rthree\r");
    repo.seed();
    repo.write("cr.txt", "one\rCHANGED\rthree\r");
    let output = repo.summary(&repo.entries(), &[], 20_000).unwrap();
    assert!(output.diff.contains("-two\r"));
    assert!(output.diff.contains("+CHANGED\r"));
}
