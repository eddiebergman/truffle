use super::*;
use crate::backend::commands;
use crate::backend::operations::Caller;
use crate::backend::uuid;
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, String, Vec<String>) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let ids = (0..32).map(|_| uuid()).collect::<Vec<_>>();
    let url = "https://example.invalid/metadata-report";
    let artifact = id(url);
    repo.transaction(|state|{state["sessions"]=json!(ids.iter().map(|id|json!({"id":id,"cwd":temp.path(),"items":[]})).collect::<Vec<_>>());state["artifacts"]=json!([{"id":artifact,"title":"Original title","sessionIds":ids[..31],"url":url,"format":"link","at":42,"snapshot":true,"presetId":"original-owner","role":"skill-output","privateNs":{"keep":"private-metadata-marker"}}]);Ok(())}).unwrap();
    (temp, repo, artifact, ids)
}
fn update(repo: &Repository, read: &Value, patch: &Value) -> Result<Value> {
    let mut expected = json!({});
    for key in patch.as_object().unwrap().keys() {
        expected[key] = read["expected"][key].clone();
    }
    metadata_update(
        repo,
        &json!({"id":read["artifact"]["id"],"patch":patch,"expected":expected}),
    )
}
#[test]
fn artifact_metadata_full_read_is_pure_registered_metadata_and_preserves_more_than_twenty_links() {
    let (_temp, repo, id, ids) = fixture();
    let config = repo.data.parent().unwrap().join("config/skills.json");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(config, b"invalid catalog config").unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let result = metadata_read(&repo, &json!({"id":id})).unwrap();
    assert_eq!(result["origin"], json!({"type":"registered"}));
    assert_eq!(
        result["artifact"]["sessionIds"].as_array().unwrap().len(),
        31
    );
    let mut sorted = ids[..31].to_vec();
    sorted.sort();
    assert_eq!(result["artifact"]["sessionIds"], json!(sorted));
    assert!(!result.to_string().contains("private-metadata-marker"));
    assert!(!result.to_string().contains("example.invalid"));
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}
#[test]
fn artifact_metadata_field_cas_commutes_and_preserves_immutable_data_and_extensions() {
    let (_temp, repo, id, ids) = fixture();
    let baseline = metadata_read(&repo, &json!({"id":id})).unwrap();
    let links = update(&repo, &baseline, &json!({"sessionIds":ids})).unwrap();
    let renamed = update(&repo, &baseline, &json!({"title":"New title"})).unwrap();
    assert_eq!(
        renamed["artifact"]["sessionIds"].as_array().unwrap().len(),
        32
    );
    assert_eq!(renamed["changedFields"], json!(["title"]));
    assert_eq!(
        update(&repo, &baseline, &json!({"title":"Stale title"}))
            .unwrap_err()
            .status,
        409
    );
    let final_links = update(&repo, &links, &json!({"sessionIds":[]})).unwrap();
    assert_eq!(final_links["artifact"]["title"], "New title");
    let artifact = repo.state.lock().unwrap()["artifacts"][0].clone();
    assert_eq!(artifact["presetId"], "original-owner");
    assert_eq!(artifact["role"], "skill-output");
    assert_eq!(artifact["snapshot"], true);
    assert_eq!(artifact["at"], 42);
    assert_eq!(artifact["privateNs"]["keep"], "private-metadata-marker");
    assert_eq!(artifact["url"], "https://example.invalid/metadata-report");
}
#[test]
fn artifact_metadata_conflicts_invalid_patch_and_unchecked_legacy_save_leave_canonical_unchanged() {
    let (_temp, repo, id, ids) = fixture();
    let read = metadata_read(&repo, &json!({"id":id})).unwrap();
    update(&repo, &read, &json!({"sessionIds":[ids[0]]})).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        update(&repo, &read, &json!({"sessionIds":[]}))
            .unwrap_err()
            .status,
        409
    );
    for args in [
        json!({"id":id,"patch":{"title":"New"},"expected":{}}),
        json!({"id":id,"patch":{"title":"New"},"expected":read["expected"]}),
        json!({"id":id,"patch":{"path":"/private/changed"},"expected":{"path":"0".repeat(64)}}),
        json!({"id":id,"patch":{"sessionIds":[uuid()]},"expected":{"sessionIds":metadata_read(&repo,&json!({"id":id})).unwrap()["expected"]["sessionIds"]}}),
    ] {
        assert!(metadata_update(&repo, &args).is_err());
    }
    assert!(repo.edit(&json!({"action":"save-artifact","id":id,"title":"Unchecked","sessionIds":[],"replaceSessions":true})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    repo.edit(&json!({"action":"save-artifact","id":id,"patch":{"title":"Checked legacy"},"expected":{"title":read["expected"]["title"]}})).unwrap();
    assert_eq!(
        metadata_read(&repo, &json!({"id":id})).unwrap()["artifact"]["title"],
        "Checked legacy"
    );
}
fn discovery_fixture() -> (
    tempfile::TempDir,
    Arc<Repository>,
    String,
    std::path::PathBuf,
) {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("skill");
    let dir = cwd.join("reports/2026-10-27");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(cwd.join("SKILL.md"), "Fixture skill").unwrap();
    let path = dir.join("report.md");
    std::fs::write(&path, "Initial report").unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["skillPresets"]=json!([{"id":"fixture-skill","title":"Fixture skill","cwd":cwd,"skill":"SKILL.md","reports":"reports"}]);Ok(())}).unwrap();
    let artifact = id(&path.canonicalize().unwrap().to_string_lossy());
    (temp, repo, artifact, path)
}
#[test]
fn artifact_metadata_discovery_read_does_not_register_and_materialization_checks_current_fingerprint(
) {
    let (_temp, repo, id, path) = discovery_fixture();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let read = metadata_read(&repo, &json!({"id":id})).unwrap();
    assert_eq!(read["origin"]["type"], "discovered");
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert!(update(&repo, &read, &json!({"title":"No silent registration"})).is_err());
    std::fs::write(&path, "Changed report with a different size").unwrap();
    let args = json!({"id":id,"patch":{"sessionIds":[]},"expected":{"sessionIds":read["expected"]["sessionIds"]}});
    assert_eq!(register_discovered(&repo, &args).unwrap_err().status, 409);
    assert_eq!(*repo.state.lock().unwrap(), before);
    let fresh = metadata_read(&repo, &json!({"id":id})).unwrap();
    let result=register_discovered(&repo,&json!({"id":id,"patch":{"title":"Registered title"},"expected":{"title":fresh["expected"]["title"]}})).unwrap();
    assert_eq!(result["materialized"], true);
    assert_eq!(result["origin"]["type"], "registered");
    let stored = repo.state.lock().unwrap()["artifacts"][0].clone();
    assert_eq!(stored["presetId"], "fixture-skill");
    assert_eq!(stored["role"], "skill-output");
    for key in ["available", "size", "discovered"] {
        assert!(stored.get(key).is_none());
    }
    assert_eq!(stored["path"], json!(path.canonicalize().unwrap()));
}
#[test]
fn artifact_metadata_failed_save_and_reattachment_preserve_extensions_and_latest_metadata() {
    let (_temp, repo, id, ids) = fixture();
    let read = metadata_read(&repo, &json!({"id":id})).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    assert!(update(&repo, &read, &json!({"title":"Not committed"})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
    update(&repo, &read, &json!({"title":"Current title"})).unwrap();
    attach_to_chat(
        &repo,
        &ids[31],
        &json!({"url":"https://example.invalid/metadata-report"}),
    )
    .unwrap();
    let state = repo.state.lock().unwrap();
    assert_eq!(state["artifacts"][0]["title"], "Current title");
    assert_eq!(
        state["artifacts"][0]["privateNs"]["keep"],
        "private-metadata-marker"
    );
    assert_eq!(state["artifacts"][0]["at"], 42);
    assert_eq!(
        state["artifacts"][0]["sessionIds"]
            .as_array()
            .unwrap()
            .len(),
        32
    );
}
#[test]
fn artifact_metadata_actionable_ids_and_tokens_are_exact_or_fail_before_any_write() {
    for mode in ["artifact-id", "session-id", "field-token"] {
        let temp = tempfile::tempdir().unwrap();
        let artifact = id("https://example.invalid/safe");
        let session = uuid();
        let values = json!({"title":"Original","sessionIds":[session]});
        let token = metadata_token(&values, "title", None).unwrap();
        let secret = match mode {
            "artifact-id" => artifact.as_str(),
            "session-id" => session.as_str(),
            _ => token.as_str(),
        };
        std::fs::write(
            temp.path().join(".env"),
            format!("TRUFFLE_TEST_SECRET={secret}\n"),
        )
        .unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|state|{state["sessions"]=json!([{"id":session}]);state["artifacts"]=json!([{"id":artifact,"title":"Original","sessionIds":[session],"url":"https://example.invalid/safe"}]);Ok(())}).unwrap();
        let before = repo.state.lock().unwrap().clone();
        let bytes = std::fs::read(&repo.data).unwrap();
        let error = commands::invoke(
            &repo,
            &json!({"name":"artifacts.metadata.read","args":{"id":artifact}}),
            Caller::Assistant,
        )
        .unwrap_err();
        assert!(!error.message.contains(secret));
        assert!(commands::invoke(&repo,&json!({"name":"artifacts.metadata.update","args":{"id":artifact,"patch":{"title":"Changed"},"expected":{"title":token}}}),Caller::Assistant).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
}

#[test]
fn artifact_metadata_duplicate_no_id_registration_is_additive_and_cannot_rename_or_reassign_provenance(
) {
    let (_temp, repo, id, ids) = fixture();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    for extra in [
        json!({"title":"Different title"}),
        json!({"presetId":"other-owner"}),
    ] {
        let mut input = json!({"url":"https://example.invalid/metadata-report","sessionIds":[ids[31]],"replaceSessions":true});
        for (key, value) in extra.as_object().unwrap() {
            input[key] = value.clone();
        }
        let error = repo
            .transaction(|state| save(&repo, state, &input))
            .unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(error.data, json!({"artifactId":id}));
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
    let result=repo.transaction(|state|save(&repo,state,&json!({"url":"https://example.invalid/metadata-report","title":"Original title","presetId":"original-owner","sessionIds":[ids[31]],"replaceSessions":true}))).unwrap();
    assert_eq!(result["sessionIds"].as_array().unwrap().len(), 32);
    assert_eq!(result["title"], "Original title");
    assert_eq!(result["presetId"], "original-owner");
    assert_eq!(result["role"], "skill-output");
    assert_eq!(result["at"], 42);
    assert_eq!(result["privateNs"]["keep"], "private-metadata-marker");
}
#[test]
fn artifact_metadata_discovery_configuration_revision_invalidates_old_tokens_without_materializing()
{
    let (_temp, repo, id, _path) = discovery_fixture();
    let read = metadata_read(&repo, &json!({"id":id})).unwrap();
    let path = repo.data.parent().unwrap().join("config/skills.json");
    let mut config = super::super::config::read(&path).unwrap();
    config["privateRevisionCanary"] = json!("changed configuration");
    std::fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let result = register_discovered(
        &repo,
        &json!({"id":id,"patch":{"sessionIds":[]},"expected":{"sessionIds":read["expected"]["sessionIds"]}}),
    );
    assert_eq!(result.unwrap_err().status, 409);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}

#[test]
fn artifact_metadata_complete_link_limit_applies_to_incremental_reattachment() {
    let (_temp, repo, id, _) = fixture();
    let ids = (0..101).map(|_| uuid()).collect::<Vec<_>>();
    repo.transaction(|state| {
        state["sessions"] = json!(ids.iter().map(|id| json!({"id":id})).collect::<Vec<_>>());
        state["artifacts"][0]["sessionIds"] = json!(ids[..100]);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        metadata_read(&repo, &json!({"id":id})).unwrap()["artifact"]["sessionIds"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert!(attach_to_chat(
        &repo,
        &ids[100],
        &json!({"url":"https://example.invalid/metadata-report"})
    )
    .is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
