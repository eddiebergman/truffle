use super::*;
/// A canonical temporary root with the linked folder `checkout` and the unlinked folder `outside`.
fn folders() -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("checkout/src")).unwrap();
    fs::create_dir_all(root.join("outside/inner")).unwrap();
    let checkout = root.join("checkout").to_string_lossy().into_owned();
    (temp, root, checkout)
}
#[test]
fn another_spelling_of_a_linked_worktree_finds_the_stored_path() {
    let (_temp, root, checkout) = folders();
    let chat = json!({"cwd":root.join("outside"),"worktrees":[{"path":checkout}]});
    for spelling in [format!("{checkout}/"), format!("{checkout}/src/..")] {
        assert_eq!(
            find(&chat, &spelling),
            Some(checkout.as_str()),
            "{spelling}"
        );
    }
}
#[test]
fn a_dot_dot_path_out_of_a_checkout_is_not_linked() {
    let (_temp, _root, checkout) = folders();
    let chat = json!({"cwd":checkout});
    assert_eq!(find(&chat, &format!("{checkout}/../outside")), None);
}
#[test]
fn a_symlink_out_of_a_checkout_is_not_linked() {
    let (_temp, root, checkout) = folders();
    std::os::unix::fs::symlink(root.join("outside/inner"), root.join("checkout/escape")).unwrap();
    let chat = json!({"cwd":checkout});
    // Read as text, `escape/..` is the checkout itself; on disk it is `outside`.
    assert_eq!(find(&chat, &format!("{checkout}/escape/..")), None);
    assert_eq!(find(&chat, &format!("{checkout}/escape")), None);
}
#[test]
fn base_prefers_the_fetched_origin_main_over_the_local_main() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let commit = || {
        git::run(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
            ],
        )
        .unwrap();
    };
    git::run(root, &["init", "-b", "main"]).unwrap();
    commit();
    let fetched = git::run(root, &["rev-parse", "HEAD"]).unwrap();
    commit();
    let local = git::run(root, &["rev-parse", "HEAD"]).unwrap();
    let base = base(root).unwrap();
    assert_eq!((base.name, base.commit), ("main", local.trim().to_owned()));
    git::run(
        root,
        &["update-ref", "refs/remotes/origin/main", fetched.trim()],
    )
    .unwrap();
    let base = super::base(root).unwrap();
    assert_eq!(
        (base.name, base.commit),
        ("origin/main", fetched.trim().to_owned())
    );
}
