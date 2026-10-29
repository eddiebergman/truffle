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
    if value["version"] != 1
        || value["revision"]
            .as_u64()
            .is_none_or(|revision| revision > 9_007_199_254_740_991)
        || !value["enabled"].is_boolean()
        || !value["tips"].is_object()
    {
        return Err(Error::new("Invalid guidance progress."));
    }
    let mut tips = json!({});
    for id in IDS {
        if value["tips"][*id]
            .as_str()
            .is_some_and(|status| ["completed", "dismissed"].contains(&status))
        {
            tips[*id] = value["tips"][*id].clone();
        }
    }
    let mut result =
        json!({"version":1,"revision":value["revision"],"enabled":value["enabled"],"tips":tips});
    if let Some(replayed) = value["replayed"].as_bool() {
        result["replayed"] = json!(replayed);
    }
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
    let definitions = json!({"version":1,"tips":arr(&definitions["tips"]).iter().map(|tip|json!({"id":tip["id"],"title":tip["title"],"text":tip["text"]})).collect::<Vec<_>>()});
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
        let mut error = Error::new("Guidance changed in another window. Try again.");
        error.status = 409;
        return Err(error);
    }
    let fields: &[&str] = match text(payload, "action") {
        "dismiss" | "complete" => &["revision", "action", "id"],
        "enabled" => &["revision", "action", "enabled"],
        "replay" => &["revision", "action"],
        _ => return Err(Error::new("Unknown guidance action.")),
    };
    if payload.as_object().is_none_or(|object| {
        object.len() != fields.len() || object.keys().any(|key| !fields.contains(&key.as_str()))
    }) {
        return Err(Error::new(
            "Supply only the fields required by this guidance action.",
        ));
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
            for id in IDS {
                next["tips"].as_object_mut().unwrap().remove(*id);
            }
            next["replayed"] = json!(true);
            next["enabled"] = json!(true);
        }
        _ => return Err(Error::new("Unknown guidance action.")),
    }
    next["revision"] = json!(next["revision"]
        .as_u64()
        .unwrap()
        .checked_add(1)
        .filter(|revision| *revision <= 9_007_199_254_740_991)
        .ok_or(Error::new("Guidance revision is exhausted."))?);
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
mod tests;
