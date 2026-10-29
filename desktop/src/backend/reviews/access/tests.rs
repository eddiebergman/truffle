use super::*;
use crate::backend::uuid;

fn fixture() -> (tempfile::TempDir, std::sync::Arc<Repository>, String) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "REVIEW_ACCESS_SECRET=benign-review-secret-value\n",
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    let review = json!({"revision":3,"note":"Check benign-review-secret-value carefully","privateNs":"private-review-canary",
            "comments":[{"id":"comment-a","author":"You","text":"Local feedback","path":"code.rs","root":"/missing-review-checkout","side":"additions","start":1,"end":2,"excerpt":"let x = 1;","privateNs":"private-review-canary"}],
            "assessments":[{"id":"assessment-a","author":"Reviewer","text":"Assessment","pass":"correctness","privateNs":"private-review-canary"}],
            "run":{"id":"run-a","state":"complete","inputNote":"private-review-canary","inputCommentIds":["private-review-canary"],"versions":{"private-review-canary":{}},"privateNs":"private-review-canary","passes":{"private-review-canary":"complete"}}});
    repo.transaction(|state| {
            state["sessions"] = json!([{"id":id,"cwd":"/missing-review-checkout","privateNs":"private-review-canary","reviewDraft":review,"reviewHistory":{"version":1,"privateNs":"private-review-canary","entries":[{"id":"history-a","archivedAt":1,"review":review,"privateNs":"private-review-canary"}]}}]);
            Ok(())
        }).unwrap();
    (temp, repo, id)
}

fn invoke(repo: &std::sync::Arc<Repository>, name: &str, args: Value) -> Result<Value> {
    let mut request = json!({"name":name});
    request["args"] = args;
    commands::invoke(repo, &request, true)
}

#[test]
fn named_review_reads_project_current_and_historical_data_without_writes() {
    let (_temp, repo, id) = fixture();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let current = invoke(&repo, "review.read", json!({"id":id})).unwrap();
    let history = invoke(&repo, "review.history", json!({"id":id})).unwrap();
    let archived = invoke(
        &repo,
        "review.read",
        json!({"id":id,"historyId":history["reviews"][0]["id"]}),
    )
    .unwrap();
    assert_eq!(current["readOnly"], false);
    assert_eq!(current["review"]["revision"], 3);
    assert_eq!(current["review"]["note"], "Check [redacted] carefully");
    assert_eq!(current["review"]["contentRedacted"], true);
    assert_eq!(current["review"]["comments"][0]["id"], "comment-a");
    assert_eq!(current["review"]["assessments"][0]["id"], "assessment-a");
    assert_eq!(archived["readOnly"], true);
    assert!(archived.get("noteToken").is_none());
    assert_eq!(archived["review"], current["review"]);
    assert_eq!(history["reviews"][0]["commentCount"], 1);
    assert!(history["reviews"][0].get("review").is_none());
    for value in [current, history, archived] {
        let text = value.to_string();
        for private in [
            "private-review-canary",
            "benign-review-secret-value",
            "inputNote",
            "inputCommentIds",
            "versions",
            "passes",
        ] {
            assert!(
                !text.contains(private),
                "Private review data escaped: {private}"
            );
        }
    }
    assert!(invoke(&repo, "review.read", json!({"id":id,"historyId":"missing"})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}

#[test]
fn review_note_update_uses_current_tokens_and_preserves_unrelated_state() {
    let (_temp, repo, id) = fixture();
    let current = invoke(&repo, "review.read", json!({"id":id})).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let args =
        json!({"id":id,"revision":3,"noteToken":current["noteToken"],"note":"Replacement note"});
    let receipt = invoke(&repo, "review.note.update", args.clone()).unwrap();
    assert_eq!(receipt, json!({"id":id,"revision":4,"changed":true}));
    let mut expected = before;
    expected["sessions"][0]["reviewDraft"]["note"] = json!("Replacement note");
    expected["sessions"][0]["reviewDraft"]["revision"] = json!(4);
    assert_eq!(*repo.state.lock().unwrap(), expected);
    let bytes = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        invoke(&repo, "review.note.update", args)
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    let latest = invoke(&repo, "review.read", json!({"id":id})).unwrap();
    assert_eq!(
        invoke(
            &repo,
            "review.note.update",
            json!({"id":id,"revision":4,"noteToken":latest["noteToken"],"note":"Replacement note"})
        )
        .unwrap()["changed"],
        false
    );
    for invalid in [
        json!({"id":id,"revision":4,"noteToken":current["noteToken"],"note":"stale"}),
        json!({"id":id,"revision":4,"noteToken":latest["noteToken"],"note":"é".repeat(4001)}),
        json!({"id":id,"revision":4,"noteToken":latest["noteToken"],"note":"bad","historyId":"history-a"}),
    ] {
        assert!(invoke(&repo, "review.note.update", invalid).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), expected);
}

#[test]
fn named_review_reads_reject_unsafe_identities_and_oversized_notes() {
    let (_temp, repo, id) = fixture();
    repo.transaction(|state| {
        state["sessions"][0]["reviewDraft"]["comments"][0]["id"] =
            json!("benign-review-secret-value");
        Ok(())
    })
    .unwrap();
    assert!(invoke(&repo, "review.read", json!({"id":id})).is_err());
    repo.transaction(|state| {
        state["sessions"][0]["reviewDraft"]["comments"][0]["id"] = json!("comment-a");
        state["sessions"][0]["reviewDraft"]["note"] = json!("x".repeat(8001));
        Ok(())
    })
    .unwrap();
    assert!(invoke(&repo, "review.read", json!({"id":id})).is_err());
}

#[test]
fn named_review_reads_redact_before_display_caps_and_enforce_row_bounds() {
    let (_temp, repo, id) = fixture();
    repo.transaction(|state| {
        state["sessions"][0]["reviewDraft"]["comments"][0]["excerpt"] =
            json!("x".repeat(4090) + "benign-review-secret-value" + &"y".repeat(30));
        Ok(())
    })
    .unwrap();
    let current = invoke(&repo, "review.read", json!({"id":id})).unwrap();
    assert_eq!(
        current["review"]["comments"][0]["excerpt"],
        "x".repeat(4090) + "[redac"
    );
    assert_eq!(current["review"]["contentTruncated"], true);
    assert_eq!(current["review"]["contentRedacted"], true);
    repo.transaction(|state| {
        let row = state["sessions"][0]["reviewDraft"]["comments"][0].clone();
        state["sessions"][0]["reviewDraft"]["comments"] = json!((0..101)
            .map(|index| {
                let mut row = row.clone();
                row["id"] = json!(format!("comment-{index}"));
                row
            })
            .collect::<Vec<_>>());
        Ok(())
    })
    .unwrap();
    assert!(invoke(&repo, "review.read", json!({"id":id})).is_err());
}
