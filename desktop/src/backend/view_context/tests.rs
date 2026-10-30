use super::*;
use crate::backend::chat_preparation::{self, Kind};
use std::path::PathBuf;
#[test]
fn snapshot_preserves_the_filtered_set_overrides_links_and_retry_identity() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|s|{s["items"]=json!([{"id":"gh:a","source":"github","title":"First","url":"https://github.com/org/repo/pull/1","status":"draft","ci":{"status":"passed"}},{"id":"gh:b","source":"github","title":"Second","url":"https://github.com/org/repo/pull/2","status":"todo"},{"id":"gh:excluded","source":"github","title":"Outside filter","status":"todo"}]);s["overrides"]["gh:a"]=json!({"status":"blocked"});Ok(())}).unwrap();
    let payload = json!({"requestId":uuid(),"source":"github","agent":"claude","question":"Which needs attention?","folderPath":temp.path(),"itemIds":["gh:b","gh:a","gh:b"],"filters":{"query":"example","githubView":"mine"}});
    let record = chat_preparation::save(&repo, Kind::View, &payload).unwrap();
    let again = chat_preparation::save(&repo, Kind::View, &payload).unwrap();
    assert_eq!(record["id"], again["id"]);
    assert_eq!(arr(&record["items"]), [] as [serde_json::Value; 0]);
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
        assert!(chat_preparation::save(&repo, Kind::View, &payload).is_err());
        assert_eq!(
            arr(&repo.snapshot(false)["sessions"]),
            [] as [serde_json::Value; 0]
        );
    }
}
