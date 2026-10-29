use super::*;
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, Value) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    std::fs::write(temp.path().join("SKILL.md"), "Fixture skill").unwrap();
    let preset = save_preset(&repo, &json!({"title":"Fixture","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex"})).unwrap()["preset"].clone();
    (temp, repo, preset)
}
#[test]
fn skill_launch_receipt_distinguishes_started_reused_and_missing_presets() {
    let (_temp, repo, preset) = fixture();
    let run = uuid();
    let launched = execute_with(
        &repo,
        text(&preset, "id"),
        false,
        Some(run.clone()),
        &json!({"candidates":[]}),
        |_, _, _| {
            Ok(json!({"status":"accepted","localSaved":true,"terminal":{"state":"acknowledged"}}))
        },
    )
    .unwrap();
    assert_eq!(launched["runReceipt"]["presetId"], preset["id"]);
    assert_eq!(launched["runReceipt"]["recordId"], launched["id"]);
    assert_eq!(launched["runReceipt"]["runId"], run);
    assert_eq!(launched["runReceipt"]["status"], "launched");
    assert_eq!(launched["runReceipt"]["openReceipt"]["status"], "accepted");
    assert_eq!(launched["runReceipt"]["runSaved"], true);
    assert!(arr(&repo.state.lock().unwrap()["sessions"])
        .iter()
        .all(|record| record.get("runReceipt").is_none()));
    repo.cooldown.lock().unwrap().clear();
    let reused = execute_with(
        &repo,
        text(&preset, "id"),
        false,
        None,
        &json!({"candidates":[{"recordId":launched["id"],"live":true,"lifecycle":"active"}]}),
        |_, _, _| panic!("Active run must not launch another agent"),
    )
    .unwrap();
    assert_eq!(reused["runReceipt"]["status"], "reused");
    assert_eq!(reused["runReceipt"]["runId"], run);
    assert_eq!(reused["runReceipt"]["recordId"], launched["id"]);
    repo.cooldown.lock().unwrap().clear();
    let before = repo.state.lock().unwrap().clone();
    assert!(execute_with(
        &repo,
        "removed-preset",
        false,
        None,
        &json!({"candidates":[]}),
        |_, _, _| panic!("Missing preset must not launch")
    )
    .is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn skill_launch_unknown_and_accepted_metadata_failure_are_not_retried_or_misreported() {
    for status in ["unknown", "accepted"] {
        let (_temp, repo, preset) = fixture();
        let mut calls = 0;
        let result=execute_with(&repo,text(&preset,"id"),false,None,&json!({"candidates":[]}),|_,id,_|{calls+=1;Ok(json!({"status":status,"recordId":id,"target":"embedded","localSaved":false,"terminal":{"state":if status=="unknown"{"unconfirmed"}else{"acknowledged"},"private":"private-terminal-marker"},"private":"private-open-marker"}))}).unwrap();
        assert_eq!(calls, 1);
        assert_eq!(
            result["runReceipt"]["status"],
            if status == "unknown" {
                "unknown"
            } else {
                "launched"
            }
        );
        assert_eq!(result["runReceipt"]["openReceipt"]["localSaved"], false);
        assert!(!result["runReceipt"].to_string().contains("private-"));
        assert_eq!(
            repo.state.lock().unwrap()["skillRuns"][0]["outcome"],
            if status == "unknown" {
                "unknown"
            } else {
                "launched"
            }
        );
    }
}
#[test]
fn skill_launch_postack_run_save_failure_preserves_receipt_and_completion_race() {
    for fail in [false, true] {
        let (_temp, repo, preset) = fixture();
        let result=execute_with(&repo,text(&preset,"id"),false,None,&json!({"candidates":[]}),|repo,id,_|{if fail{std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();}else{repo.transaction(|s|{s["skillRuns"][0]["outcome"]=json!("completed");Ok(())}).unwrap();}Ok(json!({"status":"accepted","recordId":id,"localSaved":true,"terminal":{"state":"acknowledged"}}))}).unwrap();
        assert_eq!(result["runReceipt"]["status"], "launched");
        assert_eq!(result["runReceipt"]["runSaved"], !fail);
        assert_eq!(
            repo.state.lock().unwrap()["skillRuns"][0]["outcome"],
            if fail { "starting" } else { "completed" }
        );
    }
}
#[test]
fn failed_skill_launch_error_identifies_the_exact_failed_run() {
    let (_temp, repo, preset) = fixture();
    let run = uuid();
    let error = execute_with(
        &repo,
        text(&preset, "id"),
        false,
        Some(run.clone()),
        &json!({"candidates":[]}),
        |_, _, _| Err(Error::new("Fixture launch failed")),
    )
    .unwrap_err();
    let rows = arr(&repo.state.lock().unwrap()["skillRuns"]);
    let stored = rows.iter().find(|row| row["id"] == run).unwrap();
    assert_eq!(stored["outcome"], "failed");
    assert_eq!(
        error.data["runReceipt"],
        json!({"presetId":preset["id"],"recordId":stored["sessionId"],"runId":run,"status":"failed"})
    );
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}
