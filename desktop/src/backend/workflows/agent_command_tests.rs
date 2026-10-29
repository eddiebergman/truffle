use super::*;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, String) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s| {
        s["sessions"] =
            json!([{"id":id,"agent":"codex","title":"Agent","cwd":temp.path(),"items":[]}]);
        Ok(())
    })
    .unwrap();
    (temp, repo, id)
}
fn log(repo: &Repository, id: &str) -> Vec<Value> {
    let state = repo.state.lock().unwrap();
    arr(&state["sessions"])
        .into_iter()
        .find(|row| row["id"] == id)
        .map(|row| arr(&row["agentCommands"]))
        .unwrap()
}
fn notes(repo: &Repository) -> usize {
    arr(&repo.state.lock().unwrap()["notes"]).len()
}
fn command(id: Option<&str>, name: &str, args: &Value) -> Value {
    let mut p = json!({"operation":"command","name":name,"args":args});
    if let Some(id) = id {
        p["id"] = json!(id);
    }
    p
}
#[test]
fn agent_writes_run_and_show_in_their_chat() {
    let (_temp, repo, id) = fixture();
    let before = notes(&repo);
    let result = register(
        &repo,
        &command(Some(&id), "projects.create", &json!({"title":"From agent"})),
    )
    .unwrap();
    assert!(result["commandResult"].is_object());
    assert_eq!(notes(&repo), before + 1);
    let log = log(&repo, &id);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["name"], "projects.create");
    assert_eq!(log[0]["ok"], true);
}
#[test]
fn agent_user_action_commands_are_allowed_and_failures_are_shown() {
    let (_temp, repo, id) = fixture();
    let error = register(
        &repo,
        &command(Some(&id), "chats.prepare.quick", &json!({})),
    )
    .unwrap_err();
    assert!(!error.message.contains("requires a user action"));
    let log = log(&repo, &id);
    assert_eq!(log[0]["name"], "chats.prepare.quick");
    assert_eq!(log[0]["ok"], false);
}
#[test]
fn agent_writes_without_a_chat_are_refused_before_they_run() {
    let (_temp, repo, id) = fixture();
    let before = notes(&repo);
    for p in [
        command(None, "projects.create", &json!({"title":"Hidden"})),
        command(Some(&uuid()), "projects.create", &json!({"title":"Hidden"})),
    ] {
        assert!(register(&repo, &p).is_err());
    }
    assert_eq!(notes(&repo), before);
    assert_eq!(log(&repo, &id), Vec::<Value>::new());
}
#[test]
fn agent_reads_need_no_chat_and_leave_no_notice() {
    let (_temp, repo, id) = fixture();
    assert!(
        register(&repo, &command(None, "commands.list", &json!({}))).unwrap()["commandResult"]
            .is_object()
    );
    assert_eq!(log(&repo, &id), Vec::<Value>::new());
}
#[test]
fn agent_command_notices_keep_the_latest_twenty() {
    let (_temp, repo, id) = fixture();
    for index in 0..25 {
        register(
            &repo,
            &command(
                Some(&id),
                "projects.create",
                &json!({"title":format!("P{index}")}),
            ),
        )
        .unwrap();
    }
    let log = log(&repo, &id);
    assert_eq!(log.len(), AGENT_COMMAND_LOG);
    assert!(log.iter().all(|row| row["ok"] == true));
}
