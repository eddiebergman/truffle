use super::*;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, String) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s|{s["sessions"]=json!([{"id":id,"title":"Captured title","agent":"codex","cwd":temp.path(),"destination":"embedded","launchNew":true,"items":[],"checkpoint":{"next":"Captured next"},"privateNs":{"keep":true}}]);Ok(())}).unwrap();
    (temp, repo, id)
}
#[test]
fn saved_chat_open_receipt_vocabulary_is_projection_safe_before_effects() {
    for secret in ["accepted", "embedded", "acknowledged", "observed"] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".env"),
            format!("TRUFFLE_TEST_SECRET={secret}\n"),
        )
        .unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|_| Ok(())).unwrap();
        let before = std::fs::read(&repo.data).unwrap();
        let error = commands::invoke(
            &repo,
            &json!({"name":"chats.open","args":{"recordId":uuid()}}),
            false,
        )
        .unwrap_err();
        assert!(error.message.contains("safely projected"));
        assert!(!error.message.contains(secret));
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        assert!(repo.opening.lock().unwrap().is_empty());
        assert!(repo.cooldown.lock().unwrap().is_empty());
    }
}
#[test]
fn saved_chat_open_preeffect_failure_does_not_consume_admission() {
    let (_temp, repo, id) = fixture();
    repo.transaction(|s| {
        s["sessions"][0]["sessionId"] = json!(uuid());
        Ok(())
    })
    .unwrap();
    let key = format!("open:{id}");
    repo.cooldown.lock().unwrap().insert(key.clone(), 1.0);
    let before = repo.cooldown.lock().unwrap().clone();
    assert!(
        open_chat_with(&repo, &json!({"recordId":id}), |_, _| Err(Error::new(
            "Invalid launch arguments"
        )))
        .is_err()
    );
    assert_eq!(*repo.cooldown.lock().unwrap(), before);
    let mut calls = 0;
    let receipt = open_chat_with(&repo, &json!({"recordId":id}), |_, _| {
        calls += 1;
        Ok(terminal::OpenOutcome::Accepted)
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(receipt["status"], "accepted");
    let again = open_chat_with(&repo, &json!({"recordId":id}), |_, _| {
        panic!("Cooldown must not launch again")
    })
    .unwrap();
    assert_eq!(again["status"], "unknown");
}
#[test]
fn saved_chat_open_preserves_latest_metadata_and_uses_captured_context() {
    let (_temp, repo, id) = fixture();
    let receipt = open_chat_with(&repo, &json!({"recordId":id}), |record, path| {
        assert_eq!(record["title"], "Captured title");
        let context = std::fs::read_to_string(path).unwrap();
        assert!(context.contains("Captured title") && context.contains("Captured next"));
        repo.transaction(|s| {
            let row = &mut s["sessions"][0];
            row["title"] = json!("Concurrent title");
            row["items"] = json!([{"id":"note:new-link"}]);
            row["checkpoint"] = json!({"next":"New next"});
            Ok(())
        })
        .unwrap();
        Ok(terminal::OpenOutcome::Accepted)
    })
    .unwrap();
    assert_eq!(receipt["status"], "accepted");
    assert_eq!(receipt["localSaved"], true);
    assert_eq!(receipt["terminal"], json!({"state":"acknowledged"}));
    assert!(receipt.get("generation").is_none());
    let s = repo.state.lock().unwrap();
    assert_eq!(s["sessions"][0]["title"], "Concurrent title");
    assert_eq!(s["sessions"][0]["items"][0]["id"], "note:new-link");
    assert_eq!(s["sessions"][0]["checkpoint"]["next"], "New next");
    assert_eq!(s["sessions"][0]["privateNs"]["keep"], true);
    assert_eq!(s["sessions"][0]["launchNew"], false);
}
#[test]
fn saved_chat_open_accepted_then_save_failure_or_deleted_record_stays_accepted() {
    for delete in [false, true] {
        let (_temp, repo, id) = fixture();
        let before = std::fs::read(&repo.data).unwrap();
        let receipt = open_chat_with(&repo, &json!({"recordId":id}), |_, _| {
            if delete {
                repo.transaction(|s| {
                    s["sessions"] = json!([]);
                    Ok(())
                })
                .unwrap();
            } else {
                std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
            }
            Ok(terminal::OpenOutcome::Accepted)
        })
        .unwrap();
        assert_eq!(receipt["status"], "accepted");
        assert_eq!(receipt["localSaved"], false);
        assert!(repo.opening.lock().unwrap().is_empty());
        if !delete {
            assert_eq!(std::fs::read(&repo.data).unwrap(), before);
            assert_eq!(repo.state.lock().unwrap()["sessions"][0]["launchNew"], true);
        }
    }
}
#[test]
fn saved_chat_open_unknown_never_patches_launch_metadata() {
    let (_temp, repo, id) = fixture();
    let before = std::fs::read(&repo.data).unwrap();
    let mut calls = 0;
    let result = open_chat_with(&repo, &json!({"recordId":id}), |_, _| {
        calls += 1;
        Ok(terminal::OpenOutcome::Unknown)
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["terminal"]["state"], "unconfirmed");
    assert_eq!(result["localSaved"], false);
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
}
#[test]
fn saved_chat_open_snapshot_projection_excludes_private_dispatch_and_review_drafts() {
    let (_temp, repo, id) = fixture();
    repo.transaction(|s| {
        s["sessionDispatch"] = json!({"secret":"private-dispatch-canary"});
        s["sessions"][0]["workflow"] = json!("work-review");
        s["sessions"][0]["reviewDraft"] = json!({"note":"private-draft-canary"});
        Ok(())
    })
    .unwrap();
    let (_, public) = repo.capture_for_open().unwrap();
    assert_eq!(public, repo.snapshot(false));
    open_chat_with(&repo, &json!({"recordId":id}), |_, path| {
        let value = std::fs::read_to_string(path.with_extension("json")).unwrap();
        assert!(!value.contains("private-dispatch-canary"));
        assert!(!value.contains("private-draft-canary"));
        Ok(terminal::OpenOutcome::Preview)
    })
    .unwrap();
}
#[test]
fn saved_chat_open_rejects_stale_record_fields_before_launch() {
    let (_temp, repo, id) = fixture();
    for args in [
        json!({"recordId":id,"title":"Stale"}),
        json!({"recordId":id,"target":"desktop"}),
        json!({"recordId":id,"items":[]}),
        json!({"recordId":"missing"}),
    ] {
        assert!(open_chat_with(&repo, &args, |_, _| panic!(
            "Rejected intent must not launch"
        ))
        .is_err());
    }
    assert!(repo.opening.lock().unwrap().is_empty());
}
