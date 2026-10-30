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
const CHAT: &str = "00000000-0000-4000-8000-000000000001";
fn chat_state() -> Value {
    json!({"sessions":[{"id":CHAT,"cwd":"/start","items":[{"id":"a"},{"id":"b"}]}]})
}
fn checkout(path: &str, item_ids: &[&str], primary: bool) -> Link {
    Link {
        metadata: json!({"path":path,"branch":"main"}),
        item_ids: item_ids.iter().map(|id| json!(id)).collect(),
        primary,
    }
}
#[test]
fn linking_refuses_items_not_linked_to_the_chat() {
    let mut state = chat_state();
    let before = state.clone();
    let error = link(
        &mut state,
        CHAT,
        &checkout("/one", &["a", "c"], true),
        Remember::Replace,
    )
    .unwrap_err();
    assert!(error.message.contains("Register these work items"));
    assert_eq!(state, before);
}
#[test]
fn a_chat_links_at_most_64_checkouts_and_can_still_relink_one() {
    let mut state = chat_state();
    for n in 0..64 {
        link(
            &mut state,
            CHAT,
            &checkout(&format!("/{n}"), &[], false),
            Remember::Replace,
        )
        .unwrap();
    }
    let error = link(
        &mut state,
        CHAT,
        &checkout("/64", &[], false),
        Remember::Replace,
    )
    .unwrap_err();
    assert!(error.message.contains("at most 64"));
    link(
        &mut state,
        CHAT,
        &checkout("/0", &["a"], false),
        Remember::Replace,
    )
    .unwrap();
    assert_eq!(arr(&state["sessions"][0]["worktrees"]).len(), 64);
}
#[test]
fn one_primary_checkout_sets_the_working_directory() {
    let mut state = chat_state();
    link(
        &mut state,
        CHAT,
        &checkout("/one", &["a"], true),
        Remember::Replace,
    )
    .unwrap();
    link(
        &mut state,
        CHAT,
        &checkout("/two", &[], false),
        Remember::Replace,
    )
    .unwrap();
    assert_eq!(state["sessions"][0]["cwd"], "/one");
    link(
        &mut state,
        CHAT,
        &checkout("/two", &["b"], true),
        Remember::Replace,
    )
    .unwrap();
    // A later non-primary link of the primary path keeps it primary and merges its items.
    let chat = link(
        &mut state,
        CHAT,
        &checkout("/two", &["a"], false),
        Remember::Replace,
    )
    .unwrap();
    assert_eq!(chat["cwd"], "/two");
    let primary = arr(&chat["worktrees"])
        .into_iter()
        .filter(|tree| tree["primary"] == true)
        .map(|tree| (tree["path"].clone(), tree["itemIds"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(primary, [(json!("/two"), json!(["b", "a"]))]);
}
#[test]
fn linking_remembers_the_checkout_explicitly_unless_preserving_another_folder() {
    let mut state = chat_state();
    state["itemSettings"] = json!({"a":{"cwd":"/elsewhere"},"b":{"cwd":"/one"}});
    link(
        &mut state,
        CHAT,
        &checkout("/one", &["a", "b"], true),
        Remember::Preserve,
    )
    .unwrap();
    assert_eq!(state["itemSettings"]["a"], json!({"cwd":"/elsewhere"}));
    assert_eq!(
        state["itemSettings"]["b"],
        json!({"cwd":"/one","explicit":true})
    );
    link(
        &mut state,
        CHAT,
        &checkout("/one", &["a"], true),
        Remember::Replace,
    )
    .unwrap();
    assert_eq!(
        state["itemSettings"]["a"],
        json!({"cwd":"/one","explicit":true})
    );
}
#[test]
fn unlinking_items_and_replacing_every_checkout() {
    let mut state = chat_state();
    link(
        &mut state,
        CHAT,
        &checkout("/one", &["a", "b"], true),
        Remember::Replace,
    )
    .unwrap();
    unlink_items(&mut state, CHAT, &[json!("b")]).unwrap();
    assert_eq!(
        state["sessions"][0]["worktrees"][0]["itemIds"],
        json!(["b"])
    );
    replace_all(&mut state, CHAT, &[checkout("/two", &["a"], false)]).unwrap();
    let trees = arr(&state["sessions"][0]["worktrees"]);
    assert_eq!(trees.len(), 1);
    assert_eq!(trees[0]["path"], "/two");
}
