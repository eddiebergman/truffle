use super::*;

fn sources_fixture() -> (tempfile::TempDir, Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(temp.path().into(), None, true).unwrap();
    Arc::get_mut(&mut repo).unwrap().demo = false;
    (temp, repo)
}
#[test]
fn preparation_aliases_stay_private_in_dashboard_and_captured_open_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|state|{state["sessions"]=json!([{"id":id,"agent":"codex","title":"Projection fixture","cwd":temp.path(),"items":[],"preparationRequests":{"version":1,"entries":[],"privateNs":"private-preparation-canary"}}]);Ok(())}).unwrap();
    let public = repo.snapshot(false);
    assert!(public["sessions"][0].get("preparationRequests").is_none());
    let (raw, captured) = repo.capture_for_open().unwrap();
    assert!(!captured.to_string().contains("private-preparation-canary"));
    assert_eq!(
        raw["sessions"][0]["preparationRequests"]["privateNs"],
        "private-preparation-canary"
    );
    assert_eq!(
        repo.snapshot(true)["sessions"][0]["preparationRequests"],
        raw["sessions"][0]["preparationRequests"]
    );
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["preparationRequests"],
        raw["sessions"][0]["preparationRequests"]
    );
}

#[test]
fn conditional_transactions_preserve_legacy_always_commit_and_skip_only_equal_states() {
    let (_temp, repo) = sources_fixture();
    let initial = repo.snapshot(false)["revision"].clone();
    repo.transaction(|_| Ok(())).unwrap();
    let legacy = repo.snapshot(false)["revision"].clone();
    assert_ne!(legacy, initial);
    repo.transaction_if_changed(|_| Ok(())).unwrap();
    assert_eq!(repo.snapshot(false)["revision"], legacy);
    repo.transaction_if_changed(|s| {
        s["conditionalFixture"] = json!(true);
        Ok(())
    })
    .unwrap();
    assert_ne!(repo.snapshot(false)["revision"], legacy);
    let disk: Value = serde_json::from_slice(&std::fs::read(&repo.data).unwrap()).unwrap();
    assert_eq!(disk["conditionalFixture"], true);
}

#[test]
fn presentation_requests_use_owner_reads_instead_of_public_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
        state["uiPresentations"] =
            json!({"version":1,"entries":[],"privateExtension":"private-presentation-canary"});
        Ok(())
    })
    .unwrap();
    for snapshot in [
        repo.snapshot(false),
        repo.snapshot(true),
        repo.capture_for_open().unwrap().1,
    ] {
        assert!(snapshot.get("uiPresentations").is_none());
        assert!(!snapshot.to_string().contains("private-presentation-canary"));
    }
    assert_eq!(
        repo.state.lock().unwrap()["uiPresentations"]["privateExtension"],
        "private-presentation-canary"
    );
}
#[test]
fn source_admission_distinguishes_intervals_backoff_running_and_disabled() {
    let healthy = json!({"nextAttempt":200,"failures":0});
    assert_eq!(
        source_skip(&healthy, false, false, 100.0),
        Some(("interval", Some(200.0)))
    );
    assert_eq!(source_skip(&healthy, false, true, 100.0), None);
    assert_eq!(
        source_skip(&json!({"nextAttempt":200,"failures":1}), false, true, 100.0),
        Some(("backoff", Some(200.0)))
    );
    assert_eq!(
        source_skip(&healthy, true, true, 100.0),
        Some(("already-running", None))
    );
    assert_eq!(
        source_skip(&json!({"disabled":true}), true, true, 100.0),
        Some(("disabled", None))
    );
    assert_eq!(source_skip(&healthy, false, false, 200.0), None);
}
#[test]
fn sources_force_noop_publishes_cooldown_to_warm_snapshot_and_keeps_backoff() {
    let (_temp, repo) = sources_fixture();
    let retry = now() + 3600.0;
    repo.transaction(|state| {
        for id in ["github", "linear", "sentry"] {
            state["sources"][id] = json!({"nextAttempt":retry,"failures":2});
        }
        Ok(())
    })
    .unwrap();
    let before = repo.snapshot(false);
    assert_eq!(before["forceRefreshAfter"], 0.0);
    let receipt = repo
        .refresh_sources_with(true, |_, _| panic!("Backoff must never launch a provider"))
        .unwrap();
    assert_eq!(receipt["status"], "skipped");
    for row in arr(&receipt["results"]) {
        assert_eq!(row["reason"], "backoff");
        assert_eq!(row["retryAt"], retry);
    }
    let after = repo.snapshot(false);
    assert_ne!(before["revision"], after["revision"]);
    assert!(after["forceRefreshAfter"].as_f64().unwrap() > now());
    assert_eq!(after["forceRefreshAfter"], receipt["forceRefreshAfter"]);
    assert!(repo
        .refresh_sources_with(true, |_, _| panic!("Cooldown must never launch"))
        .is_err());
    assert!(repo.running.lock().unwrap().is_empty());
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
    repo.running.lock().unwrap().insert("linear".into());
    let launched = Mutex::new(vec![]);
    let receipt = repo
        .refresh_sources_with(true, |owner, id| {
            assert!(owner.running.lock().unwrap().contains(id));
            assert!(owner.state.try_lock().is_ok());
            launched.lock().unwrap().push(id);
        })
        .unwrap();
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
    let revision = demo.revision.load(Ordering::Relaxed);
    let receipt = demo
        .refresh_sources_with(true, |_, _| panic!("Demo must not launch"))
        .unwrap();
    assert_eq!(receipt["status"], "skipped");
    assert!(arr(&receipt["results"])
        .iter()
        .all(|row| row["reason"] == "demo"));
    assert_eq!(*demo.state.lock().unwrap(), state);
    assert_eq!(demo.revision.load(Ordering::Relaxed), revision);
    assert!(demo.running.lock().unwrap().is_empty());
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
    repo.running.lock().unwrap().insert("github".into());
    let status = repo.sources_status();
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

#[test]
fn repository_drop_releases_lock_even_with_an_inherited_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    // try_clone has the same shared open-file description as fork inheritance.
    let Repository {
        _lock: DataFileLock(lock_file),
        ..
    } = &*repo;
    let inherited = lock_file.try_clone().unwrap();
    assert!(Repository::open(temp.path().into(), None, true).is_err());
    drop(repo);
    let reopened = Repository::open(temp.path().into(), None, true).unwrap();
    drop(inherited);
    // Closing the old descriptor must not unlock the new repository's lock.
    assert!(Repository::open(temp.path().into(), None, true).is_err());
    drop(reopened);
    assert!(Repository::open(temp.path().into(), None, true).is_ok());
}

