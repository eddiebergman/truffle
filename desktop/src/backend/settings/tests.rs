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
        let patch = json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"patch":{"uiScale":120}});
        assert!(store.preview(&patch, false).is_err());
        assert!(store.apply(&patch, false).is_err());
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
        assert_eq!(store.preview(&request, false).unwrap_err().status, 409);
        assert_eq!(store.apply(&request, false).unwrap_err().status, 409);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::write(
        store.config_directory().join("workflows/agent-review.md"),
        "metadata-only-workflow-change",
    )
    .unwrap();
    assert_eq!(store.view()["contentHash"], fresh["contentHash"]);
    let request = json!({"revision":fresh["revision"],"contentHash":fresh["contentHash"],"patch":{"uiScale":120}});
    let preview = store.preview(&request, false).unwrap();
    assert_eq!(preview["contentHash"], fresh["contentHash"]);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let applied = store.apply(&request, false).unwrap();
    assert_ne!(applied["contentHash"], fresh["contentHash"]);
    let persisted: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(persisted.get("contentHash").is_none());
}

#[test]
fn routine_projection_excludes_workflow_contents_but_keeps_file_locations() {
    let document = json!({
        "values":{"theme":"dark","editor":"private-editor"},
        "defaults":{"theme":"light","quickActions":[{"command":"private-command"}]},
        "workflows":{"agent-review":"private-workflow"},
        "files":{"settings":"/config/settings.json"},
        "reviewSkill":"/config/review/SKILL.md"
    });
    let projected = routine_view(document);
    assert_eq!(projected["values"], json!({"theme":"dark"}));
    assert_eq!(projected["defaults"], json!({"theme":"light"}));
    assert!(projected.get("workflows").is_none());
    assert!(!projected.to_string().contains("private-"));
    assert_eq!(projected["files"]["settings"], "/config/settings.json");
    assert_eq!(projected["reviewSkill"], "/config/review/SKILL.md");
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
        assert!(store.preview(&request, true).is_err());
        assert!(store.apply(&request, true).is_err());
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let request = json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"defaultAgent":"codex","uiScale":120,"keybindings":store.values()["keybindings"]}});
    let preview = store.preview(&request, true).unwrap();
    assert_eq!(
        store.apply(&request, true).unwrap()["values"],
        preview["values"]
    );
    let request =
        json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"editor":"code"}});
    assert!(store.preview(&request, false).is_ok());
    assert_eq!(
        store.apply(&request, false).unwrap()["values"]["editor"],
        "code"
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
        assert!(store.preview(&request, false).is_err());
        assert!(store.apply(&request, false).is_err());
    }
    let request =
        json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"uiScale":120}});
    let preview = store.preview(&request, false).unwrap();
    assert_eq!(preview["revision"], 0);
    assert_eq!(preview["values"]["uiScale"], 120);
    assert_eq!(store.values(), initial);
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert!(store.preview(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"editor":"code"}}), true).is_err());
    let applied = store.apply(&request, false).unwrap();
    assert_eq!(applied["values"], preview["values"]);
    assert!(store.preview(&request, false).is_err());
}

#[test]
fn a_saved_theme_whose_file_was_removed_does_not_block_other_settings() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::load(temp.path().join("settings.json")).unwrap();
    store.apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"theme":"nord"}}), false).unwrap();
    std::fs::remove_file(temp.path().join("config/themes/nord.json")).unwrap();
    store.apply(&json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"diffFontSize":12}}), false).unwrap();
    assert!(store.apply(&json!({"revision":2,"contentHash":store.view()["contentHash"],"patch":{"theme":"midnight"}}), false).is_err());
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
                true,
            )
            .unwrap();
    assert_eq!(changed["revision"], 1);
    assert_eq!(Store::load(path).unwrap().values()["theme"], "github-light");
    assert!(store
            .apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"theme":"github-dark"}}), false)
            .is_err());
    assert!(store.apply(&json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"quickActions":[{"id":"test","title":"Test","command":"printf test"}]}}),true).is_err());
    assert!(store
            .apply(&json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"token":"secret"}}), false)
            .is_err());
    let mut bindings = defaults()["keybindings"].clone();
    bindings["commands"] = json!("Primary+H");
    assert!(store
            .apply(
                &json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"keybindings":bindings}}),
                false
            )
            .is_err());
    assert_eq!(store.view()["revision"], 1);
}
