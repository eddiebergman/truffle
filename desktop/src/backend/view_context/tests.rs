use super::*;
#[test]
fn snapshot_preserves_the_filtered_set_overrides_links_and_retry_identity() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|s|{s["items"]=json!([{"id":"gh:a","source":"github","title":"First","url":"https://github.com/org/repo/pull/1","status":"draft","ci":{"status":"passed"}},{"id":"gh:b","source":"github","title":"Second","url":"https://github.com/org/repo/pull/2","status":"todo"},{"id":"gh:excluded","source":"github","title":"Outside filter","status":"todo"}]);s["overrides"]["gh:a"]=json!({"status":"blocked"});Ok(())}).unwrap();
    let payload = json!({"requestId":uuid(),"source":"github","agent":"claude","question":"Which needs attention?","folderPath":temp.path(),"itemIds":["gh:b","gh:a","gh:b"],"filters":{"query":"example","githubView":"mine"}});
    let record = prepare(&repo, &payload).unwrap();
    let again = prepare(&repo, &payload).unwrap();
    assert_eq!(record["id"], again["id"]);
    assert!(arr(&record["items"]).is_empty());
    let path = PathBuf::from(text(&record["viewContext"], "path"));
    let bytes = std::fs::read(&path).unwrap();
    let saved: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(saved["itemCount"], 2);
    assert_eq!(saved["items"][0]["id"], "gh:b");
    assert_eq!(saved["items"][1]["status"], "blocked");
    assert_eq!(saved["items"][1]["ci"]["status"], "passed");
    assert_eq!(saved["filters"]["githubView"], "mine");
    repo.transaction(|s| {
        s["items"][0]["status"] = json!("done");
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    let context = workflows::context(&repo, text(&record, "id")).unwrap();
    assert!(context.contains("Which needs attention?"));
    assert!(context.contains("Start immediately"));
}
#[test]
fn empty_invalid_or_stale_views_do_not_create_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for ids in [json!([]), json!(["missing"]), json!([null])] {
        let payload = json!({"requestId":uuid(),"source":"github","agent":"codex","question":"Check these","folderPath":temp.path(),"itemIds":ids,"filters":{}});
        assert!(prepare(&repo, &payload).is_err());
        assert!(arr(&repo.snapshot(false)["sessions"]).is_empty());
    }
}
