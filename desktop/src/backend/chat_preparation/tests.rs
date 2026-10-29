use super::*;
use crate::backend::uuid;
fn fixture() -> (tempfile::TempDir, std::sync::Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    (temp, repo)
}
fn new_args(repo: &Repository) -> Value {
    json!({"requestId":uuid(),"agent":"codex","title":"Prepared fixture","cwd":repo.root,"destination":"embedded","instruction":"Private preparation instruction","itemIds":[]})
}
#[test]
fn preparation_new_identity_survives_lost_response_metadata_changes_and_restart() {
    let (temp, repo) = fixture();
    let args = new_args(&repo);
    let receipt = prepare(&repo, "new", &args).unwrap();
    assert_eq!(receipt.as_object().unwrap().len(), 5);
    assert_eq!(receipt["status"], "prepared");
    assert_eq!(receipt["reused"], false);
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
    let id = text(&receipt, "recordId");
    repo.transaction(|state| {
        let row = &mut state["sessions"][0];
        row["title"] = json!("Newer title");
        row["instruction"] = json!("Newer instruction");
        row["resources"] = json!([{"id":"preserved-resource"}]);
        row["privateNs"] = json!({"keep":true});
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    assert_eq!(prepare(&repo, "new", &args).unwrap(), receipt);
    assert_eq!(
        read(&repo, &json!({"requestId":args["requestId"]})).unwrap(),
        receipt
    );
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert!(!receipt.to_string().contains("Private"));
    assert_eq!(before["sessions"][0]["id"], id);
    drop(repo);
    let reopened = Repository::open(temp.path().into(), None, true).unwrap();
    assert_eq!(prepare(&reopened, "new", &args).unwrap(), receipt);
    assert_eq!(arr(&reopened.state.lock().unwrap()["sessions"]).len(), 1);
    assert_eq!(reopened.embedded.inventory(), [] as [serde_json::Value; 0]);
}
#[test]
fn preparation_concurrent_same_request_commits_one_record_and_one_alias() {
    let (_temp, repo) = fixture();
    let args = new_args(&repo);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut threads = vec![];
    for _ in 0..2 {
        let repo = repo.clone();
        let args = args.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            prepare(&repo, "new", &args).unwrap()
        }));
    }
    barrier.wait();
    let first = threads.remove(0).join().unwrap();
    let second = threads.remove(0).join().unwrap();
    assert_eq!(first, second);
    let state = repo.state.lock().unwrap();
    assert_eq!(arr(&state["sessions"]).len(), 1);
    assert_eq!(aliases(&state["sessions"][0]).unwrap().len(), 1);
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}
#[test]
fn preparation_reused_requests_keep_all_original_aliases_and_latest_attached_metadata() {
    let (_temp, repo) = fixture();
    let mut args = new_args(&repo);
    args.as_object_mut().unwrap().remove("instruction");
    args["sessionId"] = json!(uuid());
    let first = prepare(&repo, "attach", &args).unwrap();
    let id = first["recordId"].clone();
    repo.transaction(|state| {
        let row = &mut state["sessions"][0];
        row["title"] = json!("Newer associated title");
        row["cwd"] = json!("/not-a-current-directory");
        row["privateNs"] = json!({"keep":true});
        row["items"] = json!([{"id":"latest-link"}]);
        row["preparationRequests"]["privateNs"] = json!({"keep":true});
        Ok(())
    })
    .unwrap();
    let mut second_args = args.clone();
    second_args["requestId"] = json!(uuid());
    second_args["title"] = json!("Stale title");
    let second = prepare(&repo, "attach", &second_args).unwrap();
    assert_eq!(second["recordId"], id);
    assert_eq!(second["reused"], true);
    assert_eq!(prepare(&repo, "attach", &args).unwrap(), first);
    assert_eq!(prepare(&repo, "attach", &second_args).unwrap(), second);
    let state = repo.state.lock().unwrap();
    let row = &state["sessions"][0];
    assert_eq!(row["title"], "Newer associated title");
    assert_eq!(row["cwd"], "/not-a-current-directory");
    assert_eq!(row["items"][0]["id"], "latest-link");
    assert_eq!(row["privateNs"]["keep"], true);
    assert_eq!(row["preparationRequests"]["privateNs"]["keep"], true);
    assert_eq!(aliases(row).unwrap().len(), 2);
}
#[test]
fn preparation_changed_intent_and_malformed_state_fail_without_effects() {
    let (_temp, repo) = fixture();
    let args = new_args(&repo);
    prepare(&repo, "new", &args).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    let mut changed = args.clone();
    changed["title"] = json!("Different intent");
    assert_eq!(prepare(&repo, "new", &changed).unwrap_err().status, 409);
    let mut extra = args;
    extra["id"] = json!(uuid());
    assert!(prepare(&repo, "new", &extra).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    repo.transaction(|state| {
        state["sessions"][0]["preparationRequests"]["version"] = json!(2);
        Ok(())
    })
    .unwrap();
    let disk = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        prepare(&repo, "new", &new_args(&repo)).unwrap_err().status,
        409
    );
    assert!(read(&repo, &json!({"requestId":uuid()})).is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}
#[test]
fn preparation_full_alias_capacity_allows_duplicates_but_rejects_new_reuse_before_effects() {
    let (_temp, repo) = fixture();
    let mut args = new_args(&repo);
    args.as_object_mut().unwrap().remove("instruction");
    args["sessionId"] = json!(uuid());
    let first = prepare(&repo, "attach", &args).unwrap();
    repo.transaction(|state| {
        let entries = state["sessions"][0]["preparationRequests"]["entries"]
            .as_array_mut()
            .unwrap();
        for _ in 1..MAX_ALIASES {
            entries.push(
                json!({"requestId":uuid(),"kind":"attach","hash":"0".repeat(64),"reused":true}),
            );
        }
        Ok(())
    })
    .unwrap();
    let disk = std::fs::read(&repo.data).unwrap();
    assert_eq!(prepare(&repo, "attach", &args).unwrap(), first);
    let mut next = args.clone();
    next["requestId"] = json!(uuid());
    assert_eq!(prepare(&repo, "attach", &next).unwrap_err().status, 409);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert_eq!(
        aliases(&repo.state.lock().unwrap()["sessions"][0])
            .unwrap()
            .len(),
        MAX_ALIASES
    );
}
#[test]
fn preparation_failed_commit_has_no_alias_and_exact_retry_creates_once() {
    let (_temp, repo) = fixture();
    let args = new_args(&repo);
    let disk = std::fs::read(&repo.data).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let temporary = repo.data.with_extension("tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert!(prepare(&repo, "new", &args).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert_eq!(
        read(&repo, &json!({"requestId":args["requestId"]})).unwrap()["status"],
        "not-found"
    );
    std::fs::remove_dir(temporary).unwrap();
    let receipt = prepare(&repo, "new", &args).unwrap();
    assert_eq!(prepare(&repo, "new", &args).unwrap(), receipt);
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}
#[test]
fn preparation_read_is_state_only_and_duplicate_skill_bypasses_removed_configuration() {
    let (_temp, repo) = fixture();
    let preset = uuid();
    repo.transaction(|state|{state["skillPresets"]=json!([{"id":preset,"title":"Fixture","cwd":repo.root,"skill":"fixture.md","reports":"reports","command":"$fixture","agent":"codex"}]);Ok(())}).unwrap();
    std::fs::write(repo.root.join("fixture.md"), "Fixture skill").unwrap();
    let args = json!({"requestId":uuid(),"presetId":preset});
    let first = prepare(&repo, "skill", &args).unwrap();
    let config = repo.data.parent().unwrap().join("config/skills.json");
    std::fs::remove_file(config).unwrap();
    std::fs::remove_file(repo.root.join("fixture.md")).unwrap();
    let disk = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        read(&repo, &json!({"requestId":args["requestId"]})).unwrap(),
        first
    );
    assert_eq!(prepare(&repo, "skill", &args).unwrap(), first);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}
