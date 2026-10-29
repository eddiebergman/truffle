use super::*;
#[test]
fn work_search_uses_effective_state_and_punctuation_and_keeps_exact_results_first() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
            state["items"] = json!([
                {"id":"one","source":"linear","title":"Thinking-v2 latency","reference":"ENG-1","status":"todo","detail":""},
                {"id":"two","source":"linear","title":"Thinking very slowly 2","reference":"ENG-2","status":"todo","detail":""},
                {"id":"hidden","source":"github","title":"Thinking-v2 private","status":"todo"}
            ]);
            state["overrides"] = json!({"one":{"status":"done","pinned":true},"hidden":{"hidden":true}});
            Ok(())
        }).unwrap();
    let result = search(&repo, &json!({"query":"thinkingv2"})).unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["items"][0]["id"], "one");
    assert_eq!(result["items"][0]["status"], "done");
    assert_eq!(result["items"][0]["sourceStatusCode"], "todo");
    assert_eq!(result["items"][0]["pinned"], true);
    assert_eq!(
        search(&repo, &json!({"query":"thinkingv2","lifecycle":"active"})).unwrap()["items"][0]
            ["id"],
        "two"
    );
    assert_eq!(
        search(&repo, &json!({"query":"thinkingv2","includeHidden":true})).unwrap()["total"],
        2
    );
    assert!(search(&repo, &json!({"source":"../invalid"})).is_err());
    assert!(search(&repo, &json!({"includeHidden":"true"})).is_err());
    assert_eq!(normalized("Thinking-v2 / Élite"), "thinkingv2élite");
}
#[test]
fn saved_view_search_matches_source_status_and_relationships_after_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["items"]=json!([
            {"id":"review","source":"github","kind":"Pull request","title":"API","status":"todo","sourceStatus":"Awaiting approval","relations":["reviews"],"detail":"","createdAt":"2026-10-01T00:00:00Z"},
            {"id":"detail","source":"github","kind":"Pull request","title":"CLI","status":"todo","detail":"red elephants visit idle elephants west slowly","createdAt":"2026-10-20T00:00:00Z"}
        ]);state["overrides"]=json!({"review":{"status":"done"}});Ok(())}).unwrap();
    assert_eq!(
        search(&repo, &json!({"query":"approval"})).unwrap()["items"][0]["id"],
        "review"
    );
    assert_eq!(
        search(&repo, &json!({"query":"reviews"})).unwrap()["total"],
        1
    );
    assert_eq!(
        search(&repo, &json!({"query":"reviews","lifecycle":"active"})).unwrap()["total"],
        0
    );
    let current = super::super::views::view(&repo).unwrap();
    super::super::views::update(&repo,&json!({"revision":current["revision"],"contentHash":current["contentHash"],"views":[{"id":"complete-reviews","name":"Completed reviews","parent":"github","lifecycle":"completed","filter":{"op":"all","conditions":[]},"query":"reviews","groupBy":"status"}]})).unwrap();
    let result = search(&repo, &json!({"viewId":"complete-reviews"})).unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["items"][0]["id"], "review");
    assert_eq!(result["items"][0]["status"], "done");
    let current = super::super::views::view(&repo).unwrap();
    super::super::views::update(&repo,&json!({"revision":current["revision"],"contentHash":current["contentHash"],"views":[{"id":"older-items","name":"Created before October 10","parent":"github","lifecycle":"any","filter":{"op":"beforeDate","field":"createdAt","date":"2026-10-10"},"query":"","groupBy":"status"}]})).unwrap();
    let result = search(&repo, &json!({"viewId":"older-items"})).unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["items"][0]["id"], "review");
    assert_eq!(result["items"][0]["createdAt"], "2026-10-01T00:00:00Z");
}
