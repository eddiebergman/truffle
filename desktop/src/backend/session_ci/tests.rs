use super::*;
use crate::backend::uuid;
#[test]
fn investigation_progress_is_linked_and_request_guarded() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    let request = uuid();
    repo.transaction(|s| {
        s["items"] = json!([{"id":"github:a","source":"github","kind":"Pull request"}]);
        s["sessions"] = json!([{"id":id,"items":[{"id":"github:a"}]}]);
        Ok(())
    })
    .unwrap();
    let report = |request: &str, status: &str| {
        investigation(
            &repo,
            &json!({"id":id,"itemId":"github:a","requestId":request,"state":status}),
        )
    };
    assert_eq!(
        report(&request, "queued").unwrap()["investigation"]["state"],
        "queued"
    );
    assert!(report(&uuid(), "queued").is_err());
    assert!(report(&uuid(), "complete").is_err());
    assert_eq!(
        report(&request, "investigating").unwrap()["investigation"]["state"],
        "investigating"
    );
    assert!(investigation(
        &repo,
        &json!({"id":id,"itemId":"github:unlinked","requestId":request,"state":"complete"})
    )
    .is_err());
    assert_eq!(
        report(&request, "complete").unwrap()["investigation"]["state"],
        "complete"
    );
    assert!(report(&request, "investigating").is_err());
    assert!(report(&uuid(), "queued").is_ok());
}
#[test]
fn scope_and_updates_preserve_other_prs_relations_and_source_health() {
    let id = uuid();
    let pr = |key: &str| json!({"id":key,"source":"github","kind":"Pull request","title":"PR","status":"needs-review","relations":["authored"],"ci":{"state":"pending"}});
    let mut state = json!({"version":1,"sessions":[{"id":id,"items":[{"id":"github:a"},{"id":"linear:b"}]}],"items":[pr("github:a"),pr("github:other"),{"id":"linear:b","source":"linear","kind":"Issue"}],"sources":{"github":{"error":"Full sync failed"}}});
    domain::initialize(&mut state).unwrap();
    let linked = linked_prs(&state, &id).unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["id"], "github:a");
    let health = state["sources"].clone();
    let mut fresh = linked[0].clone();
    fresh["ci"] = json!({"state":"failed","checks":[{"name":"unit","state":"failed"}]});
    fresh["relations"] = json!(["linked"]);
    let changed = updates(&state, &[fresh.clone()]);
    assert_eq!(changed.len(), 1);
    apply(&mut state, &changed);
    assert_eq!(state["items"][0]["ci"]["state"], "failed");
    assert_eq!(state["items"][0]["relations"], json!(["authored"]));
    assert_eq!(state["items"][1]["ci"]["state"], "pending");
    assert_eq!(state["sources"], health);
    assert_eq!(updates(&state, &[fresh]), [] as [serde_json::Value; 0]);
    assert!(linked_prs(&state, &uuid()).is_err());
}
#[test]
fn demo_refresh_never_claims_a_provider_check_or_consumes_admission() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s|{s["items"]=json!([{"id":"github:a","source":"github","kind":"Pull request","ci":{"state":"pending"}}]);s["sessions"]=json!([{"id":id,"items":[{"id":"github:a"}]}]);Ok(())}).unwrap();
    let before = repo.snapshot(false)["revision"].clone();
    assert_eq!(refresh(&repo, &id).unwrap()["reason"], "demo");
    assert!(refresh(&repo, &id).unwrap()["checkedAt"].is_null());
    assert!(repo.cooldown.lock().unwrap().is_empty());
    assert_eq!(repo.snapshot(false)["revision"], before);
}
fn ci_fixture(demo: bool) -> (tempfile::TempDir, std::sync::Arc<Repository>, String) {
    let temp = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(temp.path().into(), None, true).unwrap();
    std::sync::Arc::get_mut(&mut repo).unwrap().demo = demo;
    let id = uuid();
    repo.transaction(|s|{s["items"]=json!([{"id":"github:a","source":"github","kind":"Pull request","title":"PR","ci":{"state":"pending"},"privateExtension":"private-fixture-marker"}]);s["sessions"]=json!([{"id":id,"items":[{"id":"github:a"}]}]);s["sources"]["github"]=json!({"failures":0,"nextAttempt":now()+300.0,"error":null});Ok(())}).unwrap();
    (temp, repo, id)
}
#[test]
fn ci_admission_skips_preserve_cache_and_all_reservations_without_provider_reads() {
    for reason in [
        "disabled",
        "backoff",
        "already-running",
        "ci-backoff",
        "cooldown",
        "opening",
    ] {
        let (_temp, repo, id) = ci_fixture(false);
        let retry = now() + 3600.0;
        let key = format!("session-ci:{id}");
        match reason {
            "disabled" => {
                repo.transaction(|s| {
                    s["sources"]["github"]["disabled"] = json!(true);
                    Ok(())
                })
                .unwrap();
            }
            "backoff" => {
                repo.transaction(|s| {
                    s["sources"]["github"]["failures"] = json!(2);
                    s["sources"]["github"]["nextAttempt"] = json!(retry);
                    Ok(())
                })
                .unwrap();
            }
            "already-running" => {
                repo.sync.running.lock().unwrap().insert("github".into());
            }
            "ci-backoff" => {
                repo.cooldown
                    .lock()
                    .unwrap()
                    .insert("session-ci-backoff".into(), retry);
            }
            "cooldown" => {
                repo.cooldown.lock().unwrap().insert(key.clone(), retry);
            }
            "opening" => {
                repo.opening.lock().unwrap().insert(key.clone());
            }
            _ => unreachable!(),
        }
        let before = repo.state.lock().unwrap().clone();
        let disk = std::fs::read(&repo.data).unwrap();
        let cooldown = repo.cooldown.lock().unwrap().clone();
        let opening = repo.opening.lock().unwrap().clone();
        let running = repo.sync.running.lock().unwrap().clone();
        let result = refresh_with(&repo, &id, |_| {
            panic!("blocked admission must not read GitHub")
        })
        .unwrap();
        assert_eq!(result["status"], "skipped");
        assert_eq!(result["checkedAt"], Value::Null);
        assert_eq!(
            result["reason"],
            if reason == "opening" {
                "already-running"
            } else {
                reason
            }
        );
        assert_eq!(result["itemIds"], json!(["github:a"]));
        assert_eq!(result["recordId"], id);
        assert!(!result.to_string().contains("private"));
        if ["backoff", "ci-backoff", "cooldown"].contains(&reason) {
            assert_eq!(result["retryAt"], retry);
        } else {
            assert!(result.get("retryAt").is_none());
        }
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
        assert_eq!(*repo.opening.lock().unwrap(), opening);
        assert_eq!(*repo.sync.running.lock().unwrap(), running);
    }
}
#[test]
fn ci_admission_bypasses_healthy_interval_releases_locks_and_returns_real_check_only() {
    let (_temp, repo, id) = ci_fixture(false);
    let health = repo.state.lock().unwrap()["sources"].clone();
    let before = repo.snapshot(false)["revision"].clone();
    let receipt = refresh_with(&repo, &id, |refs| {
        assert!(repo.state.try_lock().is_ok());
        assert!(repo.sync.running.try_lock().is_ok());
        assert!(repo.cooldown.try_lock().is_ok());
        assert!(repo.opening.try_lock().is_ok());
        let times = repo.cooldown.lock().unwrap().clone();
        let duplicate = refresh_with(&repo, &id, |_| {
            panic!("inflight request must not duplicate")
        })
        .unwrap();
        assert_eq!(duplicate["reason"], "already-running");
        assert_eq!(*repo.cooldown.lock().unwrap(), times);
        Ok(refs.to_vec())
    })
    .unwrap();
    assert_eq!(receipt["status"], "checked");
    assert!(receipt["checkedAt"].is_number());
    assert_eq!(receipt.as_object().unwrap().len(), 4);
    assert_eq!(receipt["itemIds"], json!(["github:a"]));
    assert!(repo.opening.lock().unwrap().is_empty());
    assert_eq!(repo.state.lock().unwrap()["sources"], health);
    assert_eq!(repo.snapshot(false)["revision"], before);
    assert_eq!(
        refresh_with(&repo, &id, |_| panic!(
            "cooldown request must not duplicate"
        ))
        .unwrap()["reason"],
        "cooldown"
    );
}
#[test]
fn ci_admission_observes_policy_changed_while_waiting_and_failure_releases_reservation() {
    let (_temp, repo, id) = ci_fixture(false);
    let mut state = repo.state.lock().unwrap();
    let (ready, started) = std::sync::mpsc::channel();
    let worker_repo = repo.clone();
    let worker_id = id.clone();
    let worker = std::thread::spawn(move || {
        ready.send(()).unwrap();
        refresh_with(&worker_repo, &worker_id, |_| {
            panic!("disabled before admission must not read")
        })
    });
    started.recv().unwrap();
    state["sources"]["github"]["disabled"] = json!(true);
    drop(state);
    assert_eq!(worker.join().unwrap().unwrap()["reason"], "disabled");
    assert!(repo.cooldown.lock().unwrap().is_empty());
    repo.transaction(|s| {
        s["sources"]["github"]["disabled"] = json!(false);
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    assert!(refresh_with(&repo, &id, |_| {
        let mut error = Error::new("Fixture GitHub read failed");
        error.retry = 120.0;
        Err(error)
    })
    .is_err());
    assert!(repo.opening.lock().unwrap().is_empty());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    let cooldown = repo.cooldown.lock().unwrap().clone();
    assert_eq!(
        refresh_with(&repo, &id, |_| panic!("CI backoff must prevent retry")).unwrap()["reason"],
        "ci-backoff"
    );
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
}
#[test]
fn ci_receipts_bound_scope_and_demo_never_claims_a_check() {
    let (_temp, repo, id) = ci_fixture(true);
    let result = refresh(&repo, &id).unwrap();
    assert_eq!(result["reason"], "demo");
    assert_eq!(result["checkedAt"], Value::Null);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    repo.transaction(|s| {
        s["sessions"][0]["items"] = json!([]);
        Ok(())
    })
    .unwrap();
    assert_eq!(refresh(&repo, &id).unwrap()["reason"], "no-linked-prs");
    repo.transaction(|s| {
        let items = (0..101)
            .map(|n| json!({"id":format!("github:{n}"),"source":"github","kind":"Pull request"}))
            .collect::<Vec<_>>();
        s["items"] = json!(items);
        s["sessions"][0]["items"] = s["items"].clone();
        Ok(())
    })
    .unwrap();
    assert!(refresh_with(&repo, &id, |_| panic!("oversized scope must not read")).is_err());
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn ci_stale_provider_result_never_overwrites_newer_protected_fields_or_claims_freshness() {
    let (_temp, repo, id) = ci_fixture(false);
    let captured = std::sync::Mutex::new(None);
    let result = refresh_with(&repo, &id, |refs| {
        repo.transaction(|s| {
            s["items"][0]["ci"] = json!({"state":"passed"});
            s["items"][0]["status"] = json!("ready-to-merge");
            Ok(())
        })?;
        let state = repo.state.lock().unwrap().clone();
        let disk = std::fs::read(&repo.data).unwrap();
        let revision = repo.snapshot(false)["revision"].clone();
        *captured.lock().unwrap() = Some((state, disk, revision));
        let mut older = refs.to_vec();
        older[0]["ci"] = json!({"state":"failed"});
        Ok(older)
    })
    .unwrap();
    assert_eq!(result["status"], "skipped");
    assert_eq!(result["reason"], "state-changed");
    assert_eq!(result["checkedAt"], Value::Null);
    let expected = captured.into_inner().unwrap().unwrap();
    assert_eq!(*repo.state.lock().unwrap(), expected.0);
    assert_eq!(std::fs::read(&repo.data).unwrap(), expected.1);
    assert_eq!(repo.snapshot(false)["revision"], expected.2);
}
#[test]
fn ci_partial_conflict_receipt_names_only_accepted_fresh_rows_and_preserves_unrelated_metadata() {
    let (_temp, repo, id) = ci_fixture(false);
    repo.transaction(|s| {
        let mut other = s["items"][0].clone();
        other["id"] = json!("github:b");
        s["items"].as_array_mut().unwrap().push(other);
        s["sessions"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"github:b"}));
        Ok(())
    })
    .unwrap();
    let result = refresh_with(&repo, &id, |refs| {
        repo.transaction(|s| {
            s["items"][0]["ci"] = json!({"state":"passed"});
            s["items"][1]["title"] = json!("Newer title");
            s["items"][1]["relations"] = json!(["authored"]);
            Ok(())
        })?;
        let mut fresh = refs.to_vec();
        for row in &mut fresh {
            row["ci"] = json!({"state":"failed"});
        }
        Ok(fresh)
    })
    .unwrap();
    assert_eq!(result["status"], "checked");
    assert!(result["checkedAt"].is_number());
    assert_eq!(result["itemIds"], json!(["github:b"]));
    assert_eq!(result["skippedItemIds"], json!(["github:a"]));
    let state = repo.state.lock().unwrap();
    assert_eq!(state["items"][0]["ci"]["state"], "passed");
    assert_eq!(state["items"][1]["ci"]["state"], "failed");
    assert_eq!(state["items"][1]["title"], "Newer title");
    assert_eq!(state["items"][1]["relations"], json!(["authored"]));
}
#[test]
fn ci_head_identity_change_alone_discards_older_checks() {
    let (_temp, repo, id) = ci_fixture(false);
    repo.transaction(|s| {
        s["items"][0]["headRefOid"] = json!("head-before");
        Ok(())
    })
    .unwrap();
    let result = refresh_with(&repo, &id, |refs| {
        repo.transaction(|s| {
            s["items"][0]["headRefOid"] = json!("head-after");
            Ok(())
        })?;
        let mut older = refs.to_vec();
        older[0]["ci"] = json!({"state":"failed"});
        Ok(older)
    })
    .unwrap();
    assert_eq!(result["reason"], "state-changed");
    assert_eq!(result["checkedAt"], Value::Null);
    let state = repo.state.lock().unwrap();
    assert_eq!(state["items"][0]["headRefOid"], "head-after");
    assert_eq!(state["items"][0]["ci"]["state"], "pending");
}
