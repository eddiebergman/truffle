use super::*;
use crate::backend::chat_preparation::{self, Kind};
use crate::backend::{http, uuid};
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, String) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s|{s["sessions"]=json!([{ "id":id,"agent":"codex","sessionId":null,"launchNew":true,"title":"Initial","cwd":temp.path(),"instruction":"Initial instruction","destination":"embedded","items":[{"id":"note:a","source":"notes","title":"A","linkExtension":{"retain":true}}],"futureNs":{"private":"retain"},"preparationRequests":{"version":1,"entries":[]},"runtime":{"state":"running"},"resources":[{"future":1}]}]);s["notes"]=json!([{ "id":"note:a","source":"notes","title":"A"},{"id":"note:b","source":"notes","title":"B"}]);s["itemSettings"]["note:a"]=json!({"cwd":"independent folder","explicit":true});Ok(())}).unwrap();
    (temp, repo, id)
}
fn baseline(repo: &Repository, id: &str) -> Value {
    read(repo, &json!({"recordId":id})).unwrap()
}
fn request(base: &Value, patch: &Value) -> Value {
    let mut expected = json!({});
    for key in patch.as_object().unwrap().keys() {
        expected[key] = base["expected"][key].clone();
    }
    json!({"recordId":base["recordId"],"patch":patch,"expected":expected})
}
#[test]
fn chat_metadata_patched_fields_commute_and_preserve_latest_extensions_folders_and_aliases() {
    let (_temp, repo, id) = fixture();
    let old = baseline(&repo, &id);
    update(
        &repo,
        &request(&old, &json!({"itemIds":["note:a","note:b"]})),
    )
    .unwrap();
    update(&repo, &request(&old, &json!({"title":"Renamed"}))).unwrap();
    let current = baseline(&repo, &id);
    assert_eq!(current["chat"]["itemIds"], json!(["note:a", "note:b"]));
    assert_eq!(current["chat"]["title"], "Renamed");
    let state = repo.state.lock().unwrap().clone();
    let row = &state["sessions"][0];
    assert_eq!(row["futureNs"]["private"], "retain");
    assert_eq!(
        row["preparationRequests"],
        json!({"version":1,"entries":[]})
    );
    assert_eq!(row["runtime"]["state"], "running");
    assert_eq!(row["items"][0]["linkExtension"]["retain"], true);
    assert_eq!(row["resources"], json!([{"future":1}]));
    assert_eq!(state["itemSettings"]["note:a"]["cwd"], "independent folder");
    assert!(state["itemSettings"].get("note:b").is_none());
    assert!(!current.to_string().contains("retain"));
}
#[test]
fn chat_metadata_stale_mixed_patch_and_bad_shape_leave_state_and_disk_unchanged() {
    let (_temp, repo, id) = fixture();
    let old = baseline(&repo, &id);
    update(&repo, &request(&old, &json!({"title":"Concurrent"}))).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        update(
            &repo,
            &request(&old, &json!({"title":"Stale","instruction":"Do not apply"}))
        )
        .unwrap_err()
        .status,
        409
    );
    let valid = baseline(&repo, &id);
    for bad in [
        json!({"recordId":id,"patch":{},"expected":{}}),
        json!({"recordId":id,"patch":{"title":"Bad"},"expected":{}}),
        json!({"recordId":id,"patch":{"agent":"claude"},"expected":{"agent":"0".repeat(64)}}),
        json!({"recordId":id,"patch":{"title":"Bad"},"expected":{"title":valid["expected"]["title"],"cwd":valid["expected"]["cwd"]}}),
    ] {
        assert!(update(&repo, &bad).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn chat_metadata_link_tokens_ignore_ref_refresh_but_conflict_on_relationship_change() {
    let (_temp, repo, id) = fixture();
    let old = baseline(&repo, &id);
    repo.transaction(|s| {
        s["sessions"][0]["items"][0]["title"] = json!("Refreshed source title");
        Ok(())
    })
    .unwrap();
    assert_eq!(
        baseline(&repo, &id)["expected"]["itemIds"],
        old["expected"]["itemIds"]
    );
    update(&repo, &request(&old, &json!({"itemIds":["note:b"]}))).unwrap();
    assert_eq!(
        update(&repo, &request(&old, &json!({"itemIds":["note:a"]})))
            .unwrap_err()
            .status,
        409
    );
}
#[test]
fn chat_metadata_association_only_fills_empty_identity_and_detects_conflicts() {
    let (_temp, repo, id) = fixture();
    let old = baseline(&repo, &id);
    let session = uuid();
    update(&repo, &request(&old, &json!({"sessionId":session}))).unwrap();
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["launchNew"],
        false
    );
    assert_eq!(
        update(&repo, &request(&old, &json!({"sessionId":uuid()})))
            .unwrap_err()
            .status,
        409
    );
    let latest = baseline(&repo, &id);
    assert_eq!(
        update(&repo, &request(&latest, &json!({"sessionId":""})))
            .unwrap_err()
            .status,
        409
    );
    let (_other, repo, id) = fixture();
    let old = baseline(&repo, &id);
    repo.transaction(|s| {
        let mut other = s["sessions"][0].clone();
        other["id"] = json!(uuid());
        other["sessionId"] = json!(session);
        s["sessions"].as_array_mut().unwrap().push(other);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        update(&repo, &request(&old, &json!({"sessionId":session})))
            .unwrap_err()
            .status,
        409
    );
}
#[test]
fn chat_metadata_url_resolution_rechecks_tokens_and_never_overwrites_newer_cache() {
    let (_temp, repo, id) = fixture();
    let old = baseline(&repo, &id);
    let mut args = request(&old, &json!({"itemIds":["note:a"]}));
    args["urls"] = json!(["https://example.invalid/work"]);
    let mut empty = args.clone();
    empty["urls"] = json!([""]);
    assert!(update_with(&repo, &empty, |_| panic!(
        "Empty URLs must fail before provider resolution"
    ))
    .is_err());
    let bytes = std::fs::read(&repo.data).unwrap();
    let result = update_with(&repo, &args, |_| {
        repo.transaction(|s| {
            s["sessions"][0]["items"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"note:b","source":"notes","title":"B"}));
            Ok(())
        })?;
        Ok(
            json!({"id":"github:new","source":"github","title":"New","url":"https://example.invalid/work"}),
        )
    });
    assert_eq!(result.unwrap_err().status, 409);
    assert!(!domain::catalog(&repo.state.lock().unwrap()).contains_key("github:new"));
    assert_ne!(std::fs::read(&repo.data).unwrap(), bytes);
    let latest = baseline(&repo, &id);
    let mut args = request(&latest, &json!({"itemIds":["note:a","note:b"]}));
    args["urls"] = json!(["https://example.invalid/work"]);
    update_with(&repo,&args,|_|{repo.transaction(|s|{s["items"].as_array_mut().unwrap().push(json!({"id":"github:new","source":"github","title":"Newer","url":"https://example.invalid/work"}));Ok(())})?;Ok(json!({"id":"github:new","source":"github","title":"Old result","url":"https://example.invalid/work"}))}).unwrap();
    assert_eq!(
        domain::catalog(&repo.state.lock().unwrap())["github:new"]["title"],
        "Newer"
    );
    assert_eq!(
        baseline(&repo, &id)["chat"]["itemIds"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
#[test]
fn chat_metadata_failed_persistence_rolls_back_and_retry_succeeds() {
    let (_temp, repo, id) = fixture();
    let old = baseline(&repo, &id);
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let blocker = repo.data.with_extension("tmp");
    std::fs::create_dir(&blocker).unwrap();
    assert!(update(&repo, &request(&old, &json!({"title":"After failure"}))).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::remove_dir(blocker).unwrap();
    update(&repo, &request(&old, &json!({"title":"After failure"}))).unwrap();
}
#[test]
fn chat_metadata_legacy_requires_checked_patch_and_does_not_resolve_unchecked_urls() {
    let (_temp, repo, id) = fixture();
    let old = baseline(&repo, &id);
    let before = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        http::post_route(
            &repo,
            "/api/session/prepare",
            &json!({"id":id,"agent":"codex","title":"Unchecked","urls":["not a URL"]})
        )
        .err()
        .unwrap()
        .status,
        409
    );
    assert!(repo
        .edit(&json!({"action":"save-session","id":id,"title":"Unchecked"}))
        .is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    assert!(chat_preparation::save(
        &repo,
        Kind::New,
        &json!({"id":3,"agent":"codex","title":"Invalid ID"})
    )
    .is_err());
    let checked = request(&old, &json!({"title":"Checked legacy"}));
    http::post_route(
        &repo,
        "/api/session/prepare",
        &json!({"id":id,"patch":checked["patch"],"expected":checked["expected"]}),
    )
    .unwrap();
    assert_eq!(baseline(&repo, &id)["chat"]["title"], "Checked legacy");
}
#[test]
fn chat_metadata_command_redaction_preserves_tokens_and_hides_reference_text() {
    let temp = tempfile::tempdir().unwrap();
    let secret = "private-chat-metadata-canary";
    std::fs::write(
        temp.path().join(".env"),
        format!("METADATA_TEST_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s|{s["sessions"]=json!([{"id":id,"agent":"codex","title":"Title","cwd":temp.path(),"instruction":format!("Use {secret} as reference"),"destination":"embedded","sessionId":null,"items":[]}]);Ok(())}).unwrap();
    let base = commands::invoke(
        &repo,
        &json!({"name":"chats.metadata.read","args":{"recordId":id}}),
        true,
    )
    .unwrap();
    assert_eq!(base["redacted"], true);
    assert!(!base.to_string().contains(secret));
    let result = commands::invoke(
        &repo,
        &json!({"name":"chats.metadata.update","args":request(&base, &json!({"title":"Changed"}))}),
        false,
    )
    .unwrap();
    assert_eq!(result["recordId"], id);
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["instruction"],
        format!("Use {secret} as reference")
    );
    repo.transaction(|s| {
        s["sessions"][0]["items"] = json!([{"id":secret}]);
        Ok(())
    })
    .unwrap();
    assert!(read(&repo, &json!({"recordId":id})).is_err());
    let bytes = std::fs::read(&repo.data).unwrap();
    assert!(update(&repo, &request(&base, &json!({"itemIds":[secret]}))).is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn chat_metadata_concurrent_same_field_allows_one_commit() {
    let (_temp, repo, id) = fixture();
    let old = baseline(&repo, &id);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let threads: [_; 2] = std::array::from_fn(|n| {
        let repo = repo.clone();
        let barrier = barrier.clone();
        let args = request(&old, &json!({"title":format!("Writer {n}")}));
        std::thread::spawn(move || {
            barrier.wait();
            update(&repo, &args)
        })
    });
    barrier.wait();
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r.as_ref().err().is_some_and(|e| e.status == 409))
            .count(),
        1
    );
}
#[test]
fn chat_metadata_invalid_stored_fields_fail_closed_and_legacy_association_cannot_bypass() {
    let (_temp, repo, id) = fixture();
    let session = uuid();
    repo.transaction(|s| {
        s["sessions"][0]["sessionId"] = json!(session);
        Ok(())
    })
    .unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    let payload = json!({"agent":"codex","sessionId":session,"title":"Unchecked","cwd":"invalid","urls":["invalid"]});
    assert_eq!(
        chat_preparation::save(&repo, Kind::New, &payload)
            .unwrap_err()
            .status,
        409
    );
    let mut edit = payload;
    edit["action"] = json!("save-session");
    assert_eq!(repo.edit(&edit).unwrap_err().status, 409);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    for (field, value) in [
        ("title", json!(42)),
        ("cwd", json!(null)),
        ("instruction", json!({"bad":true})),
        ("destination", json!(false)),
        ("destination", json!("unexpected")),
        ("sessionId", json!("not-a-uuid")),
    ] {
        repo.transaction(|s| {
            s["sessions"][0][field] = value.clone();
            Ok(())
        })
        .unwrap();
        assert!(read(&repo, &json!({"recordId":id})).is_err());
        repo.transaction(|s| {
            s["sessions"][0][field] = json!(match field {
                "title" => "Initial",
                "cwd" => "/unvalidated/read-only/saved/path",
                "instruction" => "Reference",
                "destination" => "embedded",
                _ => session.as_str(),
            });
            Ok(())
        })
        .unwrap();
    }
}
#[test]
fn chat_metadata_read_is_pure_and_preserves_complete_link_set() {
    let (_temp, repo, id) = fixture();
    repo.transaction(|s| {
        s["sessions"][0]["items"] = json!((0..100)
            .map(|i| json!({"id":format!("archived:{i}"),"title":"Cached"}))
            .collect::<Vec<_>>());
        Ok(())
    })
    .unwrap();
    let state = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let result = baseline(&repo, &id);
    assert_eq!(result["chat"]["itemIds"].as_array().unwrap().len(), 100);
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    update(&repo, &request(&result, &json!({"title":"Only title"}))).unwrap();
    assert_eq!(
        baseline(&repo, &id)["chat"]["itemIds"],
        result["chat"]["itemIds"]
    );
}
