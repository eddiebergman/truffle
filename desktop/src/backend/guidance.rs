use super::*;
use std::collections::BTreeSet;

pub(crate) const IDS: &[&str] = &[
    "ci.investigate",
    "review.comment-selection",
    "review.pending-comments",
    "review.agent-scope",
    "chat.shell",
    "assistant.after-setup",
    "chat.snooze",
];

fn progress(value: &Value) -> Value {
    if value["version"] == 1 {
        value.clone()
    } else {
        json!({"version":1,"revision":0,"enabled":true,"tips":{}})
    }
}

fn public_progress(value: &Value) -> Result<Value> {
    if value["version"]!=1 || value["revision"].as_u64().is_none_or(|revision|revision>9_007_199_254_740_991)
        || !value["enabled"].is_boolean() || !value["tips"].is_object() {
        return Err(Error::new("Invalid guidance progress."));
    }
    let mut tips=json!({});
    for id in IDS {
        if value["tips"][*id].as_str().is_some_and(|status|["completed","dismissed"].contains(&status)) {tips[*id]=value["tips"][*id].clone();}
    }
    let mut result=json!({"version":1,"revision":value["revision"],"enabled":value["enabled"],"tips":tips});
    if let Some(replayed)=value["replayed"].as_bool(){result["replayed"]=json!(replayed);}
    Ok(result)
}

pub fn view(repo: &Repository) -> Result<Value> {
    let definitions =
        super::config::read(&repo.data.parent().unwrap().join("config/guidance.json"))?;
    if definitions["version"] != 1 || arr(&definitions["tips"]).len() > IDS.len() {
        return Err(Error::new("Unsupported guidance configuration."));
    }
    let mut seen = BTreeSet::new();
    for tip in arr(&definitions["tips"]) {
        let id = text(&tip, "id");
        if !IDS.contains(&id)
            || !seen.insert(id.to_owned())
            || text(&tip, "text").is_empty()
            || text(&tip, "text").len() > 400
            || text(&tip, "title").is_empty()
            || text(&tip, "title").len() > 80
        {
            return Err(Error::new("Invalid guidance definition."));
        }
    }
    let state = repo.state.lock().map_err(|_| Error::io())?;
    let mut current = public_progress(&progress(&state["guidance"]))?;
    if current["replayed"] != true {
        for session in arr(&state["sessions"]) {
            let draft = &session["reviewDraft"];
            if !arr(&draft["comments"]).is_empty() {
                current["tips"]["review.comment-selection"] = json!("completed");
            }
            if ["reviewing", "complete"].contains(&text(&draft["run"], "state")) {
                current["tips"]["review.agent-scope"] = json!("completed");
            }
            if session["ciInvestigations"].as_object().is_some_and(|m| {
                m.values()
                    .any(|v| ["investigating", "complete"].contains(&text(v, "state")))
            }) {
                current["tips"]["ci.investigate"] = json!("completed");
            }
        }
    }
    let definitions=json!({"version":1,"tips":arr(&definitions["tips"]).iter().map(|tip|json!({"id":tip["id"],"title":tip["title"],"text":tip["text"]})).collect::<Vec<_>>()});
    Ok(
        json!({"progress":current,"definitions":definitions,"setupComplete":state["onboarding"]["completed"]==true,"assistantUsed":!arr(&state["appChat"]["messages"]).is_empty()||!arr(&state["appChat"]["history"]).is_empty(),"demo":repo.demo}),
    )
}

fn advance(previous: &Value, payload: &Value) -> Result<Value> {
    if !previous.is_null() && previous["version"] != 1 {
        return Err(Error::new("Unsupported guidance state version."));
    }
    let mut next = progress(previous);
    public_progress(&next)?;
    if payload["revision"].as_u64().is_none() || payload["revision"] != next["revision"] {
        let mut error=Error::new("Guidance changed in another window. Try again.");error.status=409;return Err(error);
    }
    let fields: &[&str]=match text(payload,"action") {
        "dismiss"|"complete"=>&["revision","action","id"],
        "enabled"=>&["revision","action","enabled"],
        "replay"=>&["revision","action"],
        _=>return Err(Error::new("Unknown guidance action.")),
    };
    if payload.as_object().is_none_or(|object|object.len()!=fields.len() || object.keys().any(|key|!fields.contains(&key.as_str()))) {
        return Err(Error::new("Supply only the fields required by this guidance action."));
    }
    match text(payload, "action") {
        "dismiss" | "complete" => {
            let id = text(payload, "id");
            if !IDS.contains(&id) {
                return Err(Error::new("Unknown guidance tip."));
            }
            if next["tips"][id] != "completed" {
                next["tips"][id] = json!(if payload["action"] == "complete" {
                    "completed"
                } else {
                    "dismissed"
                });
            }
        }
        "enabled" => {
            next["enabled"] = json!(payload["enabled"]
                .as_bool()
                .ok_or(Error::new("Choose whether guidance is enabled."))?);
        }
        "replay" => {
            for id in IDS { next["tips"].as_object_mut().unwrap().remove(*id); }
            next["replayed"] = json!(true);
            next["enabled"] = json!(true);
        }
        _ => return Err(Error::new("Unknown guidance action.")),
    }
    next["revision"] = json!(next["revision"].as_u64().unwrap().checked_add(1).filter(|revision|*revision<=9_007_199_254_740_991).ok_or(Error::new("Guidance revision is exhausted."))?);
    Ok(next)
}

