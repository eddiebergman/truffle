use super::*;
use crate::backend::arr;
use crate::backend::operations;
use crate::backend::sessions;
use crate::backend::settings::Approval;
use crate::backend::{now, uuid};

#[test]
fn page_scopes_command_exposes_exact_cached_and_configured_choices_without_effects() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{
            state["items"]=json!([
                {"id":"linear:1","source":"linear","project":"Exact Case","status":"done","archivedAt":123,"privateExtension":"private-choice-canary"},
                {"id":"linear:2","source":"linear","project":"Exact Case"},
                {"id":"linear:3","source":"linear","project":"","status":"todo"},
                {"id":"github:1","source":"github","url":"https://github.com/Owner/Repo/pull/1","project":"Wrong"},
                {"id":"sentry:1","source":"sentry","project":"Observed"}
            ]);
            state["overrides"]["linear:1"]=json!({"hidden":true});state["snoozed"]=json!({"linear:1":9_999_999_999_u64});
            state["itemArchive"]=json!({"linear:archive":{"source":"linear","project":"Archive only"}});
            state["sessions"]=json!([{"items":[{"source":"linear","project":"Session only"}]}]);Ok(())
        }).unwrap();
    let current = super::super::page_settings::view(&repo).unwrap();
    super::super::page_settings::update(&repo,&json!({"revision":current["revision"],"contentHash":current["contentHash"],"page":"linear","scopes":["Exact Case","Retained",""]})).unwrap();
    let path = repo.settings.config_directory().join("page-settings.json");
    let disk = std::fs::read(&path).unwrap();
    let core = std::fs::read(&repo.data).unwrap();
    let state = repo.state.lock().unwrap().clone();
    let read = |args| {
        invoke(
            &repo,
            &json!({"name":"pages.scopes.read","args":args}),
            Caller::Assistant,
        )
        .unwrap()
    };
    let result = read(json!({"page":"linear"}));
    assert_eq!(result["mode"], "selected");
    assert_eq!(result["scope"], "cached");
    assert_eq!(result["total"], 4);
    assert_eq!(
        result["choices"],
        json!([
            {"key":"","cached":false,"visible":true},{"key":"Exact Case","cached":true,"visible":true},
            {"key":"No project","cached":true,"visible":false},{"key":"Retained","cached":false,"visible":true}
        ])
    );
    assert!(!result.to_string().contains("private-choice-canary"));
    assert_eq!(
        read(json!({"page":"github"}))["choices"],
        json!([{"key":"Owner/Repo","cached":true,"visible":true}])
    );
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), core);
    assert_eq!(std::fs::read(&path).unwrap(), disk);
    for (scopes, mode, visible) in [(Value::Null, "all", true), (json!([]), "none", false)] {
        let current = super::super::page_settings::view(&repo).unwrap();
        super::super::page_settings::update(&repo,&json!({"revision":current["revision"],"contentHash":current["contentHash"],"page":"sentry","scopes":scopes})).unwrap();
        let before = std::fs::read(&path).unwrap();
        let result = read(json!({"page":"sentry"}));
        assert_eq!(result["mode"], mode);
        assert_eq!(result["choices"][0]["visible"], visible);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    let disk = std::fs::read(&path).unwrap();
    read(json!({"page":"linear","query":"cAsE"}));
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), core);
    assert_eq!(std::fs::read(&path).unwrap(), disk);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
    let spec = table()
        .into_iter()
        .find(|entry| entry.name == "pages.scopes.read")
        .unwrap();
    assert_eq!(spec.effect, Effect::Read);
    assert_eq!(spec.access, Access::Anyone);
}

#[test]
fn page_scopes_command_pagination_binds_full_catalog_query_and_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["items"]=json!((0..110).map(|i|json!({"id":format!("linear:{i}"),"source":"linear","project":format!("Scope {i:03}")})).collect::<Vec<_>>());Ok(())}).unwrap();
    let read = |args| {
        invoke(
            &repo,
            &json!({"name":"pages.scopes.read","args":args}),
            Caller::Assistant,
        )
    };
    let first = read(json!({"page":"linear"})).unwrap();
    assert_eq!(first["choices"].as_array().unwrap().len(), 100);
    assert_eq!(first["total"], 110);
    assert_eq!(first["nextOffset"], 100);
    let continuation = json!({"page":"linear","offset":100,"snapshot":first["snapshot"]});
    let second = read(continuation.clone()).unwrap();
    assert_eq!(second["choices"].as_array().unwrap().len(), 10);
    assert_eq!(second["choices"][0]["key"], "Scope 100");
    assert!(second["nextOffset"].is_null());
    let query = read(json!({"page":"linear","query":"sCoPe 109"})).unwrap();
    assert_eq!(
        query["choices"],
        json!([{"key":"Scope 109","cached":true,"visible":true}])
    );
    for args in [
        json!({"page":"linear","offset":100}),
        json!({"page":"linear","query":"Scope","offset":100,"snapshot":first["snapshot"]}),
        json!({"page":"github","offset":100,"snapshot":first["snapshot"]}),
    ] {
        assert_eq!(read(args).unwrap_err().status, 409);
    }
    repo.transaction(|state| {
        state["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"linear:new","source":"linear","project":"Scope 999"}));
        Ok(())
    })
    .unwrap();
    assert_eq!(read(continuation).unwrap_err().status, 409);
    let first = read(json!({"page":"linear"})).unwrap();
    let path = repo.settings.config_directory().join("page-settings.json");
    let mut config = super::super::config::read(&path).unwrap();
    config["pages"]["github"]["scopes"] = json!([]);
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    assert_eq!(
        read(json!({"page":"linear","offset":100,"snapshot":first["snapshot"]}))
            .unwrap_err()
            .status,
        409
    );
    let core = std::fs::read(&repo.data).unwrap();
    let disk = std::fs::read(&path).unwrap();
    for args in [
        json!({}),
        json!({"page":"notes"}),
        json!({"page":"linear","offset":-1}),
        json!({"page":"linear","offset":4097}),
        json!({"page":"linear","offset":0.5}),
        json!({"page":"linear","snapshot":"bad"}),
        json!({"page":"linear","query":true}),
        json!({"page":"linear","query":"x".repeat(501)}),
        json!({"page":"linear","query":"bad\0query"}),
        json!({"page":"linear","sync":true}),
    ] {
        assert!(read(args).is_err());
    }
    assert_eq!(std::fs::read(&repo.data).unwrap(), core);
    assert_eq!(std::fs::read(&path).unwrap(), disk);
}

