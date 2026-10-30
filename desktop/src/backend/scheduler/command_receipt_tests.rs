use super::super::terminal::OpenOutcome;
use super::super::terminals::fake::Fake;
use super::*;
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, Arc<Fake>, Value) {
    let temp = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let fake = Fake::install(&mut repo);
    std::fs::write(temp.path().join("SKILL.md"), "Fixture skill").unwrap();
    let preset = save_preset(&repo, &json!({"title":"Fixture","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex"})).unwrap()["preset"].clone();
    (temp, repo, fake, preset)
}
#[test]
fn skill_launch_receipt_distinguishes_started_reused_and_missing_presets() {
    let (_temp, repo, fake, preset) = fixture();
    let run = uuid();
    let launched = execute(&repo, text(&preset, "id"), false, Some(run.clone())).unwrap();
    assert_eq!(fake.opened(), 1);
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
    fake.live(&launched["id"], &uuid());
    let reused = execute(&repo, text(&preset, "id"), false, None).unwrap();
    assert_eq!(
        fake.opened(),
        1,
        "an active run must not launch another agent"
    );
    assert_eq!(reused["runReceipt"]["status"], "reused");
    assert_eq!(reused["runReceipt"]["runId"], run);
    assert_eq!(reused["runReceipt"]["recordId"], launched["id"]);
    repo.cooldown.lock().unwrap().clear();
    let before = repo.state.lock().unwrap().clone();
    assert!(execute(&repo, "removed-preset", false, None).is_err());
    assert_eq!(fake.opened(), 1);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn skill_launch_with_unknown_terminal_state_reports_the_daemon_error() {
    let (_temp, repo, fake, preset) = fixture();
    fake.fail("Terminal daemon is unavailable.");
    let before = repo.state.lock().unwrap().clone();
    let error = execute(&repo, text(&preset, "id"), false, None).unwrap_err();
    assert_eq!(error.message, "Terminal daemon is unavailable.");
    assert_eq!(fake.opened(), 0);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn skill_launch_unknown_and_accepted_metadata_failure_are_not_retried_or_misreported() {
    for outcome in [OpenOutcome::Unknown, OpenOutcome::Accepted] {
        let unknown = outcome == OpenOutcome::Unknown;
        let (_temp, repo, fake, preset) = fixture();
        let during = repo.clone();
        fake.on_open(move |_, _| {
            // Losing the chat record makes the open path's metadata save fail.
            during.transaction(|s| {
                s["sessions"] = json!([]);
                Ok(())
            })?;
            Ok(outcome)
        });
        let result = execute(&repo, text(&preset, "id"), false, None).unwrap();
        assert_eq!(fake.opened(), 1);
        let status = if unknown { "unknown" } else { "launched" };
        assert_eq!(result["runReceipt"]["status"], status);
        assert_eq!(result["runReceipt"]["openReceipt"]["localSaved"], false);
        assert_eq!(
            repo.state.lock().unwrap()["skillRuns"][0]["outcome"],
            status
        );
    }
}
#[test]
fn skill_launch_receipt_projects_only_public_open_fields() {
    let receipt = public_open_receipt(
        &json!({"status":"accepted","recordId":"id","target":"embedded","localSaved":false,"terminal":{"state":"acknowledged","private":"private-terminal-marker"},"private":"private-open-marker"}),
    );
    assert_eq!(
        receipt,
        json!({"status":"accepted","recordId":"id","target":"embedded","localSaved":false,"terminal":{"state":"acknowledged"}})
    );
}
#[test]
fn skill_launch_postack_run_save_failure_preserves_receipt_and_completion_race() {
    for fail in [false, true] {
        let (_temp, repo, fake, preset) = fixture();
        let during = repo.clone();
        fake.on_open(move |_, _| {
            if fail {
                std::fs::create_dir(during.data.with_extension("tmp")).unwrap();
            } else {
                during
                    .transaction(|s| {
                        s["skillRuns"][0]["outcome"] = json!("completed");
                        Ok(())
                    })
                    .unwrap();
            }
            Ok(OpenOutcome::Accepted)
        });
        let result = execute(&repo, text(&preset, "id"), false, None).unwrap();
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
    let (_temp, repo, fake, preset) = fixture();
    let run = uuid();
    fake.on_open(|_, _| Err(Error::new("Fixture launch failed")));
    let error = execute(&repo, text(&preset, "id"), false, Some(run.clone())).unwrap_err();
    let rows = arr(&repo.state.lock().unwrap()["skillRuns"]);
    let stored = rows.iter().find(|row| row["id"] == run).unwrap();
    assert_eq!(stored["outcome"], "failed");
    assert_eq!(
        error.data["runReceipt"],
        json!({"presetId":preset["id"],"recordId":stored["sessionId"],"runId":run,"status":"failed"})
    );
}
