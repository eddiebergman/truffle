use super::*;
fn fixture() -> (tempfile::TempDir, PathBuf, Value, Value, Value) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("work");
    std::fs::create_dir(&root).unwrap();
    for name in ["configured", "explicit", "mapped", "cached", "manual"] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    let project = json!({"id":"note:fixture","source":"notes","title":"Folder fixture","cwd":root.join("configured")});
    let state =
        json!({"itemSettings":{"note:fixture":{"cwd":root.join("cached")}},"folderMappings":{}});
    let candidates = json!([{"name":"manual","path":root.join("manual")}]);
    (temp, root, state, project, candidates)
}
fn canonical(path: &std::path::Path) -> String {
    path.canonicalize().unwrap().to_string_lossy().into()
}
#[test]
fn project_folder_uses_configured_path_after_explicit_settings_and_mappings_before_cache() {
    let (_temp, root, mut state, project, candidates) = fixture();
    let resolve = |state: &Value| {
        choose_with(state, &project, &json!({}), &root, &candidates)
            .unwrap()
            .0
    };
    assert_eq!(resolve(&state), canonical(&root.join("configured")));
    state["folderMappings"]["item:note:fixture"] = json!({"path":root.join("mapped")});
    assert_eq!(resolve(&state), canonical(&root.join("mapped")));
    state["itemSettings"]["note:fixture"] = json!({"explicit":true,"cwd":root.join("explicit")});
    assert_eq!(resolve(&state), canonical(&root.join("explicit")));
    std::fs::remove_dir(root.join("explicit")).unwrap();
    assert_eq!(resolve(&state), canonical(&root.join("mapped")));
    std::fs::remove_dir(root.join("mapped")).unwrap();
    assert_eq!(resolve(&state), canonical(&root.join("configured")));
}
#[test]
fn project_folder_invalid_deleted_file_and_root_paths_fall_through_or_ask() {
    let (_temp, root, state, mut project, candidates) = fixture();
    let file = root.join("not-a-directory");
    std::fs::write(&file, b"fixture").unwrap();
    let deleted = root.join("configured");
    std::fs::remove_dir(&deleted).unwrap();
    for path in [root.join("missing"), file, root.clone(), deleted] {
        project["cwd"] = json!(path);
        assert_eq!(
            choose_with(&state, &project, &json!({}), &root, &candidates)
                .unwrap()
                .0,
            canonical(&root.join("cached"))
        );
        let mut without_cache = state.clone();
        without_cache["itemSettings"] = json!({});
        let error =
            choose_with(&without_cache, &project, &json!({}), &root, &candidates).unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(error.data["location"]["title"], "Folder fixture");
        assert_eq!(
            automatic_folder(&without_cache, &project, &root, &candidates),
            None
        );
    }
}
#[test]
fn project_folder_accepts_nested_external_and_symlink_targets_and_excludes_root_aliases() {
    use std::os::unix::fs::symlink;
    let (temp, root, mut state, mut project, candidates) = fixture();
    let nested = root.join("configured/nested");
    let outside = temp.path().join("outside");
    std::fs::create_dir(&nested).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let alias = temp.path().join("outside-alias");
    symlink(&outside, &alias).unwrap();
    for path in [&nested, &outside, &alias] {
        project["cwd"] = json!(path);
        assert_eq!(
            choose_with(&state, &project, &json!({}), &root, &candidates)
                .unwrap()
                .0,
            canonical(path)
        );
    }
    let root_alias = temp.path().join("root-alias");
    symlink(&root, &root_alias).unwrap();
    project["cwd"] = json!(&root_alias);
    state["itemSettings"] = json!({});
    assert_eq!(
        automatic_folder(&state, &project, &root_alias, &candidates),
        None
    );
    state["itemSettings"]["note:fixture"] = json!({"explicit":true,"cwd":&root_alias});
    assert_eq!(
        automatic_folder(&state, &project, &root_alias, &candidates),
        None
    );
    state["folderMappings"]["item:note:fixture"] = json!({"path":&root_alias});
    assert_eq!(
        automatic_folder(&state, &project, &root_alias, &candidates),
        None
    );
}
#[test]
fn project_folder_manual_new_and_force_picker_precedence_are_unchanged() {
    let (_temp, root, state, project, candidates) = fixture();
    let chosen = choose_with(
        &state,
        &project,
        &json!({"folderPath":root.join("manual"),"chooseFolder":true}),
        &root,
        &candidates,
    )
    .unwrap();
    assert_eq!(chosen, (canonical(&root.join("manual")), None));
    let new = choose_with(
        &state,
        &project,
        &json!({"newFolder":"new-project","folderPath":root.join("manual"),"chooseFolder":true}),
        &root,
        &candidates,
    )
    .unwrap();
    assert_eq!(
        new,
        (
            root.join("new-project").to_string_lossy().into(),
            Some("new-project".into())
        )
    );
    assert!(!root.join("new-project").exists());
    assert_eq!(
        choose_with(
            &state,
            &project,
            &json!({"chooseFolder":true}),
            &root,
            &candidates
        )
        .unwrap_err()
        .status,
        409
    );
    assert!(choose_with(
        &state,
        &project,
        &json!({"folderPath":root.join("configured")}),
        &root,
        &candidates
    )
    .is_err());
    assert!(choose_with(
        &state,
        &project,
        &json!({"newFolder":"../escape"}),
        &root,
        &candidates
    )
    .is_err());
}
#[test]
fn project_folder_is_notes_only_and_other_sources_keep_inferred_then_cached_order() {
    let (_temp, root, mut state, project, _candidates) = fixture();
    let mut foreign = project;
    foreign["source"] = json!("github");
    foreign["url"] = json!("https://github.com/example/project/pull/1");
    let candidates = json!([{"name":"project","path":root.join("manual")}]);
    assert_eq!(
        automatic_folder(&state, &foreign, &root, &candidates),
        Some(canonical(&root.join("manual")))
    );
    std::fs::remove_dir(root.join("manual")).unwrap();
    assert_eq!(
        automatic_folder(&state, &foreign, &root, &candidates),
        Some(canonical(&root.join("cached")))
    );
    state["itemSettings"] = json!({});
    assert_eq!(automatic_folder(&state, &foreign, &root, &candidates), None);
    let root_candidate = json!([{"name":"project","path":&root}]);
    assert_eq!(
        automatic_folder(&state, &foreign, &root, &root_candidate),
        None
    );
}
