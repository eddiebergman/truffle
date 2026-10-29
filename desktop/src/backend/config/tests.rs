use super::*;

#[test]
fn bootstrap_preserves_existing_skills_with_missing_or_incomplete_receipt() {
    for receipt in [None, Some(json!(["config/defaults.json"]))] {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("config");
        std::fs::create_dir(&directory).unwrap();
        let target = directory.join("skills.json");
        let custom = b"{\"version\":1,\"presets\":[],\"extension\":\"user-owned-skill-marker\"}\n";
        std::fs::write(&target, custom).unwrap();
        let legacy = b"[{\"id\":\"legacy-marker\"}]";
        std::fs::write(temp.path().join("newsletters.json"), legacy).unwrap();
        if let Some(receipt) = receipt {
            std::fs::write(
                directory.join(".installed-defaults.json"),
                serde_json::to_vec(&receipt).unwrap(),
            )
            .unwrap();
        }
        install(&directory).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), custom);
        install(&directory).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), custom);
        assert_eq!(
            std::fs::read(temp.path().join("newsletters.json")).unwrap(),
            legacy
        );
    }
}

#[test]
fn install_updates_untouched_themes_from_earlier_releases_and_keeps_edited_ones() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("config");
    std::fs::create_dir_all(directory.join("themes")).unwrap();
    let receipt = super::super::assets::CONFIG_FILES
        .iter()
        .filter(|n| !n.ends_with("/tokyo-night.json"))
        .collect::<Vec<_>>();
    std::fs::write(
        directory.join(".installed-defaults.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let themes = directory.join("themes");
    std::fs::write(
        themes.join("rose-pine.json"),
        include_bytes!("../fixtures/rose-pine-before-rename.json"),
    )
    .unwrap();
    std::fs::write(
        themes.join("midnight.json"),
        include_bytes!("../fixtures/midnight.json"),
    )
    .unwrap();
    let mut edited = include_bytes!("../fixtures/midnight.json").to_vec();
    edited.push(b'\n');
    std::fs::write(themes.join("light.json"), &edited).unwrap();
    std::fs::write(themes.join("nord.json"), b"{\"edited\":true}").unwrap();
    install(&directory).unwrap();
    assert_eq!(
        std::fs::read(themes.join("rose-pine.json")).unwrap(),
        super::super::assets::asset("config/themes/rose-pine.json").unwrap()
    );
    assert!(!themes.join("midnight.json").exists());
    assert_eq!(std::fs::read(themes.join("light.json")).unwrap(), edited);
    assert_eq!(
        std::fs::read(themes.join("nord.json")).unwrap(),
        b"{\"edited\":true}"
    );
    assert!(themes.join("tokyo-night.json").is_file());
    install(&directory).unwrap();
    assert_eq!(std::fs::read(themes.join("light.json")).unwrap(), edited);
}

#[test]
fn bootstrap_legacy_validation_failure_can_retry_without_a_partial_target() {
    for invalid in [b"{broken".as_slice(), b"{}".as_slice()] {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("config");
        let legacy = temp.path().join("newsletters.json");
        std::fs::write(&legacy, invalid).unwrap();
        assert!(install(&directory).is_err());
        assert!(!directory.join("skills.json").exists());
        assert_eq!(std::fs::read(&legacy).unwrap(), invalid);
        let presets = json!([{"id":"repaired-legacy-marker"}]);
        std::fs::write(&legacy, serde_json::to_vec(&presets).unwrap()).unwrap();
        install(&directory).unwrap();
        assert_eq!(
            read(&directory.join("skills.json")).unwrap(),
            json!({"version":1,"presets":presets})
        );
    }
}
#[test]
fn bootstrap_imports_existing_legacy_skills_only_when_target_is_absent() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("config");
    let presets = json!([{"id":"legacy-marker"}]);
    std::fs::write(
        temp.path().join("newsletters.json"),
        serde_json::to_vec(&presets).unwrap(),
    )
    .unwrap();
    install(&directory).unwrap();
    assert_eq!(
        read(&directory.join("skills.json")).unwrap(),
        json!({"version":1,"presets":presets})
    );
}
