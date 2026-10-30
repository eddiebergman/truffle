use super::super::terminals::{fake::Fake, Terminals};
use super::*;
use crate::backend::uuid;
struct DemoCleanup<'a> {
    terminals: &'a Terminals,
    id: String,
    generation: String,
}
impl Drop for DemoCleanup<'_> {
    fn drop(&mut self) {
        let _ = self.terminals.close(&self.id, &self.generation);
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
        "terminal agent",
        "terminal conversation",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
        let fake = Fake::install(&mut repo);
        let id = uuid();
        repo.transaction(|s|{s["sessions"]=json!([{"id":id,"agent":"codex","sessionId":null,"launchNew":false,"cwd":"/invalid","destination":"embedded","items":[]}]);Ok(())}).unwrap();
        let mut row =
            json!({"recordId":id,"agent":"codex","sessionId":null,"generation":uuid(),"live":true});
        match changed {
            "unavailable" => fake.fail("Terminal daemon did not respond."),
            "terminal agent" => row["agent"] = json!("claude"),
            "terminal conversation" => row["sessionId"] = json!(uuid()),
            _ => {
                let during = repo.clone();
                fake.before_read(move || {
                    during
                        .transaction(|s| {
                            match changed {
                                "deleted" => s["sessions"] = json!([]),
                                "sessionId" => s["sessions"][0][changed] = json!(uuid()),
                                "agent" => s["sessions"][0][changed] = json!("claude"),
                                _ => s["sessions"][0][changed] = json!("desktop"),
                            }
                            Ok(())
                        })
                        .unwrap();
                });
            }
        }
        if changed != "unavailable" {
            fake.rows(vec![row]);
        }
        let receipt = open_chat(&repo, &json!({"recordId":id})).unwrap();
        assert_eq!(fake.opened(), 0, "{changed}");
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
    repo.terminals.open_demo(&record).unwrap();
    let generation = text(&repo.terminals.inventory().unwrap().rows()[0], "generation").to_owned();
    let _cleanup = DemoCleanup {
        terminals: &repo.terminals,
        id: id.clone(),
        generation: generation.clone(),
    };
    let result = (|| -> Result<()> {
        let before = std::fs::read(&repo.data)?;
        let cooldown = repo.cooldown.lock().unwrap().clone();
        for _ in 0..2 {
            let receipt = open_chat(&repo, &json!({"recordId":id}))?;
            assert_eq!(receipt["status"], "accepted");
            assert_eq!(receipt["terminal"]["state"], "observed");
            assert_eq!(receipt["terminal"]["generation"], generation);
            assert_eq!(receipt["localSaved"], true);
        }
        assert_eq!(std::fs::read(&repo.data)?, before);
        assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
        assert!(!repo.data.parent().unwrap().join("contexts").exists());
        assert_eq!(
            text(&repo.terminals.inventory().unwrap().rows()[0], "generation"),
            generation
        );
        repo.transaction(|s| {
            s["sessions"][0]["destination"] = json!("desktop");
            Ok(())
        })?;
        assert!(open_chat(&repo, &json!({"recordId":id})).is_err());
        Ok(())
    })();
    let close = repo.terminals.close(&id, &generation);
    result.unwrap();
    close.unwrap();
    let error = open_chat(&repo, &json!({"recordId":id,"target":"embedded"})).unwrap_err();
    assert!(error
        .message
        .contains("No live terminal could be confirmed"));
}
