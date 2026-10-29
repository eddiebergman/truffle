use super::*;
fn payload(current: &Value, scopes: Value) -> Value {
    json!({"revision":current["revision"],"contentHash":current["contentHash"],"page":"linear","scopes":scopes})
}

#[test]
fn page_scopes_match_gear_keys_without_saved_view_normalization() {
    for (page, item, expected) in [
        (
            "github",
            json!({"url":"https://github.com/Owner/Repo/pull/7","project":"Wrong"}),
            "Owner/Repo",
        ),
        (
            "github",
            json!({"url":"https://github.com/Owner/Repo/","project":"Wrong"}),
            "Owner/Repo",
        ),
        (
            "github",
            json!({"url":"https://github.com/Owner/Repo","project":"Fallback"}),
            "Fallback",
        ),
        (
            "github",
            json!({"url":"https://github.com//Repo/pull/7","project":"Fallback"}),
            "Fallback",
        ),
        (
            "github",
            json!({"url":"https://example.invalid/Owner/Repo/pull/7","project":""}),
            "Unknown repository",
        ),
        (
            "linear",
            json!({"project":"Mixed Case / 空"}),
            "Mixed Case / 空",
        ),
        ("sentry", json!({"project":""}), "No project"),
        ("linear", json!({}), "No project"),
    ] {
        assert_eq!(cached_scope(&item, page).unwrap(), expected);
    }
    for item in [
        json!({"project":"x".repeat(501)}),
        json!({"project":"bad\0key"}),
    ] {
        assert!(cached_scope(&item, "linear").is_err());
    }
    for (page, item) in [
        ("linear", json!({"project":42})),
        ("sentry", json!({"project":{"private":"do not echo"}})),
        ("github", json!({"url":42,"project":"Fallback"})),
    ] {
        assert_eq!(
            cached_scope(&item, page).unwrap_err().message,
            "Cached visibility scope metadata is invalid."
        );
    }
}

#[test]
fn page_scopes_enforce_complete_catalog_bounds_without_silent_truncation() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for keys in [
            (0..4097).map(|i|json!({"id":format!("linear:{i}"),"source":"linear","project":format!("Scope {i}")})).collect::<Vec<_>>(),
            (0..4096).map(|i|json!({"id":format!("linear:{i}"),"source":"linear","project":format!("{}-{i:04}","x".repeat(495))})).collect::<Vec<_>>(),
        ] {
            repo.transaction(|state|{state["items"]=json!(keys);Ok(())}).unwrap();
            let before=std::fs::read(&repo.data).unwrap();
            assert!(scopes(&repo,&json!({"page":"linear","query":"no matching rows"})).is_err());
            assert_eq!(std::fs::read(&repo.data).unwrap(),before);
        }
}
#[test]
fn visibility_updates_require_current_revision_and_file_content_hash() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let path = repo.settings.config_directory().join("page-settings.json");
    let baseline = view(&repo).unwrap();
    let mut external = super::super::config::read(&path).unwrap();
    external["pages"]["github"]["scopes"] = json!(["external/project"]);
    std::fs::write(&path, serde_json::to_vec_pretty(&external).unwrap()).unwrap();
    let saved = std::fs::read(&path).unwrap();
    let stale = update(&repo, &payload(&baseline, json!([]))).unwrap_err();
    assert_eq!(stale.status, 409);
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    let fresh = view(&repo).unwrap();
    assert_eq!(fresh["revision"], baseline["revision"]);
    assert_ne!(fresh["contentHash"], baseline["contentHash"]);
    let none = update(&repo, &payload(&fresh, json!([]))).unwrap();
    assert_eq!(none["pages"]["linear"]["scopes"], json!([]));
    assert_eq!(
        none["pages"]["github"]["scopes"],
        json!(["external/project"])
    );
    assert_ne!(none["contentHash"], fresh["contentHash"]);
    let persisted = super::super::config::read(&path).unwrap();
    assert!(persisted.get("contentHash").is_none());
    assert_eq!(persisted.as_object().unwrap().len(), 3);
    assert_eq!(none, view(&repo).unwrap());
    let selected = update(&repo, &payload(&none, json!(["Latency"]))).unwrap();
    assert_eq!(selected["pages"]["linear"]["scopes"], json!(["Latency"]));
    let all = update(&repo, &payload(&selected, Value::Null)).unwrap();
    assert!(all["pages"]["linear"]["scopes"].is_null());
    assert_eq!(
        update(&repo, &payload(&selected, json!([])))
            .unwrap_err()
            .status,
        409
    );
}
#[test]
fn invalid_or_extra_fields_never_modify_the_document() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let path = repo.settings.config_directory().join("page-settings.json");
    let current = view(&repo).unwrap();
    let saved = std::fs::read(&path).unwrap();
    let mut extra = payload(&current, json!([]));
    extra["token"] = json!("benign-secret-marker");
    let mut unknown = payload(&current, json!([]));
    unknown["page"] = json!("unknown");
    for invalid in [
        payload(&current, json!(["x", "x"])),
        payload(&current, json!([true])),
        payload(&current, json!("not-an-array")),
        extra,
        unknown,
    ] {
        assert!(update(&repo, &invalid).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
    for key in ["revision", "contentHash", "page", "scopes"] {
        let mut invalid = payload(&current, json!([]));
        invalid.as_object_mut().unwrap().remove(key);
        let error = update(&repo, &invalid).unwrap_err();
        if ["revision", "contentHash"].contains(&key) {
            assert_eq!(error.status, 409);
        }
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
    let mut invalid = payload(&current, json!([]));
    invalid["contentHash"] = json!("not-a-hash");
    assert!(update(&repo, &invalid).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    let mut invalid = super::super::config::read(&path).unwrap();
    invalid["contentHash"] = current["contentHash"].clone();
    std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(view(&repo).is_err());
}
