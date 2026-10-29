use super::*;
struct DemoCleanup {
    host: Arc<super::super::terminal_daemon::Host>,
    id: String,
    generation: String,
}
impl Drop for DemoCleanup {
    fn drop(&mut self) {
        let _ = self.host.close(&self.id, &self.generation);
    }
}
#[test]
fn saved_chat_open_observation_rechecks_canonical_identity_and_keeps_uncertainty_read_only() {
    for changed in [
        "sessionId",
        "agent",
        "destination",
        "deleted",
        "unavailable",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
        let id = uuid();
        repo.transaction(|s|{s["sessions"]=json!([{"id":id,"agent":"codex","sessionId":null,"launchNew":false,"cwd":"/invalid","destination":"embedded","items":[]}]);Ok(())}).unwrap();
        let receipt = open_chat_observing(
            &repo,
            &json!({"recordId":id}),
            |_, _| panic!("Changed or uncertain runtime must never launch"),
            |_| {
                if changed == "unavailable" {
                    return terminal::RuntimeObservation::Unavailable;
                }
                repo.transaction(|s| {
                    match changed {
                        "deleted" => s["sessions"] = json!([]),
                        "sessionId" => s["sessions"][0][changed] = json!(uuid()),
                        "agent" => s["sessions"][0][changed] = json!("claude"),
                        _ => s["sessions"][0][changed] = json!("desktop"),
                    }
                    Ok(())
                })
                .unwrap();
                terminal::RuntimeObservation::Observed(uuid())
            },
        )
        .unwrap();
        assert_eq!(receipt["status"], "unknown", "{changed}");
        assert_eq!(receipt["terminal"]["state"], "unconfirmed");
        assert!(!repo.data.parent().unwrap().join("contexts").exists());
        assert!(repo.cooldown.lock().unwrap().is_empty());
        assert!(repo.opening.lock().unwrap().is_empty());
    }
}
#[test]
fn saved_chat_open_observes_unregistered_live_terminal_without_launch_packet_or_cooldown() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let id = uuid();
    let record = json!({"id":id,"agent":"codex","sessionId":null,"launchNew":false,"title":"Live unregistered","cwd":"/no-longer-existing-folder","destination":"embedded","items":[]});
    repo.transaction(|s| {
        s["sessions"] = json!([record]);
        Ok(())
    })
    .unwrap();
    repo.embedded.open_demo(&record).unwrap();
    let generation = text(&repo.embedded.inventory()[0], "generation").to_owned();
    let _cleanup = DemoCleanup {
        host: repo.embedded.clone(),
        id: id.clone(),
        generation: generation.clone(),
    };
    let result = (|| -> Result<()> {
        let before = std::fs::read(&repo.data)?;
        let cooldown = repo.cooldown.lock().unwrap().clone();
        for _ in 0..2 {
            let receipt = open_chat_with(&repo, &json!({"recordId":id}), |_, _| {
                panic!("Observational reuse must not launch")
            })?;
            assert_eq!(receipt["status"], "accepted");
            assert_eq!(receipt["terminal"]["state"], "observed");
            assert_eq!(receipt["terminal"]["generation"], generation);
            assert_eq!(receipt["localSaved"], true);
        }
        assert_eq!(std::fs::read(&repo.data)?, before);
        assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
        assert!(!repo.data.parent().unwrap().join("contexts").exists());
        assert_eq!(
            text(&repo.embedded.inventory()[0], "generation"),
            generation
        );
        repo.transaction(|s| {
            s["sessions"][0]["destination"] = json!("desktop");
            Ok(())
        })?;
        assert!(
            open_chat_with(&repo, &json!({"recordId":id}), |_, _| panic!(
                "Desktop null ID cannot borrow embedded runtime"
            ))
            .is_err()
        );
        Ok(())
    })();
    let close = repo.embedded.close(&id, &generation);
    result.unwrap();
    close.unwrap();
    assert!(open_chat_with(
        &repo,
        &json!({"recordId":id,"target":"embedded"}),
        |_, _| panic!("Dead runtime must not launch")
    )
    .is_err());
}
