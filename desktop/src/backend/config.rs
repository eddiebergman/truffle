use super::*;
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn install(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let manifest = directory.join(".installed-defaults.json");
    let mut installed = if manifest.is_file() {
        arr(&read(&manifest)?)
    } else {
        Vec::new()
    };
    for name in super::assets::CONFIG_FILES {
        let target = directory.join(name.strip_prefix("config/").unwrap());
        if !installed.contains(&json!(name)) {
            if !target.exists() {
                let legacy = directory.parent().unwrap().join("newsletters.json");
                let bytes = if *name == "config/skills.json" && legacy.is_file() {
                    let old = read(&legacy)?;
                    if !old.is_array() {
                        return Err(Error::new("Legacy skills configuration must be an array."));
                    }
                    serde_json::to_vec_pretty(&json!({"version":1,"presets":old}))?
                } else {
                    super::assets::asset(name)
                        .ok_or(Error::new("Missing built-in configuration."))?.to_vec()
                };
                std::fs::create_dir_all(target.parent().unwrap())?;
                super::http::private_write(&target, &bytes)?;
            }
            installed.push(json!(name));
        }
    }
    super::http::private_write(&manifest, &serde_json::to_vec_pretty(&json!(installed))?)?;
    Ok(())
}
pub fn read(path: &Path) -> Result<Value> {
    let bytes = context_kit::read_bounded(path, 512 * 1024)
        .map_err(|_| Error::new(format!("Cannot read configuration: {}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| Error::new(format!("Invalid JSON configuration: {}", path.display())))
}
pub fn defaults(directory: &Path) -> Result<Value> {
    let mut values = read(&directory.join("defaults.json"))?["values"].clone();
    let commands = commands(directory)?;
    values["keybindings"] = json!(commands
        .iter()
        .map(|c| (text(c, "id").to_owned(), c["defaultBinding"].clone()))
        .collect::<serde_json::Map<_, _>>());
    super::settings::validate(&values)?;
    Ok(values)
}
pub fn commands(directory: &Path) -> Result<Vec<Value>> {
    let value = read(&directory.join("commands.json"))?;
    if value["version"] != 1 {
        return Err(Error::new("Unsupported command configuration version."));
    }
    let mut commands = arr(&value["commands"]);
    for builtin in super::settings::commands(){if !commands.iter().any(|c|c["id"]==builtin["id"]){commands.push(builtin);}}
    let builtins = super::settings::commands();
    let ids = commands
        .iter()
        .map(|c| text(c, "id"))
        .collect::<std::collections::BTreeSet<_>>();
    if commands.len() != builtins.len()
        || ids.len() != commands.len()
        || builtins.iter().any(|c| !ids.contains(text(c, "id")))
        || commands.iter().any(|c| {
            text(c, "label").is_empty()
                || text(c, "label").len() > 120
                || !c["defaultBinding"].is_string()
        })
    {
        return Err(Error::new("Command configuration must keep the supported command IDs and define a label and default binding for each."));
    }
    Ok(commands)
}
pub fn fields(directory: &Path) -> Result<Value> {
    let value = read(&directory.join("settings-fields.json"))?;
    if value["version"] != 1 || !value["categories"].is_array() || !value["fields"].is_array() {
        return Err(Error::new("Invalid settings field configuration."));
    }
    let known = super::settings::defaults();
    if arr(&value["fields"]).iter().any(|f| {
        known.get(text(f, "key")).is_none()
            || text(f, "label").is_empty()
            || !["number", "text", "select", "checkbox"].contains(&text(f, "type"))
    }) {
        return Err(Error::new(
            "Settings fields must reference supported settings and input types.",
        ));
    }
    Ok(value)
}
pub fn themes(directory: &Path) -> Result<Value> {
    let mut themes = serde_json::Map::new();
    for entry in std::fs::read_dir(directory.join("themes"))?.take(65) {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if id.is_empty()
            || id.len() > 80
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
        {
            return Err(Error::new(
                "Theme filenames must use letters, numbers, hyphens or underscores.",
            ));
        }
        let theme = read(&path)?;
        if !["dark", "light"].contains(&text(&theme, "type"))
            || !theme["colors"].is_object()
            || !theme["tokenColors"].is_array()
        {
            return Err(Error::new(format!(
                "Theme {id} must define type, colors and tokenColors."
            )));
        }
        for value in theme["colors"].as_object().unwrap().values() {
            let Some(color) = value.as_str() else {
                return Err(Error::new("Theme colors must be hex strings."));
            };
            if !color.starts_with('#')
                || ![4, 5, 7, 9].contains(&color.len())
                || !color[1..].bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(Error::new("Theme colors must be hex strings."));
            }
        }
        let bytes = serde_json::to_vec(&theme)?;
        let hash = format!("{:x}", Sha256::digest(bytes));
        themes.insert(id.into(),json!({"id":id,"label":theme["displayName"].as_str().unwrap_or(id),"type":theme["type"],"syntaxName":format!("truffle-{id}-{}",&hash[..12]),"theme":theme}));
        if themes.len() > 32 {
            return Err(Error::new("Load at most 32 local themes."));
        }
    }
    Ok(json!({"themes":themes}))
}

#[cfg(test)]
mod tests {
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
                std::fs::write(directory.join(".installed-defaults.json"), serde_json::to_vec(&receipt).unwrap()).unwrap();
            }
            install(&directory).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), custom);
            install(&directory).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), custom);
            assert_eq!(std::fs::read(temp.path().join("newsletters.json")).unwrap(), legacy);
        }
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
            assert_eq!(read(&directory.join("skills.json")).unwrap(), json!({"version":1,"presets":presets}));
        }
    }
    #[test]
    fn bootstrap_imports_existing_legacy_skills_only_when_target_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("config");
        let presets = json!([{"id":"legacy-marker"}]);
        std::fs::write(temp.path().join("newsletters.json"), serde_json::to_vec(&presets).unwrap()).unwrap();
        install(&directory).unwrap();
        assert_eq!(read(&directory.join("skills.json")).unwrap(), json!({"version":1,"presets":presets}));
    }
}
