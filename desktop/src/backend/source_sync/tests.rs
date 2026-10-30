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
#[test]
fn sentry_environment_paused_claim_rejects_without_cache_cooldown_or_provider_admission() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
        state["sources"]["sentry"]["disabled"] = json!(true);
        state["sentryEnvironments"] =
            json!({"qa":{"lastSuccess":42,"itemIds":["cached"],"extension":"keep"}});
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let cooldown = repo.cooldown.lock().unwrap().clone();
    let mut starts = 0;
    let claim = claim_environment(&repo, "new-environment");
    if matches!(claim, Ok(true)) {
        starts += 1;
    }
    assert!(claim.unwrap_err().message.contains("paused"));
    assert_eq!(starts, 0);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
}

fn with_fixture(
    fixture: Fixture,
    state: impl FnOnce(&mut Value),
) -> (tempfile::TempDir, Arc<Repository>) {
    let (temp, mut repo) = sources_fixture();
    Arc::get_mut(&mut repo).unwrap().sync = SourceSync::new(Arc::new(fixture));
    repo.transaction(|s| {
        // Opening marks Sentry disabled because no credentials are configured.
        s["sources"]["sentry"]["disabled"] = json!(false);
        state(s);
        Ok(())
    })
    .unwrap();
    (temp, repo)
}
fn listings(
    github: Result<Vec<Value>>,
    linear: Result<Vec<Value>>,
    sentry: Result<Vec<Value>>,
) -> BTreeMap<&'static str, Result<Vec<Value>>> {
    BTreeMap::from([("github", github), ("linear", linear), ("sentry", sentry)])
}
/// Run every admitted source on this thread through the real admission path.
fn sync_all(repo: &Arc<Repository>) -> Value {
    json!(request_with(repo, false, |repo, source| run(&repo, source)).unwrap())
}
fn chat(id: &str, items: Value, archived: bool) -> Value {
    json!({"id":id,"agent":"codex","title":"Sync fixture","cwd":"/tmp","items":items,
        "archivedAt":if archived {json!(1)} else {Value::Null}})
}
fn cached(state: &Value, id: &str) -> Value {
    arr(&state["items"])
        .into_iter()
        .find(|item| item["id"] == id)
        .unwrap_or(Value::Null)
}
fn ids(state: &Value, source: &str) -> Vec<String> {
    arr(&state["items"])
        .iter()
        .filter(|item| item["source"] == source)
        .map(|item| text(item, "id").to_owned())
        .collect()
}

#[test]
fn sync_adds_new_items_and_updates_cached_items_and_chat_references() {
    let (_temp, repo) = with_fixture(
        Fixture {
            listings: listings(
                Ok(vec![
                    json!({"id":"github:1","source":"github","title":"New title","status":"in-progress"}),
                    json!({"id":"github:2","source":"github","title":"Brand new","status":"todo"}),
                ]),
                Ok(vec![]),
                Ok(vec![]),
            ),
            refreshed: vec![],
        },
        |s| {
            s["items"] = json!([
                {"id":"github:1","source":"github","title":"Old title","status":"todo"},
                {"id":"local:1","source":"local","title":"Untouched"}
            ]);
            s["sessions"] = json!([chat(
                "chat-1",
                json!([{"id":"github:1","source":"github","title":"Old title"}]),
                false
            )]);
        },
    );
    let receipt = sync_all(&repo);
    assert_eq!(receipt["status"], "started");
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(ids(&state, "github"), ["github:1", "github:2"]);
    assert_eq!(cached(&state, "github:1")["title"], "New title");
    assert_eq!(cached(&state, "github:2")["title"], "Brand new");
    assert_eq!(cached(&state, "local:1")["title"], "Untouched");
    let reference = &state["sessions"][0]["items"][0];
    assert_eq!(reference["title"], "New title");
    assert_eq!(reference["status"], "in-progress");
    assert!(state["itemArchive"].as_object().unwrap().is_empty());
    assert!(state["changeBaselines"]["github:2"].is_object());
    for source in SOURCES {
        assert_eq!(state["sources"][source]["failures"], 0);
        assert!(state["sources"][source]["lastSuccess"].is_number());
    }
    assert!(repo.sync.running.lock().unwrap().is_empty());
}

