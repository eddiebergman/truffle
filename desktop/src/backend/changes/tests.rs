use super::*;
use crate::backend::domain;
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["changes"]=json!([{"id":"a","itemId":"work:a","title":"Title","source":"github","message":"Ready","read":false,"privateNs":{"secret":"do-not-project"}},{"id":"b","itemId":"work:b","source":"linear","read":true},{"id":"c","itemId":"work:a","source":"github","read":false}]);Ok(())}).unwrap();
    (temp, repo)
}
#[test]
fn changes_list_is_bounded_read_only_whitelisted_and_stable_under_arrivals() {
    let (_temp, repo) = fixture();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let first = list(&repo, &json!({"limit":1,"unread":true})).unwrap();
    assert_eq!(first["changes"][0]["id"], "a");
    assert_eq!(first["nextAfterId"], "a");
    assert!(!first.to_string().contains("do-not-project"));
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    repo.transaction(|state| {
        state["changes"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"id":"new","source":"github","read":false}));
        Ok(())
    })
    .unwrap();
    let second = list(
        &repo,
        &json!({"afterId":"a","unread":true,"source":"github","itemId":"work:a"}),
    )
    .unwrap();
    assert_eq!(second["changes"].as_array().unwrap().len(), 1);
    assert_eq!(second["changes"][0]["id"], "c");
    assert_eq!(second["nextAfterId"], Value::Null);
    assert_eq!(
        list(&repo, &json!({"afterId":"pruned"}))
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn changes_mark_exact_ids_preserves_late_arrivals_extensions_and_reports_pruned() {
    let (_temp, repo) = fixture();
    let captured = json!({"ids":["a","b","pruned"]});
    repo.transaction(|state| {
        state["changes"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"id":"late","read":false,"extension":17}));
        Ok(())
    })
    .unwrap();
    let result = mark_read(&repo, &captured).unwrap();
    assert_eq!(
        result,
        json!({"requestedIds":["a","b","pruned"],"markedIds":["a"],"alreadyReadIds":["b"],"absentIds":["pruned"],"markedCount":1,"scope":"local"})
    );
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(
        state["changes"][0],
        json!({"id":"late","read":false,"extension":17})
    );
    assert_eq!(state["changes"][1]["privateNs"]["secret"], "do-not-project");
    assert_eq!(state["changes"][3]["read"], false);
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&repo.data).unwrap()).unwrap()["changes"],
        state["changes"]
    );
    let before = std::fs::read(&repo.data).unwrap();
    assert_eq!(mark_read(&repo, &captured).unwrap()["markedCount"], 0);
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
}
#[test]
fn changes_invalid_and_legacy_global_arguments_fail_before_effects() {
    let (_temp, repo) = fixture();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    for args in [
        json!({}),
        json!({"ids":["a","a"]}),
        json!({"ids":[""]}),
        json!({"ids":[7]}),
        json!({"ids":vec!["a";501]}),
        json!({"ids":["a"],"all":true}),
    ] {
        assert!(mark_read(&repo, &args).is_err());
    }
    for args in [
        json!({"limit":0}),
        json!({"limit":101}),
        json!({"unread":7}),
        json!({"source":"other"}),
        json!({"offset":1}),
    ] {
        assert!(list(&repo, &args).is_err());
    }
    for args in [
        json!({"action":"read-changes","all":true}),
        json!({"action":"read-changes","all":false,"id":"a"}),
        json!({"action":"read-changes","id":"a","ids":["a"]}),
    ] {
        assert!(repo
            .transaction(|state| domain::edit(state, &args))
            .is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    repo.transaction(|state| domain::edit(state, &json!({"action":"read-changes","id":"a"})))
        .unwrap();
    assert_eq!(repo.state.lock().unwrap()["changes"][0]["read"], true);
    assert_eq!(repo.state.lock().unwrap()["changes"][2]["read"], false);
}
#[test]
fn changes_redact_before_display_limits_and_reject_unsafe_urls() {
    let temp = tempfile::tempdir().unwrap();
    let secret = format!("private-change-{}", "X".repeat(350));
    std::fs::write(
        temp.path().join(".env"),
        format!("TRUFFLE_TEST_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["changes"]=json!([{"id":"a","title":format!("{secret} {}","界".repeat(400)),"message":secret,"source":"github","url":"https://user:password@example.invalid/private","at":-2,"privateExtension":"hidden-projection"}]);Ok(())}).unwrap();
    let result = list(&repo, &json!({})).unwrap();
    assert!(text(&result["changes"][0], "title").starts_with("[redacted] "));
    assert_eq!(text(&result["changes"][0], "title").chars().count(), 300);
    assert_eq!(result["changes"][0]["message"], "[redacted]");
    assert_eq!(result["changes"][0]["url"], "");
    assert_eq!(result["changes"][0]["at"], Value::Null);
    assert!(!result.to_string().contains("hidden-projection"));
}
#[test]
fn changes_failed_save_preserves_memory_and_canonical_bytes_then_allows_exact_retry() {
    let (_temp, repo) = fixture();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    assert!(mark_read(&repo, &json!({"ids":["a"]})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
    assert_eq!(
        mark_read(&repo, &json!({"ids":["a"]})).unwrap()["markedIds"],
        json!(["a"])
    );
}

#[test]
fn changes_identity_projection_is_exact_or_fails_without_exposing_or_marking_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let secret = "private-cursor-identity";
    std::fs::write(
        temp.path().join(".env"),
        format!("TRUFFLE_TEST_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for (field, value) in [
        ("id", json!(secret)),
        ("id", json!("x".repeat(161))),
        ("id", json!("unsafe\nidentity")),
        ("itemId", json!(secret)),
        ("itemId", json!("x".repeat(161))),
        ("itemId", json!("unsafe\tidentity")),
        ("source", json!("unknown-source")),
    ] {
        repo.transaction(|state|{state["changes"]=json!([{"id":"safe-id","itemId":"safe-work","source":"github","read":false},{"id":"next","source":"linear","read":false}]);state["changes"][0][field]=value.clone();Ok(())}).unwrap();
        let before = repo.state.lock().unwrap().clone();
        let bytes = std::fs::read(&repo.data).unwrap();
        let error = commands::invoke(
            &repo,
            &json!({"name":"changes.list","args":{"limit":1}}),
            true,
        )
        .unwrap_err();
        assert!(!error.message.contains(secret));
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
    for present in [false, true] {
        repo.transaction(|state|{state["changes"]=json!([{"id":if present{secret}else{"safe-id"},"source":"github","read":false},{"id":"next","source":"linear","read":false}]);Ok(())}).unwrap();
        let before = repo.state.lock().unwrap().clone();
        let bytes = std::fs::read(&repo.data).unwrap();
        assert!(commands::invoke(
            &repo,
            &json!({"name":"changes.mark-read","args":{"ids":["next",secret]}}),
            true
        )
        .is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
    repo.transaction(|state| {
        state["changes"] =
            json!([{"id":"safe-id","source":"github"},{"id":"next","source":"linear"}]);
        Ok(())
    })
    .unwrap();
    let result = commands::invoke(
        &repo,
        &json!({"name":"changes.list","args":{"limit":1}}),
        true,
    )
    .unwrap();
    assert_eq!(result["changes"][0]["id"], "safe-id");
    assert_eq!(result["changes"][0]["itemId"], "");
    assert_eq!(result["nextAfterId"], "safe-id");
}
