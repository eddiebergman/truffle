use super::*;
fn fixture() -> (tempfile::TempDir, std::sync::Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    (temp, repo)
}
fn configured(repo: &Repository, value: &Value) -> Vec<u8> {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    std::fs::write(config_path(repo), &bytes).unwrap();
    bytes
}
#[test]
fn raw_skill_source_preserves_document_paths_extensions_and_overlay_precedence() {
    let (_temp, repo) = fixture();
    let document = json!({"version":1,"privateRoot":{"keep":true},"presets":[
        {"id":"first","cwd":"~/not-required-to-exist","skill":"relative/SKILL.md","reports":"historic/reports","privateNs":{"keep":1}},
        {"id":"override","title":"Configured","privateNs":{"configured":true}}
    ]});
    let bytes = configured(&repo, &document);
    let custom = json!({"id":"override","title":"Custom","cwd":"relative folder","skill":"raw.md","reports":"custom-reports","privateNs":{"custom":true}});
    let state = json!({"skillPresets":[custom.clone(),{"id":"last","reports":"last-reports"}]});
    let source = load(&repo).unwrap();
    assert_eq!(source.document, document);
    assert_eq!(source.hash, context_kit::sha256(&bytes));
    assert!(source.is_current(&repo).unwrap());
    let entries = source.overlay(&state).unwrap();
    assert_eq!(
        entries,
        vec![
            document["presets"][0].clone(),
            custom.clone(),
            state["skillPresets"][1].clone()
        ]
    );
    assert_eq!(source.find(&state, "override").unwrap(), (custom, true));
    assert_eq!(
        source.find(&state, "first").unwrap(),
        (document["presets"][0].clone(), false)
    );
    assert_eq!(std::fs::read(config_path(&repo)).unwrap(), bytes);
}
#[test]
fn raw_skill_source_validation_is_identical_for_catalog_and_editor() {
    let (_temp, repo) = fixture();
    for document in [
        json!({"version":2,"presets":[]}),
        json!({"version":1,"presets":{}}),
        json!({"version":1,"presets":[{"id":"duplicate"},{"id":"duplicate"}]}),
        json!({"version":1,"presets":[{"id":" padded "}]}),
        json!({"version":1,"presets":(0..31).map(|i|json!({"id":format!("skill-{i}")})).collect::<Vec<_>>()}),
    ] {
        configured(&repo, &document);
        let state =
            json!({"skillPresets":[{"id":"duplicate","cwd":"/missing","skill":"SKILL.md"}]});
        let catalog_error = super::super::artifacts::presets(&repo, &state).unwrap_err();
        let editor_error =
            super::super::scheduler::preset_read(&repo, &json!({"presetId":"duplicate"}))
                .unwrap_err();
        assert_eq!(catalog_error.message, editor_error.message);
        assert!(load(&repo).is_err());
    }
    std::fs::write(
        config_path(&repo),
        vec![b' '; MAX_CONFIG_BYTES as usize + 1],
    )
    .unwrap();
    assert!(load(&repo).err().unwrap().message.contains("Cannot read"));
}
#[test]
fn raw_skill_overlay_rejects_ambiguous_custom_ids_and_combined_capacity() {
    let (_temp, repo) = fixture();
    configured(&repo, &json!({"version":1,"presets":[{"id":"configured"}]}));
    let source = load(&repo).unwrap();
    for custom in [
        json!([{"id":"same"},{"id":"same"}]),
        json!({"id":"not-an-array"}),
        json!((0..30)
            .map(|i| json!({"id":format!("custom-{i}")}))
            .collect::<Vec<_>>()),
    ] {
        let state = json!({"skillPresets":custom});
        assert!(source.overlay(&state).is_err());
        assert!(source.find(&state, "configured").is_err());
    }
    assert_eq!(
        source.overlay(&json!({})).unwrap(),
        vec![json!({"id":"configured"})]
    );
}
#[test]
fn raw_skill_source_exact_byte_fence_and_raw_materialization_stay_separate_from_projection() {
    let (temp, repo) = fixture();
    std::fs::write(temp.path().join("SKILL.md"), "Fixture").unwrap();
    let row = json!({"id":"configured","title":"Original","cwd":temp.path(),"skill":"SKILL.md","reports":"historic/reports","command":"$fixture","agent":"codex","provenance":{"keep":true}});
    let document = json!({"version":1,"presets":[row],"privateRoot":"keep"});
    let bytes = configured(&repo, &document);
    let source = load(&repo).unwrap();
    let baseline =
        super::super::scheduler::preset_read(&repo, &json!({"presetId":"configured"})).unwrap();
    let state = repo.state.lock().unwrap().clone();
    let projected = super::super::artifacts::presets(&repo, &state).unwrap();
    assert_eq!(projected[0]["skill"], json!(temp.path().join("SKILL.md")));
    assert_eq!(
        source.find(&state, "configured").unwrap().0["skill"],
        "SKILL.md"
    );
    let canonical_bytes = std::fs::read(&repo.data).unwrap();
    let mut changed_bytes = bytes.clone();
    changed_bytes.push(b'\n');
    std::fs::write(config_path(&repo), &changed_bytes).unwrap();
    assert!(!source.is_current(&repo).unwrap());
    let args = json!({"presetId":"configured","patch":{"title":"Edited"},"expected":{"title":baseline["expected"]["title"]}});
    assert_eq!(
        super::super::scheduler::update_preset(&repo, &args)
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), canonical_bytes);
    std::fs::write(config_path(&repo), &bytes).unwrap();
    super::super::scheduler::update_preset(&repo, &args).unwrap();
    let saved = repo.state.lock().unwrap()["skillPresets"][0].clone();
    assert_eq!(saved["skill"], "SKILL.md");
    assert_eq!(saved["reports"], "historic/reports");
    assert_eq!(saved["provenance"], row["provenance"]);
    assert!(saved.get("available").is_none());
    assert_eq!(std::fs::read(config_path(&repo)).unwrap(), bytes);
}
