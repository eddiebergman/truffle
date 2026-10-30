use super::*;
#[test]
fn manual_edits_reload_and_invalid_files_do_not_replace_good_values() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let baseline = store.view();
    let mut external: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    external["values"]["theme"] = json!("catppuccin-latte");
    std::fs::write(&path, serde_json::to_vec(&external).unwrap()).unwrap();
    assert_eq!(store.view()["values"]["theme"], "catppuccin-latte");
    assert!(store
            .apply(&json!({"revision":0,"contentHash":baseline["contentHash"],"patch":{"diffFontSize":11}}), Approval::User)
            .is_err());
    std::fs::write(&path, b"broken").unwrap();
    assert_eq!(store.view()["values"]["theme"], "catppuccin-latte");
    assert!(store.view()["configError"].is_string());
    assert!(store
            .apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"diffFontSize":11}}), Approval::User)
            .is_err());
}
#[test]
fn a_deleted_settings_file_reads_as_the_defaults_and_is_saved_again() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let initial = store.view();
    store
        .apply(
            &json!({"revision":0,"contentHash":initial["contentHash"],"patch":{"diffFontSize":11}}),
            Approval::User,
        )
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    let reset = store.view();
    assert_eq!(reset["values"], initial["values"]);
    assert_eq!(reset["contentHash"], initial["contentHash"]);
    store
        .apply(
            &json!({"revision":0,"contentHash":reset["contentHash"],"patch":{"diffFontSize":12}}),
            Approval::User,
        )
        .unwrap();
    assert_eq!(Store::load(path).unwrap().values()["diffFontSize"], 12);
}
#[test]
fn editable_metadata_themes_and_removed_presets_survive_bootstrap() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::load(temp.path().join("settings.json")).unwrap();
    let dir = store.config_directory();
    let mut fields = super::super::config::read(&dir.join("settings-fields.json")).unwrap();
    fields["fields"][0]["label"] = json!("My palette");
    std::fs::write(
        dir.join("settings-fields.json"),
        serde_json::to_vec(&fields).unwrap(),
    )
    .unwrap();
    assert_eq!(store.view()["fields"][0]["label"], "My palette");
    let catalog = store.themes().unwrap();
    assert_eq!(catalog["themes"]["catppuccin-latte"]["type"], "light");
    assert_eq!(catalog["themes"]["dracula"]["type"], "dark");
    assert!(catalog["themes"].as_object().unwrap().len() >= 12);
    std::fs::remove_file(dir.join("themes/dracula.json")).unwrap();
    super::super::config::install(dir).unwrap();
    assert!(!dir.join("themes/dracula.json").exists());
}
