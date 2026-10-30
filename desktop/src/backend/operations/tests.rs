use super::*;
use crate::backend::uuid;

fn fixture() -> (tempfile::TempDir, Arc<Repository>, String) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let chat = uuid();
    repo.transaction(|s| {
        s["sessions"] =
            json!([{"id":chat,"agent":"codex","title":"Agent","cwd":temp.path(),"items":[]}]);
        Ok(())
    })
    .unwrap();
    (temp, repo, chat)
}
fn quick_action_patch(repo: &Repository) -> Value {
    let current = repo.settings.view();
    json!({"revision":current["revision"],"contentHash":current["contentHash"],
        "patch":{"quickActions":[{"id":"echo","title":"Echo","command":"printf hello"}]}})
}

#[test]
fn only_the_browser_approves_executable_settings() {
    let (_temp, repo, chat) = fixture();
    let args = quick_action_patch(&repo);
    for caller in [Caller::Assistant, Caller::Agent { chat: &chat }] {
        for name in ["settings.preview", "settings.apply"] {
            let error = run(&repo, caller, name, &args).unwrap_err();
            assert!(
                error.message.contains("Ask the user to approve"),
                "{caller:?}"
            );
        }
    }
    assert_eq!(repo.settings.values()["quickActions"], json!([]));
    let saved = run(&repo, Caller::Browser, "settings.apply", &args).unwrap();
    assert_eq!(
        saved["values"]["quickActions"][0]["command"],
        "printf hello"
    );
}

#[test]
fn only_the_browser_reads_settings_beyond_routine_preferences() {
    let (_temp, repo, chat) = fixture();
    run(
        &repo,
        Caller::Browser,
        "settings.apply",
        &quick_action_patch(&repo),
    )
    .unwrap();
    let full = run(&repo, Caller::Browser, "settings.read", &json!({})).unwrap();
    assert_eq!(full["values"]["quickActions"][0]["command"], "printf hello");
    for caller in [Caller::Assistant, Caller::Agent { chat: &chat }] {
        let routine = run(&repo, caller, "settings.read", &json!({})).unwrap();
        assert!(
            routine["values"].get("quickActions").is_none(),
            "{caller:?}"
        );
        assert_eq!(routine["revision"], full["revision"]);
    }
}

#[test]
fn app_chat_cannot_run_user_actions_that_agents_and_the_browser_can() {
    let (_temp, repo, chat) = fixture();
    let error = run(&repo, Caller::Assistant, "quick-actions.list", &json!({})).unwrap_err();
    assert!(error.message.contains("requires a user action"));
    for caller in [Caller::Browser, Caller::Agent { chat: &chat }] {
        assert!(run(&repo, caller, "quick-actions.list", &json!({})).is_ok());
    }
}
