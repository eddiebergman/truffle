use super::*;

fn sources_fixture() -> (tempfile::TempDir, Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(temp.path().into(), None, true).unwrap();
    Arc::get_mut(&mut repo).unwrap().demo = false;
    (temp, repo)
}
#[test]
fn source_admission_distinguishes_intervals_backoff_running_and_disabled() {
    let healthy = json!({"nextAttempt":200,"failures":0});
    assert_eq!(
        admission(&healthy, false, false, 100.0),
        Some(Skip::Interval(200.0))
    );
    assert_eq!(admission(&healthy, false, true, 100.0), None);
    assert_eq!(
        admission(&json!({"nextAttempt":200,"failures":1}), false, true, 100.0),
        Some(Skip::Backoff(200.0))
    );
    assert_eq!(
        admission(&healthy, true, true, 100.0),
        Some(Skip::AlreadyRunning)
    );
    assert_eq!(
        admission(&json!({"disabled":true}), true, true, 100.0),
        Some(Skip::Disabled)
    );
    assert_eq!(admission(&healthy, false, false, 200.0), None);
}
#[test]
fn sources_force_noop_publishes_cooldown_to_warm_snapshot_and_keeps_backoff() {
    let (_temp, repo) = sources_fixture();
    let retry = now() + 3600.0;
    repo.transaction(|state| {
        for id in SOURCES {
            state["sources"][id] = json!({"nextAttempt":retry,"failures":2});
        }
        Ok(())
    })
    .unwrap();
    let before = repo.snapshot(false);
    assert_eq!(before["forceRefreshAfter"], 0.0);
    let receipt = json!(request_with(&repo, true, |_, _| panic!(
        "Backoff must never launch a provider"
    ))
    .unwrap());
    assert_eq!(receipt["status"], "skipped");
    for row in arr(&receipt["results"]) {
        assert_eq!(row["reason"], "backoff");
        assert_eq!(row["retryAt"], retry);
    }
    let after = repo.snapshot(false);
    assert_ne!(before["revision"], after["revision"]);
    assert!(after["forceRefreshAfter"].as_f64().unwrap() > now());
    assert_eq!(after["forceRefreshAfter"], receipt["forceRefreshAfter"]);
    assert!(request_with(&repo, true, |_, _| panic!("Cooldown must never launch")).is_err());
    assert!(repo.sync.running.lock().unwrap().is_empty());
}
#[test]
fn sources_refresh_receipt_reports_only_reserved_workers_and_demo_has_no_side_effects() {
    let (_temp, repo) = sources_fixture();
    repo.transaction(|state| {
        state["sources"]["github"] = json!({"nextAttempt":now()+300.0,"failures":0});
        state["sources"]["linear"] = json!({});
        state["sources"]["sentry"] = json!({"disabled":true});
        Ok(())
    })
    .unwrap();
    repo.sync.running.lock().unwrap().insert("linear".into());
    let launched = Mutex::new(vec![]);
    let receipt = json!(request_with(&repo, true, |owner, id| {
        assert!(owner.sync.running.lock().unwrap().contains(id));
        assert!(owner.state.try_lock().is_ok());
        launched.lock().unwrap().push(id);
    })
    .unwrap());
    assert_eq!(*launched.lock().unwrap(), vec!["github"]);
    assert_eq!(receipt["status"], "started");
    assert_eq!(
        receipt["results"][0],
        json!({"id":"github","status":"started"})
    );
    assert_eq!(receipt["results"][1]["reason"], "already-running");
    assert_eq!(receipt["results"][2]["reason"], "disabled");
    let demo_temp = tempfile::tempdir().unwrap();
    let demo = Repository::open(demo_temp.path().into(), None, true).unwrap();
    let state = demo.state.lock().unwrap().clone();
    let revision = demo.snapshot(false)["revision"].clone();
    let receipt = json!(request_with(&demo, true, |_, _| panic!("Demo must not launch")).unwrap());
    assert_eq!(receipt["status"], "skipped");
    assert!(arr(&receipt["results"])
        .iter()
        .all(|row| row["reason"] == "demo"));
    assert_eq!(*demo.state.lock().unwrap(), state);
    assert_eq!(demo.snapshot(false)["revision"], revision);
    assert!(demo.sync.running.lock().unwrap().is_empty());
    assert_eq!(demo.sync.force_after(), 0.0);
    assert!(demo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn sources_status_is_bounded_and_redacts_full_error_before_truncation() {
    let temp = tempfile::tempdir().unwrap();
    let secret = "benign-private-sources-secret-boundary-marker";
    std::fs::write(
        temp.path().join(".env"),
        format!("TRUFFLE_TEST_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
            state["sources"]["github"]=json!({"disabled":false,"lastSuccess":123,"nextAttempt":456,"failures":2,"error":format!("{}{secret} suffix","x".repeat(1990)),"privateExtension":"private-extension-marker"});
            state["sources"]["linear"]=json!({"disabled":true,"lastSuccess":"private-timestamp-marker","nextAttempt":-1,"failures":-1,"error":{"private":"private-object-marker"}});
            state["sources"]["unknown"]=json!({"error":"private-unknown-marker"});
            state["items"]=json!([{"id":"a","source":"github","hidden":true,"status":"done"},{"id":"b","source":"github"},{"id":"c","source":"linear"}]);Ok(())
        }).unwrap();
    repo.sync.running.lock().unwrap().insert("github".into());
    let status = status(&repo);
    let rows = arr(&status["sources"]);
    assert_eq!(rows.len(), 3);
    let github = &rows[0];
    assert_eq!(github["cachedItemCount"], 2);
    assert_eq!(github["refreshing"], true);
    assert_eq!(github["hasError"], true);
    let error = text(github, "error");
    assert_eq!(error.chars().count(), 2000);
    assert!(error.ends_with("[redacted]"));
    assert!(!error.contains("benign"));
    assert_eq!(rows[1]["syncEnabled"], false);
    assert_eq!(rows[1]["cachedItemCount"], 1);
    assert_eq!(rows[1]["lastSuccess"], Value::Null);
    assert_eq!(rows[1]["nextAttempt"], Value::Null);
    assert_eq!(rows[1]["failures"], 0);
    assert_eq!(rows[1]["error"], Value::Null);
    assert_eq!(rows[1]["hasError"], false);
    assert_eq!(github.as_object().unwrap().len(), 9);
    let encoded = status.to_string();
    for private in [
        secret,
        "private-extension-marker",
        "private-object-marker",
        "private-unknown-marker",
        "private-timestamp-marker",
    ] {
        assert!(!encoded.contains(private));
    }
}
