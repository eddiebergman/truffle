use super::*;
use std::sync::{Arc, Barrier};
fn fixture() -> (tempfile::TempDir, Arc<Repository>, Value) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let created=create(&repo,&json!({"title":"Initial project","detail":"Initial detail","url":"https://example.invalid/project","cwd":temp.path(),"status":"todo"})).unwrap();
    (temp, repo, created)
}
fn expected(view: &Value, patch: &Value) -> Value {
    let mut expected = json!({});
    for key in patch.as_object().unwrap().keys() {
        expected[key] = view["expected"][key].clone();
    }
    expected
}
fn patch(repo: &Repository, baseline: &Value, patch: &Value) -> Result<Value> {
    update(
        repo,
        &json!({"id":baseline["project"]["id"],"expected":expected(baseline,patch),"patch":patch}),
    )
}
#[test]
fn projects_patch_preserves_latest_other_fields_unknown_metadata_and_status_override() {
    let (_temp, repo, initial) = fixture();
    let id = text(&initial["project"], "id");
    repo.transaction(|state| {
        state["notes"][0]["unknownNs"] = json!({"private":"project extension marker"});
        state["notes"][0]["archivedAt"] = json!(123);
        state["overrides"][id] =
            json!({"status":"blocked","pinned":true,"futureNs":{"preserve":true}});
        state["itemSettings"][id] = json!({"cwd":"unrelated mapped folder","futureNs":true});
        Ok(())
    })
    .unwrap();
    let baseline = read(&repo, &json!({"id":id})).unwrap();
    assert_eq!(baseline["project"]["status"], "blocked");
    assert!(!baseline.to_string().contains("extension marker"));
    patch(&repo, &baseline, &json!({"detail":"Concurrent detail"})).unwrap();
    let changed = patch(&repo, &baseline, &json!({"title":"New title"})).unwrap();
    assert_eq!(changed["changedFields"], json!(["title"]));
    assert_eq!(changed["project"]["detail"], "Concurrent detail");
    assert_eq!(changed["project"]["status"], "blocked");
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(
        state["notes"][0]["unknownNs"]["private"],
        "project extension marker"
    );
    assert_eq!(state["notes"][0]["archivedAt"], 123);
    assert_eq!(state["overrides"][id]["status"], "blocked");
    assert_eq!(state["overrides"][id]["futureNs"]["preserve"], true);
    assert_eq!(state["itemSettings"][id]["cwd"], "unrelated mapped folder");
}
#[test]
fn projects_status_tokens_guard_raw_and_override_and_only_explicit_status_clears_override() {
    let (_temp, repo, baseline) = fixture();
    let id = text(&baseline["project"], "id");
    repo.transaction(|s| {
        s["overrides"][id] = json!({"status":"blocked","hidden":true,"futureNs":7});
        Ok(())
    })
    .unwrap();
    let current = read(&repo, &json!({"id":id})).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        patch(&repo, &baseline, &json!({"status":"done"}))
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(
        patch(
            &repo,
            &baseline,
            &json!({"title":"Must remain initial","status":"done"})
        )
        .unwrap_err()
        .status,
        409
    );
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    let result = patch(&repo, &current, &json!({"status":"done"})).unwrap();
    assert_eq!(result["project"]["status"], "done");
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(state["notes"][0]["status"], "done");
    assert!(state["overrides"][id].get("status").is_none());
    assert_eq!(state["overrides"][id]["hidden"], true);
    assert_eq!(state["overrides"][id]["futureNs"], 7);
    // A status action does not invalidate a captured token for an unchanged title.
    patch(&repo, &baseline, &json!({"title":"Independent title"})).unwrap();
    assert_eq!(
        read(&repo, &json!({"id":id})).unwrap()["project"]["status"],
        "done"
    );
    let baseline = read(&repo, &json!({"id":id})).unwrap();
    super::super::agent_access::update_work(&repo, &json!({"id":id,"status":"blocked"})).unwrap();
    assert_eq!(
        patch(&repo, &baseline, &json!({"status":"todo"}))
            .unwrap_err()
            .status,
        409
    );
}
#[test]
fn projects_same_field_race_has_one_winner_and_unrelated_field_races_both_commit() {
    for same in [true, false] {
        let (_temp, repo, baseline) = fixture();
        let gate = Arc::new(Barrier::new(3));
        let mut workers = vec![];
        for index in 0..2 {
            let repo = repo.clone();
            let baseline = baseline.clone();
            let gate = gate.clone();
            workers.push(std::thread::spawn(move || {
                let field = if index == 0 || same {
                    "title"
                } else {
                    "detail"
                };
                let mut change = json!({});
                change[field] = json!(format!("value-{index}"));
                gate.wait();
                patch(&repo, &baseline, &change)
            }));
        }
        gate.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            if same { 1 } else { 2 }
        );
        if same {
            assert_eq!(
                results.into_iter().find_map(Result::err).unwrap().status,
                409
            );
        } else {
            let result = read(&repo, &json!({"id":baseline["project"]["id"]})).unwrap();
            assert_eq!(result["project"]["title"], "value-0");
            assert_eq!(result["project"]["detail"], "value-1");
        }
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&repo.data).unwrap()).unwrap(),
            *repo.state.lock().unwrap()
        );
    }
}
#[test]
fn projects_validate_exact_patch_tokens_and_fail_without_any_canonical_change() {
    let (_temp, repo, baseline) = fixture();
    let id = &baseline["project"]["id"];
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let token = baseline["expected"]["title"].clone();
    for (patch, expected) in [
        (json!({}), json!({})),
        (json!({"title":"New"}), json!({})),
        (
            json!({"title":"New"}),
            json!({"title":token,"detail":baseline["expected"]["detail"]}),
        ),
        (json!({"title":"New"}), json!({"title":"bad"})),
        (json!({"title":"  "}), json!({"title":token})),
        (json!({"title":7}), json!({"title":token})),
        (json!({"title":"x".repeat(301)}), json!({"title":token})),
        (
            json!({"instruction":"run command"}),
            json!({"instruction":"0".repeat(64)}),
        ),
        (
            json!({"url":"https://secret:token@example.invalid"}),
            json!({"url":baseline["expected"]["url"]}),
        ),
        (
            json!({"cwd":"/nonexistent/project-owner-fixture-path"}),
            json!({"cwd":baseline["expected"]["cwd"]}),
        ),
    ] {
        assert!(update(&repo, &json!({"id":id,"patch":patch,"expected":expected})).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
    assert!(update(
        &repo,
        &json!({"id":"github:foreign","patch":{"title":"No upsert"},"expected":{"title":token}})
    )
    .is_err());
    assert!(create(
        &repo,
        &json!({"title":"Client ID forbidden","id":"note:chosen"})
    )
    .is_err());
    assert!(read(&repo, &json!({"id":id,"extra":true})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
}
#[test]
fn projects_legacy_updates_need_same_tokens_and_creation_uses_same_canonical_owner() {
    let (_temp, repo, baseline) = fixture();
    let id = &baseline["project"]["id"];
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert!(repo
        .edit(&json!({"action":"save-note","id":id,"title":"Unchecked legacy overwrite"}))
        .is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    repo.edit(&json!({"action":"save-note","id":id,"title":"Checked legacy edit","expected":{"title":baseline["expected"]["title"]}})).unwrap();
    let latest = read(&repo, &json!({"id":id})).unwrap();
    assert_eq!(latest["project"]["title"], "Checked legacy edit");
    assert_eq!(latest["project"]["detail"], "Initial detail");
    repo.edit(&json!({"action":"save-note","title":"Workflow project","status":"todo","cwd":null}))
        .unwrap();
    let row = repo.state.lock().unwrap()["notes"][1].clone();
    assert!(text(&row, "id").starts_with("note:"));
    assert_eq!(row["cwd"], "");
    assert_eq!(row["kind"], "Project");
    let created = create(&repo, &json!({"title":"Command project"})).unwrap();
    assert_eq!(created["created"], true);
    assert_eq!(created["project"]["detail"], "");
    assert_eq!(created["project"]["status"], "todo");
    for token in created["expected"].as_object().unwrap().values() {
        assert_eq!(token.as_str().unwrap().len(), 64);
    }
}
#[test]
fn projects_legacy_mixed_unknown_and_missing_identity_never_bypass_checked_patch() {
    let (_temp, repo, baseline) = fixture();
    let id = &baseline["project"]["id"];
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    for args in [
        json!({"action":"save-note","id":id,"title":"Flat","patch":{"title":"Patch"},"expected":{"title":baseline["expected"]["title"]}}),
        json!({"action":"save-note","id":id,"instruction":"ignored executable","title":"No","expected":{"title":baseline["expected"]["title"]}}),
        json!({"action":"save-note","title":"Create","patch":{"title":"Unexpected update"}}),
        json!({"action":"save-note","title":"Create","expected":{}}),
        json!({"action":"save-note","id":7,"title":"No new project"}),
        json!({"action":"save-note","id":"note:missing","title":"No upsert","expected":{"title":baseline["expected"]["title"]}}),
    ] {
        assert!(repo.edit(&args).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
    repo.edit(&json!({"action":"save-note","id":id,"patch":{"detail":"Checked legacy patch"},"expected":{"detail":baseline["expected"]["detail"]}})).unwrap();
    assert_eq!(
        read(&repo, &json!({"id":id})).unwrap()["project"]["detail"],
        "Checked legacy patch"
    );
}
#[test]
fn projects_failed_canonical_save_never_changes_metadata_or_status_override() {
    let (_temp, repo, baseline) = fixture();
    let id = text(&baseline["project"], "id");
    repo.transaction(|state| {
        state["overrides"][id] = json!({"status":"blocked","pinned":true});
        Ok(())
    })
    .unwrap();
    let baseline = read(&repo, &json!({"id":id})).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    assert_eq!(
        patch(&repo, &baseline, &json!({"status":"done"}))
            .unwrap_err()
            .status,
        500
    );
    assert!(create(&repo, &json!({"title":"Not saved"})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
    patch(&repo, &baseline, &json!({"status":"done"})).unwrap();
    assert_eq!(
        read(&repo, &json!({"id":id})).unwrap()["project"]["status"],
        "done"
    );
}

#[test]
fn project_folder_read_uses_launch_selector_without_metadata_cooldown_or_terminal_mutation() {
    let (temp, repo, created) = fixture();
    let id = text(&created["project"], "id");
    let root = temp.path().join("work");
    std::fs::create_dir(&root).unwrap();
    let configured = temp.path().join("configured-outside");
    std::fs::create_dir(&configured).unwrap();
    repo.transaction(|state| {
        state["notes"][0]["cwd"] = json!(&configured);
        state["notes"][0]["privateNs"] = json!("hidden-folder-projection-marker");
        state["itemSettings"][id] = json!({"cwd":temp.path()});
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let revision = repo.snapshot(false)["revision"].clone();
    let cooldown = repo.cooldown.lock().unwrap().clone();
    let opening = repo.opening.lock().unwrap().clone();
    let running = repo.running.lock().unwrap().clone();
    let inventory = repo.embedded.inventory();
    let result = folder_with(&repo, &json!({"id":id}), &root).unwrap();
    assert_eq!(
        result,
        json!({"id":id,"cwd":configured.canonicalize().unwrap(),"requiresSelection":false})
    );
    assert!(!result
        .to_string()
        .contains("hidden-folder-projection-marker"));
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(repo.snapshot(false)["revision"], revision);
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
    assert_eq!(*repo.opening.lock().unwrap(), opening);
    assert_eq!(*repo.running.lock().unwrap(), running);
    assert_eq!(repo.embedded.inventory(), inventory);
    std::fs::remove_dir(&configured).unwrap();
    let fallback = folder_with(&repo, &json!({"id":id}), &root).unwrap();
    assert_eq!(fallback["cwd"], json!(temp.path().canonicalize().unwrap()));
    repo.transaction(|state| {
        state["itemSettings"] = json!({});
        Ok(())
    })
    .unwrap();
    assert_eq!(
        folder_with(&repo, &json!({"id":id}), &root).unwrap(),
        json!({"id":id,"cwd":null,"requiresSelection":true})
    );
}
#[test]
fn project_folder_rejects_unknown_or_non_project_identity() {
    let (temp, repo, created) = fixture();
    let id = &created["project"]["id"];
    for args in [
        json!({"id":"github:foreign"}),
        json!({"id":id,"cwd":"/caller/chosen"}),
        json!({"id":7}),
        json!({}),
    ] {
        assert!(folder_with(&repo, &args, temp.path()).is_err());
    }
}