pub fn complete(state: &mut Value, id: &str) {
    if !IDS.contains(&id) {
        return;
    }
    let mut next = progress(&state["guidance"]);
    if next["tips"][id] == "completed" {
        return;
    }
    next["tips"][id] = json!("completed");
    next["revision"] = json!(next["revision"].as_u64().unwrap_or(0) + 1);
    state["guidance"] = next;
}

pub fn update(repo: &Repository, payload: &Value) -> Result<Value> {
    repo.transaction(|state| {
        state["guidance"] = advance(&state["guidance"], payload)?;
        public_progress(&state["guidance"])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_guidance_is_bounded_and_extensions_survive_reads_and_preference_changes() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        repo.transaction(|state| {state["guidance"]=json!({"version":1,"revision":7,"enabled":true,"tips":{"chat.shell":"dismissed","future-tip":"benign-private-tip-extension"},"extension":"benign-private-progress-extension"});Ok(())}).unwrap();
        let path=repo.settings.config_directory().join("guidance.json");let mut definitions=super::super::config::read(&path).unwrap();definitions["extension"]=json!("benign-private-definition-extension");definitions["tips"][0]["extension"]=json!("benign-private-definition-field");std::fs::write(&path,serde_json::to_vec(&definitions).unwrap()).unwrap();
        let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let configuration=std::fs::read(&path).unwrap();
        let public=view(&repo).unwrap();assert!(!public.to_string().contains("benign-private"));assert_eq!(public["definitions"]["tips"].as_array().unwrap().len(),IDS.len());assert!(arr(&public["definitions"]["tips"]).iter().all(|tip|tip.as_object().unwrap().len()==3));
        assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);
        let disabled=update(&repo,&json!({"revision":7,"action":"enabled","enabled":false})).unwrap();assert_eq!(disabled["enabled"],false);assert!(!disabled.to_string().contains("benign-private"));
        let after=repo.state.lock().unwrap().clone();assert_eq!(after["guidance"]["extension"],before["guidance"]["extension"]);assert_eq!(after["guidance"]["tips"]["future-tip"],before["guidance"]["tips"]["future-tip"]);
        let saved=std::fs::read(&repo.data).unwrap();assert_eq!(update(&repo,&json!({"revision":7,"action":"replay"})).unwrap_err().status,409);assert_eq!(std::fs::read(&repo.data).unwrap(),saved);
        let replay=update(&repo,&json!({"revision":8,"action":"replay"})).unwrap();assert_eq!(replay["enabled"],true);assert_eq!(replay["tips"],json!({}));assert_eq!(repo.state.lock().unwrap()["guidance"]["tips"]["future-tip"],"benign-private-tip-extension");
        assert_eq!(std::fs::read(path).unwrap(),configuration);
    }
    #[test]
    fn guidance_action_fields_are_exact_and_conflicts_do_not_mutate_state() {
        let previous=json!({"version":1,"revision":0,"enabled":true,"tips":{}});
        for payload in [json!({"revision":0,"action":"enabled"}),json!({"revision":0,"action":"enabled","enabled":true,"id":"chat.shell"}),json!({"revision":0,"action":"replay","enabled":true}),json!({"revision":0,"action":"dismiss"}),json!({"revision":0,"action":"dismiss","id":"unknown"}),json!({"revision":0,"action":"dismiss","id":"chat.shell","extra":true})] {assert!(advance(&previous,&payload).is_err());}
        assert_eq!(advance(&previous,&json!({"action":"replay"})).unwrap_err().status,409);
        assert_eq!(advance(&previous,&json!({"revision":1,"action":"replay"})).unwrap_err().status,409);
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
}
