use super::*;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let id = uuid();
    let session = uuid();
    repo.transaction(|s|{s["sessions"]=json!([{"id":id,"sessionId":session,"agent":"codex","title":"Before","cwd":temp.path(),"instruction":"Keep instruction","destination":"embedded","launchNew":false,"items":[{"id":"note:old","source":"notes","title":"Old","linkExtension":{"keep":true}}],"preparationRequests":{"version":1,"entries":[{"requestId":uuid(),"kind":"new","hash":"f".repeat(64),"reused":false}]},"futureNs":{"keep":true}}]);s["notes"]=json!([{ "id":"note:old","source":"notes","title":"Old"},{"id":"note:concurrent","source":"notes","title":"Concurrent"}]);s["itemSettings"]["note:old"]=json!({"cwd":"existing folder","explicit":true});Ok(())}).unwrap();
    (temp, repo, id, session)
}
#[test]
fn registration_resolution_merges_latest_links_and_omitted_title_without_overwriting_metadata() {
    let (temp, repo, id, session) = fixture();
    let before_alias = repo.state.lock().unwrap()["sessions"][0]["preparationRequests"].clone();
    let payload = json!({"id":id,"agent":"codex","sessionId":session,"cwd":temp.path(),"preserveItemFolders":true,"title":"","itemIds":[],"urls":["https://example.invalid/new"]});
    let result=register_resolving(&repo,&payload,|_|{repo.transaction(|s|{s["sessions"][0]["title"]=json!("Concurrent title");s["sessions"][0]["items"].as_array_mut().unwrap().push(json!({"id":"note:concurrent","source":"notes","title":"Concurrent","newExtension":9}));s["sessions"][0]["runtime"]=json!({"state":"working"});Ok(())})?;Ok(json!({"id":"github:new","source":"github","title":"Imported","url":"https://example.invalid/new"}))}).unwrap();
    assert_eq!(result["id"], id);
    assert_eq!(result["title"], "Concurrent title");
    assert_eq!(result["items"].as_array().unwrap().len(), 3);
    assert_eq!(result["items"][0]["linkExtension"]["keep"], true);
    assert_eq!(result["items"][1]["newExtension"], 9);
    assert_eq!(result["preparationRequests"], before_alias);
    assert_eq!(result["futureNs"]["keep"], true);
    assert_eq!(result["runtime"]["state"], "working");
    assert_eq!(result["instruction"], "Keep instruction");
    let state = repo.state.lock().unwrap();
    assert_eq!(state["itemSettings"]["note:old"]["cwd"], "existing folder");
    assert!(state["itemSettings"].get("note:concurrent").is_none());
    drop(state);
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}
#[test]
fn registration_explicit_title_is_intentional_and_repeated_association_keeps_exact_record() {
    let (temp, repo, id, session) = fixture();
    let result=register(&repo,&json!({"agent":"codex","sessionId":session.to_uppercase(),"title":"Intentional title","cwd":temp.path(),"itemIds":["note:concurrent"]})).unwrap();
    assert_eq!(result["id"], id);
    assert_eq!(result["title"], "Intentional title");
    assert_eq!(result["items"][0]["linkExtension"]["keep"], true);
    assert_eq!(
        repo.state.lock().unwrap()["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let again = register(
        &repo,
        &json!({"agent":"codex","sessionId":session,"cwd":temp.path(),"itemIds":[]}),
    )
    .unwrap();
    assert_eq!(again["id"], id);
    assert_eq!(again["title"], "Intentional title");
    assert_eq!(again["items"].as_array().unwrap().len(), 2);
}
#[test]
fn registration_postprobe_preserves_current_mapped_folders_for_git_and_plain_directories() {
    for git in [false, true] {
        let (temp, repo, _, _) = fixture();
        let record = repo.state.lock().unwrap()["sessions"][0].clone();
        let selected = temp.path().join("selected-elsewhere");
        std::fs::create_dir(&selected).unwrap();
        let mapping = json!({"cwd":selected,"explicit":true,"futureNs":{"keep":true}});
        repo.transaction(|s| {
            s["itemSettings"]["note:old"] = mapping.clone();
            Ok(())
        })
        .unwrap();
        let metadata = json!({"path":record["cwd"],"repoPath":record["cwd"],"branch":"main"});
        let result = finish_registration(
            &repo,
            &record,
            &[],
            &[],
            if git { Some(&metadata) } else { None },
        )
        .unwrap();
        assert_eq!(
            repo.state.lock().unwrap()["itemSettings"]["note:old"],
            mapping
        );
        assert_eq!(result["cwd"], record["cwd"]);
        if git {
            assert_eq!(result["worktrees"][0]["itemIds"], json!(["note:old"]));
        }
    }
}
#[test]
fn registration_postprobe_does_not_overwrite_new_folder_or_recreate_deleted_record() {
    let (temp, repo, id, _) = fixture();
    let record = repo.state.lock().unwrap()["sessions"][0].clone();
    let next = temp.path().join("new-folder");
    std::fs::create_dir(&next).unwrap();
    repo.transaction(|s| {
        s["sessions"][0]["cwd"] = json!(next);
        s["itemSettings"]["note:old"]["cwd"] = json!(next);
        Ok(())
    })
    .unwrap();
    finish_registration(
        &repo,
        &record,
        &[],
        &[],
        Some(&json!({"path":temp.path(),"repoPath":temp.path(),"branch":"stale"})),
    )
    .unwrap();
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(state["sessions"][0]["cwd"], json!(next));
    assert_eq!(state["itemSettings"]["note:old"]["cwd"], json!(next));
    assert!(state["sessions"][0].get("worktrees").is_none());
    repo.transaction(|s| {
        s["sessions"] = json!([]);
        Ok(())
    })
    .unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert!(finish_registration(&repo, &record, &[], &[], None).is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(repo.state.lock().unwrap()["sessions"], json!([]));
    assert_eq!(id, record["id"]);
}
