use super::*;
#[test]
fn public_guidance_is_bounded_and_extensions_survive_reads_and_preference_changes() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {state["guidance"]=json!({"version":1,"revision":7,"enabled":true,"tips":{"chat.shell":"dismissed","future-tip":"benign-private-tip-extension"},"extension":"benign-private-progress-extension"});Ok(())}).unwrap();
    let path = repo.settings.config_directory().join("guidance.json");
    let mut definitions = super::super::config::read(&path).unwrap();
    definitions["extension"] = json!("benign-private-definition-extension");
    definitions["tips"][0]["extension"] = json!("benign-private-definition-field");
    std::fs::write(&path, serde_json::to_vec(&definitions).unwrap()).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let configuration = std::fs::read(&path).unwrap();
    let public = view(&repo).unwrap();
    assert!(!public.to_string().contains("benign-private"));
    assert_eq!(
        public["definitions"]["tips"].as_array().unwrap().len(),
        IDS.len()
    );
    assert!(arr(&public["definitions"]["tips"])
        .iter()
        .all(|tip| tip.as_object().unwrap().len() == 3));
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    let disabled = update(
        &repo,
        &json!({"revision":7,"action":"enabled","enabled":false}),
    )
    .unwrap();
    assert_eq!(disabled["enabled"], false);
    assert!(!disabled.to_string().contains("benign-private"));
    let after = repo.state.lock().unwrap().clone();
    assert_eq!(
        after["guidance"]["extension"],
        before["guidance"]["extension"]
    );
    assert_eq!(
        after["guidance"]["tips"]["future-tip"],
        before["guidance"]["tips"]["future-tip"]
    );
    let saved = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        update(&repo, &json!({"revision":7,"action":"replay"}))
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), saved);
    let replay = update(&repo, &json!({"revision":8,"action":"replay"})).unwrap();
    assert_eq!(replay["enabled"], true);
    assert_eq!(replay["tips"], json!({}));
    assert_eq!(
        repo.state.lock().unwrap()["guidance"]["tips"]["future-tip"],
        "benign-private-tip-extension"
    );
    assert_eq!(std::fs::read(path).unwrap(), configuration);
}
#[test]
fn guidance_action_fields_are_exact_and_conflicts_do_not_mutate_state() {
    let previous = json!({"version":1,"revision":0,"enabled":true,"tips":{}});
    for payload in [
        json!({"revision":0,"action":"enabled"}),
        json!({"revision":0,"action":"enabled","enabled":true,"id":"chat.shell"}),
        json!({"revision":0,"action":"replay","enabled":true}),
        json!({"revision":0,"action":"dismiss"}),
        json!({"revision":0,"action":"dismiss","id":"unknown"}),
        json!({"revision":0,"action":"dismiss","id":"chat.shell","extra":true}),
    ] {
        assert!(advance(&previous, &payload).is_err());
    }
    assert_eq!(
        advance(&previous, &json!({"action":"replay"}))
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(
        advance(&previous, &json!({"revision":1,"action":"replay"}))
            .unwrap_err()
            .status,
        409
    );
}

#[test]
fn saved_progress_survives_reopen_and_copy_stays_editable() {
    let temp = tempfile::tempdir().unwrap();
    {
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        update(
            &repo,
            &json!({"revision":0,"action":"dismiss","id":"assistant.after-setup"}),
        )
        .unwrap();
        repo.transaction(|s| {
            super::complete(s, "chat.shell");
            Ok(())
        })
        .unwrap();
    }
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let current = view(&repo).unwrap();
    assert_eq!(
        current["progress"]["tips"]["assistant.after-setup"],
        "dismissed"
    );
    assert_eq!(current["progress"]["tips"]["chat.shell"], "completed");
    assert_eq!(current["definitions"]["tips"].as_array().unwrap().len(), 7);
}
#[test]
fn dismissal_completion_and_replay_are_profile_scoped_and_revision_guarded() {
    let dismissed = advance(
        &Value::Null,
        &json!({"revision":0,"action":"dismiss","id":"ci.investigate"}),
    )
    .unwrap();
    assert!(advance(&dismissed, &json!({"revision":0,"action":"replay"})).is_err());
    let done = advance(
        &dismissed,
        &json!({"revision":1,"action":"complete","id":"ci.investigate"}),
    )
    .unwrap();
    assert_eq!(done["tips"]["ci.investigate"], "completed");
    let reset = advance(&done, &json!({"revision":2,"action":"replay"})).unwrap();
    assert_eq!(reset["tips"], json!({}));
    assert_eq!(reset["enabled"], true);
    assert!(advance(
        &reset,
        &json!({"revision":3,"action":"complete","id":"arbitrary"})
    )
    .is_err());
}
