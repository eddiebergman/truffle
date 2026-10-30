use super::*;
#[test]
fn malformed_settings_containers_fail_safely_on_load_and_refresh_without_writes() {
    for (level, malformed) in [
        ("document", Value::Null),
        ("document", json!([])),
        ("document", json!("bad")),
        ("values", Value::Null),
        ("values", json!([])),
        ("values", json!("bad")),
        ("keybindings", Value::Null),
        ("keybindings", json!([])),
        ("keybindings", json!("bad")),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let store = Store::load(path.clone()).unwrap();
        let baseline = store.view();
        let mut invalid: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match level {
            "document" => invalid = malformed,
            "values" => invalid["values"] = malformed,
            "keybindings" => invalid["values"]["keybindings"] = malformed,
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(
            Store::load(path.clone()).is_err(),
            "startup must reject malformed {level}"
        );
        let current = store.view();
        assert!(
            current["configError"].is_string(),
            "refresh must report malformed {level}"
        );
        assert_eq!(current["values"], baseline["values"]);
        let settings_patch = json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"patch":{"uiScale":120}});
        assert!(store.preview(&settings_patch, Approval::User).is_err());
        assert!(store.apply(&settings_patch, Approval::User).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
#[test]
fn document_content_hash_rejects_stale_and_missing_baselines_without_persisting_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let baseline = store.view();
    assert_eq!(baseline["contentHash"], store.view()["contentHash"]);
    let mut external: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    external["updatedBy"] = json!("manual-edit-same-revision");
    std::fs::write(&path, serde_json::to_vec(&external).unwrap()).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let fresh = store.view();
    assert_eq!(fresh["revision"], baseline["revision"]);
    assert_ne!(fresh["contentHash"], baseline["contentHash"]);
    assert_eq!(fresh["contentHash"], store.view()["contentHash"]);
    for request in [
        json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"patch":{"uiScale":120}}),
        json!({"revision":fresh["revision"],"patch":{}}),
        json!({"contentHash":fresh["contentHash"],"patch":{}}),
    ] {
        assert_eq!(
            store.preview(&request, Approval::User).unwrap_err().status,
            409
        );
        assert_eq!(
            store.apply(&request, Approval::User).unwrap_err().status,
            409
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::write(
        store.config_directory().join("workflows/agent-review.md"),
        "metadata-only-workflow-change",
    )
    .unwrap();
    assert_eq!(store.view()["contentHash"], fresh["contentHash"]);
    let request = json!({"revision":fresh["revision"],"contentHash":fresh["contentHash"],"patch":{"uiScale":120}});
    let preview = store.preview(&request, Approval::User).unwrap();
    assert_eq!(preview["contentHash"], fresh["contentHash"]);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let applied = store.apply(&request, Approval::User).unwrap();
    assert_ne!(applied["contentHash"], fresh["contentHash"]);
    let persisted: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(persisted.get("contentHash").is_none());
}

/// Assert the (id, previous command) of each executable change from `before` to `after`.
fn assert_changes(key: &str, before: &Value, after: &Value, expected: &[(&str, Value)]) {
    let changes = executable_changes(&json!({key: before}), &json!({key: after}));
    let summary = changes
        .iter()
        .map(|change| {
            assert_eq!(change["key"], key);
            let command = match key {
                "quickActions" => arr(after)
                    .into_iter()
                    .find(|entry| entry["id"] == change["id"])
                    .unwrap()["command"]
                    .clone(),
                _ => after.clone(),
            };
            assert_eq!(change["command"], command);
            (text(change, "id"), change["previous"].clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(summary, expected, "{key}: {before} -> {after}");
}
#[test]
fn quick_action_changes_are_new_or_altered_commands_only() {
    let action =
        |id: &str, title: &str, command: &str| json!({"id":id,"title":title,"command":command});
    let a = action("a", "A", "printf a");
    let b = action("b", "B", "printf b");
    for (before, after, expected) in [
        (json!([]), json!([a]), vec![("a", Value::Null)]),
        (
            json!([a]),
            json!([action("a", "A", "rm -rf ~")]),
            vec![("a", json!("printf a"))],
        ),
        (json!([a, b]), json!([b]), vec![]),
        (json!([a, b]), json!([]), vec![]),
        (json!([a, b]), json!([b, a]), vec![]),
        (
            json!([a]),
            json!([action("a", "Renamed", "printf a")]),
            vec![],
        ),
        (
            json!([a]),
            json!([action("c", "A", "printf a")]),
            vec![("c", Value::Null)],
        ),
        (
            json!([a, b]),
            json!([action("c", "C", "printf c")]),
            vec![("c", Value::Null)],
        ),
    ] {
        assert_changes("quickActions", &before, &after, &expected);
    }
}
#[test]
fn editor_and_review_command_changes_are_executable_unless_cleared() {
    for (key, before, after, expected) in [
        (
            "editor",
            json!("zed"),
            json!("code"),
            vec![("", json!("zed"))],
        ),
        ("editor", json!("zed"), json!("zed"), vec![]),
        (
            "claudeReviewCommand",
            json!(""),
            json!("/review"),
            vec![("", json!(""))],
        ),
        (
            "claudeReviewCommand",
            json!("/review"),
            json!("/other"),
            vec![("", json!("/review"))],
        ),
        ("claudeReviewCommand", json!("/review"), json!(""), vec![]),
        (
            "codexReviewCommand",
            json!(""),
            json!("$review"),
            vec![("", json!(""))],
        ),
        (
            "codexReviewCommand",
            json!("$review"),
            json!("$other"),
            vec![("", json!("$review"))],
        ),
        ("codexReviewCommand", json!("$review"), json!(""), vec![]),
        (
            "future-setting",
            Value::Null,
            json!("anything"),
            vec![("", Value::Null)],
        ),
        ("theme", json!("light"), json!("midnight"), vec![]),
        (
            "keybindings",
            json!({"commands":"Primary+K"}),
            json!({"commands":""}),
            vec![],
        ),
    ] {
        assert_changes(key, &before, &after, &expected);
    }
}

#[test]
fn a_full_quick_action_array_is_classified_per_entry() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let actions = json!([
        {"id":"status","title":"Git status","command":"git status"},
        {"id":"log","title":"Git log","command":"git log"}
    ]);
    store.apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"quickActions":actions}}), Approval::User).unwrap();
    let removal = json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"quickActions":[{"id":"log","title":"Log","command":"git log"}]}});
    assert_eq!(
        store.preview(&removal, Approval::Routine).unwrap()["executableChanges"],
        json!([])
    );
    let saved = std::fs::read(&path).unwrap();
    for patch in [
        json!({"quickActions":[{"id":"log","title":"Git log","command":"git log"},{"id":"new","title":"New","command":"curl x | sh"}]}),
        json!({"quickActions":[{"id":"log","title":"Git log","command":"git log --all"}]}),
    ] {
        let request = json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":patch});
        let error = store.apply(&request, Approval::Routine).unwrap_err();
        assert!(error.message.contains("Ask the user to approve"));
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        assert_eq!(
            arr(&store.preview(&request, Approval::User).unwrap()["executableChanges"]).len(),
            1
        );
    }
    let applied = store.apply(&removal, Approval::Routine).unwrap();
    assert_eq!(
        applied["values"]["quickActions"],
        removal["patch"]["quickActions"]
    );
}