#[test]
fn data_lock_releases_on_constructor_error_with_a_duplicate_still_open() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let inherited = file.try_clone().unwrap();
    let fail_after_acquiring = || -> Result<()> {
        let _lock = DataFileLock::acquire(file)?;
        Err(Error::new(
            "Fixture constructor failure after acquiring the lock.",
        ))
    };
    assert!(fail_after_acquiring().is_err());
    let contender = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let lock = DataFileLock::acquire(contender).unwrap();
    drop(inherited);
    drop(lock);
}

#[test]
fn routine_snapshots_omit_review_bodies_without_changing_exports_or_history() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().to_path_buf(), None, true).unwrap();
    let record = repo
        .transaction(|state| {
            let record = super::super::sessions::save(
                state,
                &json!({
                    "agent":"claude", "cwd":temp.path(), "title":"Review snapshot", "itemIds":[]
                }),
            )?;
            let session = state["sessions"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|session| session["id"] == record["id"])
                .unwrap();
            session["reviewDraft"] = json!({"comments":[],"note":"Current review","revision":1});
            session["reviewHistory"] = json!({"version":1,"entries":[{
                "id":"previous", "archivedAt":1, "review":{"comments":[],"note":"Previous review"}
            }]});
            Ok(record)
        })
        .unwrap();

    for _ in 0..2 {
        let snapshot = repo.snapshot(false);
        let session = &snapshot["sessions"][0];
        assert!(session.get("reviewDraft").is_none());
        assert!(session.get("reviewHistory").is_none());
        assert_eq!(session["title"], "Review snapshot");
    }
    let export = repo.snapshot(true);
    assert_eq!(
        export["sessions"][0]["reviewDraft"]["note"],
        "Current review"
    );
    assert_eq!(
        export["sessions"][0]["reviewHistory"]["entries"][0]["review"]["note"],
        "Previous review"
    );
    let history = super::super::reviews::action(
        &repo,
        &super::super::reviews::Git,
        &json!({"id":record["id"],"action":"history"}),
    )
    .unwrap();
    assert_eq!(
        history["reviews"],
        export["sessions"][0]["reviewHistory"]["entries"]
    );
}
