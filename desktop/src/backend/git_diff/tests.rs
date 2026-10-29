use super::*;
fn commit(root: &Path) {
    git(root, &["add", "-A"]).unwrap();
    git(
        root,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    )
    .unwrap();
}
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo with spaces");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    git(&root, &["init", "-b", "main"]).unwrap();
    fs::write(root.join("a.txt"), b"base\nkeep\n").unwrap();
    fs::write(root.join("deleted.txt"), b"deleted\n").unwrap();
    fs::write(root.join("binary.bin"), [0, 1, 2]).unwrap();
    commit(&root);
    git(&root, &["checkout", "-b", "feature"]).unwrap();
    fs::write(root.join("a.txt"), b"branch\nkeep\n").unwrap();
    fs::write(root.join("committed.txt"), b"committed\n").unwrap();
    commit(&root);
    git(&root, &["checkout", "main"]).unwrap();
    fs::write(root.join("main-only.txt"), b"not part of this work\n").unwrap();
    commit(&root);
    git(&root, &["checkout", "feature"]).unwrap();
    fs::write(root.join("a.txt"), b"branch\nkeep\nlocal\n").unwrap();
    fs::remove_file(root.join("deleted.txt")).unwrap();
    fs::write(root.join("binary.bin"), [0, 2, 3]).unwrap();
    fs::write(root.join("new\tfile.csv"), b"col\n1\n").unwrap();
    (temp, root)
}
#[test]
fn overview_includes_committed_and_dirty_files_but_excludes_main_only_changes() {
    let (_temp, root) = fixture();
    let data = overview(&root).unwrap();
    let files = arr(&data["files"]);
    assert_eq!(data["branch"], "feature");
    assert_eq!(data["baseRef"], "main");
    assert_eq!(files.len(), 5);
    let a = files.iter().find(|f| f["path"] == "a.txt").unwrap();
    assert_eq!(a["additions"], 2);
    assert_eq!(a["deletions"], 1);
    let binary = files.iter().find(|f| f["path"] == "binary.bin").unwrap();
    assert!(binary["additions"].is_null());
    let untracked = files.iter().find(|f| f["path"] == "new\tfile.csv").unwrap();
    assert_eq!(untracked["additions"], 2);
    assert_eq!(untracked["untracked"], true);
    assert!(!files.iter().any(|f| f["path"] == "main-only.txt"));
}
#[test]
fn zed_diff_uses_base_snapshots_and_live_worktree_files_without_changing_git() {
    let (temp, root) = fixture();
    let before = git(&root, &["status", "--porcelain=v1", "-z"]).unwrap();
    let args = zed_arguments(&root, &temp.path().join("snapshots")).unwrap();
    assert_eq!(args.last().unwrap(), root.to_str().unwrap());
    assert_eq!(args.len(), 16);
    let pairs = args[..args.len() - 1]
        .as_chunks::<3>()
        .0
        .iter()
        .collect::<Vec<_>>();
    let a = pairs
        .iter()
        .find(|p| p[2] == root.join("a.txt").to_str().unwrap())
        .unwrap();
    assert_eq!(a[0], "--diff");
    assert_eq!(fs::read(&a[1]).unwrap(), b"base\nkeep\n");
    let deleted = pairs
        .iter()
        .find(|p| p[1].ends_with("/deleted.txt"))
        .unwrap();
    assert_eq!(fs::read(&deleted[2]).unwrap(), b"");
    let binary = pairs
        .iter()
        .find(|p| p[1].ends_with("/binary.bin"))
        .unwrap();
    assert_eq!(fs::read(&binary[1]).unwrap(), [0, 1, 2]);
    assert_eq!(
        git(&root, &["status", "--porcelain=v1", "-z"]).unwrap(),
        before
    );
}
#[test]
fn overview_uses_the_selected_worktree_and_leaves_other_checkouts_out() {
    let (temp, root) = fixture();
    let tree = temp.path().join("agent-worktree");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-b",
            "agent-worktree",
            tree.to_str().unwrap(),
            "HEAD",
        ],
    )
    .unwrap();
    fs::write(tree.join("only-in-worktree.rs"), b"worktree\n").unwrap();
    let data = overview(&tree).unwrap();
    let files = arr(&data["files"]);
    assert_eq!(data["branch"], "agent-worktree");
    assert!(files.iter().any(|f| f["path"] == "only-in-worktree.rs"));
    assert!(!files.iter().any(|f| f["path"] == "new\tfile.csv"));
    let args = zed_arguments(&tree, &temp.path().join("snapshots")).unwrap();
    assert_eq!(
        PathBuf::from(args.last().unwrap()),
        tree.canonicalize().unwrap()
    );
}
#[test]
fn inline_file_preview_is_scoped_to_the_session_diff_and_fingerprints_content() {
    let (temp, root) = fixture();
    let home = temp.path().join("truffle");
    fs::create_dir(&home).unwrap();
    let repo = Repository::open(home, None, true).unwrap();
    repo.transaction(|s| {
        s["sessions"] = json!([{"id":"review-agent","cwd":root}]);
        Ok(())
    })
    .unwrap();
    let first = file_for_session(&repo, "review-agent", "a.txt", "").unwrap();
    assert_eq!(first["oldContent"], "base\nkeep\n");
    assert_eq!(first["newContent"], "branch\nkeep\nlocal\n");
    fs::write(root.join("a.txt"), b"changed again\n").unwrap();
    let next = file_for_session(&repo, "review-agent", "a.txt", "").unwrap();
    assert_ne!(first["fingerprint"], next["fingerprint"]);
    assert!(file_for_session(&repo, "review-agent", "../../escape", "").is_err());
    assert!(file_for_session(&repo, "other-agent", "a.txt", "").is_err());
    assert!(file_for_session(&repo, "review-agent", "binary.bin", "").is_err());
    assert_eq!(
        file_for_session(&repo, "review-agent", "new\tfile.csv", "").unwrap()["oldContent"],
        ""
    );
}
#[test]
fn session_diff_can_select_only_its_linked_checkouts() {
    let (temp, root) = fixture();
    let tree = temp.path().join("second");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-b",
            "second",
            tree.to_str().unwrap(),
            "HEAD",
        ],
    )
    .unwrap();
    std::fs::write(tree.join("only-second.rs"), "fn second() {}\n").unwrap();
    let repo = Repository::open(temp.path().join("truffle"), None, true).unwrap();
    repo.transaction(|s| {
        s["sessions"] = json!([{"id":"agent","cwd":root,"worktrees":[{"path":tree}]}]);
        Ok(())
    })
    .unwrap();
    let overview = for_session(&repo, "agent", tree.to_str().unwrap()).unwrap();
    assert_eq!(overview["branch"], "second");
    assert!(arr(&overview["files"])
        .iter()
        .any(|f| f["path"] == "only-second.rs"));
    let file = file_for_session(&repo, "agent", "only-second.rs", tree.to_str().unwrap()).unwrap();
    assert_eq!(file["newContent"], "fn second() {}\n");
    assert!(for_session(&repo, "agent", temp.path().to_str().unwrap()).is_err());
    assert!(file_for_session(&repo, "agent", "only-second.rs", "").is_err());
}
#[test]
fn missing_main_and_invalid_paths_report_errors_instead_of_guessing() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-b", "trunk"]).unwrap();
    fs::write(temp.path().join("a"), b"x").unwrap();
    commit(temp.path());
    assert!(overview(temp.path())
        .unwrap_err()
        .message
        .contains("No main branch"));
    assert!(parse_numstat("1\t0\t../../escape\0").is_err());
    assert!(parse_numstat("1\t0\t/absolute\0").is_err());
}
#[test]
fn chat_diff_commands_reuse_linked_owner_and_whitelist_cached_overview() {
    let (temp, root) = fixture();
    let repo = Repository::open(temp.path().join("truffle"), None, true).unwrap();
    repo.transaction(|s| {
        s["sessions"] =
            json!([{"id":"command-agent","cwd":root,"privateNs":"private-diff-canary"}]);
        Ok(())
    })
    .unwrap();
    let bytes = fs::read(&repo.data).unwrap();
    let initial = for_session(&repo, "command-agent", "").unwrap();
    let mut cached = initial.clone();
    cached["privateNs"] = json!("private-diff-canary");
    cached["files"][0]["privateNs"] = json!("private-diff-canary");
    repo.git_diff_cache.lock().unwrap().entry =
        Some((root.to_string_lossy().into_owned(), Instant::now(), cached));
    let result = super::super::commands::invoke(
        &repo,
        &json!({"name":"chats.diff","args":{"recordId":"command-agent"}}),
        false,
    )
    .unwrap();
    assert_eq!(result["root"], initial["root"]);
    assert_eq!(result["files"], initial["files"]);
    assert!(!result.to_string().contains("private-diff-canary"));
    let file = super::super::commands::invoke(
        &repo,
        &json!({"name":"chats.diff.file","args":{"recordId":"command-agent","path":"a.txt"}}),
        false,
    )
    .unwrap();
    assert_eq!(file["oldContent"], "base\nkeep\n");
    assert_eq!(file["newContent"], "branch\nkeep\nlocal\n");
    assert_eq!(file["contentRedacted"], false);
    assert_eq!(
        command_file(
            &repo,
            &json!({"recordId":"command-agent","path":"deleted.txt"})
        )
        .unwrap()["newContent"],
        ""
    );
    for args in [
        json!({"recordId":"command-agent","cwd":temp.path()}),
        json!({"recordId":"other-agent"}),
        json!({"recordId":"command-agent","root":root}),
    ] {
        assert!(command_overview(&repo, &args).is_err());
    }
    for path in ["../escape", "/absolute", "not-listed"] {
        assert!(command_file(&repo, &json!({"recordId":"command-agent","path":path})).is_err());
    }
    assert_eq!(fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn chat_diff_anchored_reader_rejects_swapped_symlink_and_changed_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    let file = root.join("code.txt");
    fs::write(&file, "captured source").unwrap();
    let captured = fs::symlink_metadata(&file).unwrap();
    let outside = temp.path().join("outside");
    fs::write(&outside, "private outside source").unwrap();
    fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(&outside, &file).unwrap();
    assert!(working_file_bytes(&root, "code.txt", &captured, 512 * 1024).is_err());
    fs::remove_file(&file).unwrap();
    fs::write(&file, "replacement source").unwrap();
    assert!(working_file_bytes(&root, "code.txt", &captured, 512 * 1024).is_err());
    let current = fs::symlink_metadata(&file).unwrap();
    assert_eq!(
        working_file_bytes(&root, "code.txt", &current, 512 * 1024).unwrap(),
        b"replacement source"
    );
}
#[test]
fn chat_diff_command_limits_binary_symlink_size_lines_and_overview_rows() {
    let (temp, root) = fixture();
    let repo = Repository::open(temp.path().join("truffle"), None, true).unwrap();
    repo.transaction(|s| {
        s["sessions"] = json!([{"id":"bounded-agent","cwd":root}]);
        Ok(())
    })
    .unwrap();
    fs::write(root.join("large.txt"), vec![b'x'; 512 * 1024 + 1]).unwrap();
    fs::write(root.join("lines.txt"), "x\n".repeat(10001)).unwrap();
    std::os::unix::fs::symlink(temp.path(), root.join("symlink-file")).unwrap();
    for path in ["large.txt", "lines.txt", "binary.bin", "symlink-file"] {
        assert!(command_file(&repo, &json!({"recordId":"bounded-agent","path":path})).is_err());
    }
    for index in 0..501 {
        fs::write(root.join(format!("bounded-{index}.txt")), b"").unwrap();
    }
    let result = command_overview(&repo, &json!({"recordId":"bounded-agent"})).unwrap();
    assert_eq!(arr(&result["files"]).len(), 500);
    assert_eq!(result["truncated"], true);
}
#[test]
fn chat_diff_command_redacts_source_but_preserves_original_fingerprint() {
    let (temp, root) = fixture();
    let home = temp.path().join("truffle");
    fs::create_dir(&home).unwrap();
    let secret = "fixture-source-secret";
    fs::write(home.join(".env"), format!("TRUFFLE_TEST_SECRET={secret}\n")).unwrap();
    fs::write(root.join("a.txt"), format!("before\n{secret}\nafter\n")).unwrap();
    let repo = Repository::open(home, None, true).unwrap();
    repo.transaction(|s| {
        s["sessions"] = json!([{"id":"redacted-agent","cwd":root}]);
        Ok(())
    })
    .unwrap();
    let raw = file_for_session(&repo, "redacted-agent", "a.txt", "").unwrap();
    let result = super::super::commands::invoke(
        &repo,
        &json!({"name":"chats.diff.file","args":{"recordId":"redacted-agent","path":"a.txt"}}),
        false,
    )
    .unwrap();
    assert_eq!(result["fingerprint"], raw["fingerprint"]);
    assert_eq!(result["contentRedacted"], true);
    assert!(!result.to_string().contains(secret));
    assert!(text(&result, "newContent").contains("redacted"));
}

#[test]
fn chat_diff_rejects_an_unlinked_valid_checkout_before_cache_and_accepts_explicit_link() {
    let (temp, root) = fixture();
    let (_other, other) = fixture();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    repo.transaction(|s| {
        s["sessions"] = json!([{"id":"linked-agent","cwd":root}]);
        Ok(())
    })
    .unwrap();
    let other_overview = overview(&other).unwrap();
    repo.git_diff_cache.lock().unwrap().entry = Some((
        other.to_string_lossy().into_owned(),
        Instant::now(),
        other_overview,
    ));
    let bytes = fs::read(&repo.data).unwrap();
    assert!(command_overview(&repo, &json!({"recordId":"linked-agent","cwd":other})).is_err());
    assert!(command_file(
        &repo,
        &json!({"recordId":"linked-agent","cwd":other,"path":"a.txt"})
    )
    .is_err());
    assert_eq!(fs::read(&repo.data).unwrap(), bytes);
    repo.transaction(|s| {
        s["sessions"][0]["worktrees"] = json!([{"path":other}]);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        command_overview(&repo, &json!({"recordId":"linked-agent","cwd":other})).unwrap()["root"],
        json!(other)
    );
}
