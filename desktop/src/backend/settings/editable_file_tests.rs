use super::*;
#[test]
fn manual_edits_reload_and_invalid_files_do_not_replace_good_values() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = Store::load(path.clone()).unwrap();
    let mut external: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    external["values"]["theme"] = json!("catppuccin-latte");
    std::fs::write(&path, serde_json::to_vec(&external).unwrap()).unwrap();
    assert_eq!(store.view()["values"]["theme"], "catppuccin-latte");
    assert_eq!(store.view()["revision"], 1);
    assert!(store
            .apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"diffFontSize":11}}), Approval::User)
            .is_err());
    std::fs::write(&path, b"broken").unwrap();
    assert_eq!(store.view()["values"]["theme"], "catppuccin-latte");
    assert!(store.view()["configError"].is_string());
    assert!(store
            .apply(&json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"diffFontSize":11}}), Approval::User)
            .is_err());
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