#[test]
fn preparation_view_failed_validation_and_persistence_do_not_consume_admission() {
    let (_temp, repo) = fixture();
    let args = json!({"requestId":uuid(),"agent":"codex","source":"github","question":"Which needs attention?","itemIds":["known"],"filters":{},"folderPath":repo.root});
    let cooldown = repo.cooldown.lock().unwrap().clone();
    assert!(prepare(&repo, "view", &args).is_err());
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
    repo.transaction(|state| {
        state["items"] = json!([{"id":"known","source":"github","title":"Known"}]);
        Ok(())
    })
    .unwrap();
    let disk = std::fs::read(&repo.data).unwrap();
    let temporary = repo.data.with_extension("tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert!(prepare(&repo, "view", &args).is_err());
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    std::fs::remove_dir(temporary).unwrap();
    let receipt = prepare(&repo, "view", &args).unwrap();
    assert_eq!(prepare(&repo, "view", &args).unwrap(), receipt);
    assert_eq!(arr(&repo.state.lock().unwrap()["sessions"]).len(), 1);
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}
#[test]
fn preparation_unverifiable_legacy_view_collision_is_rejected_without_writes_or_admission() {
    let (_temp, repo) = fixture();
    let request = uuid();
    let record_id = uuid();
    repo.transaction(|state| {
            state["sessions"] = json!([{"id":record_id,"viewRequestId":request,"title":"Legacy view","agent":"codex","cwd":repo.root,"instruction":"Inspect","launchNew":true,"items":[]}]);
            Ok(())
        }).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    let args = json!({"requestId":request,"agent":"codex","source":"github","question":"Inspect","itemIds":["not-required-for-existing-view"],"filters":{},"folderPath":repo.root});
    for question in ["Inspect", "Different question"] {
        let mut changed = args.clone();
        changed["question"] = json!(question);
        let error = prepare(&repo, "view", &changed).unwrap_err();
        assert_eq!(error.status, 409);
        assert!(error.message.contains("no verified intent hash"));
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    }
    let legacy = super::super::view_context::prepare(&repo, &args).unwrap();
    assert_eq!(legacy["id"], record_id);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert!(!repo.data.parent().unwrap().join("views").exists());
    assert!(repo.cooldown.lock().unwrap().is_empty());
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}
#[test]
fn preparation_malformed_alias_record_identity_never_becomes_actionable() {
    let (_temp, repo) = fixture();
    let args = new_args(&repo);
    let first = prepare(&repo, "new", &args).unwrap();
    repo.transaction(|state| {
        state["sessions"][0]["id"] = json!("not-a-chat-uuid");
        Ok(())
    })
    .unwrap();
    let disk = std::fs::read(&repo.data).unwrap();
    assert!(read(&repo, &json!({"requestId":first["requestId"]})).is_err());
    assert!(prepare(&repo, "new", &args).is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}

#[test]
fn preparation_transaction_rechecks_identity_after_two_preflight_misses() {
    let (_temp, repo) = fixture();
    let args = new_args(&repo);
    let (id, payload) = input("new", &args).unwrap();
    let hash = context_kit::sha256(
        &serde_json::to_vec(&json!({"version":1,"kind":"new","intent":payload})).unwrap(),
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let lock = repo.state.lock().unwrap();
    let mut threads = vec![];
    for _ in 0..2 {
        let repo = repo.clone();
        let payload = payload.clone();
        let request = Request {
            id: id.clone(),
            kind: "new".into(),
            hash: hash.clone(),
        };
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            super::super::workflows::prepare_with_request(&repo, &payload, Some(&request)).unwrap()
                [RECEIPT]
                .clone()
        }));
    }
    barrier.wait();
    drop(lock);
    let first = threads.remove(0).join().unwrap();
    assert_eq!(threads.remove(0).join().unwrap(), first);
    let state = repo.state.lock().unwrap();
    assert_eq!(arr(&state["sessions"]).len(), 1);
    assert_eq!(aliases(&state["sessions"][0]).unwrap().len(), 1);
}
#[test]
fn preparation_multiple_reuse_aliases_survive_restart_without_retargeting() {
    let (temp, repo) = fixture();
    let mut args = new_args(&repo);
    args.as_object_mut().unwrap().remove("instruction");
    args["sessionId"] = json!(uuid());
    let mut receipts = vec![];
    for _ in 0..12 {
        args["requestId"] = json!(uuid());
        let receipt = prepare(&repo, "attach", &args).unwrap();
        receipts.push((args.clone(), receipt));
    }
    let target = receipts[0].1["recordId"].clone();
    assert!(receipts
        .iter()
        .all(|(_, receipt)| receipt["recordId"] == target));
    drop(repo);
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let disk = std::fs::read(&repo.data).unwrap();
    for (args, receipt) in receipts {
        assert_eq!(prepare(&repo, "attach", &args).unwrap(), receipt);
    }
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert_eq!(arr(&repo.state.lock().unwrap()["sessions"]).len(), 1);
}
