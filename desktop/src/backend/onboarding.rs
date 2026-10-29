use super::{arr, json, text, Error, PathBuf, Repository, Result, Value};
use std::os::unix::fs::PermissionsExt;

pub const STEPS: &[&str] = &[
    "assistant",
    "theme",
    "editor",
    "keymap",
    "integrations",
    "skills",
    "review",
    "company",
    "finish",
];

pub(super) fn detected(name: &str) -> bool {
    let path = PathBuf::from(super::terminal::executable(name));
    path.is_absolute()
        && path
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
fn valid_progress(value: &Value) -> bool {
    value["version"] == 1
        && value["revision"]
            .as_u64()
            .is_some_and(|revision| revision < u64::MAX)
        && value["step"]
            .as_str()
            .is_some_and(|step| STEPS.contains(&step))
        && value["completed"].is_boolean()
        && value["steps"].as_object().is_some_and(|steps| {
            steps.iter().all(|(step, status)| {
                STEPS.contains(&step.as_str())
                    && status
                        .as_str()
                        .is_some_and(|status| ["complete", "skipped", "pending"].contains(&status))
            })
        })
}
fn unsupported_progress(value: &Value) -> Option<&'static str> {
    value.get("version").filter(|version| **version != json!(1)).map(|_| {
        "This setup progress uses an unsupported version. Open it with a compatible app. Saved progress has not changed."
    })
}
fn progress(value: &Value) -> Value {
    if valid_progress(value) {
        value.clone()
    } else {
        json!({"version":1,"revision":0,"step":"assistant","steps":{},"completed":false})
    }
}
fn public_progress(value: &Value) -> Value {
    let stored = progress(value);
    let mut steps = json!({});
    for step in STEPS {
        if stored["steps"][*step]
            .as_str()
            .is_some_and(|status| ["complete", "skipped", "pending"].contains(&status))
        {
            steps[*step] = stored["steps"][*step].clone();
        }
    }
    json!({"version":stored["version"],"revision":stored["revision"],"step":stored["step"],"completed":stored["completed"],"steps":steps})
}
fn public_last_success(value: &Value) -> Value {
    if value
        .as_f64()
        .is_some_and(|value| value.is_finite() && value >= 0.0)
    {
        value.clone()
    } else {
        Value::Null
    }
}
pub fn view(repo: &Repository) -> Value {
    let state = repo.state.lock().unwrap();
    let connections = ["linear", "github", "sentry"].iter().map(|name| {
        let source = &state["sources"][*name];
        json!({"id":name,"disabled":source["disabled"]==true,"lastSuccess":public_last_success(&source["lastSuccess"]),"hasError":source["error"].is_string()})
    }).collect::<Vec<_>>();
    let stored = &state["onboarding"];
    let progress_error = unsupported_progress(stored).or_else(|| {
        (!stored.is_null() && !valid_progress(stored)).then_some(
            "Saved setup progress is invalid. Continuing restarts setup and retains the original progress for recovery.",
        )
    });
    let progress = public_progress(stored);
    let progress_read_only = unsupported_progress(stored).is_some();
    drop(state);
    json!({"progress":progress,"progressReadOnly":progress_read_only,"progressError":progress_error,"assistants":[{"id":"claude","label":"Claude","installed":detected("claude")},{"id":"codex","label":"Codex","installed":detected("codex")}],"editors":[{"id":"zed","label":"Zed","installed":detected("zed"),"diffSupported":true},{"id":"code","label":"VS Code","installed":detected("code"),"diffSupported":false},{"id":"cursor","label":"Cursor","installed":detected("cursor"),"diffSupported":false}],"connections":connections,"credentialBrokerAvailable":false})
}
/// Read setup metadata without retrieving credentials.
pub fn assistant_context(repo: &Repository) -> Value {
    let document = view(repo);
    let mut progress = document["progress"].clone();
    progress.as_object_mut().unwrap().remove("version");
    let assistants = ["claude", "codex"]
        .iter()
        .map(|id| {
            let installed = arr(&document["assistants"])
                .iter()
                .any(|item| item["id"] == *id && item["installed"] == true);
            json!({"id":id,"installed":installed})
        })
        .collect::<Vec<_>>();
    let editors = ["zed", "code", "cursor"]
        .iter()
        .map(|id| {
            let installed = arr(&document["editors"])
                .iter()
                .any(|item| item["id"] == *id && item["installed"] == true);
            json!({"id":id,"installed":installed,"diffSupported":*id=="zed"})
        })
        .collect::<Vec<_>>();
    let metadata = super::connections::status(repo).ok();
    let connections=["linear","github","sentry"].iter().map(|id| {
        let source=arr(&document["connections"]).into_iter().find(|item|item["id"]==*id).unwrap_or(Value::Null);
        let mut result=json!({"id":id,"syncEnabled":source["disabled"]==false,"hasError":source["hasError"]==true});
        if let Some(metadata)=&metadata {
            result["savedInApp"]=json!(arr(&metadata["providers"]).iter().any(|item|item["id"]==*id&&item["configured"]==true));
        }
        if source["lastSuccess"].as_f64().is_some_and(|value|value.is_finite()&&value>=0.0) {
            result["lastSuccess"]=source["lastSuccess"].clone();
        }
        result
    }).collect::<Vec<_>>();
    json!({"progress":progress,"progressAvailable":document["progressError"].is_null(),"assistants":assistants,"editors":editors,
        "credentialMetadata":if metadata.is_some(){"available"}else{"unavailable"},"connections":connections})
}

fn advance(previous: &Value, payload: &Value) -> Result<Value> {
    if let Some(message) = unsupported_progress(previous) {
        return Err(Error::new(message));
    }
    if payload.as_object().is_none_or(|p| {
        p.keys()
            .any(|k| !["revision", "step", "status", "reset"].contains(&k.as_str()))
    }) {
        return Err(Error::new("Unknown onboarding field."));
    }
    let step = text(payload, "step");
    let status = text(payload, "status");
    if payload["reset"] != true
        && (!STEPS.contains(&step)
            || !["current", "complete", "skipped", "pending"].contains(&status))
    {
        return Err(Error::new("Invalid onboarding step or status."));
    }
    let mut next = progress(previous);
    if payload["revision"].as_u64() != next["revision"].as_u64() {
        return Err(Error::new("Setup changed in another window. Reopen setup."));
    }
    let revision = next["revision"].as_u64().unwrap_or(0) + 1;
    if payload["reset"] == true {
        next = progress(&Value::Null);
    } else {
        next["step"] = json!(step);
        if status != "current" {
            next["steps"][step] = json!(status);
        }
        if step == "finish" && status == "complete" {
            next["completed"] = json!(true);
        }
    }
    next["revision"] = json!(revision);
    Ok(next)
}
fn apply_update(state: &mut Value, payload: &Value) -> Result<Value> {
    let previous = &state["onboarding"];
    let next = advance(previous, payload)?;
    if !previous.is_null() && !valid_progress(previous) {
        if previous.to_string().len() > 65_536 {
            return Err(Error::new("Invalid setup progress exceeds the recovery limit. Saved progress has not changed."));
        }
        let recovery = json!({"version":1,"original":previous});
        state["onboardingRecovery"] = recovery;
    }
    state["onboarding"] = next.clone();
    Ok(next)
}
pub fn update(repo: &Repository, payload: &Value) -> Result<Value> {
    repo.transaction(|state| apply_update(state, payload))
}

#[cfg(test)]
mod tests;