#[test]
fn sync_removes_unlisted_items_and_archives_tracked_items_it_refreshes() {
    let (_temp, repo) = with_fixture(
        Fixture {
            listings: listings(Ok(vec![]), Ok(vec![]), Ok(vec![])),
            refreshed: vec![
                json!({"id":"linear:open","source":"linear","title":"Now closed","status":"done"}),
                json!({"id":"sentry:chat","source":"sentry","title":"Fresh error","status":"todo"}),
            ],
        },
        |s| {
            s["items"] = json!([
                {"id":"linear:done","source":"linear","title":"Finished","status":"done"},
                {"id":"linear:open","source":"linear","title":"Open","status":"todo"},
                {"id":"linear:gone","source":"linear","title":"Deleted upstream","status":"todo"},
                {"id":"sentry:old","source":"sentry","title":"Resolved","status":"todo"}
            ]);
            s["sessions"] = json!([
                chat(
                    "active",
                    json!([{"id":"sentry:chat","source":"sentry","title":"Stale error"}]),
                    false
                ),
                chat(
                    "archived",
                    json!([{"id":"sentry:archived","source":"sentry","title":"Old chat"}]),
                    true
                )
            ]);
        },
    );
    sync_all(&repo);
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(ids(&state, "linear"), ["linear:gone", "linear:open"]);
    assert_eq!(ids(&state, "sentry"), ["sentry:chat"]);
    assert_eq!(cached(&state, "linear:open")["status"], "done");
    assert_eq!(cached(&state, "linear:gone")["sourceUnavailable"], true);
    let archive = state["itemArchive"].as_object().unwrap();
    assert_eq!(
        archive.keys().collect::<Vec<_>>(),
        ["linear:gone", "linear:open", "sentry:chat"]
    );
    assert_eq!(archive["linear:open"]["title"], "Now closed");
    assert_eq!(archive["linear:gone"]["sourceUnavailable"], true);
    assert_eq!(state["sessions"][0]["items"][0]["title"], "Fresh error");
    assert_eq!(state["sessions"][1]["items"][0]["title"], "Old chat");
}

#[test]
fn hidden_items_stay_cached_when_the_provider_omits_them() {
    let (_temp, repo) = with_fixture(
        Fixture {
            listings: listings(
                Ok(vec![
                    json!({"id":"github:listed","source":"github","title":"Listed again","status":"todo"}),
                ]),
                Ok(vec![]),
                Ok(vec![]),
            ),
            refreshed: vec![
                json!({"id":"github:hidden","source":"github","title":"Hidden, refreshed","status":"done"}),
            ],
        },
        |s| {
            s["items"] = json!([
                {"id":"github:hidden","source":"github","title":"Hidden","status":"done"},
                {"id":"github:listed","source":"github","title":"Listed","status":"done"},
                {"id":"github:dropped","source":"github","title":"Dropped","status":"done"}
            ]);
            s["overrides"] =
                json!({"github:hidden":{"hidden":true},"github:listed":{"hidden":true}});
        },
    );
    sync_all(&repo);
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(ids(&state, "github"), ["github:hidden", "github:listed"]);
    assert_eq!(
        cached(&state, "github:hidden")["title"],
        "Hidden, refreshed"
    );
    assert_eq!(cached(&state, "github:listed")["title"], "Listed again");
    assert_eq!(state["overrides"]["github:hidden"]["hidden"], true);

    // Without a refresh, the merge itself keeps the cached hidden copy.
    let mut state = state;
    assert_eq!(merge(&mut state, "github", vec![], &BTreeSet::new()), 2);
    assert_eq!(ids(&state, "github"), ["github:hidden", "github:listed"]);
    assert_eq!(
        cached(&state, "github:hidden")["title"],
        "Hidden, refreshed"
    );
}

