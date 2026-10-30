use super::*;
use crate::backend::uuid;
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>) {
    fixture_secret(None)
}
fn fixture_secret(secret: Option<&str>) -> (tempfile::TempDir, Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    if let Some(secret) = secret {
        std::fs::write(
            temp.path().join(".env"),
            format!("TRUFFLE_PRESENTATION_SECRET={secret}\n"),
        )
        .unwrap();
    }
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["items"]=json!([{"id":"linear:a","source":"linear","kind":"Issue","title":"Cached work","status":"todo","detail":"Details","privateExtension":"PRIVATE_WORK","ci":{"state":"failed","total":7,"passed":5,"failed":2,"privateExtension":"PRIVATE_CI"}}]);state["sessions"]=json!([{"id":"10000000-0000-4000-8000-000000000001","agent":"codex","title":"Saved chat","cwd":"/safe/reference","items":[{"id":"linear:a","title":"Linked","privateExtension":"PRIVATE_LINK"}],"privateExtension":"PRIVATE_CHAT"}]);Ok(())}).unwrap();
    (temp, repo)
}
fn args(id: &str) -> Value {
    json!({"requestId":id,"presentation":{"type":"display-items","title":"Chosen results","items":[{"type":"work","id":"linear:a"},{"type":"chat","id":"10000000-0000-4000-8000-000000000001"}],"recommended":{"type":"work","id":"linear:a"},"reason":"Verified cached matches"}})
}
#[test]
fn presentation_canonical_builder_is_shared_bounded_and_strips_private_extensions() {
    let (_temp, repo) = fixture();
    let args = args(&uuid());
    let expected = display_items(&repo, &args["presentation"]).unwrap();
    let receipt = present(&repo, &args).unwrap();
    assert_eq!(
        receipt,
        json!({"requestId":args["requestId"],"status":"queued"})
    );
    let result = read(&repo).unwrap();
    assert_eq!(result["pending"][0]["presentation"], expected);
    for marker in [
        "PRIVATE_WORK",
        "PRIVATE_CHAT",
        "PRIVATE_LINK",
        "PRIVATE_CI",
        "hash",
    ] {
        assert!(!result.to_string().contains(marker));
    }
    assert_eq!(
        result["pending"][0]["presentation"]["items"][0]["item"]["ci"]["state"],
        "failed"
    );
    assert_eq!(
        result["pending"][0]["presentation"]["items"][0]["item"]["ci"]["total"],
        7
    );
    assert_eq!(
        result["pending"][0]["presentation"]["items"][1]["item"]["items"][0]["title"],
        "Linked"
    );
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn presentation_replay_precedes_catalog_resolution_and_conflicts_before_mutation() {
    let (_temp, repo) = fixture();
    let args = args(&uuid());
    let first = present(&repo, &args).unwrap();
    repo.transaction(|state| {
        state["items"] = json!([]);
        state["sessions"] = json!([]);
        Ok(())
    })
    .unwrap();
    let before = std::fs::read(&repo.data).unwrap();
    assert_eq!(present(&repo, &args).unwrap(), first);
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    let mut changed = args;
    changed["presentation"]["title"] = json!("Other intent");
    assert_eq!(present(&repo, &changed).unwrap_err().status, 409);
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
}
#[test]
fn presentation_ack_is_exact_and_releases_only_its_pending_descriptor() {
    let (_temp, repo) = fixture();
    let first = args(&uuid());
    let second = args(&uuid());
    present(&repo, &first).unwrap();
    present(&repo, &second).unwrap();
    repo.transaction(|state| {
        state["uiPresentations"]["extension"] = json!({"private":"ROOT_PRIVATE"});
        state["uiPresentations"]["entries"][0]["extension"] = json!("ENTRY_PRIVATE");
        Ok(())
    })
    .unwrap();
    let ack = json!({"requestId":first["requestId"],"outcome":"shown"});
    assert_eq!(acknowledge(&repo, &ack).unwrap()["status"], "shown");
    let before = std::fs::read(&repo.data).unwrap();
    assert_eq!(acknowledge(&repo, &ack).unwrap()["status"], "shown");
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    assert_eq!(
        acknowledge(
            &repo,
            &json!({"requestId":first["requestId"],"outcome":"dismissed"})
        )
        .unwrap_err()
        .status,
        409
    );
    assert_eq!(
        acknowledge(&repo, &json!({"requestId":uuid(),"outcome":"shown"}))
            .unwrap_err()
            .status,
        409
    );
    let result = read(&repo).unwrap();
    assert_eq!(result["pending"].as_array().unwrap().len(), 1);
    assert_eq!(result["pending"][0]["requestId"], second["requestId"]);
    assert!(!result.to_string().contains("PRIVATE"));
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(
        state["uiPresentations"]["extension"]["private"],
        "ROOT_PRIVATE"
    );
    assert_eq!(
        state["uiPresentations"]["entries"][0]["extension"],
        "ENTRY_PRIVATE"
    );
    assert!(state["uiPresentations"]["entries"][0]
        .get("presentation")
        .is_none());
}
#[test]
fn presentation_pending_capacity_never_evicts_and_acknowledged_retention_is_bounded() {
    let (_temp, repo) = fixture();
    let mut requests = vec![];
    for _ in 0..8 {
        let args = args(&uuid());
        present(&repo, &args).unwrap();
        requests.push(args);
    }
    let before = std::fs::read(&repo.data).unwrap();
    assert_eq!(present(&repo, &args(&uuid())).unwrap_err().status, 409);
    assert_eq!(present(&repo, &requests[0]).unwrap()["status"], "queued");
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    for index in 0..40 {
        let request_id = requests.get(index).filter(|_| index < 7).map_or_else(
            || {
                let next = args(&uuid());
                present(&repo, &next).unwrap();
                next["requestId"].as_str().unwrap().to_owned()
            },
            |request| request["requestId"].as_str().unwrap().to_owned(),
        );
        acknowledge(
            &repo,
            &json!({"requestId":request_id,"outcome":"dismissed"}),
        )
        .unwrap();
    }
    let result = read(&repo).unwrap();
    assert_eq!(result["pending"].as_array().unwrap().len(), 1);
    assert_eq!(result["pending"][0]["requestId"], requests[7]["requestId"]);
    assert_eq!(result["receipts"].as_array().unwrap().len(), 33);
    assert_eq!(
        acknowledge(
            &repo,
            &json!({"requestId":requests[0]["requestId"],"outcome":"shown"})
        )
        .unwrap_err()
        .status,
        409
    );
}
#[test]
fn presentation_invalid_versions_targets_and_arguments_fail_without_writes() {
    let (_temp, repo) = fixture();
    let before = std::fs::read(&repo.data).unwrap();
    for value in [
        json!({"requestId":uuid(),"presentation":{"type":"display-items","items":[{"type":"work","id":"missing"}]}}),
        json!({"requestId":uuid(),"presentation":{"type":"display-items","items":[{"type":"work","id":"linear:a","private":"not accepted"}]}}),
        json!({"requestId":uuid(),"presentation":{"type":"script","code":"not executable"}}),
        json!({"requestId":uuid(),"presentation":{"type":"display-items","items":vec![json!({"type":"work","id":"linear:a"});41]}}),
    ] {
        assert!(present(&repo, &value).is_err());
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    }
    repo.transaction(|state| {
        state["uiPresentations"] = json!({"version":2,"entries":[]});
        Ok(())
    })
    .unwrap();
    let before = std::fs::read(&repo.data).unwrap();
    assert!(read(&repo).is_err());
    assert!(present(&repo, &args(&uuid())).is_err());
    assert!(acknowledge(&repo, &json!({"requestId":uuid(),"outcome":"shown"})).is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
}
#[test]
fn presentation_failed_save_and_concurrent_same_request_have_one_canonical_entry() {
    let (_temp, repo) = fixture();
    let args = args(&uuid());
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    assert!(present(&repo, &args).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
    let threads = (0..4)
        .map(|_| {
            let repo = repo.clone();
            let args = args.clone();
            std::thread::spawn(move || present(&repo, &args).unwrap())
        })
        .collect::<Vec<_>>();
    for thread in threads {
        assert_eq!(thread.join().unwrap()["status"], "queued");
    }
    assert_eq!(read(&repo).unwrap()["pending"].as_array().unwrap().len(), 1);
}
#[test]
fn presentation_saved_view_is_resolved_canonically_and_not_created_by_read() {
    let (_temp, repo) = fixture();
    let doc = views::load(&repo).unwrap();
    let view = doc.views.first().unwrap();
    let args = json!({"requestId":uuid(),"presentation":{"type":"saved-view","id":view.id}});
    let before = std::fs::read(&repo.data).unwrap();
    assert_eq!(read(&repo).unwrap(), json!({"pending":[],"receipts":[]}));
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    present(&repo, &args).unwrap();
    assert_eq!(
        read(&repo).unwrap()["pending"][0]["presentation"],
        saved_view(&repo, &args["presentation"]).unwrap()
    );
}
#[test]
fn presentation_routing_tags_are_exact_or_fail_before_queue_and_on_read() {
    for secret in ["linear", "Issue", "todo", "codex", "failed"] {
        let (_temp, repo) = fixture_secret(Some(secret));
        let before = std::fs::read(&repo.data).unwrap();
        assert!(present(&repo, &args(&uuid())).is_err(), "{secret}");
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    }
    let (temp, repo) = fixture();
    present(&repo, &args(&uuid())).unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "TRUFFLE_PRESENTATION_SECRET=linear\n",
    )
    .unwrap();
    drop(repo);
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = std::fs::read(&repo.data).unwrap();
    assert!(read(&repo).is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
}
#[test]
fn presentation_redacts_complete_display_values_before_truncation() {
    let secret = format!("private-display-{}", "Z".repeat(350));
    let (_temp, repo) = fixture_secret(Some(&secret));
    repo.transaction(|state| {
        state["items"][0]["detail"] = json!(format!("{secret} visible"));
        Ok(())
    })
    .unwrap();
    let value = display_items(&repo, &args(&uuid())["presentation"]).unwrap();
    assert_eq!(value["items"][0]["item"]["detail"], "[redacted] visible");
    assert!(!value.to_string().contains("private-display-"));
}