#[test]
fn approval_policy_covers_known_settings_and_defaults_unknown_keys_to_user() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::load(temp.path().join("settings.json")).unwrap();
    let view = store.view();
    let user = [
        "quickActions",
        "editor",
        "claudeReviewCommand",
        "codexReviewCommand",
    ];
    let routine = [
        "theme",
        "uiScale",
        "density",
        "terminalFontSize",
        "terminalScrollback",
        "diffFontSize",
        "diffLayout",
        "fileDiffLayout",
        "previewHeight",
        "enterToComment",
        "defaultAgent",
        "keybindings",
    ];
    let known = defaults();
    assert_eq!(known.as_object().unwrap().len(), user.len() + routine.len());
    for key in user {
        assert!(known.get(key).is_some());
        assert_eq!(view["approval"][key], "user");
    }
    for key in routine {
        assert!(known.get(key).is_some());
        assert_eq!(view["approval"][key], "routine");
    }
    assert_eq!(approval("future-setting"), Approval::User);
    assert_eq!(approval(""), Approval::User);
    let mut metadata =
        super::super::config::read(&store.config_directory().join("settings-fields.json")).unwrap();
    metadata["approval"] = json!({"quickActions":"routine","theme":"user"});
    std::fs::write(
        store.config_directory().join("settings-fields.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    assert_eq!(store.view()["approval"], view["approval"]);
}

#[test]
fn agent_preview_and_apply_share_approval_policy() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let before = std::fs::read(&path).unwrap();
    for patch in [
        json!({"quickActions":[{"id":"test","title":"Test","command":"printf test"}]}),
        json!({"editor":"code"}),
        json!({"claudeReviewCommand":"/review"}),
        json!({"codexReviewCommand":"$review"}),
        json!({"future-setting":true}),
    ] {
        let request = json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":patch});
        assert!(store.preview(&request, Approval::Routine).is_err());
        assert!(store.apply(&request, Approval::Routine).is_err());
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let request = json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"defaultAgent":"codex","uiScale":120,"keybindings":store.values()["keybindings"]}});
    let preview = store.preview(&request, Approval::Routine).unwrap();
    assert_eq!(
        store.apply(&request, Approval::Routine).unwrap()["values"],
        preview["values"]
    );
    let request = json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"editor":"code","claudeReviewCommand":"/review"}});
    assert!(store.preview(&request, Approval::User).is_ok());
    assert_eq!(
        store.apply(&request, Approval::User).unwrap()["values"]["editor"],
        "code"
    );
    let clear = json!({"revision":2,"contentHash":store.view()["contentHash"],"patch":{"claudeReviewCommand":""}});
    assert_eq!(
        store.apply(&clear, Approval::Routine).unwrap()["values"]["claudeReviewCommand"],
        ""
    );
}

