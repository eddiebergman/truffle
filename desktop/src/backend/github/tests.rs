use super::*;
fn pr() -> Value {
    json!({"id":"github:PR_test","source":"github","kind":"Pull request","status":"needs-review","isDraft":false,"relations":["reviews"],"title":"Test PR","url":"https://github.com/test/repo/pull/1","canUpdate":true,"githubState":"OPEN","requestedReviewers":[]})
}
#[test]
fn notification_cycles_survive_seen_and_reappear_only_after_a_new_request_or_ready_transition() {
    let mut s = json!({"items":[pr()]});
    observe(&mut s);
    let first = s["githubReviewInbox"]["github:PR_test"]["version"].clone();
    s["githubReviewInbox"]["github:PR_test"]["seenAt"] = json!(1);
    observe(&mut s);
    assert_eq!(s["githubReviewInbox"]["github:PR_test"]["version"], first);
    s["items"][0]["isDraft"] = json!(true);
    observe(&mut s);
    assert_eq!(s["githubReviewInbox"]["github:PR_test"]["active"], false);
    s["items"][0]["isDraft"] = json!(false);
    observe(&mut s);
    assert_ne!(s["githubReviewInbox"]["github:PR_test"]["version"], first);
    assert!(s["githubReviewInbox"]["github:PR_test"]["seenAt"].is_null());
    s["items"][0]["relations"] = json!(["linked"]);
    observe(&mut s);
    assert_eq!(s["githubReviewInbox"]["github:PR_test"]["active"], false);
}
#[test]
fn demo_actions_preserve_existing_reviewers_and_stale_seen_does_not_clear_a_new_cycle() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(
        temp.path().to_path_buf(),
        Some(temp.path().join("test.json")),
        true,
    )
    .unwrap();
    repo.transaction(|s| {
        s["items"] = json!([pr()]);
        observe(s);
        Ok(())
    })
    .unwrap();
    seen(
        &repo,
        &json!({"reviews":[{"id":"github:PR_test","version":uuid()}]}),
    )
    .unwrap();
    assert!(repo.snapshot(false)["githubReviewInbox"]["github:PR_test"]["seenAt"].is_null());
    action(
        &repo,
        &json!({"id":"github:PR_test","action":"request-review","login":"someone"}),
    )
    .unwrap();
    assert_eq!(
        repo.snapshot(false)["items"][0]["requestedReviewers"][0]["login"],
        "someone"
    );
    assert!(action(
        &repo,
        &json!({"id":"github:PR_test","action":"request-review","login":"bad\"input"})
    )
    .is_err());
    assert!(action(&repo, &json!({"id":"linear:wrong","action":"ready"})).is_err());
}
fn action_fixture(demo: bool) -> (tempfile::TempDir, std::sync::Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, demo).unwrap();
    repo.transaction(|state| {
        let mut item = pr();
        item["privateExtension"] = json!("private-item-sentinel");
        state["items"] = json!([item]);
        Ok(())
    })
    .unwrap();
    (temp, repo)
}
fn remote_pr() -> Value {
    json!({"id":"PR_test","isDraft":false,"state":"OPEN","title":"Test PR","url":"https://github.com/test/repo/pull/1","repository":{"nameWithOwner":"test/repo"},"author":{"login":"author"},"reviewRequests":{"nodes":[{"requestedReviewer":{"__typename":"User","login":"existing"}},{"requestedReviewer":{"__typename":"User","login":"someone"}}]}})
}
#[test]
fn action_receipts_report_preview_or_confirmed_cache_commit_without_private_items() {
    let (_temp, repo) = action_fixture(true);
    for operation in ["ready", "request-review", "refresh"] {
        let receipt = action_with(
            &repo,
            &json!({"id":"github:PR_test","action":operation,"login":"someone"}),
            |_, _, _| panic!("Demo must not contact GitHub"),
        )
        .unwrap();
        assert_eq!(
            receipt,
            json!({"id":"github:PR_test","action":operation,"status":"preview","remote":"not-contacted","localSaved":true})
        );
        assert!(!receipt.to_string().contains("private-item"));
    }
    let (_temp, repo) = action_fixture(false);
    let mut called = 0;
    let receipt = action_with(
        &repo,
        &json!({"id":"github:PR_test","action":"request-review","login":"someone"}),
        |id, action, login| {
            called += 1;
            assert_eq!(
                (id, action, login),
                ("PR_test", "request-review", "someone")
            );
            Ok((remote_pr(), "viewer".into()))
        },
    )
    .unwrap();
    assert_eq!(called, 1);
    assert_eq!(
        receipt,
        json!({"id":"github:PR_test","action":"request-review","status":"completed","remote":"confirmed","localSaved":true})
    );
    let state = repo.state.lock().unwrap();
    assert_eq!(arr(&state["items"][0]["requestedReviewers"]).len(), 2);
    assert!(state["items"][0]["reviewerUpdatedAt"].as_f64().unwrap() > 0.0);
    assert!(repo.opening.lock().unwrap().is_empty());
}
#[test]
fn confirmed_remote_result_with_failed_cache_save_is_not_reported_as_rolled_back() {
    let (_temp, repo) = action_fixture(false);
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    let error = action_with(
        &repo,
        &json!({"id":"github:PR_test","action":"ready"}),
        |_, _, _| Ok((remote_pr(), "viewer".into())),
    )
    .unwrap_err();
    assert!(error
        .message
        .contains("any remote change may already be applied"));
    assert!(error.message.contains("Refresh the PR before retrying"));
    assert_eq!(
        error.data,
        json!({"githubReceipt":{"id":"github:PR_test","action":"ready","status":"remote-confirmed-local-failed","remote":"confirmed","localSaved":false}})
    );
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert!(repo.opening.lock().unwrap().is_empty());
}
#[test]
fn unconfirmed_remote_failure_preserves_refresh_before_retry_error_and_cache() {
    let (_temp, repo) = action_fixture(false);
    let before = repo.state.lock().unwrap().clone();
    let error = action_with(
        &repo,
        &json!({"id":"github:PR_test","action":"ready"}),
        |_, _, _| {
            Err(Error::new(
                "GitHub did not confirm this change. Refresh the PR before retrying.",
            ))
        },
    )
    .unwrap_err();
    assert!(error.message.contains("did not confirm"));
    assert_eq!(error.data, json!({}));
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(repo.opening.lock().unwrap().is_empty());
}

#[test]
fn post_dispatch_uncertainty_has_bounded_receipt_and_preflight_errors_do_not() {
    for dispatched in [false, true] {
        let (_temp, repo) = action_fixture(false);
        let before = repo.state.lock().unwrap().clone();
        let error=action_with(&repo,&json!({"id":"github:PR_test","action":"ready"}),|_,_,_|{
                let mut error=Error::new(if dispatched{"GitHub did not confirm this change. Refresh the PR before retrying."}else{"Your GitHub account cannot update this pull request."});
                if dispatched{error.data=json!({"githubActionStage":"dispatched","private":"private-provider-marker"});}Err(error)
            }).unwrap_err();
        assert_eq!(
            error.data,
            if dispatched {
                json!({"githubReceipt":{"id":"github:PR_test","action":"ready","status":"confirmation-unknown","remote":"unconfirmed","localSaved":false}})
            } else {
                json!({})
            }
        );
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert!(repo.opening.lock().unwrap().is_empty());
    }
}
