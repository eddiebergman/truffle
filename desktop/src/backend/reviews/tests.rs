use super::*;
use crate::backend::operations::Caller;
use crate::backend::sessions;
use crate::backend::workflows;
use std::path::PathBuf;
fn fixture() -> (
    tempfile::TempDir,
    std::sync::Arc<Repository>,
    String,
    PathBuf,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("checkout");
    std::fs::create_dir(&root).unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(status.status.success());
    };
    git(&["init", "-b", "main"]);
    std::fs::write(root.join("code.txt"), "old\n").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "-m",
        "fixture",
    ]);
    std::fs::write(root.join("code.txt"), "new\nmore\n").unwrap();
    let repo = Repository::open(temp.path().to_path_buf(), None, true).unwrap();
    let record = repo
        .transaction(|s| {
            sessions::save(
                s,
                &json!({"agent":"claude","cwd":root,"title":"Review fixture","itemIds":[]}),
            )
        })
        .unwrap();
    (temp, repo, text(&record, "id").to_owned(), root)
}
fn append(repo: &Repository, id: &str, key: &str, author: &str) -> Value {
    action(repo,&Git,&json!({"id":id,"action":"append","comments":[{"id":key,"author":author,"path":"code.txt","side":"additions","start":1,"end":2,"text":"Check this"}]})).unwrap()
}
#[test]
fn new_review_preserves_history_and_rejects_stale_or_active_resets() {
    let (_tmp, repo, id, _root) = fixture();
    let first = append(&repo, &id, "one", "You");
    assert!(action(&repo, &Git, &json!({"id":id,"action":"new","revision":0})).is_err());
    let request = uuid();
    let active = action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":request}),
    )
    .unwrap();
    assert!(action(
        &repo,
        &Git,
        &json!({"id":id,"action":"new","revision":active["review"]["revision"]})
    )
    .is_err());
    let done = action(
        &repo,
        &Git,
        &json!({"id":id,"action":"status","requestId":request,"state":"complete"}),
    )
    .unwrap();
    let fresh = action(
        &repo,
        &Git,
        &json!({"id":id,"action":"new","revision":done["review"]["revision"]}),
    )
    .unwrap();
    assert_eq!(fresh["review"]["comments"], json!([]));
    assert!(fresh["review"]["run"].is_null());
    assert_eq!(
        fresh["review"]["revision"].as_u64().unwrap(),
        done["review"]["revision"].as_u64().unwrap() + 1
    );
    let history = action(&repo, &Git, &json!({"id":id,"action":"history"})).unwrap();
    assert_eq!(history["reviews"][0]["review"], done["review"]);
    assert_eq!(
        history["reviews"][0]["review"]["comments"][0]["text"],
        first["review"]["comments"][0]["text"]
    );
    assert!(action(
        &repo,
        &Git,
        &json!({"id":id,"action":"status","requestId":request,"state":"complete"})
    )
    .is_err());
    let retained = action(&repo, &Git, &json!({"id":id,"action":"history"})).unwrap();
    assert_eq!(retained, history);
    for n in 0..22 {
        let note = action(
            &repo,
            &Git,
            &json!({"id":id,"action":"note","noteToken":note_token(""),"note":format!("Round {n}")}),
        )
        .unwrap();
        action(
            &repo,
            &Git,
            &json!({"id":id,"action":"new","revision":note["review"]["revision"]}),
        )
        .unwrap();
    }
    let history = action(&repo, &Git, &json!({"id":id,"action":"history"})).unwrap();
    assert_eq!(arr(&history["reviews"]).len(), 20);
    assert_eq!(history["reviews"][0]["review"]["note"], "Round 21");
}
#[test]
fn multiple_sources_append_idempotently_and_ack_preserves_later_comments() {
    let (tmp, repo, id, root) = fixture();
    let alias = tmp.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    repo.transaction(|s| {
        s["sessions"][0]["cwd"] = json!(alias);
        Ok(())
    })
    .unwrap();
    let first = append(&repo, &id, "one", "You");
    assert_eq!(arr(&first["review"]["comments"]).len(), 1);
    let again = append(&repo, &id, "one", "You");
    assert_eq!(again, first);
    append(&repo, &id, "two", "Background agent");
    let result = action(
        &repo,
        &Git,
        &json!({"id":id,"action":"ack","commentIds":["one"],"note":""}),
    )
    .unwrap();
    assert_eq!(arr(&result["review"]["comments"]).len(), 1);
    assert_eq!(
        result["review"]["comments"][0]["author"],
        "Background agent"
    );
    let reply = workflows::registration_reply(&result);
    assert_eq!(reply["review"], result["review"]);
}
#[test]
fn bad_batch_rolls_back_and_note_edit_is_guarded() {
    let (_tmp, repo, id, root) = fixture();
    let first = append(&repo, &id, "one", "You");
    let bad = json!({"id":id,"action":"append","comments":[{"id":"two","path":"code.txt","side":"additions","start":1,"text":"Valid"},{"path":"../outside","side":"additions","start":1,"text":"Invalid"}]});
    assert!(action(&repo, &Git, &bad).is_err());
    assert_eq!(
        action(&repo, &Git, &json!({"id":id,"action":"read"})).unwrap(),
        first
    );
    assert!(action(&repo,&Git,&json!({"id":id,"action":"append","comments":[{"path":"code.txt","root":root,"side":"additions","start":1,"fingerprint":"outdated","text":"Check"}]})).is_err());
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"note","note":"first","noteToken":note_token("")}),
    )
    .unwrap();
    assert!(action(
        &repo,
        &Git,
        &json!({"id":id,"action":"note","note":"stale edit","noteToken":note_token("")})
    )
    .is_err());
    // static/diff-review.js computes the same token.
    assert_eq!(
        note_token("Previous note"),
        "4324a0cfe77bce0c3655cba138ff0c8c7889ebc0a6cd4e29ff7ed7725d781707"
    );
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"ack","commentIds":[],"note":"old"}),
    )
    .unwrap();
    assert_eq!(
        action(&repo, &Git, &json!({"id":id,"action":"read"})).unwrap()["review"]["note"],
        "first"
    );
}
#[test]
fn reviewer_assessments_are_scoped_durable_and_ack_preserves_new_arrivals() {
    let (_tmp, repo, id, root) = fixture();
    let request = uuid();
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":request,"root":root}),
    )
    .unwrap();
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"note","note":"My summary","noteToken":note_token("")}),
    )
    .unwrap();
    let report = json!({"id":id,"operation":"review","requestId":request,"pass":"simplification","state":"complete","report":{"comments":[],"overall":{"author":"Agent A","text":"No simplification needed. Scope: code.txt."}}});
    let first = workflows::register(&repo, &report).unwrap();
    assert_eq!(first["review"]["assessments"][0]["author"], "Agent A");
    assert_eq!(workflows::register(&repo, &report).unwrap(), first);
    let mut invalid = report.clone();
    invalid["requestId"] = json!(uuid());
    assert!(workflows::register(&repo, &invalid).is_err());
    invalid = report.clone();
    invalid["report"]["overall"]["text"] = json!("Changed");
    assert!(workflows::register(&repo, &invalid).is_err());
    invalid = report.clone();
    invalid["pass"] = json!("correctness");
    invalid["report"]["overall"]["text"] = json!("x".repeat(8001));
    assert!(workflows::register(&repo, &invalid).is_err());
    let mut second = report;
    second["pass"] = json!("performance");
    second["report"]["overall"]["author"] = json!("Agent B");
    workflows::register(&repo, &second).unwrap();
    let result=action(&repo,&Git,&json!({"id":id,"action":"ack","commentIds":[],"assessmentIds":[first["review"]["assessments"][0]["id"]],"note":"old summary"})).unwrap();
    assert_eq!(arr(&result["review"]["assessments"]).len(), 2);
    assert!(result["review"]["assessments"][0]["sentAt"].is_number());
    assert!(result["review"]["assessments"][1]["sentAt"].is_null());
    assert_eq!(result["review"]["note"], "My summary");
    assert_eq!(
        workflows::registration_reply(&result)["review"],
        result["review"]
    );
}
#[test]
fn completion_consumes_only_verified_feedback_and_keeps_later_comments_pending() {
    let (_tmp, repo, id, _root) = fixture();
    let request = uuid();
    append(&repo, &id, "before", "You");
    let previous = uuid();
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":previous}),
    )
    .unwrap();
    action(&repo,&Git,&json!({"id":id,"action":"append","requestId":previous,"pass":"correctness","overall":{"author":"Claude","text":"Earlier review"}})).unwrap();
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"status","requestId":previous,"state":"cancelled"}),
    )
    .unwrap();
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":request}),
    )
    .unwrap();
    append(&repo, &id, "during", "You");
    let read = action(&repo, &Git, &json!({"id":id,"action":"read"})).unwrap();
    let revision = read["review"]["revision"].clone();
    append(&repo, &id, "late", "You");
    assert!(action(&repo,&Git,&json!({"id":id,"action":"status","requestId":request,"state":"complete","reviewedRevision":revision})).is_err());
    let current = action(&repo, &Git, &json!({"id":id,"action":"read"})).unwrap();
    let done=action(&repo,&Git,&json!({"id":id,"action":"status","requestId":request,"state":"complete","reviewedRevision":current["review"]["revision"]})).unwrap();
    assert_eq!(arr(&done["review"]["comments"]).len(), 3);
    assert!(arr(&done["review"]["comments"])
        .iter()
        .all(|c| c["reviewedAt"].is_number()));
    assert!(done["review"]["assessments"][0]["reviewedAt"].is_number());
    let after = append(&repo, &id, "after", "You");
    assert!(after["review"]["comments"][3]["reviewedAt"].is_null());
}
#[test]
fn conversation_review_accepts_new_roots_and_marks_changed_versions_without_writing() {
    let (tmp, repo, id, root) = fixture();
    let other = tmp.path().join("other");
    assert!(std::process::Command::new("git")
        .args(["clone", "--quiet"])
        .arg(&root)
        .arg(&other)
        .status()
        .unwrap()
        .success());
    std::fs::write(other.join("code.txt"), "review this second repository\n").unwrap();
    let request = uuid();
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":request}),
    )
    .unwrap();
    repo.transaction(|s| {
        s["sessions"][0]["worktrees"] = json!([{"path":other}]);
        Ok(())
    })
    .unwrap();
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"status","requestId":request,"state":"reviewing"}),
    )
    .unwrap();
    action(&repo,&Git,&json!({"id":id,"action":"append","requestId":request,"pass":"cross-repo","overall":{"author":"Agent","text":"Checked both repositories."},"comments":[{"id":"other-comment","root":other,"path":"code.txt","side":"additions","start":1,"end":1,"text":"Check the other repository."}]})).unwrap();
    let revision = repo.snapshot(false)["revision"].clone();
    let checked = action(&repo, &Git, &json!({"id":id,"action":"check"})).unwrap();
    assert_eq!(checked["review"]["freshness"], "current");
    assert_eq!(checked["review"]["comments"][0]["freshness"], "current");
    std::fs::write(other.join("code.txt"), "changed in second repository\n").unwrap();
    let checked = action(&repo, &Git, &json!({"id":id,"action":"check"})).unwrap();
    assert_eq!(checked["review"]["freshness"], "outdated");
    assert_eq!(checked["review"]["assessments"][0]["freshness"], "outdated");
    assert_eq!(checked["review"]["comments"][0]["freshness"], "outdated");
    assert_eq!(repo.snapshot(false)["revision"], revision);
    assert_eq!(
        freshness(&json!({"/missing":"before"}), &json!({"/missing":null})),
        "unverified"
    );
}
#[test]
fn review_progress_accepts_dynamic_reviewers_and_parent_completion() {
    let (_tmp, repo, id, root) = fixture();
    let request = uuid();
    assert!(workflows::register(
        &repo,
        &json!({"id":id,"operation":"review","report":{"comments":"invalid"}})
    )
    .is_err());
    let start = action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":request,"root":root}),
    )
    .unwrap();
    assert_eq!(start["review"]["run"]["state"], "requested");
    assert!(action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":uuid(),"root":root})
    )
    .is_err());
    assert!(action(
        &repo,
        &Git,
        &json!({"id":id,"action":"status","requestId":uuid(),"state":"complete"})
    )
    .is_err());
    assert_eq!(start["review"]["run"]["passes"], json!({}));
    for pass in ["security", "api-contract"] {
        workflows::register(&repo,&json!({"id":id,"operation":"review","requestId":request,"pass":pass,"state":"reviewing"})).unwrap();
        workflows::register(&repo,&json!({"id":id,"operation":"review","requestId":request,"pass":pass,"state":"complete"})).unwrap();
    }
    workflows::register(&repo,&json!({"id":id,"operation":"review","requestId":request,"pass":"extra-reviewer","state":"reviewing"})).unwrap();
    assert!(workflows::register(&repo,&json!({"id":id,"operation":"review","requestId":request,"pass":"invalid name","state":"reviewing"})).is_err());
    let end = workflows::register(
        &repo,
        &json!({"id":id,"operation":"review","requestId":request,"state":"complete"}),
    )
    .unwrap();
    assert_eq!(end["review"]["run"]["state"], "complete");
    assert!(action(
        &repo,
        &Git,
        &json!({"id":id,"action":"status","requestId":request,"state":"reviewing"})
    )
    .is_err());
    assert!(action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":uuid(),"root":root})
    )
    .is_ok());
}
#[test]
fn review_check_command_summary_is_read_only_fresh_and_excludes_private_draft_payloads() {
    let (_temp, repo, id, root) = fixture();
    append(&repo, &id, "summary-comment", "You");
    let request = uuid();
    action(
        &repo,
        &Git,
        &json!({"id":id,"action":"request","requestId":request}),
    )
    .unwrap();
    action(&repo,&Git,&json!({"id":id,"action":"append","requestId":request,"pass":"correctness","overall":{"author":"Agent","text":"Private assessment text"}})).unwrap();
    repo.transaction(|s| {
        let review = &mut s["sessions"][0]["reviewDraft"];
        review["privateNs"] = json!("private-summary-canary");
        review["note"] = json!("private-summary-canary");
        review["run"]["privateNs"] = json!("private-summary-canary");
        review["comments"][0]["privateNs"] = json!("private-summary-canary");
        review["comments"][0]["reviewedAt"] = json!(10);
        review["assessments"][0]["sentAt"] = json!(20);
        review["assessments"][0]["privateNs"] = json!("private-summary-canary");
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let result = super::super::commands::invoke(
        &repo,
        &json!({"name":"review.check","args":{"id":id}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(result["id"], id);
    assert_eq!(
        result["revision"],
        before["sessions"][0]["reviewDraft"]["revision"]
    );
    assert_eq!(result["freshness"], "current");
    assert_eq!(result["run"]["id"], request);
    assert_eq!(
        result["comments"][0],
        json!({"id":"summary-comment","freshness":"current","reviewedAt":10.0})
    );
    assert_eq!(result["assessments"][0]["sentAt"], 20.0);
    for private in [
        "private-summary-canary",
        "Private assessment text",
        "Check this",
        "versions",
        "code.txt",
    ] {
        assert!(!result.to_string().contains(private));
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::write(root.join("code.txt"), "Changed since review\n").unwrap();
    let changed = check_summary(&repo, &json!({"id":id})).unwrap();
    assert_eq!(changed["freshness"], "outdated");
    assert_eq!(changed["comments"][0]["freshness"], "outdated");
    assert_eq!(changed["assessments"][0]["freshness"], "outdated");
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        check_summary(&repo, &json!({"id":id})).unwrap()["freshness"],
        "unverified"
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn review_check_summary_rejects_extra_arguments_and_excessive_status_rows() {
    let (_temp, repo, id, _) = fixture();
    for args in [
        json!({"id":id,"cwd":"/private"}),
        json!({"id":id,"action":"clear"}),
        json!({"id":7}),
    ] {
        assert!(check_summary(&repo, &args).is_err());
    }
    repo.transaction(|s|{s["sessions"][0]["reviewDraft"]=json!({"revision":0,"comments":[],"assessments":(0..101).map(|index|json!({"id":format!("assessment-{index}")})).collect::<Vec<_>>()});Ok(())}).unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert!(check_summary(&repo, &json!({"id":id}))
        .unwrap_err()
        .message
        .contains("100"));
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