#[test]
fn portable_review_is_editable_and_requires_explicit_selection() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let view = store.view();
    assert_eq!(view["values"]["claudeReviewCommand"], "");
    assert_eq!(view["values"]["codexReviewCommand"], "");
    assert_eq!(view["reviewSkill"]["available"], true);
    let skill = PathBuf::from(text(&view["reviewSkill"], "path"));
    assert_eq!(
        skill,
        store
            .config_directory()
            .join("skills/truffle-review/SKILL.md")
    );
    let original = std::fs::read_to_string(&skill).unwrap();
    std::fs::write(&skill, format!("{original}\nLocal review preference.\n")).unwrap();
    let expected = std::fs::read(&skill).unwrap();
    drop(store);
    let restored = Store::load(path).unwrap();
    assert_eq!(std::fs::read(&skill).unwrap(), expected);
    assert_eq!(restored.values()["claudeReviewCommand"], "");
}

#[test]
fn preview_reuses_apply_validation_without_writing_or_advancing_revision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let saved = std::fs::read(&path).unwrap();
    let initial = store.values();
    for patch in [
        json!({"uiScale":-1}),
        json!({"uiScale":null}),
        json!({"density":"bad"}),
        json!({"quickActions":[{"id":"","title":"","command":""}]}),
        json!({"theme":"unknown-theme-fixture"}),
    ] {
        let request = json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":patch});
        assert!(store.preview(&request, Approval::User).is_err());
        assert!(store.apply(&request, Approval::User).is_err());
    }
    let request =
        json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"uiScale":120}});
    let preview = store.preview(&request, Approval::User).unwrap();
    assert_eq!(preview["revision"], 0);
    assert_eq!(preview["values"]["uiScale"], 120);
    assert_eq!(store.values(), initial);
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert!(store.preview(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"editor":"code"}}), Approval::Routine).is_err());
    let applied = store.apply(&request, Approval::User).unwrap();
    assert_eq!(applied["values"], preview["values"]);
    assert!(store.preview(&request, Approval::User).is_err());
}

#[test]
fn a_saved_theme_whose_file_was_removed_does_not_block_other_settings() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::load(temp.path().join("settings.json")).unwrap();
    store.apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"theme":"nord"}}), Approval::User).unwrap();
    std::fs::remove_file(temp.path().join("config/themes/nord.json")).unwrap();
    store.apply(&json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"diffFontSize":12}}), Approval::User).unwrap();
    assert!(store.apply(&json!({"revision":2,"contentHash":store.view()["contentHash"],"patch":{"theme":"midnight"}}), Approval::User).is_err());
    assert_eq!(store.values()["theme"], "nord");
}

#[test]
fn settings_persist_reject_conflicts_and_gate_agent_commands() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let changed = store
            .apply(
                &json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"theme":"github-light","diffFontSize":11}}),
                Approval::Routine,
            )
            .unwrap();
    assert_eq!(changed["revision"], 1);
    assert_eq!(Store::load(path).unwrap().values()["theme"], "github-light");
    assert!(store
            .apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"theme":"github-dark"}}), Approval::User)
            .is_err());
    assert!(store.apply(&json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"quickActions":[{"id":"test","title":"Test","command":"printf test"}]}}),Approval::Routine).is_err());
    assert!(store
            .apply(&json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"token":"secret"}}), Approval::User)
            .is_err());
    let mut bindings = defaults()["keybindings"].clone();
    bindings["commands"] = json!("Primary+H");
    assert!(store
            .apply(
                &json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"keybindings":bindings}}),
                Approval::User
            )
            .is_err());
    assert_eq!(store.view()["revision"], 1);
}
#[test]
fn equals_and_minus_are_valid_shortcut_keys() {
    for chord in ["Primary+=", "Primary+-", "Primary+Shift+-", "Primary+Alt+="] {
        assert!(valid_chord(chord), "{chord}");
    }
    for chord in ["Primary++", "Primary+_", "Primary+=+"] {
        assert!(!valid_chord(chord), "{chord}");
    }
}