#[test]
fn page_scopes_command_rejects_secret_scope_identities_without_altering_storage() {
    let temp = tempfile::tempdir().unwrap();
    let secret = "fixture-private-catalog-key";
    std::fs::write(
        temp.path().join(".env"),
        format!("PAGE_SCOPES_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for configured in [false, true] {
        repo.transaction(|state|{state["items"]=if configured{json!([])}else{json!([{"id":"linear:secret","source":"linear","project":format!("Prefix {secret}")}])};Ok(())}).unwrap();
        if configured {
            let current = super::super::page_settings::view(&repo).unwrap();
            super::super::page_settings::update(&repo,&json!({"revision":current["revision"],"contentHash":current["contentHash"],"page":"linear","scopes":[secret]})).unwrap();
        }
        let path = repo.settings.config_directory().join("page-settings.json");
        let disk = std::fs::read(&path).unwrap();
        let core = std::fs::read(&repo.data).unwrap();
        let state = repo.state.lock().unwrap().clone();
        let error = invoke(
            &repo,
            &json!({"name":"pages.scopes.read","args":{"page":"linear"}}),
            Caller::Assistant,
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "Page visibility scopes cannot be safely projected."
        );
        assert!(!error.message.contains(secret));
        assert_eq!(*repo.state.lock().unwrap(), state);
        assert_eq!(std::fs::read(&repo.data).unwrap(), core);
        assert_eq!(std::fs::read(&path).unwrap(), disk);
    }
    let current = super::super::page_settings::view(&repo).unwrap();
    super::super::page_settings::update(&repo,&json!({"revision":current["revision"],"contentHash":current["contentHash"],"page":"linear","scopes":null})).unwrap();
    repo.transaction(|state|{
            state["items"]=json!((0..110).map(|i|json!({"id":format!("linear:{i}"),"source":"linear","project":format!("A safe {i:03}")})).collect::<Vec<_>>());
            state["items"].as_array_mut().unwrap().push(json!({"id":"linear:secret","source":"linear","project":format!("Z {secret}")}));Ok(())
        }).unwrap();
    let path = repo.settings.config_directory().join("page-settings.json");
    let disk = std::fs::read(&path).unwrap();
    let core = std::fs::read(&repo.data).unwrap();
    for args in [
        json!({"page":"linear"}),
        json!({"page":"linear","query":"A safe"}),
    ] {
        assert_eq!(
            invoke(
                &repo,
                &json!({"name":"pages.scopes.read","args":args}),
                Caller::Assistant
            )
            .unwrap_err()
            .message,
            "Page visibility scopes cannot be safely projected."
        );
        assert_eq!(std::fs::read(&repo.data).unwrap(), core);
        assert_eq!(std::fs::read(&path).unwrap(), disk);
    }
}

#[test]
fn page_visibility_commands_reject_redacted_current_and_new_scope_identities() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = "fixture-private-scope-identity";
    std::fs::write(
        temp.path().join(".env"),
        format!("PAGE_VISIBILITY_SECRET={sentinel}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let path = repo.settings.config_directory().join("page-settings.json");
    let payload = |current: &Value, page: &str, scopes: Value| {
        json!({
            "revision":current["revision"],"contentHash":current["contentHash"],"page":page,"scopes":scopes
        })
    };
    let initial = super::super::page_settings::view(&repo).unwrap();
    let before = std::fs::read(&path).unwrap();
    let proposed = payload(&initial, "linear", json!([format!("project-{sentinel}")]));
    let error = invoke(
        &repo,
        &json!({"name":"pages.settings.apply","args":proposed}),
        Caller::Assistant,
    )
    .unwrap_err();
    assert_eq!(
        error.message,
        "Page visibility scopes cannot be safely projected."
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // The direct UI owner retains exact configured scopes. Named commands must
    // reject an unsafe scope even when updating a different page.
    let current =
        super::super::page_settings::update(&repo, &payload(&initial, "github", json!([sentinel])))
            .unwrap();
    let before = std::fs::read(&path).unwrap();
    for request in [
        json!({"name":"pages.settings.read"}),
        json!({"name":"pages.settings.apply","args":payload(&current,"sentry",json!(["Safe project"]))}),
    ] {
        let error = invoke(&repo, &request, Caller::Assistant).unwrap_err();
        assert_eq!(
            error.message,
            "Page visibility scopes cannot be safely projected."
        );
        assert!(!error.message.contains(sentinel));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    assert_eq!(super::super::page_settings::view(&repo).unwrap(), current);
}

#[test]
fn page_visibility_commands_preserve_exact_scopes_and_checked_baselines() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let path = repo.settings.config_directory().join("page-settings.json");
    let read = || {
        invoke(
            &repo,
            &json!({"name":"pages.settings.read"}),
            Caller::Assistant,
        )
        .unwrap()
    };
    let initial = read();
    let payload = |current: &Value, scopes: Value| json!({"revision":current["revision"],"contentHash":current["contentHash"],"page":"linear","scopes":scopes});
    let mut current = initial.clone();
    for scopes in [json!(["Exact Case / scope", ""]), json!([]), Value::Null] {
        let next = invoke(
            &repo,
            &json!({"name":"pages.settings.apply","args":payload(&current,scopes.clone())}),
            Caller::Assistant,
        )
        .unwrap();
        assert_eq!(next["pages"]["linear"]["scopes"], scopes);
        assert_eq!(next["pages"]["github"], initial["pages"]["github"]);
        assert_eq!(next["pages"]["sentry"], initial["pages"]["sentry"]);
        assert_eq!(read(), next);
        current = next;
    }
    let before = std::fs::read(&path).unwrap();
    for key in ["revision", "contentHash"] {
        let mut stale = payload(&current, json!(["Replacement"]));
        stale[key] = initial[key].clone();
        assert_eq!(
            invoke(
                &repo,
                &json!({"name":"pages.settings.apply","args":stale}),
                Caller::Assistant
            )
            .unwrap_err()
            .status,
            409
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[test]
fn presentation_commands_validate_tagged_forms_and_acknowledgement_policy() {
    let id = uuid();
    for presentation in [
        json!({"type":"saved-view","id":"standup"}),
        json!({"type":"display-items","title":"Matches","items":[{"type":"work","id":"github:one"}],"recommended":{"type":"work","id":"github:one"},"reason":"Exact title"}),
    ] {
        validate_presentation_arguments(
            "present",
            &json!({"requestId":id,"presentation":presentation}),
        )
        .unwrap();
    }
    for presentation in [
        json!({"type":"saved-view","id":"standup","items":[]}),
        json!({"type":"display-items","id":"standup","items":[{"type":"work","id":"one"}]}),
        json!({"type":"display-items","items":[]}),
        json!({"type":"display-items","items":[{"type":"shell","id":"one"}]}),
        json!({"type":"display-items","items":[{"type":"chat","id":"one","command":"run"}]}),
        json!({"type":"open-chat","id":"one"}),
        json!({"type":"saved-view"}),
    ] {
        assert!(validate_presentation_arguments(
            "present",
            &json!({"requestId":id,"presentation":presentation})
        )
        .is_err());
    }
    let ambiguous = json!({"type":"string","oneOf":[{"type":"string"},{"type":"string"}]});
    assert!(validate_arguments(&ambiguous, &json!("twice"), "args").is_err());
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = std::fs::read(&repo.data).ok();
    assert!(invoke(
        &repo,
        &json!({"name":"ui.presentations.ack","args":{"requestId":id,"outcome":"shown"}}),
        Caller::Assistant
    )
    .unwrap_err()
    .message
    .contains("requires a user action"));
    assert_eq!(std::fs::read(&repo.data).ok(), before);
    for (name, read_only, assistant) in [
        ("ui.present", false, true),
        ("ui.presentations.read", true, true),
        ("ui.presentations.ack", false, false),
    ] {
        let row = arr(&catalog()["commands"])
            .into_iter()
            .find(|row| row["name"] == name)
            .unwrap();
        assert_eq!(row["readOnly"], read_only);
        assert_eq!(row["appChat"], assistant);
    }
}

#[test]
fn external_presentation_commands_queue_and_acknowledge_without_opening_chats() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let project =
        super::super::projects::create(&repo, &json!({"title":"Presentation fixture"})).unwrap();
    let item_id = &project["project"]["id"];
    let request_id = uuid();
    let before = repo.state.lock().unwrap().clone();
    let args = json!({"requestId":request_id,"presentation":{"type":"display-items","title":"Selected work","items":[{"type":"work","id":item_id}],"recommended":{"type":"work","id":item_id}}});
    let receipt = invoke(
        &repo,
        &json!({"name":"ui.present","args":args}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(receipt, json!({"requestId":request_id,"status":"queued"}));
    assert_eq!(
        invoke(
            &repo,
            &json!({"name":"ui.present","args":args}),
            Caller::Assistant
        )
        .unwrap(),
        receipt
    );
    let read = invoke(
        &repo,
        &json!({"name":"ui.presentations.read"}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(read["pending"].as_array().unwrap().len(), 1);
    assert_eq!(
        read["pending"][0]["presentation"]["items"][0]["item"]["id"],
        *item_id
    );
    assert!(repo.snapshot(false).get("uiPresentations").is_none());
    let acknowledged = invoke(
        &repo,
        &json!({"name":"ui.presentations.ack","args":{"requestId":request_id,"outcome":"shown"}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(
        acknowledged,
        json!({"requestId":request_id,"status":"shown"})
    );
    assert_eq!(
        invoke(
            &repo,
            &json!({"name":"ui.presentations.read"}),
            Caller::Assistant
        )
        .unwrap()["pending"]
            .as_array()
            .unwrap()
            .as_slice(),
        [] as [Value; 0]
    );
    let mut after = repo.state.lock().unwrap().clone();
    after.as_object_mut().unwrap().remove("uiPresentations");
    assert_eq!(after, before);
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
    assert!(repo.cooldown.lock().unwrap().is_empty());
}

/// A dirty checkout whose clean filter writes the returned marker when git runs it.
fn checkout_with_clean_filter(home: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let root = home.join("checkout");
    std::fs::create_dir(&root).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    std::fs::write(root.join("code.txt"), "base\n").unwrap();
    git(&["add", "code.txt"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-m",
        "fixture",
    ]);
    let marker = home.join("helper-called");
    let script = home.join("clean.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf called > '{}'\n/bin/cat\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let driver = format!("truffle-{}", uuid());
    git(&[
        "config",
        &format!("filter.{driver}.clean"),
        script.to_str().unwrap(),
    ]);
    git(&["config", "core.fsmonitor", "false"]);
    git(&["config", "core.hooksPath", "/dev/null"]);
    git(&["config", "core.attributesFile", "/dev/null"]);
    std::fs::write(
        root.join(".gitattributes"),
        format!("code.txt filter={driver}\n"),
    )
    .unwrap();
    std::fs::write(root.join("code.txt"), "changed\n").unwrap();
    (root, marker)
}
#[test]
fn git_backed_read_commands_deny_assistant_before_configured_helpers_and_validate_args() {
    let temp = tempfile::tempdir().unwrap();
    let (root, marker) = checkout_with_clean_filter(temp.path());
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let id = repo
        .transaction(|s| Ok(sessions::fixture(s, &root, |_| {})))
        .unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    let before = repo.state.lock().unwrap().clone();
    for (name, args) in [
        ("review.check", json!({"id":id})),
        ("chats.diff", json!({"recordId":id})),
        ("chats.diff.file", json!({"recordId":id,"path":"code.txt"})),
    ] {
        let spec = table().into_iter().find(|s| s.name == name).unwrap();
        assert_eq!(spec.effect, Effect::Read);
        assert_eq!(spec.access, Access::UserAction);
        assert!(
            invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant)
                .unwrap_err()
                .message
                .contains("requires a user action")
        );
        assert!(!marker.exists());
    }
    for (name, args) in [
        ("review.check", json!({"id":id,"cwd":root})),
        ("chats.diff", json!({"recordId":id,"root":root})),
        ("chats.diff.file", json!({"recordId":id})),
        ("chats.diff.file", json!({"recordId":id,"path":7})),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).is_err());
        assert!(!marker.exists());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    invoke(
        &repo,
        &json!({"name":"chats.diff","args":{"recordId":id}}),
        Caller::Browser,
    )
    .unwrap();
    assert!(
        marker.exists(),
        "positive control must exercise the configured local helper"
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn skills_editor_commands_are_strict_user_writes_with_bounded_read_and_exact_receipts() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("SKILL.md"), "Fixture skill").unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    let create = json!({"title":"Command editor","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex"});
    for name in ["skills.save", "skills.preset.update", "skills.schedule"] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, Effect::Write);
        assert_eq!(spec.access, Access::UserAction);
        assert!(
            invoke(&repo, &json!({"name":name,"args":{}}), Caller::Assistant)
                .unwrap_err()
                .message
                .contains("requires a user action")
        );
    }
    let saved = invoke(
        &repo,
        &json!({"name":"skills.save","args":create}),
        Caller::Browser,
    )
    .unwrap();
    let id = &saved["presetId"];
    assert_eq!(saved["preset"]["id"], *id);
    assert!(uuid::Uuid::parse_str(id.as_str().unwrap()).is_ok());
    assert_eq!(saved["created"], true);
    repo.transaction(|s| {
        s["skillPresets"][0]["privateNs"] = json!("private-skills-command-canary");
        Ok(())
    })
    .unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    let read = invoke(
        &repo,
        &json!({"name":"skills.preset.read","args":{"presetId":id}}),
        Caller::Assistant,
    )
    .unwrap();
    assert!(!read.to_string().contains("private-skills-command-canary"));
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert!(read["schedule"]["nextRun"].is_null());
    for (name, args) in [
        (
            "skills.save",
            json!({"id":id,"title":"Overwrite","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex"}),
        ),
        (
            "skills.preset.update",
            json!({"presetId":id,"patch":{"reports":"move"},"expected":{}}),
        ),
        (
            "skills.preset.update",
            json!({"presetId":id,"patch":{"title":"New"},"expected":{}}),
        ),
        (
            "skills.schedule",
            json!({"presetId":id,"mode":"interval","everyMinutes":5,"missed":"latest"}),
        ),
        (
            "skills.schedule",
            json!({"presetId":id,"patch":{"nextRun":0},"expected":{}}),
        ),
        (
            "skills.preset.read",
            json!({"presetId":id,"path":"/private"}),
        ),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).is_err());
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
    let updated=invoke(&repo,&json!({"name":"skills.preset.update","args":{"presetId":id,"patch":{"title":"Updated"},"expected":{"title":read["expected"]["title"]}}}),Caller::Browser).unwrap();
    assert_eq!(updated["changedFields"], json!(["title"]));
    assert_eq!(updated["preset"]["title"], "Updated");
    let scheduled=invoke(&repo,&json!({"name":"skills.schedule","args":{"presetId":id,"patch":{"enabled":false},"expected":{"enabled":read["scheduleExpected"]["enabled"]}}}),Caller::Browser).unwrap();
    assert_eq!(scheduled["saved"], true);
    assert_eq!(scheduled["schedule"]["enabled"], false);
    assert_eq!(scheduled.as_object().unwrap().len(), 5);
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn saved_chat_open_command_is_strict_user_action_and_returns_only_bounded_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s|{s["sessions"]=json!([{"id":id,"agent":"codex","cwd":temp.path(),"title":"Private title marker","destination":"embedded","launchNew":true,"items":[],"private":"private-open-marker"}]);Ok(())}).unwrap();
    let before = std::fs::read(&repo.data).unwrap();
    let spec = table()
        .into_iter()
        .find(|s| s.name == "chats.open")
        .unwrap();
    assert_eq!(spec.effect, Effect::Write);
    assert_eq!(spec.access, Access::UserAction);
    let denied = invoke(
        &repo,
        &json!({"name":"chats.open","args":{"recordId":id}}),
        Caller::Assistant,
    )
    .unwrap_err();
    assert!(denied.message.contains("requires a user action"));
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    for args in [
        json!({"recordId":id,"title":"Stale"}),
        json!({"recordId":id,"target":"desktop"}),
        json!({"id":id}),
        json!({"recordId":7}),
    ] {
        assert!(invoke(
            &repo,
            &json!({"name":"chats.open","args":args}),
            Caller::Browser
        )
        .is_err());
    }
    let receipt = invoke(
        &repo,
        &json!({"name":"chats.open","args":{"recordId":id}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(receipt["status"], "preview");
    assert_eq!(receipt["terminal"]["state"], "not-started");
    assert_eq!(receipt.as_object().unwrap().len(), 6);
    assert!(!receipt.to_string().contains("private-open-marker"));
    assert!(!receipt.to_string().contains("Private title marker"));
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}
#[test]
fn artifact_metadata_commands_share_cas_owner_and_deny_discovery_registration_to_app_chat() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = crate::backend::artifacts::id("https://example.invalid/command-metadata");
    repo.transaction(|state|{state["artifacts"]=json!([{"id":id,"title":"Original","url":"https://example.invalid/command-metadata","sessionIds":[],"privateNs":"private-command-artifact"}]);Ok(())}).unwrap();
    for (name, effect, policy) in [
        ("artifacts.metadata.read", Effect::Read, Access::Anyone),
        ("artifacts.metadata.update", Effect::Write, Access::Anyone),
        (
            "artifacts.register-discovered",
            Effect::Write,
            Access::UserAction,
        ),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, effect);
        assert_eq!(spec.access, policy);
    }
    let read = invoke(
        &repo,
        &json!({"name":"artifacts.metadata.read","args":{"id":id}}),
        Caller::Assistant,
    )
    .unwrap();
    assert!(!read.to_string().contains("private-command-artifact"));
    let denied=invoke(&repo,&json!({"name":"artifacts.register-discovered","args":{"id":id,"patch":{"title":"Not allowed"},"expected":{}}}),Caller::Assistant).unwrap_err();
    assert!(denied.message.contains("requires a user action"));
    let before = repo.state.lock().unwrap().clone();
    for args in [
        json!({"id":id,"patch":{"presetId":"changed"},"expected":{}}),
        json!({"id":id,"patch":{"title":"New"},"expected":{},"path":"/private"}),
        json!({"id":id,"patch":{"title":7},"expected":{"title":read["expected"]["title"]}}),
    ] {
        assert!(invoke(
            &repo,
            &json!({"name":"artifacts.metadata.update","args":args}),
            Caller::Assistant
        )
        .is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    let updated=invoke(&repo,&json!({"name":"artifacts.metadata.update","args":{"id":id,"patch":{"title":"Changed"},"expected":{"title":read["expected"]["title"]}}}),Caller::Assistant).unwrap();
    assert_eq!(updated["saved"], true);
    assert_eq!(updated["changedFields"], json!(["title"]));
    assert_eq!(
        repo.state.lock().unwrap()["artifacts"][0]["privateNs"],
        "private-command-artifact"
    );
}
#[test]
fn changes_commands_are_strict_cached_routine_owners() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["changes"]=json!([{"id":"observed","title":"Visible","source":"github","read":false,"privateNs":"private-change-marker"}]);Ok(())}).unwrap();
    for (name, effect) in [
        ("changes.list", Effect::Read),
        ("changes.mark-read", Effect::Write),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, effect);
        assert_eq!(spec.access, Access::Anyone);
    }
    let before = std::fs::read(&repo.data).unwrap();
    let result = invoke(
        &repo,
        &json!({"name":"changes.list","args":{}}),
        Caller::Assistant,
    )
    .unwrap();
    assert!(!result.to_string().contains("private-change-marker"));
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    for (name, args) in [
        ("changes.list", json!({"limit":101})),
        ("changes.list", json!({"offset":1})),
        ("changes.mark-read", json!({"all":true})),
        ("changes.mark-read", json!({"ids":["observed"],"all":true})),
        ("changes.mark-read", json!({"ids":[7]})),
        ("changes.mark-read", json!({"ids":vec!["observed";501]})),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant).is_err());
    }
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    let receipt = invoke(
        &repo,
        &json!({"name":"changes.mark-read","args":{"ids":["observed","pruned"]}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(receipt["markedIds"], json!(["observed"]));
    assert_eq!(receipt["absentIds"], json!(["pruned"]));
    assert_eq!(
        repo.state.lock().unwrap()["changes"][0]["privateNs"],
        "private-change-marker"
    );
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn project_folder_command_is_strict_read_only_and_available_to_the_assistant() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let project = super::super::projects::create(
        &repo,
        &json!({"title":"Folder command fixture","cwd":temp.path()}),
    )
    .unwrap();
    let id = &project["project"]["id"];
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let spec = table()
        .into_iter()
        .find(|spec| spec.name == "projects.folder")
        .unwrap();
    assert_eq!(spec.effect, Effect::Read);
    assert_eq!(spec.access, Access::Anyone);
    let result = invoke(
        &repo,
        &json!({"name":"projects.folder","args":{"id":id}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(
        result,
        json!({"id":id,"cwd":temp.path().canonicalize().unwrap(),"requiresSelection":false})
    );
    for args in [
        json!({"id":id,"chooseFolder":true}),
        json!({"id":id,"cwd":temp.path()}),
        json!({"id":7}),
        json!({}),
    ] {
        assert!(invoke(
            &repo,
            &json!({"name":"projects.folder","args":args}),
            Caller::Assistant
        )
        .is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}
#[test]
fn projects_commands_are_strict_routine_metadata_with_bounded_owner_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for (name, effect) in [
        ("projects.read", Effect::Read),
        ("projects.create", Effect::Write),
        ("projects.update", Effect::Write),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, effect);
        assert_eq!(spec.access, Access::Anyone);
    }
    let result=invoke(&repo,&json!({"name":"projects.create","args":{"title":"Routine project","detail":"Full editable detail"}}),Caller::Assistant).unwrap();
    let id = text(&result["project"], "id");
    assert!(id.starts_with("note:"));
    assert_eq!(result["created"], true);
    repo.transaction(|state| {
        state["notes"][0]["privateNs"] = json!({"marker":"private-project-command-marker"});
        Ok(())
    })
    .unwrap();
    let read = invoke(
        &repo,
        &json!({"name":"projects.read","args":{"id":id}}),
        Caller::Assistant,
    )
    .unwrap();
    assert!(!read.to_string().contains("private-project-command-marker"));
    let changed=invoke(&repo,&json!({"name":"projects.update","args":{"id":id,"patch":{"title":"Changed project"},"expected":{"title":read["expected"]["title"]}}}),Caller::Assistant).unwrap();
    assert_eq!(changed["changedFields"], json!(["title"]));
    assert_eq!(changed["project"]["detail"], "Full editable detail");
    assert_eq!(
        repo.state.lock().unwrap()["notes"][0]["privateNs"]["marker"],
        "private-project-command-marker"
    );
    let before = repo.state.lock().unwrap().clone();
    for (name, args) in [
        ("projects.create", json!({"title":"New","id":"note:chosen"})),
        ("projects.create", json!({"title":"New","command":"run"})),
        ("projects.read", json!({"id":id,"includePrivate":true})),
        (
            "projects.update",
            json!({"id":id,"patch":{"status":"remote-closed"},"expected":{"status":read["expected"]["status"]}}),
        ),
        ("projects.update", json!({"id":id,"patch":{"title":"New"}})),
        (
            "projects.update",
            json!({"id":id,"patch":{"title":"New"},"expected":{"title":7}}),
        ),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
}
#[test]
fn tracked_dispatch_commands_deny_assistant_writes_before_preparation_and_expose_safe_read_only_receipts(
) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s| {
        s["sessions"] =
            json!([{"id":id,"agent":"codex","title":"Fixture","cwd":temp.path(),"items":[]}]);
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    for name in [
        "review.request.prepare",
        "review.send.prepare",
        "chats.ci.investigate.prepare",
        "chats.dispatch.submit",
        "chats.dispatch.finalize",
        "chats.dispatch.resolve",
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.access, Access::UserAction);
        assert_eq!(spec.effect, Effect::Write);
        let args = json!({"recordId":id,"operationId":uuid(),"revision":0,"itemId":"github:pr","decision":"submitted"});
        assert!(
            invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant)
                .unwrap_err()
                .message
                .contains("requires a user action")
        );
    }
    for (name, args) in [
        (
            "review.send.prepare",
            json!({"recordId":id,"operationId":uuid()}),
        ),
        (
            "review.request.prepare",
            json!({"recordId":id,"operationId":uuid(),"revision":-1}),
        ),
        (
            "chats.ci.investigate.prepare",
            json!({"recordId":id,"operationId":uuid(),"itemId":"github:pr","data":"arbitrary input"}),
        ),
        (
            "chats.dispatch.finalize",
            json!({"recordId":id,"operationId":3}),
        ),
        (
            "chats.dispatch.resolve",
            json!({"recordId":id,"operationId":uuid(),"decision":"retry"}),
        ),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(
        invoke(
            &repo,
            &json!({"name":"chats.dispatch.read","args":{"recordId":id}}),
            Caller::Assistant
        )
        .unwrap()["operations"],
        json!([])
    );
    let spec = table()
        .into_iter()
        .find(|spec| spec.name == "chats.dispatch.read")
        .unwrap();
    assert_eq!(spec.effect, Effect::Read);
    assert_eq!(spec.access, Access::Anyone);
}
#[test]
fn tracked_dispatch_v2_retires_creation_names_and_never_submits_unknown_ids() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s| {
        s["sessions"] = json!([{"id":id,"items":[]}]);
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    for name in ["review.request", "review.send", "chats.ci.investigate"] {
        assert!(invoke(
            &repo,
            &json!({"name":name,"args":{"recordId":id,"operationId":uuid(),"revision":0}}),
            Caller::Browser
        )
        .is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    for name in [
        "review.request.prepare",
        "review.send.prepare",
        "chats.ci.investigate.prepare",
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert!(spec.args["properties"].get("operationId").is_none());
        let mut args = json!({"recordId":id,"operationId":uuid()});
        if name == "chats.ci.investigate.prepare" {
            args["itemId"] = json!("github:pr");
        } else {
            args["revision"] = json!(0);
        }
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(invoke(&repo,&json!({"name":"chats.dispatch.submit","args":{"recordId":id,"operationId":uuid(),"revision":0}}),Caller::Browser).is_err());
    let missing = invoke(
        &repo,
        &json!({"name":"chats.dispatch.submit","args":{"recordId":id,"operationId":uuid()}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(missing["type"], "unavailable");
    assert_eq!(missing["reason"], "possibly-expired");
    for key in ["status", "finalized", "kind", "payloadHash", "generation"] {
        assert!(missing.get(key).is_none());
    }
    assert_eq!(
        arr(&repo.state.lock().unwrap()["sessionDispatch"]["entries"]),
        [] as [serde_json::Value; 0]
    );
}
#[test]
fn chat_ci_command_is_strict_user_action_and_denies_before_admission() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s| {
        s["items"] = json!([{"id":"github:a","source":"github","kind":"Pull request"}]);
        s["sessions"] = json!([{"id":id,"items":[{"id":"github:a"}]}]);
        Ok(())
    })
    .unwrap();
    let spec = table()
        .into_iter()
        .find(|spec| spec.name == "chats.ci.refresh")
        .unwrap();
    assert_eq!(spec.effect, Effect::Write);
    assert_eq!(spec.access, Access::UserAction);
    let before = repo.state.lock().unwrap().clone();
    assert!(invoke(
        &repo,
        &json!({"name":"chats.ci.refresh","args":{"recordId":id}}),
        Caller::Assistant
    )
    .unwrap_err()
    .message
    .contains("requires a user action"));
    for args in [
        json!({}),
        json!({"recordId":3}),
        json!({"recordId":"not-uuid"}),
        json!({"recordId":id,"force":true}),
        json!({"recordId":uuid()}),
    ] {
        assert!(invoke(
            &repo,
            &json!({"name":"chats.ci.refresh","args":args}),
            Caller::Browser
        )
        .is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    assert!(repo.opening.lock().unwrap().is_empty());
    let result = invoke(
        &repo,
        &json!({"name":"chats.ci.refresh","args":{"recordId":id}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(result["reason"], "demo");
    assert_eq!(result["checkedAt"], Value::Null);
    assert_eq!(result["recordId"], id);
}
#[test]
fn link_work_commands_have_strict_schemas_and_deny_url_resolution_before_owner() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().to_path_buf(), None, true).unwrap();
    let chat = repo
        .transaction(|s| {
            s["items"] = json!([{"id":"known","title":"Known"}]);
            super::super::sessions::save(
                s,
                &json!({"agent":"codex","title":"Chat","cwd":temp.path(),"itemIds":[]}),
            )
        })
        .unwrap();
    let id = text(&chat, "id");
    for (name, policy) in [
        ("chats.link-work", Access::Anyone),
        ("chats.link-work-url", Access::UserAction),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, Effect::Write);
        assert_eq!(spec.access, policy);
    }
    let before = repo.state.lock().unwrap().clone();
    let denied=invoke(&repo,&json!({"name":"chats.link-work-url","args":{"recordId":id,"url":"https://github.com/example/project/issues/1"}}),Caller::Assistant).unwrap_err();
    assert!(denied.message.contains("requires a user action"));
    assert_eq!(*repo.state.lock().unwrap(), before);
    for (name, args) in [
        ("chats.link-work", json!({"recordId":id,"itemIds":[]})),
        (
            "chats.link-work",
            json!({"recordId":id,"itemIds":["known"],"title":"stale"}),
        ),
        ("chats.link-work", json!({"recordId":id,"itemIds":[3]})),
        (
            "chats.link-work",
            json!({"recordId":id,"itemIds":vec!["known";101]}),
        ),
        (
            "chats.link-work-url",
            json!({"recordId":id,"url":"http://github.com/example/project/issues/1"}),
        ),
        (
            "chats.link-work-url",
            json!({"recordId":id,"url":"https://example.invalid","cwd":"/tmp"}),
        ),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    let result = invoke(
        &repo,
        &json!({"name":"chats.link-work","args":{"recordId":id,"itemIds":["known"]}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(result["recordId"], id);
    assert_eq!(result["addedItemIds"], json!(["known"]));
    assert_eq!(result.as_object().unwrap().len(), 6);
    assert_eq!(result["saved"], true);
}
#[test]
fn artifact_attachment_commands_are_strict_and_policy_denial_precedes_file_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let path = temp.path().join("output.md");
    std::fs::write(&path, "fixture output").unwrap();
    let record = repo
        .transaction(|state| Ok(sessions::fixture(state, temp.path(), |_| {})))
        .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    for (name, args) in [
        (
            "artifacts.attach-file",
            json!({"recordId":record,"path":path}),
        ),
        (
            "artifacts.attach-url",
            json!({"recordId":record,"url":"https://example.invalid/result"}),
        ),
    ] {
        let entry = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(entry.effect, Effect::Write);
        assert_eq!(entry.access, Access::UserAction);
        assert!(
            invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant)
                .unwrap_err()
                .message
                .contains("requires a user action")
        );
    }
    for (name, args) in [
        (
            "artifacts.attach-file",
            json!({"recordId":record,"path":""}),
        ),
        (
            "artifacts.attach-file",
            json!({"recordId":record,"path":path,"url":"https://example.invalid/result"}),
        ),
        (
            "artifacts.attach-file",
            json!({"recordId":record,"path":path,"presetId":"injected"}),
        ),
        (
            "artifacts.attach-file",
            json!({"recordId":record,"path":path,"skillOutput":"true"}),
        ),
        (
            "artifacts.attach-url",
            json!({"recordId":record,"url":"http://example.invalid/result"}),
        ),
        (
            "artifacts.attach-url",
            json!({"recordId":"not-uuid","url":"https://example.invalid/result"}),
        ),
        (
            "artifacts.attach-file",
            json!({"recordId":uuid(),"path":path}),
        ),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert!(!repo.data.parent().unwrap().join("artifacts").exists());
    let result = invoke(
        &repo,
        &json!({"name":"artifacts.attach-file","args":{"recordId":record,"path":path}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(result["saved"], true);
    assert_eq!(result["recordId"], record);
    assert_eq!(result["artifactId"].as_str().unwrap().len(), 24);
    assert_eq!(result.as_object().unwrap().len(), 8);
    assert!(!result.to_string().contains("sourcePath"));
}
#[test]
fn github_pr_commands_are_discoverable_strict_and_deny_app_chat_before_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["items"]=json!([{"id":"github:PR_test","source":"github","kind":"Pull request","isDraft":true,"privateExtension":"private-item-marker"}]);Ok(())}).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    for name in [
        "github.pr.ready",
        "github.pr.request-review",
        "github.pr.refresh",
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, Effect::Write);
        assert_eq!(spec.access, Access::UserAction);
        let entry = arr(&catalog()["commands"])
            .into_iter()
            .find(|entry| entry["name"] == name)
            .unwrap();
        assert_eq!(entry["appChat"], false);
        assert_eq!(entry["readOnly"], false);
        let args = if name.ends_with("request-review") {
            json!({"id":"github:PR_test","login":"someone"})
        } else {
            json!({"id":"github:PR_test"})
        };
        let error =
            invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant).unwrap_err();
        assert!(error.message.contains("requires a user action"));
        assert!(repo.cooldown.lock().unwrap().is_empty());
        assert!(repo.opening.lock().unwrap().is_empty());
    }
    for (name, args) in [
        ("github.pr.ready", json!({"id":"github:"})),
        (
            "github.pr.ready",
            json!({"id":"github:PR_test","login":"someone"}),
        ),
        (
            "github.pr.ready",
            json!({"id":"github:PR_test","action":"request-review"}),
        ),
        ("github.pr.refresh", json!({"id":"linear:one"})),
        (
            "github.pr.refresh",
            json!({"id":format!("github:{}","a".repeat(200))}),
        ),
        ("github.pr.request-review", json!({"id":"github:PR_test"})),
        (
            "github.pr.request-review",
            json!({"id":"github:PR_test","login":"@someone"}),
        ),
        (
            "github.pr.request-review",
            json!({"id":"github:PR_test","login":""}),
        ),
        (
            "github.pr.request-review",
            json!({"id":"github:PR_test","login":"bad name"}),
        ),
        (
            "github.pr.request-review",
            json!({"id":"github:PR_test","login":true}),
        ),
        ("github.pr.ready", json!({"id":"github:uncached"})),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).is_err());
        assert!(repo.cooldown.lock().unwrap().is_empty());
        assert!(repo.opening.lock().unwrap().is_empty());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
}
#[test]
fn github_pr_named_commands_delegate_owner_and_return_only_truthful_demo_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["items"]=json!([{"id":"github:PR_test","source":"github","kind":"Pull request","status":"draft","isDraft":true,"requestedReviewers":[],"privateExtension":"private-item-marker"}]);Ok(())}).unwrap();
    for (name, action) in [
        ("github.pr.ready", "ready"),
        ("github.pr.request-review", "request-review"),
        ("github.pr.refresh", "refresh"),
    ] {
        let args = if action == "request-review" {
            json!({"id":"github:PR_test","login":"someone"})
        } else {
            json!({"id":"github:PR_test"})
        };
        let receipt = invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).unwrap();
        assert_eq!(
            receipt,
            json!({"id":"github:PR_test","action":action,"status":"preview","remote":"not-contacted","localSaved":true})
        );
        assert!(!receipt.to_string().contains("private-item"));
    }
    let state = repo.state.lock().unwrap();
    assert_eq!(state["items"][0]["isDraft"], false);
    assert_eq!(
        state["items"][0]["requestedReviewers"][0]["login"],
        "someone"
    );
    assert_eq!(state["items"][0]["privateExtension"], "private-item-marker");
}
#[test]
fn source_commands_publish_strict_policies_and_read_only_redacted_status() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "COMMAND_STATUS_SECRET=fixture-private-source-secret\n",
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {state["sources"]["github"]=json!({"disabled":false,"lastSuccess":123,"nextAttempt":456,"failures":2,"error":"Failed fixture-private-source-secret","privateExtension":"fixture-private-source-extension"});Ok(())}).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    for (name, effect, policy) in [
        ("sources.status", Effect::Read, Access::Anyone),
        ("sources.refresh", Effect::Write, Access::UserAction),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, effect);
        assert_eq!(spec.access, policy);
        let entry = arr(&catalog()["commands"])
            .into_iter()
            .find(|entry| entry["name"] == name)
            .unwrap();
        assert_eq!(entry["readOnly"], effect == Effect::Read);
        assert_eq!(entry["appChat"], policy == Access::Anyone);
    }
    let status = invoke(
        &repo,
        &json!({"name":"sources.status","args":{}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(arr(&status["sources"]).len(), 3);
    let github = arr(&status["sources"])
        .into_iter()
        .find(|source| source["id"] == "github")
        .unwrap();
    assert_eq!(github["syncEnabled"], true);
    assert_eq!(github["lastSuccess"], 123.0);
    assert_eq!(github["nextAttempt"], 456.0);
    assert_eq!(github["failures"], 2);
    assert_eq!(github["hasError"], true);
    assert_eq!(github["error"], "Failed [redacted]");
    assert!(!status.to_string().contains("fixture-private"));
    assert_eq!(github.as_object().unwrap().len(), 9);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    assert!(repo.running.lock().unwrap().is_empty());
}
#[test]
fn sentry_environment_read_is_cached_bounded_and_preserves_unknown_membership() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, false).unwrap();
    repo.transaction(|state| {
            state["sources"]["sentry"]=json!({"disabled":true,"failures":2,"nextAttempt":9_999_999_999_u64,"lastSuccess":42,"error":"private-source-error"});
            state["sentryEnvNextAttempt"]=json!(9_999_999_998_u64);
            state["sentryEnvironments"]=json!({
                "production":{"itemIds":(0..205).map(|n|format!("sentry:qa:{n}")).collect::<Vec<_>>(),"running":true,"lastSuccess":40,"nextAttempt":9_999_999_997_u64,"error":"private-cache-error","failures":1,"extension":"private-extension"},
                "pending":{"running":true},"empty":{"itemIds":[],"lastSuccess":41}});
            Ok(())
        }).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    let read = |args| {
        invoke(
            &repo,
            &json!({"name":"sentry.environments.read","args":args}),
            Caller::Assistant,
        )
        .unwrap()
    };
    let summary = read(json!({}));
    assert_eq!(summary["scope"], "cached");
    assert_eq!(arr(&summary["environments"]).len(), 3);
    assert_eq!(summary["source"]["syncEnabled"], false);
    assert_eq!(summary["source"]["hasError"], true);
    assert!(summary["selection"].is_null());
    assert!(!summary.to_string().contains("private-"));
    let rows = arr(&summary["environments"]);
    let production = rows
        .iter()
        .find(|row| row["environment"] == "production")
        .unwrap();
    assert_eq!(production["refreshing"], true);
    assert_eq!(production["cachedMemberCount"], 205);
    assert_eq!(production["lastSuccess"], 40.0);
    let page = read(json!({"environment":"production"}));
    assert_eq!(arr(&page["selection"]["itemIds"]).len(), 100);
    assert_eq!(page["selection"]["nextOffset"], 100);
    assert_eq!(page["selection"]["total"], 205);
    assert_eq!(page["selection"]["itemIds"][0], "sentry:qa:0");
    let last = read(json!({"environment":"production","offset":200}));
    assert_eq!(arr(&last["selection"]["itemIds"]).len(), 5);
    assert!(last["selection"]["nextOffset"].is_null());
    for name in ["pending", "missing"] {
        let row = read(json!({"environment":name}))["selection"].clone();
        assert_eq!(row["membershipKnown"], false);
        assert!(row["total"].is_null());
        assert_eq!(row["itemIds"], json!([]));
        assert_eq!(row["cached"], name == "pending");
    }
    let empty = read(json!({"environment":"empty"}));
    assert_eq!(empty["selection"]["membershipKnown"], true);
    assert_eq!(empty["selection"]["total"], 0);
    for args in [
        json!({"environment":""}),
        json!({"environment":"bad/name"}),
        json!({"offset":0}),
        json!({"environment":"production","offset":2001}),
        json!({"refresh":true}),
    ] {
        assert!(invoke(
            &repo,
            &json!({"name":"sentry.environments.read","args":args}),
            Caller::Assistant
        )
        .is_err());
    }
    let entry = arr(&catalog()["commands"])
        .into_iter()
        .find(|entry| entry["name"] == "sentry.environments.read")
        .unwrap();
    assert_eq!(entry["readOnly"], true);
    assert_eq!(entry["appChat"], true);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert!(repo.running.lock().unwrap().is_empty());
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn sentry_environment_read_rejects_oversize_malformed_and_redacted_identity() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "SENTRY_READ_SECRET=private-environment-secret\n",
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for caches in [
        json!({"private-environment-secret":{"itemIds":[]}}),
        json!({"qa":{"itemIds":["sentry:org:private-environment-secret"]}}),
        json!({"qa":{"itemIds":[3]}}),
        json!({"qa":{"itemIds":null}}),
        json!({"qa":{"itemIds":(0..2001).map(|n|format!("sentry:qa:{n}")).collect::<Vec<_>>()}}),
        json!((0..21)
            .map(|n| (format!("env{n}"), json!({})))
            .collect::<serde_json::Map<_, _>>()),
    ] {
        repo.transaction(|state| {
            state["sentryEnvironments"] = caches.clone();
            Ok(())
        })
        .unwrap();
        let before = repo.state.lock().unwrap().clone();
        assert!(invoke(
            &repo,
            &json!({"name":"sentry.environments.read","args":{"environment":"qa"}}),
            Caller::Assistant
        )
        .is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
    }
    repo.transaction(|state| {
        state["sentryEnvironments"] =
            json!({"qa":{"lastSuccess":"private-time","nextAttempt":-1,"failures":-1}});
        Ok(())
    })
    .unwrap();
    let result = invoke(
        &repo,
        &json!({"name":"sentry.environments.read","args":{}}),
        Caller::Assistant,
    )
    .unwrap();
    let row = &result["environments"][0];
    assert!(row["lastSuccess"].is_null());
    assert!(row["nextAttempt"].is_null());
    assert_eq!(row["failures"], 0);
}

#[test]
fn preparation_commands_enforce_policy_and_exact_argument_shapes_before_effects() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    let kinds = [
        "quick",
        "pr-review",
        "review",
        "skill",
        "recover",
        "new",
        "attach",
        "view",
    ];
    for kind in kinds {
        let name = format!("chats.prepare.{kind}");
        let entry = arr(&catalog()["commands"])
            .into_iter()
            .find(|row| row["name"] == name)
            .unwrap();
        assert_eq!(entry["readOnly"], false);
        assert_eq!(entry["appChat"], false);
        assert!(
            invoke(&repo, &json!({"name":name,"args":{}}), Caller::Assistant)
                .unwrap_err()
                .message
                .contains("requires a user action")
        );
        assert!(invoke(
            &repo,
            &json!({"name":name,"args":{"requestId":uuid(),"privateOverride":true}}),
            Caller::Browser
        )
        .is_err());
    }
    let id = uuid();
    let unknown = invoke(
        &repo,
        &json!({"name":"chats.preparation.read","args":{"requestId":id}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(unknown, json!({"requestId":id,"status":"not-found"}));
    for args in [
        json!({}),
        json!({"requestId":"not-a-uuid"}),
        json!({"requestId":uuid(),"retry":true}),
    ] {
        assert!(invoke(
            &repo,
            &json!({"name":"chats.preparation.read","args":args}),
            Caller::Assistant
        )
        .is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}
#[test]
fn preparation_command_receipts_are_local_only_and_recoverable_without_effects() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let request_id = uuid();
    let args = json!({"requestId":request_id,"agent":"codex","title":"Preparation command QA","cwd":temp.path(),"destination":"embedded","instruction":"Do not launch this fixture","itemIds":[]});
    let result = invoke(
        &repo,
        &json!({"name":"chats.prepare.new","args":args}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(result["status"], "prepared");
    assert_eq!(result["requestId"], request_id);
    assert_eq!(result["kind"], "new");
    assert_eq!(result["reused"], false);
    assert!(result["recordId"].is_string());
    assert_eq!(result.as_object().unwrap().len(), 5);
    let state = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        invoke(
            &repo,
            &json!({"name":"chats.prepare.new","args":args}),
            Caller::Browser
        )
        .unwrap(),
        result
    );
    assert_eq!(
        invoke(
            &repo,
            &json!({"name":"chats.preparation.read","args":{"requestId":request_id}}),
            Caller::Assistant
        )
        .unwrap(),
        result
    );
    let mut changed = args.clone();
    changed["title"] = json!("Changed captured intent");
    assert_eq!(
        invoke(
            &repo,
            &json!({"name":"chats.prepare.new","args":changed}),
            Caller::Browser
        )
        .unwrap_err()
        .status,
        409
    );
    let mut edit = args.clone();
    edit["id"] = result["recordId"].clone();
    assert!(invoke(
        &repo,
        &json!({"name":"chats.prepare.new","args":edit}),
        Caller::Browser
    )
    .is_err());
    let mut attach = args;
    attach["sessionId"] = json!(uuid());
    assert!(invoke(
        &repo,
        &json!({"name":"chats.prepare.new","args":attach}),
        Caller::Browser
    )
    .is_err());
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}

#[test]
fn chat_metadata_commands_validate_and_enforce_the_user_action_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|state|{state["sessions"]=json!([{"id":id,"agent":"codex","title":"Captured","cwd":temp.path(),"instruction":"Reference only","destination":"embedded","sessionId":null,"items":[]}]);Ok(())}).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    for (name, effect, policy) in [
        ("chats.metadata.read", Effect::Read, Access::Anyone),
        ("chats.metadata.update", Effect::Write, Access::UserAction),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, effect);
        assert_eq!(spec.access, policy);
    }
    let read = invoke(
        &repo,
        &json!({"name":"chats.metadata.read","args":{"recordId":id}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(read["recordId"], id);
    assert_eq!(read["chat"]["title"], "Captured");
    assert_eq!(read["expected"].as_object().unwrap().len(), 6);
    let args = json!({"recordId":id,"patch":{"title":"Edited"},"expected":{"title":read["expected"]["title"]}});
    assert!(invoke(
        &repo,
        &json!({"name":"chats.metadata.update","args":args}),
        Caller::Assistant
    )
    .unwrap_err()
    .message
    .contains("requires a user action"));
    for (name, args) in [
        ("chats.metadata.read", json!({"recordId":id,"patch":{}})),
        ("chats.metadata.read", json!({"recordId":"bad"})),
        (
            "chats.metadata.update",
            json!({"recordId":id,"patch":{"agent":"claude"},"expected":{}}),
        ),
        (
            "chats.metadata.update",
            json!({"recordId":id,"patch":{"title":"Edited"},"expected":{"title":"bad"}}),
        ),
        (
            "chats.metadata.update",
            json!({"recordId":id,"patch":{},"expected":{}}),
        ),
        (
            "chats.metadata.update",
            json!({"recordId":id,"patch":{"title":"Edited"},"expected":{"title":read["expected"]["title"]},"sessionId":uuid()}),
        ),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
    assert!(repo.cooldown.lock().unwrap().is_empty());
}

#[test]
fn chat_metadata_command_merges_current_unpatched_fields_and_conflicts_on_a_stale_patch() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|state|{state["sessions"]=json!([{"id":id,"agent":"codex","title":"Captured","cwd":temp.path(),"instruction":"Reference only","destination":"embedded","sessionId":null,"items":[],"privateExtension":{"keep":true}}]);state["itemSettings"]["untouched"]=json!({"cwd":"preserve"});Ok(())}).unwrap();
    let read = invoke(
        &repo,
        &json!({"name":"chats.metadata.read","args":{"recordId":id}}),
        Caller::Assistant,
    )
    .unwrap();
    let args = json!({"recordId":id,"patch":{"title":"Edited"},"expected":{"title":read["expected"]["title"]}});
    repo.transaction(|state| {
        state["sessions"][0]["runtime"] = json!({"state":"working","extension":"latest"});
        state["sessions"][0]["instruction"] = json!("New current instruction");
        Ok(())
    })
    .unwrap();
    let result = invoke(
        &repo,
        &json!({"name":"chats.metadata.update","args":args}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(result["recordId"], id);
    assert_eq!(result["saved"], true);
    assert_eq!(result["changedFields"], json!(["title"]));
    assert!(!result.to_string().contains("privateExtension"));
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(state["sessions"][0]["title"], "Edited");
    assert_eq!(
        state["sessions"][0]["instruction"],
        "New current instruction"
    );
    assert_eq!(state["sessions"][0]["runtime"]["extension"], "latest");
    assert_eq!(
        state["sessions"][0]["privateExtension"],
        json!({"keep":true})
    );
    assert_eq!(state["itemSettings"]["untouched"]["cwd"], "preserve");
    let disk = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        invoke(
            &repo,
            &json!({"name":"chats.metadata.update","args":args}),
            Caller::Browser
        )
        .unwrap_err()
        .status,
        409
    );
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}

#[test]
fn every_preparation_command_delegates_its_owner_without_opening_an_agent() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("SKILL.md"), "Inert fixture").unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let settings = repo.settings.view();
    repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":"$fixture-review"}}),Approval::User).unwrap();
    repo.transaction(|state|{state["items"]=json!([{"id":"github:prepare-fixture","source":"github","kind":"Pull request","title":"Preparation fixture","url":"https://github.com/example/project/pull/1","project":"example/project","reference":"#1","status":"todo"}]);state["itemSettings"]["github:prepare-fixture"]=json!({"explicit":true,"cwd":temp.path()});state["skillPresets"]=json!([{"id":"prepare-fixture","title":"Fixture skill","cwd":temp.path(),"skill":"SKILL.md","reports":"reports","agent":"codex","command":"$fixture"}]);Ok(())}).unwrap();
    let original=super::super::chat_preparation::save(&repo,super::super::chat_preparation::Kind::New,&json!({"agent":"codex","title":"Original fixture","cwd":temp.path(),"destination":"embedded","instruction":"Context only","itemIds":[]})).unwrap();
    for (kind, mut args) in [
        (
            "quick",
            json!({"agent":"codex","itemId":"github:prepare-fixture"}),
        ),
        (
            "pr-review",
            json!({"agent":"codex","itemId":"github:prepare-fixture"}),
        ),
        ("review", json!({"agent":"codex"})),
        ("skill", json!({"presetId":"prepare-fixture"})),
        ("recover", json!({"id":original["id"]})),
        (
            "new",
            json!({"agent":"codex","title":"New fixture","cwd":temp.path(),"destination":"embedded","instruction":"Do not launch","itemIds":[]}),
        ),
        (
            "attach",
            json!({"agent":"codex","sessionId":uuid(),"title":"Existing conversation fixture","cwd":temp.path(),"destination":"embedded"}),
        ),
        (
            "view",
            json!({"agent":"codex","source":"github","question":"Inspect these items","itemIds":["github:prepare-fixture"],"filters":{},"folderPath":temp.path()}),
        ),
    ] {
        args["requestId"] = json!(uuid());
        let name = format!("chats.prepare.{kind}");
        let result = invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).unwrap();
        assert_eq!(result["kind"], kind);
        assert_eq!(result["status"], "prepared");
        assert_eq!(result["requestId"], args["requestId"]);
        let before = repo.state.lock().unwrap().clone();
        let disk = std::fs::read(&repo.data).unwrap();
        assert_eq!(
            invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).unwrap(),
            result
        );
        assert_eq!(
            invoke(
                &repo,
                &json!({"name":"chats.preparation.read","args":{"requestId":args["requestId"]}}),
                Caller::Assistant
            )
            .unwrap(),
            result
        );
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert_eq!(
            repo.terminals.inventory().unwrap().rows(),
            [] as [serde_json::Value; 0]
        );
    }
}

#[test]
fn source_refresh_denial_and_argument_validation_precede_owner_mutations() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, false).unwrap();
    // Even the policy mutation test cannot make a provider request.
    repo.transaction(|state| {
        for id in ["github", "linear", "sentry"] {
            state["sources"][id]["disabled"] = json!(true);
        }
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let error = invoke(
        &repo,
        &json!({"name":"sources.refresh","args":{"force":true}}),
        Caller::Assistant,
    )
    .unwrap_err();
    assert!(error.message.contains("requires a user action"));
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert!(repo.cooldown.lock().unwrap().is_empty());
    assert!(repo.running.lock().unwrap().is_empty());
    for (name, args) in [
        ("sources.status", json!({"force":true})),
        ("sources.refresh", json!({"force":"true"})),
        ("sources.refresh", json!({"force":null})),
        ("sources.refresh", json!({"source":"github"})),
        (
            "sources.refresh",
            json!({"force":false,"token":"unsupported"}),
        ),
    ] {
        assert!(
            invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser)
                .unwrap_err()
                .message
                .starts_with("Invalid command arguments:")
        );
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert!(repo.cooldown.lock().unwrap().is_empty());
        assert!(repo.running.lock().unwrap().is_empty());
    }
    let demo_temp = tempfile::tempdir().unwrap();
    let demo = Repository::open(demo_temp.path().into(), None, true).unwrap();
    let before = demo.state.lock().unwrap().clone();
    for (args, force) in [(json!({}), false), (json!({"force":true}), true)] {
        let receipt = invoke(
            &demo,
            &json!({"name":"sources.refresh","args":args}),
            Caller::Browser,
        )
        .unwrap();
        assert_eq!(receipt["status"], "skipped");
        assert_eq!(receipt["force"], force);
        assert_eq!(arr(&receipt["results"]).len(), 3);
        assert!(arr(&receipt["results"])
            .iter()
            .all(|row| row["status"] == "skipped" && row["reason"] == "demo"));
        assert!(text(&receipt, "message").contains("no external requests"));
        assert_eq!(*demo.state.lock().unwrap(), before);
        assert!(demo.cooldown.lock().unwrap().is_empty());
        assert!(demo.running.lock().unwrap().is_empty());
    }
}

#[test]
fn chat_metadata_commands_are_reversible_allowed_and_return_only_updated_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let chat = repo.transaction(|state| super::super::sessions::save(state, &json!({"agent":"codex","title":"Original","cwd":temp.path(),"itemIds":[],"instruction":"fixture-private-instructions"}))).unwrap();
    let id = text(&chat, "id").to_owned();
    repo.transaction(|state| {
        state["sessions"][0]["privateExtension"] = json!({"value":"fixture-private-extension"});
        Ok(())
    })
    .unwrap();
    let original = repo.state.lock().unwrap().clone();
    for (name, args) in [
        (
            "chats.rename",
            json!({"id":id.replace('-',""),"title":"  Renamed  "}),
        ),
        ("chats.visibility", json!({"id":id,"hidden":true})),
        ("chats.visibility", json!({"id":id,"hidden":false})),
        ("chats.archive", json!({"id":id,"archived":true})),
        ("chats.archive", json!({"id":id,"archived":false})),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, Effect::Write);
        assert_eq!(spec.access, Access::Anyone);
        let result = invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant).unwrap();
        let row = repo.state.lock().unwrap()["sessions"][0].clone();
        assert_eq!(result.as_object().unwrap().len(), 2);
        assert_eq!(result["chat"].as_object().unwrap().len(), 5);
        assert_eq!(result["scope"], "local");
        assert_eq!(result["chat"]["id"], id);
        assert_eq!(result["chat"]["title"], "Renamed");
        assert_eq!(
            result["chat"]["hidden"],
            row["hidden"].as_bool().unwrap_or(false)
        );
        assert_eq!(result["chat"]["archivedAt"], row["archivedAt"]);
        assert_eq!(row["instruction"], original["sessions"][0]["instruction"]);
        assert_eq!(
            row["privateExtension"],
            original["sessions"][0]["privateExtension"]
        );
        assert!(!result.to_string().contains("fixture-private"));
        if name == "chats.archive" {
            assert_eq!(
                result["chat"]["archivedAt"].is_null(),
                args["archived"] == false
            );
        }
        if name == "chats.visibility" {
            assert_eq!(result["chat"]["hidden"], args["hidden"]);
        }
    }
    let state = repo.state.lock().unwrap();
    assert!(state["sessions"][0]["archivedAt"].is_null());
    assert_eq!(state["sessions"][0]["hidden"], false);
    for (key, value) in original.as_object().unwrap() {
        if key != "sessions" {
            assert_eq!(&state[key], value);
        }
    }
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
    assert_eq!(
        repo.shells.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}
#[test]
fn chat_metadata_commands_reject_bad_targets_and_unknown_fields_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let chat = repo
        .transaction(|state| {
            super::super::sessions::save(
                state,
                &json!({"agent":"codex","title":"Original","cwd":temp.path(),"itemIds":[]}),
            )
        })
        .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    for (name, args) in [
        ("chats.rename", json!({"id":chat["id"],"title":" "})),
        (
            "chats.rename",
            json!({"id":chat["id"],"title":"x".repeat(181)}),
        ),
        (
            "chats.rename",
            json!({"id":chat["id"],"title":"Other","command":"execute"}),
        ),
        ("chats.visibility", json!({"id":chat["id"],"hidden":"true"})),
        ("chats.visibility", json!({"id":chat["id"]})),
        (
            "chats.archive",
            json!({"id":chat["id"],"archived":true,"close":true}),
        ),
        ("chats.archive", json!({"id":uuid(),"archived":true})),
        ("chats.archive", json!({"id":"invalid","archived":true})),
        (
            "chats.visibility",
            json!({"id":"x".repeat(81),"hidden":true}),
        ),
    ] {
        assert!(invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
}

fn assert_assistant_cannot_write_skills(repo: &Arc<Repository>, preset: &Value) {
    let before = repo.state.lock().unwrap().clone();
    for (name, args) in [
        ("skills.save", preset.clone()),
        (
            "skills.schedule",
            json!({"presetId":"fixture","mode":"interval","everyMinutes":5,"missed":"skip","enabled":true}),
        ),
        ("skills.run", json!({"presetId":"fixture"})),
    ] {
        let error = invoke(repo, &json!({"name":name,"args":args}), Caller::Assistant).unwrap_err();
        assert!(error.message.contains("requires a user action"));
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert!(repo.cooldown.lock().unwrap().is_empty());
    }
}
#[test]
fn skill_commands_return_saved_identities_and_truthful_demo_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    std::fs::write(temp.path().join("SKILL.md"), "Fixture skill").unwrap();
    let preset = json!({"title":"Fixture","cwd":temp.path(),"skill":"SKILL.md","command":"$fixture","agent":"codex"});
    assert_assistant_cannot_write_skills(&repo, &preset);
    let browser =
        |name: &str, args: Value| invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser);
    let saved = browser("skills.save", preset).unwrap();
    assert_eq!(saved["saved"], true);
    assert_eq!(saved["presetId"], saved["preset"]["id"]);
    let preset_id = saved["presetId"].clone();
    let stored = arr(&repo.state.lock().unwrap()["skillPresets"]);
    let stored = stored
        .iter()
        .find(|entry| entry["id"] == preset_id)
        .unwrap();
    for key in [
        "id",
        "title",
        "cwd",
        "skill",
        "command",
        "agent",
        "description",
    ] {
        assert_eq!(stored[key], saved["preset"][key]);
    }
    assert_eq!(stored["reports"], "reports");
    assert!(saved["preset"].get("reports").is_none());
    let baseline = browser("skills.preset.read", json!({"presetId":preset_id})).unwrap();
    let patch = json!({"mode":"interval","everyMinutes":5,"missed":"skip","enabled":true});
    let mut expected = json!({});
    for key in patch.as_object().unwrap().keys() {
        expected[key] = baseline["scheduleExpected"][key].clone();
    }
    let schedule = browser(
        "skills.schedule",
        json!({"presetId":preset_id,"patch":patch,"expected":expected}),
    )
    .unwrap();
    assert_eq!(schedule["saved"], true);
    assert_eq!(schedule["presetId"], preset_id);
    let raw = repo.state.lock().unwrap()["skillSchedules"][text(&saved, "presetId")].clone();
    for key in [
        "mode",
        "time",
        "everyMinutes",
        "missed",
        "enabled",
        "nextRun",
    ] {
        assert_eq!(raw[key], schedule["schedule"][key]);
    }
    assert!(schedule["schedule"].get("updatedAt").is_none());
    assert!(schedule["schedule"]["nextRun"].as_f64().unwrap() > now());
    for args in [
        json!({}),
        json!({"presetId":" "}),
        json!({"presetId":preset_id,"command":"execute"}),
        json!({"presetId":"x".repeat(81)}),
    ] {
        assert!(browser("skills.run", args)
            .unwrap_err()
            .message
            .starts_with("Invalid command arguments:"));
    }
    let receipt = browser("skills.run", json!({"presetId":preset_id})).unwrap();
    assert_eq!(receipt["presetId"], preset_id);
    assert_eq!(receipt["status"], "preview");
    assert_eq!(receipt.as_object().unwrap().len(), 6);
    let state = repo.state.lock().unwrap();
    let runs = arr(&state["skillRuns"]);
    let run = runs
        .iter()
        .find(|run| run["id"] == receipt["runId"])
        .unwrap();
    assert_eq!(run["outcome"], "preview");
    assert_eq!(run["sessionId"], receipt["recordId"]);
    assert!(arr(&state["sessions"])
        .iter()
        .any(|record| record["id"] == receipt["recordId"]
            && record["newsletterId"] == preset_id
            && record.get("runReceipt").is_none()));
    drop(state);
    assert_eq!(
        repo.terminals.inventory().unwrap().rows(),
        [] as [serde_json::Value; 0]
    );
}

#[test]
fn sharing_commands_enforce_policy_before_owner_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    let state = repo.state.lock().unwrap().clone();
    let sharing = repo.sharing.lock().unwrap().clone();
    let cooldown = repo.cooldown.lock().unwrap().clone();
    let busy = repo.sharing_busy.lock().unwrap().clone();
    let base = repo.data.parent().unwrap().join("sharing");
    let before = base.join("state.json").exists();
    for (name, args, effect) in [
        ("sharing.state", json!({}), Effect::Read),
        ("sharing.job", json!({"id":id}), Effect::Read),
        (
            "sharing.prepare",
            json!({"recordId":id,"recentConversation":true}),
            Effect::Write,
        ),
        (
            "sharing.send",
            json!({"draftId":id,"recipients":["sam"],"message":"Hello"}),
            Effect::Write,
        ),
        ("sharing.preview", json!({"deliveryId":id}), Effect::Write),
        (
            "sharing.import",
            json!({"deliveryId":id,"agent":"codex","cwd":temp.path()}),
            Effect::Write,
        ),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, effect);
        assert_eq!(spec.access, Access::UserAction);
        validate_arguments(&spec.args, &args, "args").unwrap();
        let error =
            invoke(&repo, &json!({"name":name,"args":args}), Caller::Assistant).unwrap_err();
        assert_eq!(
            error.message,
            "This command requires a user action. It cannot run from App chat."
        );
    }
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(*repo.sharing.lock().unwrap(), sharing);
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
    assert_eq!(*repo.sharing_busy.lock().unwrap(), busy);
    assert_eq!(base.join("state.json").exists(), before);
    assert!(!base.join("jobs").exists());
}

#[test]
fn sharing_commands_reject_invalid_arguments_before_owner_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    let sharing = repo.sharing.lock().unwrap().clone();
    let cases = [
        ("sharing.state", json!({"refresh":true})),
        ("sharing.job", json!({})),
        ("sharing.job", json!({"id":"x".repeat(81)})),
        (
            "sharing.prepare",
            json!({"recordId":id,"recentConversation":"yes"}),
        ),
        (
            "sharing.prepare",
            json!({"recordId":id,"command":"execute"}),
        ),
        ("sharing.send", json!({"draftId":id,"recipients":[]})),
        (
            "sharing.send",
            json!({"draftId":id,"recipients":vec!["sam";11]}),
        ),
        (
            "sharing.send",
            json!({"draftId":id,"recipients":["sam","sam"]}),
        ),
        (
            "sharing.send",
            json!({"draftId":id,"recipients":["../sam"]}),
        ),
        (
            "sharing.send",
            json!({"draftId":id,"recipients":["sam"],"message":"x".repeat(4001)}),
        ),
        (
            "sharing.send",
            json!({"draftId":id,"recipients":["sam"],"retryId":false}),
        ),
        ("sharing.preview", json!({"deliveryId":id,"import":true})),
        (
            "sharing.import",
            json!({"deliveryId":id,"agent":"shell","cwd":temp.path()}),
        ),
        (
            "sharing.import",
            json!({"deliveryId":id,"agent":"codex","cwd":" "}),
        ),
        ("sharing.dismiss", json!({"id":id})),
        ("sharing.dismiss", json!({"id":id,"dismissed":"true"})),
    ];
    for (name, args) in cases {
        let error = invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser).unwrap_err();
        assert!(
            error.message.starts_with("Invalid command arguments: args"),
            "{name}: {}",
            error.message
        );
        assert_eq!(*repo.sharing.lock().unwrap(), sharing);
        assert!(repo.cooldown.lock().unwrap().is_empty());
        assert!(repo.sharing_busy.lock().unwrap().is_empty());
    }
}

#[test]
fn sharing_commands_read_known_private_job_and_dismiss_cached_entry() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    let job_id = uuid();
    {
        let mut sharing = repo.sharing.lock().unwrap();
        sharing["entries"][&id] = json!({"id":id,"title":"Fixture handover","read":false,"dismissed":false,"privateExtension":"fixture-private-inbox-extension"});
        sharing["jobs"][&job_id] =
            json!({"id":job_id,"kind":"prepare","status":"done","hasPreview":true});
    }
    let folder = repo.data.parent().unwrap().join("sharing/jobs");
    std::fs::create_dir_all(&folder).unwrap();
    let preview = json!({"conversation":"fixture private conversation"});
    std::fs::write(
        folder.join(format!("{job_id}.preview.json")),
        serde_json::to_vec(&preview).unwrap(),
    )
    .unwrap();
    let before = repo.sharing.lock().unwrap().clone();
    let job = invoke(
        &repo,
        &json!({"name":"sharing.job","args":{"id":job_id}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(job["id"], job_id);
    assert_eq!(job["preview"], preview);
    assert_eq!(*repo.sharing.lock().unwrap(), before);
    let full = invoke(
        &repo,
        &json!({"name":"sharing.state","args":{}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(full, super::super::sharing::view(&repo));
    assert_eq!(arr(&full["entries"]).len(), 1);
    assert!(!full.to_string().contains("fixture private conversation"));
    let spec = table()
        .into_iter()
        .find(|spec| spec.name == "sharing.dismiss")
        .unwrap();
    assert_eq!(spec.effect, Effect::Write);
    assert_eq!(spec.access, Access::Anyone);
    for dismissed in [true, false] {
        let response = invoke(
            &repo,
            &json!({"name":"sharing.dismiss","args":{"id":id,"dismissed":dismissed}}),
            Caller::Assistant,
        )
        .unwrap();
        assert_eq!(response, json!({"id":id,"dismissed":dismissed}));
        assert!(!response
            .to_string()
            .contains("fixture-private-inbox-extension"));
        assert_eq!(
            repo.sharing.lock().unwrap()["entries"][&id]["dismissed"],
            dismissed
        );
        let saved: Value = serde_json::from_slice(
            &std::fs::read(repo.data.parent().unwrap().join("sharing/state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["entries"][&id]["dismissed"], dismissed);
        assert_eq!(saved["jobs"], before["jobs"]);
        assert_eq!(
            saved["entries"][&id]["privateExtension"],
            "fixture-private-inbox-extension"
        );
    }
    let user_response = invoke(
        &repo,
        &json!({"name":"sharing.dismiss","args":{"id":id,"dismissed":true}}),
        Caller::Browser,
    )
    .unwrap();
    assert_eq!(
        arr(&user_response["entries"])[0]["privateExtension"],
        "fixture-private-inbox-extension"
    );
    assert_eq!(arr(&user_response["entries"])[0]["dismissed"], true);
}

#[test]
fn guidance_commands_inspect_and_change_only_routine_tip_preferences() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {state["guidance"]=json!({"version":1,"revision":0,"enabled":true,"tips":{},"extension":"benign-private-guidance-extension"});Ok(())}).unwrap();
    for (name, effect) in [
        ("guidance.read", Effect::Read),
        ("guidance.apply", Effect::Write),
    ] {
        let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
        assert_eq!(spec.effect, effect);
        assert_eq!(spec.access, Access::Anyone);
        let entry = arr(&catalog()["commands"])
            .into_iter()
            .find(|entry| entry["name"] == name)
            .unwrap();
        assert_eq!(entry["readOnly"], effect == Effect::Read);
        assert_eq!(entry["appChat"], true);
    }
    let state = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let read = invoke(
        &repo,
        &json!({"name":"guidance.read","args":{}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(read, super::super::guidance::view(&repo).unwrap());
    assert!(!read.to_string().contains("benign-private"));
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    for args in [
        json!({"revision":0,"action":"complete","id":"chat.shell"}),
        json!({"revision":0,"action":"dismiss","id":"invented"}),
        json!({"revision":0,"action":"enabled"}),
        json!({"revision":0,"action":"enabled","enabled":false,"id":"chat.shell"}),
        json!({"revision":0,"action":"replay","extra":true}),
    ] {
        assert!(invoke(
            &repo,
            &json!({"name":"guidance.apply","args":args}),
            Caller::Assistant
        )
        .is_err());
        assert_eq!(*repo.state.lock().unwrap(), state);
    }
    assert!(invoke(
        &repo,
        &json!({"name":"guidance.read","args":{"path":"/not-accepted"}}),
        Caller::Assistant
    )
    .is_err());
    let disabled = invoke(
        &repo,
        &json!({"name":"guidance.apply","args":{"revision":0,"action":"enabled","enabled":false}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["revision"], 1);
    let saved = std::fs::read(&repo.data).unwrap();
    assert_eq!(
        invoke(
            &repo,
            &json!({"name":"guidance.apply","args":{"revision":0,"action":"replay"}}),
            Caller::Assistant
        )
        .unwrap_err()
        .status,
        409
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), saved);
    let dismissed=invoke(&repo,&json!({"name":"guidance.apply","args":{"revision":1,"action":"dismiss","id":"chat.shell"}}),Caller::Browser).unwrap();
    assert_eq!(dismissed["tips"]["chat.shell"], "dismissed");
    let replay = invoke(
        &repo,
        &json!({"name":"guidance.apply","args":{"revision":2,"action":"replay"}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(replay["enabled"], true);
    assert_eq!(replay["tips"], json!({}));
    assert!(!replay.to_string().contains("benign-private"));
    let final_state = repo.state.lock().unwrap().clone();
    assert_eq!(
        final_state["guidance"]["extension"],
        state["guidance"]["extension"]
    );
    for key in ["items", "sessions", "notes", "onboarding"] {
        assert_eq!(final_state[key], state[key]);
    }
}

fn assert_strict_read_only_command(repo: &Arc<Repository>, catalog: &Value, name: &str) {
    let spec = table().into_iter().find(|spec| spec.name == name).unwrap();
    assert_eq!(spec.effect, Effect::Read);
    assert_eq!(spec.access, Access::Anyone);
    let row = arr(&catalog["commands"])
        .into_iter()
        .find(|row| row["name"] == name)
        .unwrap();
    assert_eq!(row["readOnly"], true);
    assert_eq!(row["appChat"], true);
    let request = json!({"name":name,"args":{}});
    assert_eq!(
        invoke(repo, &request, Caller::Browser).unwrap(),
        invoke(repo, &request, Caller::Assistant).unwrap()
    );
    assert!(invoke(
        repo,
        &json!({"name":name,"args":{"path":"/not-accepted"}}),
        Caller::Assistant
    )
    .is_err());
}
#[test]
fn theme_and_configuration_status_commands_are_discovered_strict_and_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    let custom = json!({"displayName":"Custom fixture theme","type":"dark","colors":{"editor.background":"#123456"},"tokenColors":[],"privateExtension":"benign-private-theme-content"});
    let path = repo
        .settings
        .config_directory()
        .join("themes/custom-fixture.json");
    std::fs::write(&path, serde_json::to_vec(&custom).unwrap()).unwrap();
    let state = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let catalog = catalog();
    for name in ["themes.list", "config.status"] {
        assert_strict_read_only_command(&repo, &catalog, name);
    }
    let result = operations::run(&repo, Caller::Assistant, "themes.list", &json!({})).unwrap();
    let rows = arr(&result["themes"]);
    assert!(rows.len() <= 32);
    let theme = rows
        .iter()
        .find(|row| row["id"] == "custom-fixture")
        .unwrap();
    assert_eq!(
        theme,
        &json!({"id":"custom-fixture","label":"Custom fixture theme","type":"dark"})
    );
    assert_eq!(result["version"], 1);
    assert!(!result.to_string().contains("benign-private"));
    assert!(!result.to_string().contains("#123456"));
    assert!(rows.iter().all(|row| row.as_object().unwrap().len() == 3));
    let projected = operations::run(&repo, Caller::Assistant, "config.status", &json!({})).unwrap();
    let owner = super::super::config_status::view(&repo);
    assert_eq!(projected.as_object().unwrap().len(), 2);
    assert_eq!(projected["version"], 1);
    assert_eq!(projected["resources"].as_object().unwrap().len(), 3);
    for key in ["settings", "views", "pageSettings"] {
        assert_eq!(
            projected["resources"][key]["contentHash"],
            owner["resources"][key]["token"]
        );
        assert_eq!(projected["resources"][key].as_object().unwrap().len(), 1);
        assert!(projected["resources"][key].get("token").is_none());
    }
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    let mut long = custom;
    long["displayName"] = json!("é".repeat(200));
    std::fs::write(path, serde_json::to_vec(&long).unwrap()).unwrap();
    let result = operations::run(&repo, Caller::Assistant, "themes.list", &json!({})).unwrap();
    let rows = arr(&result["themes"]);
    assert_eq!(
        text(
            rows.iter()
                .find(|row| row["id"] == "custom-fixture")
                .unwrap(),
            "label"
        )
        .chars()
        .count(),
        120
    );
    std::fs::write(
        repo.settings.config_directory().join("views.json"),
        "benign-private-invalid-config",
    )
    .unwrap();
    let status = operations::run(&repo, Caller::Assistant, "config.status", &json!({})).unwrap();
    assert_eq!(
        status["resources"]["views"],
        json!({"error":"Saved views configuration could not load."})
    );
    assert!(status["resources"]["settings"]["contentHash"].is_string());
    assert!(status["resources"]["pageSettings"]["contentHash"].is_string());
    assert!(!status.to_string().contains("benign-private"));
    std::fs::write(
        repo.settings
            .config_directory()
            .join("themes/custom-fixture.json"),
        "{malformed benign-private-theme-content",
    )
    .unwrap();
    let error = operations::run(&repo, Caller::Assistant, "themes.list", &json!({})).unwrap_err();
    assert!(!error.message.contains("benign-private"));
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}

#[test]
fn onboarding_read_is_discovered_read_only_and_matches_owner_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "ONBOARDING_TEST_SECRET=benign-private-setup-sentinel",
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
            state["onboarding"]=json!({"version":1,"revision":3,"step":"company","completed":false,"steps":{"theme":"complete","company":"pending"}});
            state["sources"]["linear"]["disabled"]=json!(true);
            Ok(())
        }).unwrap();
    let spec = table()
        .into_iter()
        .find(|spec| spec.name == "onboarding.read")
        .unwrap();
    assert_eq!(spec.effect, Effect::Read);
    assert_eq!(spec.access, Access::Anyone);
    let entry = arr(&catalog()["commands"])
        .into_iter()
        .find(|entry| entry["name"] == "onboarding.read")
        .unwrap();
    assert_eq!(entry["readOnly"], true);
    assert_eq!(entry["appChat"], true);
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let expected = super::super::onboarding::view(&repo);
    let result = invoke(
        &repo,
        &json!({"name":"onboarding.read","args":{}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(result, expected);
    assert_eq!(result["progress"]["step"], "company");
    assert!(result["assistants"].is_array());
    assert!(result["editors"].is_array());
    assert!(result["connections"].is_array());
    assert!(!result.to_string().contains("benign-private-setup-sentinel"));
    assert!(result.get("files").is_none());
    assert!(invoke(
        &repo,
        &json!({"name":"onboarding.read","args":{"path":"/not-accepted"}}),
        Caller::Assistant
    )
    .is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}

#[test]
fn registry_schemas_use_only_the_validated_subset() {
    fn check(schema: &Value) {
        let object = schema.as_object().unwrap();
        for keyword in object.keys() {
            assert!(
                matches!(
                    keyword.as_str(),
                    "type"
                        | "properties"
                        | "additionalProperties"
                        | "required"
                        | "enum"
                        | "minimum"
                        | "maximum"
                        | "maxLength"
                        | "pattern"
                        | "items"
                        | "minItems"
                        | "maxItems"
                        | "uniqueItems"
                        | "description"
                        | "default"
                        | "oneOf"
                ),
                "Unsupported schema keyword: {keyword}"
            );
        }
        let types = match &schema["type"] {
            Value::String(name) => vec![name.clone()],
            Value::Array(names) => names
                .iter()
                .map(|name| name.as_str().unwrap().to_owned())
                .collect(),
            _ => panic!("Every schema must declare its type."),
        };
        assert_ne!(types, [] as [std::string::String; 0]);
        assert!(types.iter().all(|name| matches!(
            name.as_str(),
            "object" | "array" | "string" | "integer" | "boolean" | "null"
        )));
        if let Some(properties) = schema["properties"].as_object() {
            for nested in properties.values() {
                check(nested);
            }
        }
        if let Some(items) = schema.get("items") {
            check(items);
        }
        if let Some(alternatives) = schema["oneOf"].as_array() {
            assert_ne!(alternatives.as_slice(), [] as [Value; 0]);
            for alternative in alternatives {
                check(alternative);
            }
        }
        if let Some(default) = schema.get("default") {
            validate_arguments(schema, default, "default").unwrap();
        }
    }
    for spec in table() {
        check(&spec.args);
    }
    assert!(validate_arguments(&json!({"type":"object","oneOf":[]}), &json!({}), "args").is_err());
}

fn invalid_command_arguments(repo: &Repository) -> Vec<(&'static str, Value)> {
    vec![
        ("commands.list", json!({"unknown":true})),
        ("connections.reload", json!({"provider":"unknown"})),
        (
            "connections.reload",
            json!({"provider":"github","activate":"true"}),
        ),
        (
            "connections.reload",
            json!({"provider":"github","token":"benign-not-accepted"}),
        ),
        ("diagnostics.logs", json!({"kind":"private"})),
        ("diagnostics.logs", json!({"limit":0})),
        ("diagnostics.logs", json!({"limit":101})),
        ("diagnostics.logs", json!({"limit":2.5})),
        ("diagnostics.logs", json!({"limit":"10"})),
        ("work.update", json!({"id":"item","pinned":"true"})),
        ("work.update", json!({"id":"item","status":"remote-closed"})),
        ("work.update", json!({"id":"item","action":"save-session"})),
        ("work.snooze", json!({"id":"item","mode":"forever"})),
        ("artifacts.list", json!({"limit":41})),
        (
            "artifacts.read-text",
            json!({"id":"111111111111111111111111","maxBytes":3}),
        ),
        (
            "artifacts.read-text",
            json!({"id":"111111111111111111111111","path":"/arbitrary"}),
        ),
        ("sharing.inbox", json!({"includeDismissed":"true"})),
        ("quick-actions.outputs", json!({"command":"not-accepted"})),
        ("work.query", json!({"offset":-1})),
        ("work.query", json!({"includeHidden":"true"})),
        ("work.query", json!({"source":"unknown"})),
        ("work.query", json!({"lifecycle":"unknown"})),
        ("work.query", json!({"query":"x".repeat(501)})),
        ("settings.apply", json!({"patch":{}})),
        (
            "settings.apply",
            json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":[]}),
        ),
        (
            "pages.settings.apply",
            json!({"revision":0,"page":"linear","scopes":[]}),
        ),
        (
            "pages.settings.apply",
            json!({"revision":0,"contentHash":"invalid","page":"linear","scopes":[]}),
        ),
        (
            "pages.settings.apply",
            json!({"revision":0,"contentHash":"0".repeat(64),"page":"linear","scopes":["same","same"]}),
        ),
        (
            "pages.settings.apply",
            json!({"revision":0,"contentHash":"0".repeat(64),"page":"linear","scopes":[true]}),
        ),
        (
            "pages.settings.apply",
            json!({"revision":0,"contentHash":"0".repeat(64),"page":"linear","scopes":["x".repeat(501)]}),
        ),
        (
            "pages.settings.apply",
            json!({"revision":0,"contentHash":"0".repeat(64),"page":"linear","scopes":(0..1001).map(|i|i.to_string()).collect::<Vec<_>>()}),
        ),
        (
            "chat-terminal.resize",
            json!({"id":"missing","cols":9,"rows":30}),
        ),
        (
            "chat-terminal.resize",
            json!({"id":"missing","cols":80,"rows":201}),
        ),
        (
            "skills.schedule",
            json!({"presetId":"missing","mode":"daily","missed":"skip","time":"noon"}),
        ),
    ]
}
#[test]
fn advertised_argument_constraints_reject_invalid_commands_before_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = repo.state.lock().unwrap().clone();
    for (name, args) in invalid_command_arguments(&repo) {
        let error = invoke(&repo, &json!({"name":name,"args":args}), Caller::Browser)
            .err()
            .unwrap();
        assert!(
            error.message.starts_with("Invalid command arguments:"),
            "{name}: {}",
            error.message
        );
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(invoke(
        &repo,
        &json!({"name":"work.query","args":{"includeHidden":true,"lifecycle":"any"}}),
        Caller::Assistant
    )
    .is_ok());
    let specs = table();
    let pages = specs
        .iter()
        .find(|s| s.name == "pages.settings.apply")
        .unwrap();
    assert!(validate_arguments(
        &pages.args,
        &json!({"revision":0,"contentHash":"0".repeat(64),"page":"linear","scopes":null}),
        "args"
    )
    .is_ok());
    let settings = specs.iter().find(|s| s.name == "settings.apply").unwrap();
    assert!(validate_arguments(
            &settings.args,
            &json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"theme":"system"}}),
            "args"
        )
        .is_ok());
}

#[test]
fn settings_preview_is_read_only_and_all_agent_results_keep_the_routine_projection() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"fixture","title":"Fixture","command":"printf private-command"}]}}),Approval::User).unwrap();
    let before = repo.settings.view();
    let result = invoke(&repo, &json!({"name":"settings.preview","args":{"revision":before["revision"],"contentHash":repo.settings.view()["contentHash"],"patch":{"density":"comfortable"}}}), Caller::Assistant).unwrap();
    assert_eq!(result["values"]["density"], "comfortable");
    assert!(result["values"].get("quickActions").is_none());
    assert_eq!(repo.settings.view()["values"], before["values"]);
    assert_eq!(repo.settings.view()["revision"], before["revision"]);
    assert!(invoke(&repo,&json!({"name":"settings.preview","args":{"revision":before["revision"],"contentHash":repo.settings.view()["contentHash"],"patch":{"editor":"code"}}}),Caller::Assistant).is_err());
    for result in [invoke(&repo,&json!({"name":"settings.read"}),Caller::Assistant).unwrap(),invoke(&repo,&json!({"name":"settings.apply","args":{"revision":before["revision"],"contentHash":repo.settings.view()["contentHash"],"patch":{"density":"comfortable"}}}),Caller::Assistant).unwrap()] {
            assert!(result["values"].get("quickActions").is_none());
            assert!(result["defaults"].get("editor").is_none());
            assert_eq!(result["approval"]["editor"], "user");
        }
}

#[test]
fn catalog_names_ids_policies_and_schemas_agree() {
    let specs = table();
    let catalog = catalog();
    let entries = arr(&catalog["commands"]);
    assert_eq!(entries.len(), specs.len());
    let mut names = std::collections::BTreeSet::new();
    let mut ids = vec![];
    for (spec, entry) in specs.iter().zip(entries) {
        assert!(names.insert(spec.name));
        assert!(!ids.contains(&spec.name));
        ids.push(spec.name);
        assert_eq!(entry["name"], spec.name);
        assert_eq!(entry["readOnly"], spec.effect == Effect::Read);
        assert_eq!(entry["appChat"], spec.access == Access::Anyone);
        assert_eq!(entry["args"], spec.args);
        assert_eq!(spec.args["type"], "object");
        let properties = spec.args["properties"].as_object().unwrap();
        let takes_arguments = !matches!(
            spec.name,
            "commands.list"
                | "themes.list"
                | "config.status"
                | "sources.status"
                | "guidance.read"
                | "diagnostics.storage"
                | "connections.status"
                | "onboarding.read"
                | "chats.list"
                | "settings.read"
                | "views.read"
                | "ui.presentations.read"
                | "pages.settings.read"
                | "sharing.read"
                | "sharing.state"
                | "skills.list"
                | "sharing.refresh"
                | "quick-actions.list"
        );
        assert_eq!(!properties.is_empty(), takes_arguments, "{}", spec.name);
        for required in arr(&spec.args["required"]) {
            assert!(properties.contains_key(required.as_str().unwrap()));
        }
    }
    let pages = specs
        .iter()
        .find(|s| s.name == "pages.settings.apply")
        .unwrap();
    assert_eq!(pages.effect, Effect::Write);
    assert_eq!(pages.access, Access::Anyone);
    let poll = specs
        .iter()
        .find(|s| s.name == "chat-terminal.poll")
        .unwrap();
    assert_eq!(poll.effect, Effect::Read);
    assert_eq!(poll.access, Access::UserAction);
}

#[test]
fn local_work_changes_and_bounded_reads_allow_the_assistant_without_execution() {
    let specs = table();
    for (id, effect) in [
        ("work.update", Effect::Write),
        ("work.snooze", Effect::Write),
        ("work.links.read", Effect::Read),
        ("work.links.add", Effect::Write),
        ("work.links.remove", Effect::Write),
        ("artifacts.list", Effect::Read),
        ("artifacts.read-text", Effect::Read),
        ("sharing.inbox", Effect::Read),
        ("quick-actions.outputs", Effect::Read),
    ] {
        let spec = specs.iter().find(|spec| spec.name == id).unwrap();
        assert_eq!(spec.effect, effect);
        assert_eq!(spec.access, Access::Anyone);
    }
    for id in [
        "quick-actions.run",
        "quick-actions.list",
        "sharing.refresh",
        "skills.schedule",
        "chat-terminal.input",
    ] {
        assert_eq!(
            specs.iter().find(|spec| spec.name == id).unwrap().access,
            Access::UserAction
        );
    }
}

#[test]
fn connection_reload_accepts_only_secret_free_user_requests() {
    let spec = table()
        .into_iter()
        .find(|spec| spec.name == "connections.reload")
        .unwrap();
    assert_eq!(spec.effect, Effect::Write);
    assert_eq!(spec.access, Access::UserAction);
    assert!(validate_arguments(&spec.args, &json!({"provider":"github"}), "args").is_ok());
    assert!(validate_arguments(
        &spec.args,
        &json!({"provider":"linear","activate":true}),
        "args"
    )
    .is_ok());
    assert!(validate_arguments(
        &spec.args,
        &json!({"provider":"sentry","activate":false}),
        "args"
    )
    .is_ok());
    assert!(validate_arguments(&spec.args, &json!({"activate":true}), "args").is_err());
    for field in ["token", "previewId", "expectedRevision"] {
        let mut args = json!({"provider":"github"});
        args[field] = json!("not-accepted");
        assert!(validate_arguments(&spec.args, &args, "args").is_err());
    }
}

#[test]
fn assistant_denial_precedes_every_restricted_handler() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = repo.state.lock().unwrap().clone();
    for spec in table().iter().filter(|s| s.access == Access::UserAction) {
        let result = invoke(
            &repo,
            &json!({"name":spec.name,"args":{}}),
            Caller::Assistant,
        );
        let Err(error) = result else {
            panic!("Restricted command was allowed.")
        };
        assert_eq!(
            error.message,
            "This command requires a user action. It cannot run from App chat."
        );
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    let error = invoke(&repo, &json!({"name":"settings.apply","args":{
            "revision":repo.settings.view()["revision"],"contentHash":repo.settings.view()["contentHash"],
            "patch":{"quickActions":[{"id":"unsafe","title":"Unsafe","command":"printf denied"}]}
        }}), Caller::Assistant).err().unwrap();
    assert!(error.message.contains("Ask the user to approve"));
}

#[test]
fn diagnostics_log_records_redact_before_unicode_display_limits() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = "benign-log-秘密-secret-sentinel";
    std::fs::write(
        temp.path().join(".env"),
        format!("LOG_TEST_SECRET={sentinel}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let path = repo.data.parent().unwrap().join("logs/service.log");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for start in [995, 999, 1000, 1001] {
        let raw = format!("{}{sentinel} end\n", "é".repeat(start));
        std::fs::write(&path, &raw).unwrap();
        let value = invoke(
            &repo,
            &json!({"name":"diagnostics.logs","args":{"limit":1}}),
            Caller::Assistant,
        )
        .unwrap();
        let expected = format!("{}[redacted] end", "é".repeat(start))
            .chars()
            .take(1000)
            .collect::<String>();
        assert_eq!(value["lines"], json!([expected]));
        assert_eq!(value["leadingRecordSkipped"], false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
    }
}
#[test]
fn diagnostics_logs_skip_tail_fragments_including_partial_utf8_and_keep_full_records() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = "benign-tail-秘密-secret-sentinel";
    std::fs::write(
        temp.path().join(".env"),
        format!("LOG_TEST_SECRET={sentinel}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let path = repo.data.parent().unwrap().join("logs/sync.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let retained = "Routine complete é record\nLatest complete record";
    for cut in [1, sentinel.find('秘').unwrap() + 1] {
        let padding = 65536 + cut - sentinel.len() - 1 - retained.len();
        let raw = format!(
            "{}{sentinel}{}\n{retained}",
            "p".repeat(2000),
            "x".repeat(padding)
        );
        std::fs::write(&path, &raw).unwrap();
        let value = invoke(
            &repo,
            &json!({"name":"diagnostics.logs","args":{"kind":"sync","limit":100}}),
            Caller::Assistant,
        )
        .unwrap();
        assert_eq!(
            value["lines"],
            json!(["Latest complete record", "Routine complete é record"])
        );
        assert_eq!(value["leadingRecordSkipped"], true);
        assert_eq!(value["scanTruncated"], true);
        assert_eq!(value["maxReadBytes"], 65537);
        assert_eq!(value["maxScanBytes"], 65536);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
    }
    let retained = format!("{}\nlast!", "x".repeat(65530));
    assert_eq!(retained.len(), 65536);
    std::fs::write(&path, format!("prefix\n{retained}")).unwrap();
    let value = invoke(
        &repo,
        &json!({"name":"diagnostics.logs","args":{"kind":"sync"}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(value["leadingRecordSkipped"], false);
    assert_eq!(value["lines"][0], "last!");
    assert_eq!(value["lines"][1].as_str().unwrap().len(), 1000);
}
#[test]
fn diagnostics_log_tail_redacts_multiline_environment_suffixes_at_each_cut() {
    let sentinel = "benign-秘密-first\nsecond-benign\nthird-secret";
    for cut in 1..sentinel.len() {
        if !sentinel.is_char_boundary(cut) {
            continue;
        }
        let fragment = &sentinel[cut..];
        let retained = if sentinel.as_bytes()[cut - 1] == b'\n' {
            fragment
        } else {
            fragment.find('\n').map_or("", |end| &fragment[end + 1..])
        };
        let input = if retained.is_empty() {
            "Normal é line".to_owned()
        } else {
            format!("{retained} end\nNormal é line")
        };
        let rows = redacted_log_lines(&input, "", 100, |value| {
            redact_leading_multiline_value(value, [sentinel.to_owned()]);
            *value = value.replace(sentinel, "[redacted]");
        });
        let expected = if retained.is_empty() {
            vec!["Normal é line"]
        } else {
            vec!["Normal é line", "[redacted] end"]
        };
        assert_eq!(rows, expected);
    }
    let mut normal = "Normal é line\nAnother line".to_owned();
    redact_leading_multiline_value(&mut normal, [sentinel.to_owned()]);
    assert_eq!(normal, "Normal é line\nAnother line");
    let mut longest = "long-suffix-marker end".to_owned();
    redact_leading_multiline_value(
        &mut longest,
        [
            "first\nlong".to_owned(),
            "second\nlong-suffix-marker".to_owned(),
        ],
    );
    assert_eq!(longest, "[redacted] end");
}

#[test]
fn diagnostics_log_buffer_redaction_precedes_multiline_record_splitting() {
    let sentinel = "benign-multiline\nprivate-sentinel";
    let contents = format!("Start {sentinel} end\nRoutine line");
    let rows = redacted_log_lines(&contents, "", 100, |value| {
        *value = value.replace(sentinel, "[redacted]");
    });
    assert_eq!(rows, vec!["Routine line", "Start [redacted] end"]);
    assert_eq!(
        redacted_log_lines(&contents, "routine", 100, |value| *value =
            value.replace(sentinel, "[redacted]")),
        vec!["Routine line"]
    );
}

#[test]
fn diagnostics_are_bounded_and_storage_truthful() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let value = invoke(
        &repo,
        &json!({"name":"diagnostics.storage"}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(value["engine"], "json");
    assert!(value["sqlSchema"].is_null());
    assert!(invoke(
        &repo,
        &json!({"name":"diagnostics.logs","args":{"kind":"../../.env"}}),
        Caller::Assistant
    )
    .is_err());
    assert!(invoke(
        &repo,
        &json!({"name":"commands.list","args":[]}),
        Caller::Assistant
    )
    .is_err());
    assert!(invoke(
        &repo,
        &json!({"name":"commands.list","args":{"extra":"x".repeat(64*1024)}}),
        Caller::Assistant
    )
    .is_err());
    assert!(invoke(
        &repo,
        &json!({"name":"unregistered.command"}),
        Caller::Assistant
    )
    .is_err());
}

#[test]
fn saved_view_discovery_exposes_complete_grammar_and_a_valid_calendar_example() {
    use super::super::views::{self, ScalarField};
    let catalog = catalog();
    let command = arr(&catalog["commands"])
        .into_iter()
        .find(|command| command["name"] == "views.apply")
        .unwrap();
    let grammar = text(
        &command["args"]["properties"]["views"]["items"]["properties"]["filter"],
        "description",
    );
    for field in [
        ScalarField::Source,
        ScalarField::Kind,
        ScalarField::Status,
        ScalarField::SourceStatus,
        ScalarField::Scope,
    ] {
        let spelling = serde_json::to_string(&field).unwrap();
        assert!(
            grammar.contains(&spelling),
            "undiscoverable scalar field {spelling}"
        );
    }
    for contract in [
        "Empty all matches all",
        "empty any matches no",
        "including negative rules",
        "root depth 0",
        "maximum depth 4",
        "128 total nodes",
        "completed\" includes only \"done",
        "includes unknown statuses",
        "stack is valid only",
        "preserve unrelated views",
        "revision/contentHash",
        "midnight UTC",
        "weeks start Monday",
    ] {
        assert!(
            grammar.contains(contract),
            "missing discovery contract: {contract}"
        );
    }
    let example = grammar
        .lines()
        .find_map(|line| line.strip_prefix("Calendar example: "))
        .unwrap();
    let filter: Value = serde_json::from_str(example).unwrap();
    assert_eq!(
        filter,
        json!({"op":"calendarPeriod","field":"updatedAt","period":"sinceYesterday","timezone":"Europe/Berlin"})
    );
    let mut document = json!({"version":1,"revision":0,"views":[{"id":"standup","name":"Standup","parent":"all","lifecycle":"any","filter":filter,"query":"","groupBy":"source"}]});
    views::parse_document(&document).unwrap();
    for (op, matches) in [("all", true), ("any", false)] {
        document["views"][0]["filter"] = json!({"op":op,"conditions":[]});
        document["views"][0]["lifecycle"] = json!("active");
        let parsed = views::parse_document(&document).unwrap();
        assert_eq!(
            views::evaluate_at(
                &parsed.views[0],
                &json!({"source":"linear","status":"unknown"}),
                0.0
            ),
            matches
        );
    }
    document["views"][0]["groupBy"] = json!("stack");
    assert!(views::parse_document(&document).is_err());
    document["views"][0]["parent"] = json!("github");
    views::parse_document(&document).unwrap();
}

#[test]
fn assistant_can_read_and_update_views_with_compare_and_swap() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let initial = invoke(&repo, &json!({"name":"views.read"}), Caller::Assistant).unwrap();
    let mut views = initial["views"].clone();
    views[0]["name"] = json!("Routine saved filter");
    let args =
        json!({"revision":initial["revision"],"contentHash":initial["contentHash"],"views":views});
    let updated = invoke(
        &repo,
        &json!({"name":"views.apply","args":args}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(
        updated["revision"],
        initial["revision"].as_u64().unwrap() + 1
    );
    assert_eq!(updated["views"][0]["name"], "Routine saved filter");
    let stale = invoke(
        &repo,
        &json!({"name":"views.apply","args":args}),
        Caller::Assistant,
    )
    .err()
    .unwrap();
    assert_eq!(stale.status, 409);
    let mut invalid = json!({"revision":updated["revision"],"contentHash":updated["contentHash"],"views":updated["views"]});
    invalid["views"][0]["filter"] = json!({"op":"command","command":"printf denied"});
    assert!(invoke(
        &repo,
        &json!({"name":"views.apply","args":invalid}),
        Caller::Assistant
    )
    .is_err());
    assert_eq!(
        invoke(&repo, &json!({"name":"views.read"}), Caller::Assistant).unwrap(),
        updated
    );
}

#[test]
fn review_history_reads_the_existing_saved_history() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|state| {
            state["sessions"] = json!([{"id":id,"reviewHistory":{"version":1,"entries":[{"id":"earlier-round","archivedAt":10,"review":{"revision":2,"note":"Earlier review","comments":[]}}]}}]);
            Ok(())
        }).unwrap();
    let result = invoke(
        &repo,
        &json!({"name":"review.history","args":{"id":id}}),
        Caller::Assistant,
    )
    .unwrap();
    assert_eq!(result["reviews"][0]["id"], "earlier-round");
    assert_eq!(result["reviews"][0]["notePreview"], "Earlier review");
    assert_eq!(result["reviews"][0]["readOnly"], true);
    assert_eq!(result["limit"], 20);
}
