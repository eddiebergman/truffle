use super::*;
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
                    && status.as_str().is_some_and(|status| {
                        ["complete", "skipped", "pending"].contains(&status)
                    })
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
    let stored=progress(value);
    let mut steps=json!({});
    for step in STEPS {
        if stored["steps"][*step].as_str().is_some_and(|status|["complete","skipped","pending"].contains(&status)) {
            steps[*step]=stored["steps"][*step].clone();
        }
    }
    json!({"version":stored["version"],"revision":stored["revision"],"step":stored["step"],"completed":stored["completed"],"steps":steps})
}
fn public_last_success(value: &Value) -> Value {
    if value.as_f64().is_some_and(|value|value.is_finite()&&value>=0.0) { value.clone() } else { Value::Null }
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
    json!({"progress":public_progress(stored),"progressReadOnly":unsupported_progress(stored).is_some(),"progressError":progress_error,"assistants":[{"id":"claude","label":"Claude","installed":detected("claude")},{"id":"codex","label":"Codex","installed":detected("codex")}],"editors":[{"id":"zed","label":"Zed","installed":detected("zed"),"diffSupported":true},{"id":"code","label":"VS Code","installed":detected("code"),"diffSupported":false},{"id":"cursor","label":"Cursor","installed":detected("cursor"),"diffSupported":false}],"connections":connections,"credentialBrokerAvailable":false})
}
/// Read setup metadata without retrieving credentials.
pub fn assistant_context(repo: &Repository) -> Value {
    let document=view(repo);
    let mut progress=document["progress"].clone();
    progress.as_object_mut().unwrap().remove("version");
    let assistants=["claude","codex"].iter().map(|id| {
        let installed=arr(&document["assistants"]).iter().any(|item|item["id"]==*id&&item["installed"]==true);
        json!({"id":id,"installed":installed})
    }).collect::<Vec<_>>();
    let editors=["zed","code","cursor"].iter().map(|id| {
        let installed=arr(&document["editors"]).iter().any(|item|item["id"]==*id&&item["installed"]==true);
        json!({"id":id,"installed":installed,"diffSupported":*id=="zed"})
    }).collect::<Vec<_>>();
    let metadata=super::connections::status(repo).ok();
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
mod tests {
    use super::*;
    #[test]
    fn public_setup_timestamp_validation_rejects_non_numeric_and_negative_values() {
        for value in [Value::Null,json!(true),json!(-1),json!("benign-private-timestamp")] {assert!(public_last_success(&value).is_null());}
        assert_eq!(public_last_success(&json!(123.5)),json!(123.5));
        let public=public_progress(&json!({"version":1,"revision":1,"step":"company","completed":false,"steps":{"theme":"complete"},"private":"benign-private-progress"}));
        assert!(!public.to_string().contains("benign-private"));assert_eq!(public.as_object().unwrap().len(),5);
    }
    #[test]
    fn assistant_setup_uses_only_public_progress_and_metadata_without_credential_reads() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        repo.transaction(|state| {
            state["onboarding"]=json!({"version":1,"revision":4,"step":"company","completed":false,"steps":{"theme":"complete","company":"pending"},"privateExtension":"benign-private-progress-marker"});
            state["sources"]["linear"]=json!({"disabled":false,"lastSuccess":100,"error":"benign-private-source-error"});
            state["sources"]["github"]=json!({"disabled":true,"lastSuccess":"benign-private-timestamp-marker"});
            Ok(())
        }).unwrap();
        let path=repo.data.parent().unwrap().join("config/connections.json");
        std::fs::write(&path,serde_json::to_vec(&json!({"version":1,"storage":"macos-login-keychain","revision":1,"providers":{"github":{"credential_ref":uuid(),"identity":"benign-private-account-marker","org":"","base":""}}})).unwrap()).unwrap();
        let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let configuration=std::fs::read(&path).unwrap();
        let public=view(&repo);assert!(public["progress"].get("privateExtension").is_none());assert!(!public.to_string().contains("benign-private"));
        assert!(arr(&public["connections"]).iter().find(|row|row["id"]=="github").unwrap()["lastSuccess"].is_null());
        let summary=assistant_context(&repo);assert_eq!(summary["credentialMetadata"],"available");assert_eq!(summary["progress"]["step"],"company");assert_eq!(summary["progress"]["steps"]["company"],"pending");
        assert!(summary["progress"].get("privateExtension").is_none());assert!(!summary.to_string().contains("benign-private"));
        let rows=arr(&summary["connections"]);let linear=rows.iter().find(|row|row["id"]=="linear").unwrap();let github=rows.iter().find(|row|row["id"]=="github").unwrap();
        assert_eq!(linear["savedInApp"],false);assert_eq!(linear["syncEnabled"],true);assert_eq!(linear["hasError"],true);assert_eq!(linear["lastSuccess"],100);
        assert_eq!(github["savedInApp"],true);assert_eq!(github["syncEnabled"],false);assert!(github.get("lastSuccess").is_none());
        assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(std::fs::read(&path).unwrap(),configuration);
        std::fs::write(&path,"benign-private-metadata-failure").unwrap();let unavailable=assistant_context(&repo);assert_eq!(unavailable["credentialMetadata"],"unavailable");assert!(!unavailable.to_string().contains("benign-private"));assert!(arr(&unavailable["connections"]).iter().all(|row|row.get("savedInApp").is_none()));
        std::fs::remove_file(path).unwrap();let legacy=assistant_context(&repo);assert_eq!(legacy["credentialMetadata"],"available");assert!(arr(&legacy["connections"]).iter().all(|row|row["savedInApp"]==false));
    }

    #[test]
    fn unsupported_progress_rejects_updates_and_reset_without_changes() {
        for payload in [
            json!({"revision":4,"step":"theme","status":"complete"}),
            json!({"revision":4,"reset":true}),
        ] {
            let mut state = json!({"onboarding":{"version":2,"revision":4,"future":"keep"}});
            let original = state.clone();
            assert!(apply_update(&mut state, &payload).is_err());
            assert_eq!(state, original);
        }
    }
    #[test]
    fn repairing_invalid_progress_retains_the_original_and_valid_unknown_fields() {
        let original = json!({"version":1,"revision":0,"step":3,"steps":{},"completed":true,"extension":{"keep":true}});
        let mut state = json!({"onboarding":original});
        apply_update(&mut state, &json!({"revision":0,"step":"assistant","status":"skipped"})).unwrap();
        assert_eq!(state["onboardingRecovery"]["original"], original);
        assert_eq!(state["onboarding"]["step"], "assistant");
        state["onboarding"]["extension"] = json!({"keep":true});
        apply_update(&mut state, &json!({"revision":1,"step":"theme","status":"current"})).unwrap();
        assert_eq!(state["onboarding"]["extension"]["keep"], true);
        assert_eq!(state["onboardingRecovery"]["original"], original);
    }
    #[test]
    fn oversized_invalid_progress_stays_unchanged() {
        let mut state = json!({"onboarding":{"version":1,"step":3,"data":"x".repeat(65_537)}});
        let original = state.clone();
        assert!(apply_update(&mut state, &json!({"revision":0,"reset":true})).is_err());
        assert_eq!(state, original);
    }
    #[test]
    fn malformed_progress_starts_setup_from_the_first_step() {
        for malformed in [
            json!({"version":1,"revision":0,"step":3,"steps":{},"completed":true}),
            json!({"version":1,"revision":0,"step":"invented","steps":{},"completed":false}),
            json!({"version":1,"revision":-1,"step":"finish","steps":{},"completed":true}),
            json!({"version":1,"revision":0,"step":"finish","steps":[],"completed":true}),
            json!({"version":1,"revision":0,"step":"finish","steps":{"skills":"running"},"completed":true}),
        ] {
            let next = progress(&malformed);
            assert_eq!(next["step"], "assistant");
            assert_eq!(next["revision"], 0);
            assert_eq!(next["completed"], false);
        }
    }
    #[test]
    fn progress_resumes_skipped_and_pending_steps_and_rejects_stale_writes() {
        let skipped = advance(
            &Value::Null,
            &json!({"revision":0,"step":"integrations","status":"skipped"}),
        )
        .unwrap();
        let pending = advance(
            &skipped,
            &json!({"revision":1,"step":"skills","status":"pending"}),
        )
        .unwrap();
        assert_eq!(pending["steps"]["integrations"], "skipped");
        assert_eq!(pending["steps"]["skills"], "pending");
        assert_eq!(pending["step"], "skills");
        assert!(advance(
            &pending,
            &json!({"revision":1,"step":"skills","status":"complete"})
        )
        .is_err());
        let reset = advance(&pending, &json!({"revision":2,"reset":true})).unwrap();
        assert_eq!(reset["step"], "assistant");
        assert_eq!(reset["revision"], 3);
        assert_eq!(reset["completed"], false);
    }
    #[test]
    fn progress_rejects_secret_fields_and_invalid_steps() {
        assert!(advance(
            &Value::Null,
            &json!({"revision":0,"step":"integrations","status":"pending","token":"canary"})
        )
        .is_err());
        assert!(advance(
            &Value::Null,
            &json!({"revision":0,"step":"invented","status":"complete"})
        )
        .is_err());
    }
}
