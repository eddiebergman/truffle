use super::*;
use crate::backend::uuid;
#[test]
fn public_setup_timestamp_validation_rejects_non_numeric_and_negative_values() {
    for value in [
        Value::Null,
        json!(true),
        json!(-1),
        json!("benign-private-timestamp"),
    ] {
        assert!(public_last_success(&value).is_null());
    }
    assert_eq!(public_last_success(&json!(123.5)), json!(123.5));
    let public = public_progress(
        &json!({"version":1,"revision":1,"step":"company","completed":false,"steps":{"theme":"complete"},"private":"benign-private-progress"}),
    );
    assert!(!public.to_string().contains("benign-private"));
    assert_eq!(public.as_object().unwrap().len(), 5);
}
#[test]
fn assistant_setup_uses_only_public_progress_and_metadata_without_credential_reads() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
            state["onboarding"]=json!({"version":1,"revision":4,"step":"company","completed":false,"steps":{"theme":"complete","company":"pending"},"privateExtension":"benign-private-progress-marker"});
            state["sources"]["linear"]=json!({"disabled":false,"lastSuccess":100,"error":"benign-private-source-error"});
            state["sources"]["github"]=json!({"disabled":true,"lastSuccess":"benign-private-timestamp-marker"});
            Ok(())
        }).unwrap();
    let path = repo.data.parent().unwrap().join("config/connections.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"version":1,"storage":"macos-login-keychain","revision":1,"providers":{"github":{"credential_ref":uuid(),"identity":"benign-private-account-marker","org":"","base":""}}})).unwrap()).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let configuration = std::fs::read(&path).unwrap();
    let public = view(&repo);
    assert!(public["progress"].get("privateExtension").is_none());
    assert!(!public.to_string().contains("benign-private"));
    assert!(arr(&public["connections"])
        .iter()
        .find(|row| row["id"] == "github")
        .unwrap()["lastSuccess"]
        .is_null());
    let summary = assistant_context(&repo);
    assert_eq!(summary["credentialMetadata"], "available");
    assert_eq!(summary["progress"]["step"], "company");
    assert_eq!(summary["progress"]["steps"]["company"], "pending");
    assert!(summary["progress"].get("privateExtension").is_none());
    assert!(!summary.to_string().contains("benign-private"));
    let rows = arr(&summary["connections"]);
    let linear = rows.iter().find(|row| row["id"] == "linear").unwrap();
    let github = rows.iter().find(|row| row["id"] == "github").unwrap();
    assert_eq!(linear["savedInApp"], false);
    assert_eq!(linear["syncEnabled"], true);
    assert_eq!(linear["hasError"], true);
    assert_eq!(linear["lastSuccess"], 100);
    assert_eq!(github["savedInApp"], true);
    assert_eq!(github["syncEnabled"], false);
    assert!(github.get("lastSuccess").is_none());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(std::fs::read(&path).unwrap(), configuration);
    std::fs::write(&path, "benign-private-metadata-failure").unwrap();
    let unavailable = assistant_context(&repo);
    assert_eq!(unavailable["credentialMetadata"], "unavailable");
    assert!(!unavailable.to_string().contains("benign-private"));
    assert!(arr(&unavailable["connections"])
        .iter()
        .all(|row| row.get("savedInApp").is_none()));
    std::fs::remove_file(path).unwrap();
    let legacy = assistant_context(&repo);
    assert_eq!(legacy["credentialMetadata"], "available");
    assert!(arr(&legacy["connections"])
        .iter()
        .all(|row| row["savedInApp"] == false));
}

#[test]
fn unsupported_progress_rejects_updates_and_reset_without_changes() {
    for payload in [
        json!({"revision":4,"step":"theme","status":"complete"}),
        json!({"revision":4,"reset":true}),
    ] {
        let mut state = json!({"onboarding":{"version":2,"revision":4,"future":"keep"}});
        let original = state.clone();
        assert!(apply_update(&mut state, &payload).is_err());
        assert_eq!(state, original);
    }
}
#[test]
fn repairing_invalid_progress_retains_the_original_and_valid_unknown_fields() {
    let original = json!({"version":1,"revision":0,"step":3,"steps":{},"completed":true,"extension":{"keep":true}});
    let mut state = json!({"onboarding":original});
    apply_update(
        &mut state,
        &json!({"revision":0,"step":"assistant","status":"skipped"}),
    )
    .unwrap();
    assert_eq!(state["onboardingRecovery"]["original"], original);
    assert_eq!(state["onboarding"]["step"], "assistant");
    state["onboarding"]["extension"] = json!({"keep":true});
    apply_update(
        &mut state,
        &json!({"revision":1,"step":"theme","status":"current"}),
    )
    .unwrap();
    assert_eq!(state["onboarding"]["extension"]["keep"], true);
    assert_eq!(state["onboardingRecovery"]["original"], original);
}
#[test]
fn oversized_invalid_progress_stays_unchanged() {
    let mut state = json!({"onboarding":{"version":1,"step":3,"data":"x".repeat(65_537)}});
    let original = state.clone();
    assert!(apply_update(&mut state, &json!({"revision":0,"reset":true})).is_err());
    assert_eq!(state, original);
}
#[test]
fn malformed_progress_starts_setup_from_the_first_step() {
    for malformed in [
        json!({"version":1,"revision":0,"step":3,"steps":{},"completed":true}),
        json!({"version":1,"revision":0,"step":"invented","steps":{},"completed":false}),
        json!({"version":1,"revision":-1,"step":"finish","steps":{},"completed":true}),
        json!({"version":1,"revision":0,"step":"finish","steps":[],"completed":true}),
        json!({"version":1,"revision":0,"step":"finish","steps":{"skills":"running"},"completed":true}),
    ] {
        let next = progress(&malformed);
        assert_eq!(next["step"], "assistant");
        assert_eq!(next["revision"], 0);
        assert_eq!(next["completed"], false);
    }
}
#[test]
fn progress_resumes_skipped_and_pending_steps_and_rejects_stale_writes() {
    let skipped = advance(
        &Value::Null,
        &json!({"revision":0,"step":"integrations","status":"skipped"}),
    )
    .unwrap();
    let pending = advance(
        &skipped,
        &json!({"revision":1,"step":"skills","status":"pending"}),
    )
    .unwrap();
    assert_eq!(pending["steps"]["integrations"], "skipped");
    assert_eq!(pending["steps"]["skills"], "pending");
    assert_eq!(pending["step"], "skills");
    assert!(advance(
        &pending,
        &json!({"revision":1,"step":"skills","status":"complete"})
    )
    .is_err());
    let reset = advance(&pending, &json!({"revision":2,"reset":true})).unwrap();
    assert_eq!(reset["step"], "assistant");
    assert_eq!(reset["revision"], 3);
    assert_eq!(reset["completed"], false);
}
#[test]
fn progress_rejects_secret_fields_and_invalid_steps() {
    assert!(advance(
        &Value::Null,
        &json!({"revision":0,"step":"integrations","status":"pending","token":"canary"})
    )
    .is_err());
    assert!(advance(
        &Value::Null,
        &json!({"revision":0,"step":"invented","status":"complete"})
    )
    .is_err());
}
