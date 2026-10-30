use super::*;
use crate::backend::operations::Caller;
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, Value) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("SKILL.md"), "Fixture skill").unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let created=save_preset(&repo,&json!({"title":"Editor fixture","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex"})).unwrap();
    (temp, repo, created)
}
fn tokens(view: &Value, patch: &Value, schedule: bool) -> Value {
    let mut expected = json!({});
    for field in patch.as_object().unwrap().keys() {
        expected[field] = view[if schedule {
            "scheduleExpected"
        } else {
            "expected"
        }][field]
            .clone();
    }
    expected
}
fn patch(repo: &Repository, view: &Value, patch: &Value) -> Result<Value> {
    update_preset(
        repo,
        &json!({"presetId":view["presetId"],"expected":tokens(view,patch,false),"patch":patch}),
    )
}
fn schedule(repo: &Repository, view: &Value, patch: &Value) -> Result<Value> {
    save_schedule(
        repo,
        &json!({"presetId":view["presetId"],"expected":tokens(view,patch,true),"patch":patch}),
    )
}
fn read(repo: &Repository, id: &Value) -> Value {
    preset_read(repo, &json!({"presetId":id})).unwrap()
}
#[test]
fn skills_editor_list_redacts_complete_known_values_before_display_cap() {
    let temp = tempfile::tempdir().unwrap();
    let secret = format!("sensitive-prefix-{}", "x".repeat(700));
    std::fs::write(
        temp.path().join(".env"),
        format!("TRUFFLE_TEST_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|s| {
        s["skillRuns"] =
            json!([{"id":uuid(),"message":format!("{secret} trailing"),"outcome":"failed"}]);
        Ok(())
    })
    .unwrap();
    let result = super::super::commands::invoke(
        &repo,
        &json!({"name":"skills.list","args":{}}),
        Caller::Assistant,
    )
    .unwrap();
    assert!(!result.to_string().contains("sensitive-prefix-"));
    assert!(result["runs"][0]["message"]
        .as_str()
        .unwrap()
        .contains("redacted"));
}
#[test]
fn skills_editor_list_whitelists_presets_current_schedules_and_bounded_runs_without_writes() {
    let (_temp, repo, created) = fixture();
    let id = text(&created, "presetId");
    repo.transaction(|s|{s["skillPresets"][0]["privateNs"]=json!({"canary":"private-list-canary"});s["skillSchedules"][id]=json!({"presetId":id,"mode":"interval","everyMinutes":5,"enabled":false,"nextRun":10,"updatedAt":1,"privateNs":"private-list-canary"});s["skillSchedules"]["removed-preset"]=json!({"privateNs":"private-list-canary"});s["skillRuns"]=json!((0..110).map(|index|json!({"id":format!("run-{index}"),"presetId":id,"sessionId":uuid(),"at":1,"outcome":"completed","message":"Completed","privateNs":"private-list-canary","openReceipt":{"private":"private-list-canary"}})).collect::<Vec<_>>());Ok(())}).unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    let result = super::super::commands::invoke(
        &repo,
        &json!({"name":"skills.list","args":{}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(arr(&result["presets"]).len(), 1);
    assert_eq!(arr(&result["runs"]).len(), 100);
    assert_eq!(result["schedules"].as_object().unwrap().len(), 1);
    assert_eq!(result["schedules"][id]["nextRun"], 10.0);
    assert!(!result.to_string().contains("private-list-canary"));
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn skills_editor_preset_field_updates_commute_preserve_raw_metadata_and_reject_stale_fields() {
    let (_temp, repo, created) = fixture();
    let id = &created["presetId"];
    repo.transaction(|s| {
        s["skillPresets"][0]["reports"] = json!("historic-reports");
        s["skillPresets"][0]["provenance"] = json!({"keep":true});
        s["skillPresets"][0]["privateNs"] = json!("private-editor-marker");
        Ok(())
    })
    .unwrap();
    let baseline = read(&repo, id);
    let bytes = std::fs::read(&repo.data).unwrap();
    let before = repo.state.lock().unwrap().clone();
    assert_eq!(baseline["origin"]["type"], "custom");
    assert!(!baseline.to_string().contains("historic-reports"));
    assert!(!baseline.to_string().contains("private-editor-marker"));
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(*repo.state.lock().unwrap(), before);
    patch(&repo, &baseline, &json!({"title":"New title"})).unwrap();
    let result = patch(
        &repo,
        &baseline,
        &json!({"description":"Independent description"}),
    )
    .unwrap();
    assert_eq!(result["preset"]["title"], "New title");
    assert_eq!(result["preset"]["description"], "Independent description");
    let bytes = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        patch(&repo, &baseline, &json!({"title":"Stale title"}))
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    let s = repo.state.lock().unwrap();
    assert_eq!(s["skillPresets"][0]["reports"], "historic-reports");
    assert_eq!(s["skillPresets"][0]["provenance"]["keep"], true);
    assert_eq!(s["skillPresets"][0]["privateNs"], "private-editor-marker");
}
#[test]
fn skills_editor_configured_materialization_preserves_relative_paths_and_unknown_fields() {
    let (temp, repo, _) = fixture();
    let path = repo.data.parent().unwrap().join("config/skills.json");
    let raw = json!({"id":"configured-fixture","title":"Configured","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex","reports":"historic-reports","privateNs":{"origin":"keep"}});
    std::fs::write(
        path,
        serde_json::to_vec(&json!({"version":1,"presets":[raw]})).unwrap(),
    )
    .unwrap();
    let baseline = read(&repo, &json!("configured-fixture"));
    assert_eq!(baseline["origin"]["type"], "configured");
    patch(&repo, &baseline, &json!({"title":"Edited configured"})).unwrap();
    let stored = arr(&repo.state.lock().unwrap()["skillPresets"])
        .into_iter()
        .find(|row| row["id"] == "configured-fixture")
        .unwrap();
    assert_eq!(stored["skill"], "SKILL.md");
    assert_eq!(stored["reports"], "historic-reports");
    assert_eq!(stored["privateNs"], raw["privateNs"]);
    assert!(stored.get("available").is_none());
}
#[test]
fn skills_editor_config_hash_is_checked_after_waiting_for_canonical_transaction() {
    let (_temp, repo, created) = fixture();
    let source = skill_presets::load(&repo).unwrap();
    let path = repo.data.parent().unwrap().join("config/skills.json");
    let baseline = created.clone();
    let title_patch = json!({"title":"Must conflict"});
    let args = json!({"presetId":created["presetId"],"expected":tokens(&baseline,&title_patch,false),"patch":title_patch});
    let lock = repo.state.lock().unwrap();
    let before = lock.clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let other = repo.clone();
    let writer = std::thread::spawn(move || {
        tx.send(()).unwrap();
        update_preset_from_source(&other, &args, &source)
    });
    rx.recv().unwrap();
    let mut config = super::super::config::read(&path).unwrap();
    config["privateSourceEpoch"] = json!("changed while queued");
    std::fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    drop(lock);
    assert_eq!(writer.join().unwrap().unwrap_err().status, 409);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn skills_editor_schedule_preserves_claimed_next_run_and_inactive_edits_then_recalculates_on_enable(
) {
    let (_temp, repo, created) = fixture();
    let id = &created["presetId"];
    assert!(repo.state.lock().unwrap()["skillSchedules"][text(&created, "presetId")].is_null());
    schedule(
        &repo,
        &created,
        &json!({"mode":"interval","everyMinutes":5,"enabled":true}),
    )
    .unwrap();
    repo.transaction(|s| {
        s["skillSchedules"][text(&created, "presetId")]["nextRun"] = json!(1);
        s["skillSchedules"][text(&created, "presetId")]["privateNs"] = json!("keep-schedule");
        Ok(())
    })
    .unwrap();
    let baseline = read(&repo, id);
    repo.transaction(|s| {
        claim(s, 1000.0)?;
        Ok(())
    })
    .unwrap();
    let result = schedule(&repo, &baseline, &json!({"missed":"skip"})).unwrap();
    assert_eq!(result["schedule"]["nextRun"], 1300.0);
    assert!(result["schedule"].get("privateNs").is_none());
    let baseline = read(&repo, id);
    schedule(&repo, &baseline, &json!({"time":"10:00"})).unwrap();
    assert_eq!(read(&repo, id)["schedule"]["nextRun"], 1300.0);
    let baseline = read(&repo, id);
    schedule(&repo, &baseline, &json!({"enabled":false})).unwrap();
    let baseline = read(&repo, id);
    schedule(&repo, &baseline, &json!({"everyMinutes":10})).unwrap();
    assert_eq!(read(&repo, id)["schedule"]["nextRun"], 1300.0);
    let baseline = read(&repo, id);
    let before = now();
    schedule(&repo, &baseline, &json!({"enabled":true})).unwrap();
    let next = read(&repo, id)["schedule"]["nextRun"].as_f64().unwrap();
    assert!(next >= before + 600.0 && next <= now() + 600.0);
    assert_eq!(
        repo.state.lock().unwrap()["skillSchedules"][text(&created, "presetId")]["privateNs"],
        "keep-schedule"
    );
    let baseline = read(&repo, id);
    let before = now();
    schedule(&repo, &baseline, &json!({"everyMinutes":15})).unwrap();
    let next = read(&repo, id)["schedule"]["nextRun"].as_f64().unwrap();
    assert!(next >= before + 900.0 && next <= now() + 900.0);
    let baseline = read(&repo, id);
    let bytes = std::fs::read(&repo.data).unwrap();
    schedule(&repo, &baseline, &json!({"missed":"skip"})).unwrap();
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    patch(
        &repo,
        &baseline,
        &json!({"title":"Renamed without reschedule"}),
    )
    .unwrap();
    assert_eq!(read(&repo, id)["schedule"]["nextRun"], next);
}
#[test]
fn skills_editor_schedule_rejects_stale_fields_absence_and_changed_executable() {
    let (_temp, repo, created) = fixture();
    schedule(&repo, &created, &json!({"enabled":false})).unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        schedule(&repo, &created, &json!({"enabled":true}))
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    let baseline = read(&repo, &created["presetId"]);
    patch(&repo, &baseline, &json!({"command":"$new-command"})).unwrap();
    assert_eq!(
        schedule(&repo, &baseline, &json!({"enabled":true}))
            .unwrap_err()
            .status,
        409
    );
}
#[test]
fn skills_editor_projection_identity_is_rejected_before_any_command_write() {
    let temp = tempfile::tempdir().unwrap();
    let secret = "private-skill-identity";
    std::fs::write(
        temp.path().join(".env"),
        format!("TRUFFLE_TEST_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let path = repo.data.parent().unwrap().join("config/skills.json");
    let row = json!({"id":secret,"title":"Configured","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex","reports":"historic"});
    std::fs::write(
        path,
        serde_json::to_vec(&json!({"version":1,"presets":[row]})).unwrap(),
    )
    .unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    let source = skill_presets::load(&repo).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let scratch = Repository::open(scratch.path().into(), None, true).unwrap();
    let preset = Preset {
        row: &row,
        custom: false,
        source: &source,
    };
    let expected = field_edit::tokens(&scratch, &preset).unwrap()["title"].clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let before = repo.state.lock().unwrap().clone();
    for (name, args) in [
        ("skills.preset.read", json!({"presetId":secret})),
        (
            "skills.preset.update",
            json!({"presetId":secret,"patch":{"title":"New"},"expected":{"title":expected}}),
        ),
    ] {
        let error = super::super::commands::invoke(
            &repo,
            &json!({"name":name,"args":args}),
            Caller::Browser,
        )
        .unwrap_err();
        assert!(!error.message.contains(secret));
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
        assert_eq!(*repo.state.lock().unwrap(), before);
    }
}
#[test]
fn skills_editor_validation_and_failed_commit_preserve_state_and_allow_exact_retry() {
    let (temp, repo, created) = fixture();
    let id = &created["presetId"];
    let bytes = std::fs::read(&repo.data).unwrap();
    let state = repo.state.lock().unwrap().clone();
    for change in [
        json!({"title":7}),
        json!({"title":" "}),
        json!({"agent":"other"}),
        json!({"command":"daily"}),
        json!({"skill":"../SKILL.md"}),
        json!({"cwd":"/nonexistent/skill-editor-fixture"}),
    ] {
        assert!(patch(&repo, &created, &change).is_err());
    }
    assert!(save_preset(&repo,&json!({"id":id,"title":"Overwrite","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex"})).is_err());
    assert!(save_schedule(
        &repo,
        &json!({"presetId":id,"mode":"interval","everyMinutes":5,"missed":"latest","enabled":true})
    )
    .is_err());
    assert!(schedule(&repo, &created, &json!({"enabled":"true"})).is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(*repo.state.lock().unwrap(), state);
    let blocking_dir = repo.data.with_extension("tmp");
    std::fs::create_dir(&blocking_dir).unwrap();
    assert!(patch(&repo, &created, &json!({"title":"Retry title"})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::remove_dir(blocking_dir).unwrap();
    assert_eq!(
        patch(&repo, &created, &json!({"title":"Retry title"})).unwrap()["preset"]["title"],
        "Retry title"
    );
}