#[test]
fn one_failing_provider_keeps_its_cache_and_shows_the_failure_while_others_merge() {
    let mut failure = Error::new("Linear returned HTTP 502. Cached data remains available.");
    failure.data = json!({"kind":"http","httpStatus":502});
    let (temp, repo) = with_fixture(
        Fixture {
            listings: listings(
                Ok(vec![
                    json!({"id":"github:new","source":"github","title":"New PR","status":"todo"}),
                ]),
                Err(failure),
                Ok(vec![
                    json!({"id":"sentry:new","source":"sentry","title":"New error","status":"todo"}),
                ]),
            ),
            refreshed: vec![],
        },
        |s| {
            s["items"] =
                json!([{"id":"linear:cached","source":"linear","title":"Cached","status":"done"}]);
            s["sources"]["linear"]["extension"] = json!("keep");
        },
    );
    let start = now();
    sync_all(&repo);
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(ids(&state, "github"), ["github:new"]);
    assert_eq!(ids(&state, "sentry"), ["sentry:new"]);
    assert_eq!(ids(&state, "linear"), ["linear:cached"]);
    let linear = &state["sources"]["linear"];
    assert_eq!(linear["failures"], 1);
    assert_eq!(linear["extension"], "keep");
    assert!(linear["lastSuccess"].is_null());
    let next = linear["nextAttempt"].as_f64().unwrap();
    assert!(next >= start + 300.0 && next <= now() + 300.0);

    let rows = arr(&status(&repo)["sources"]);
    assert_eq!(rows[0]["hasError"], false);
    assert_eq!(rows[1]["hasError"], true);
    assert!(text(&rows[1], "error").contains("HTTP 502"));
    assert_eq!(rows[1]["cachedItemCount"], 1);
    assert_eq!(rows[2]["hasError"], false);

    let log = std::fs::read_to_string(temp.path().join("data/logs/sync.jsonl")).unwrap();
    let events = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let linear = events.iter().find(|e| e["source"] == "linear").unwrap();
    assert_eq!(linear["outcome"], "failure");
    assert_eq!(linear["retrySeconds"], 300.0);
    assert_eq!(linear["httpStatus"], 502);
    for source in ["github", "sentry"] {
        let event = events.iter().find(|e| e["source"] == source).unwrap();
        assert_eq!(event["outcome"], "success");
        assert_eq!(event["itemCount"], 1);
    }

    run(&repo, "linear");
    assert_eq!(
        repo.state.lock().unwrap()["sources"]["linear"]["failures"],
        2
    );
    let receipt = sync_all(&repo);
    assert_eq!(receipt["results"][1]["reason"], "backoff");
}

#[test]
fn backoff_doubles_from_the_interval_to_the_hour_cap_and_honours_retry_after() {
    let delays = [1, 2, 3, 4, 5, 6, 100].map(|failures| backoff(failures, 0.0));
    assert_eq!(
        delays,
        [300.0, 600.0, 1200.0, 2400.0, 3600.0, 3600.0, 3600.0]
    );
    assert_eq!(backoff(1, 7200.0), 7200.0);
    assert_eq!(backoff(5, 7200.0), 7200.0);
}

#[test]
fn environment_fetch_records_members_and_backs_off_on_failure() {
    let members = vec![json!({"id":"sentry:1"}), json!({"id":"sentry:2"})];
    let (_temp, repo) = with_fixture(
        Fixture {
            listings: BTreeMap::from([("sentry", Ok(members))]),
            refreshed: vec![],
        },
        |_| {},
    );
    assert!(claim_environment(&repo, "qa").unwrap());
    assert_eq!(
        repo.state.lock().unwrap()["sentryEnvironments"]["qa"]["running"],
        true
    );
    fetch_environment(&repo, "qa");
    let cache = repo.state.lock().unwrap()["sentryEnvironments"]["qa"].clone();
    assert_eq!(cache["itemIds"], json!(["sentry:1", "sentry:2"]));
    assert_eq!(cache["running"], false);
    assert_eq!(cache["failures"], 0);

    let (_temp, repo) = with_fixture(
        Fixture {
            listings: BTreeMap::from([("sentry", Err(Error::new("Sentry timed out.")))]),
            refreshed: vec![],
        },
        |s| s["sentryEnvironments"] = json!({"qa":{"itemIds":["sentry:1"],"failures":1}}),
    );
    let start = now();
    fetch_environment(&repo, "qa");
    let cache = repo.state.lock().unwrap()["sentryEnvironments"]["qa"].clone();
    assert_eq!(cache["failures"], 2);
    assert_eq!(cache["error"], "Sentry timed out.");
    assert_eq!(cache["itemIds"], json!(["sentry:1"]));
    let next = cache["nextAttempt"].as_f64().unwrap();
    assert!(next >= start + 600.0 && next <= now() + 600.0);
}
