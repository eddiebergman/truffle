use super::*;
use crate::backend::settings::Approval;
use crate::backend::{now, uuid};
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("SKILL.md"), "Fixture skill").unwrap();
    std::fs::create_dir(temp.path().join("reports")).unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["notes"]=json!([{"id":"note:preparation-fixture","source":"notes","kind":"Project","title":"Fixture project","cwd":temp.path(),"status":"todo"}]);state["skillPresets"]=json!([{"id":"admission-fixture","title":"Fixture","cwd":temp.path(),"skill":"SKILL.md","reports":"reports","agent":"codex"}]);Ok(())}).unwrap();
    (temp, repo)
}
fn key(kind: &str) -> &str {
    match kind {
        "quick" => "quick-session",
        "review" => "work-review",
        _ => "newsletter:admission-fixture",
    }
}
fn prepare(repo: &Repository, kind: &str) -> Result<Value> {
    match kind {
        "quick" => save(
            repo,
            Kind::Quick,
            &json!({"agent":"codex","itemId":"note:preparation-fixture"}),
        ),
        "review" => save(repo, Kind::Review, &json!({"agent":"codex"})),
        _ => save(repo, Kind::Skill, &json!({"presetId":"admission-fixture"})),
    }
}
#[test]
fn preparation_validation_failure_preserves_expired_admission_and_allows_immediate_corrected_retry()
{
    let (temp, repo) = fixture();
    repo.cooldown
        .lock()
        .unwrap()
        .insert("quick-session".into(), 1.0);
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let cooldown = repo.cooldown.lock().unwrap().clone();
    assert!(save(
        &repo,
        Kind::Quick,
        &json!({"agent":"codex","folderPath":temp.path().join("missing-folder")}),
    )
    .is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
    let result = prepare(&repo, "quick").unwrap();
    assert_ne!(text(&result, "id"), "");
    assert!(repo.cooldown.lock().unwrap()["quick-session"] > now());
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}
#[test]
fn preparation_failed_canonical_save_preserves_every_admission_and_corrected_retry_succeeds() {
    for kind in ["quick", "review", "newsletter"] {
        let (_temp, repo) = fixture();
        repo.cooldown.lock().unwrap().insert(key(kind).into(), 1.0);
        repo.cooldown
            .lock()
            .unwrap()
            .insert("unrelated".into(), 123.0);
        let before = repo.state.lock().unwrap().clone();
        let bytes = std::fs::read(&repo.data).unwrap();
        let cooldown = repo.cooldown.lock().unwrap().clone();
        std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
        assert!(prepare(&repo, kind).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before, "{kind}");
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes, "{kind}");
        assert_eq!(*repo.cooldown.lock().unwrap(), cooldown, "{kind}");
        std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
        prepare(&repo, kind).unwrap();
        assert!(repo.cooldown.lock().unwrap()[key(kind)] > now());
        assert_eq!(repo.cooldown.lock().unwrap()["unrelated"], 123.0);
        assert_eq!(
            repo.state.lock().unwrap()["sessions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            repo.terminals.inventory().unwrap().rows(),
            [] as [serde_json::Value; 0]
        );
    }
}
#[test]
fn preparation_reuses_queued_reviews_and_newsletters_without_admission_or_deadline_extension() {
    for kind in ["review", "newsletter"] {
        let (_temp, repo) = fixture();
        let first = prepare(&repo, kind).unwrap();
        repo.cooldown
            .lock()
            .unwrap()
            .insert(key(kind).into(), now() + 86400.0);
        let cooldown = repo.cooldown.lock().unwrap().clone();
        let state = repo.state.lock().unwrap().clone();
        let second = prepare(&repo, kind).unwrap();
        assert_eq!(first["id"], second["id"]);
        assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
        assert_eq!(*repo.state.lock().unwrap(), state);
    }
}
#[test]
fn preparation_concurrent_requests_serialize_one_admission_and_preserve_review_reuse() {
    for kind in ["quick", "review", "newsletter"] {
        let (_temp, repo) = fixture();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = vec![];
        for _ in 0..2 {
            let repo = repo.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                prepare(&repo, kind)
            }));
        }
        barrier.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            repo.state.lock().unwrap()["sessions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(repo.cooldown.lock().unwrap().len(), 1);
        assert!(repo.cooldown.lock().unwrap()[key(kind)] > now());
        if kind == "quick" {
            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
            assert!(results.iter().any(|result| result
                .as_ref()
                .is_err_and(|error| error.message.contains("just started"))));
        } else {
            assert_eq!(
                results[0].as_ref().unwrap()["id"],
                results[1].as_ref().unwrap()["id"]
            );
        }
        assert_eq!(
            repo.terminals.inventory().unwrap().rows(),
            [] as [serde_json::Value; 0]
        );
    }
}
#[test]
fn preparation_newsletter_validation_failure_does_not_consume_the_retry_window() {
    let (temp, repo) = fixture();
    repo.transaction(|state| {
        state["skillPresets"][0]["cwd"] = json!(temp.path().join("missing-folder"));
        Ok(())
    })
    .unwrap();
    repo.cooldown
        .lock()
        .unwrap()
        .insert("newsletter:admission-fixture".into(), 1.0);
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let cooldown = repo.cooldown.lock().unwrap().clone();
    assert!(prepare(&repo, "newsletter").is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
    repo.transaction(|state| {
        state["skillPresets"][0]["cwd"] = json!(temp.path());
        Ok(())
    })
    .unwrap();
    prepare(&repo, "newsletter").unwrap();
}
#[test]
fn preparation_saved_pr_review_reuse_bypasses_admission_even_after_it_was_opened() {
    let (_temp, repo) = fixture();
    let settings = repo.settings.view();
    repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":"$fixture-review"}}),Approval::User).unwrap();
    let id = uuid();
    repo.transaction(|state|{state["items"]=json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Fixture PR","status":"todo"}]);state["sessions"]=json!([{"id":id,"agent":"codex","workflow":"pr-review","reviewItemId":"github:fixture","archivedAt":null,"launchNew":false,"title":"Saved review","items":[]}]);Ok(())}).unwrap();
    repo.cooldown
        .lock()
        .unwrap()
        .insert("pr-review-prepare".into(), now() + 86400.0);
    let before = repo.state.lock().unwrap().clone();
    let cooldown = repo.cooldown.lock().unwrap().clone();
    let reused = save(
        &repo,
        Kind::PrReview,
        &json!({"agent":"codex","itemId":"github:fixture"}),
    )
    .unwrap();
    assert_eq!(reused["id"], id);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
}
