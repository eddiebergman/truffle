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
                        .ok_or(Error::new("Missing built-in configuration."))?
                        .to_vec()
                };
                std::fs::create_dir_all(target.parent().unwrap())?;
                super::http::private_write(&target, &bytes)?;
            }
            installed.push(json!(name));
        }
    }
    super::http::private_write(&manifest, &serde_json::to_vec_pretty(&json!(installed))?)?;
    update_untouched_themes(directory)
}

// SHA-256 of theme files shipped by earlier releases. A local copy with one of
// these hashes was never edited, so it follows the packaged file: replaced, or
// removed when the theme is no longer packaged.
const PREVIOUS_THEMES: &[(&str, &str)] = &[
    (
        "catppuccin-latte",
        "6f6a644bdfc2f9f726fbe4500019c115dd6a18ba0116357537bcc44ca782d1a5",
    ),
    (
        "catppuccin-mocha",
        "43298c436f7527ef2deee6c5af9fbea61f5eaf547f5b36295c00654e87a9bd88",
    ),
    (
        "dracula",
        "f026b056d5321f7e8469fd811ced975d98e0222a2515d27879f149f751763573",
    ),
    (
        "github-dark",
        "69b7a9693e9814d478a7d1fdafa52c24cb609b0b0832f50c0a415bf988364d91",
    ),
    (
        "github-light",
        "711147741f72fc0db072b09a28de8bedb8984997d572cee9c52c47fa409f17d8",
    ),
    (
        "gruvbox-dark-medium",
        "8d301867af3eb863e1ae289e8657264e244ffbf2c12f4c40cd13635b102f0046",
    ),
    (
        "gruvbox-light-medium",
        "0fee7e7311f0c1c7b15d01cdb327b9651f7220b1702a3bdfcf9b1ead55752d5b",
    ),
    (
        "light",
        "f635b4f6b82f6d680336fd80480e40f06515fb943d5dbac7f8be6ef40ffcf560",
    ),
    (
        "midnight",
        "c080a63c087c6a6ce5194f256685c7d250a87b7570c4fd3904dad38e0311d1ab",
    ),
    (
        "nord",
        "0ea054997b6bbe5ca17baae04576190ca34ab06e52908d8831a6cca91b49d090",
    ),
    (
        "rose-pine-dawn",
        "9553200699233f23be58851bdd5deac89cdc23a0112bf15910993869c38c15a2",
    ),
    (
        "rose-pine",
        "84e7b0392c6ff3daff10814a5e8dbfd4d1d3649e1ed2d7942ed4986bb0558418",
    ),
    (
        "rose-pine",
        "db12f0f527acd086ca47a31eb7ffacc50ae68c982c3139fab20033c0195880a5",
    ),
    (
        "solarized-dark",
        "ed5257c5f01ecd940f370cd354dda483555e1f3155195777216ff5242d40d566",
    ),
    (
        "solarized-light",
        "9f0e5d45dd69e0c25bfd585c17a1d78e2921668b0e29e615d419aadb16cbbe12",
    ),
];

fn update_untouched_themes(directory: &Path) -> Result<()> {
    for (id, hash) in PREVIOUS_THEMES {
        let target = directory.join(format!("themes/{id}.json"));
        let Ok(bytes) = std::fs::read(&target) else {
            continue;
        };
        if format!("{:x}", Sha256::digest(&bytes)) != *hash {
            continue;
        }
        match super::assets::asset(&format!("config/themes/{id}.json")) {
            Some(packaged) => super::http::private_write(&target, packaged)?,
            None => std::fs::remove_file(&target)?,
        }
    }
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
    for builtin in super::settings::commands() {
        if !commands.iter().any(|c| c["id"] == builtin["id"]) {
            commands.push(builtin);
        }
    }
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
    for entry in std::fs::read_dir(directory.join("themes"))?.take(129) {
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
        if themes.len() > 64 {
            return Err(Error::new("Load at most 64 local themes."));
        }
    }
    Ok(json!({"themes":themes}))
}

#[cfg(test)]
mod tests;
