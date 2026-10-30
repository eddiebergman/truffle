use super::document::{Document, Format, Version};
use super::{arr, json, now, text, Error, PathBuf, Result, Value};
use std::{collections::BTreeSet, sync::Mutex};

/// Who must approve a settings change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Approval {
    Routine,
    User,
}

// This policy is code-owned. Editable labels and field metadata cannot grant access.
fn approval(key: &str) -> Approval {
    match key {
        "theme" | "uiScale" | "density" | "terminalFontSize" | "terminalScrollback"
        | "diffFontSize" | "diffLayout" | "fileDiffLayout" | "previewHeight" | "enterToComment"
        | "defaultAgent" | "keybindings" => Approval::Routine,
        _ => Approval::User,
    }
}

pub fn routine_view(mut document: Value) -> Value {
    document.as_object_mut().unwrap().remove("workflows");
    for section in ["values", "defaults"] {
        if let Some(values) = document[section].as_object_mut() {
            values.retain(|key, _| approval(key) == Approval::Routine);
        }
    }
    document
}

pub fn commands() -> Vec<Value> {
    arr(
        &serde_json::from_str::<Value>(include_str!("../../../static/config/commands.json"))
            .unwrap()["commands"],
    )
}
pub fn defaults() -> Value {
    let mut values: Value =
        serde_json::from_str::<Value>(include_str!("../../../static/config/defaults.json"))
            .unwrap()["values"]
            .clone();
    values["keybindings"] = json!(commands()
        .iter()
        .map(|c| (text(c, "id").to_owned(), c["defaultBinding"].clone()))
        .collect::<serde_json::Map<_, _>>());
    values
}
pub fn valid_chord(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let parts = value.split('+').collect::<Vec<_>>();
    let key = parts.last().copied().unwrap_or("");
    parts.first() == Some(&"Primary")
        && parts.len() <= 4
        && parts.len() >= 2
        && parts[..parts.len() - 1]
            .iter()
            .all(|s| ["Primary", "Shift", "Alt"].contains(s))
        && parts[..parts.len() - 1]
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            == parts.len() - 1
        && (key == "Enter"
            || key.len() == 1
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b',' || b == b'.' || b == b'/'))
}
pub fn validate(values: &Value) -> Result<()> {
    let base = defaults();
    let object = values
        .as_object()
        .ok_or_else(|| Error::new("Settings must be an object."))?;
    if object.keys().any(|k| base.get(k).is_none())
        || object.len() != base.as_object().unwrap().len()
    {
        return Err(Error::new("Unknown or missing setting."));
    }
    for (key, allowed) in [
        ("density", vec!["compact", "comfortable"]),
        ("diffLayout", vec!["unified", "split"]),
        ("fileDiffLayout", vec!["unified", "split"]),
        ("defaultAgent", vec!["claude", "codex"]),
        ("editor", vec!["zed", "code", "cursor"]),
    ] {
        if !allowed.contains(&text(values, key)) {
            return Err(Error::new(format!("Invalid {key} setting.")));
        }
    }
    let theme = text(values, "theme");
    if theme.is_empty()
        || theme.len() > 80
        || !theme
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
    {
        return Err(Error::new("Invalid theme ID."));
    }
    for (key, min, max) in [
        ("uiScale", 85, 130),
        ("terminalFontSize", 9, 22),
        ("terminalScrollback", 500, 20000),
        ("diffFontSize", 8, 20),
        ("previewHeight", 30, 85),
    ] {
        if !values[key].as_u64().is_some_and(|v| v >= min && v <= max) {
            return Err(Error::new(format!(
                "{key} must be between {min} and {max}."
            )));
        }
    }
    if !values["enterToComment"].is_boolean() {
        return Err(Error::new("Enter to comment must be true or false."));
    }
    for (key, prefix) in [("claudeReviewCommand", "/"), ("codexReviewCommand", "$")] {
        let s = values[key]
            .as_str()
            .ok_or_else(|| Error::new("Review commands must be text."))?;
        if !s.is_empty()
            && (!s.starts_with(prefix)
                || s.len() > 100
                || !s[1..]
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
                || s.len() < 2)
        {
            return Err(Error::new(
                "Choose a skill name, such as /review for Claude or $review for Codex.",
            ));
        }
    }
    validate_keybindings(values)?;
    validate_quick_actions(values)
}
fn validate_keybindings(values: &Value) -> Result<()> {
    let bindings = values["keybindings"]
        .as_object()
        .ok_or_else(|| Error::new("Keybindings must be an object."))?;
    if bindings.len() != commands().len()
        || bindings
            .keys()
            .any(|k| !commands().iter().any(|c| text(c, "id") == k))
    {
        return Err(Error::new("Unknown or missing keyboard command."));
    }
    let mut seen = BTreeSet::new();
    for chord in bindings.values() {
        let chord = chord
            .as_str()
            .ok_or_else(|| Error::new("Shortcut must be text."))?;
        if !valid_chord(chord) {
            return Err(Error::new("Use Primary, optional Shift/Alt, and a letter, number, comma, period, slash or Enter."));
        }
        if !chord.is_empty() && !seen.insert(chord.to_ascii_uppercase()) {
            return Err(Error::new("Two commands cannot use the same shortcut."));
        }
        if [
            "Primary+C",
            "Primary+V",
            "Primary+X",
            "Primary+A",
            "Primary+Z",
            "Primary+Shift+Z",
            "Primary+Q",
        ]
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(chord))
        {
            return Err(Error::new(
                "This shortcut is reserved for editing or quitting.",
            ));
        }
    }
    Ok(())
}
fn validate_quick_actions(values: &Value) -> Result<()> {
    let actions = values["quickActions"]
        .as_array()
        .ok_or_else(|| Error::new("Quick actions must be an array."))?;
    if actions.len() > 20 {
        return Err(Error::new("Save at most 20 quick actions."));
    }
    let mut ids = BTreeSet::new();
    for action in actions {
        let a = action
            .as_object()
            .ok_or_else(|| Error::new("Quick action must be an object."))?;
        if a.keys()
            .any(|k| !["id", "title", "command"].contains(&k.as_str()))
        {
            return Err(Error::new("Unknown quick action field."));
        }
        let id = text(action, "id");
        let title = text(action, "title");
        let command = text(action, "command");
        if id.is_empty()
            || id.len() > 80
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            || !ids.insert(id)
            || title.trim().is_empty()
            || title.len() > 100
            || command.trim().is_empty()
            || command.len() > 4000
            || command.contains('\0')
        {
            return Err(Error::new(
                "Supply a unique action ID, title and command within their limits.",
            ));
        }
    }
    Ok(())
}
// Validate containers before adding newly supported shortcut defaults. Bad disk
// documents stay untouched; both startup and refresh use this same boundary.
fn normalize_document(document: &mut Value) -> Result<()> {
    if !document.is_object() || document["version"] != 1 || !document["revision"].is_u64() {
        return Err(Error::new("Unsupported settings file version or revision."));
    }
    let values = document["values"]
        .as_object_mut()
        .ok_or_else(|| Error::new("Settings values must be an object."))?;
    let bindings = values
        .get_mut("keybindings")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::new("Keybindings must be an object."))?;
    for command in commands() {
        let id = text(&command, "id");
        if !bindings.contains_key(id) {
            let binding = if bindings
                .values()
                .any(|value| value == &command["defaultBinding"])
            {
                json!("")
            } else {
                command["defaultBinding"].clone()
            };
            bindings.insert(id.to_owned(), binding);
        }
    }
    validate(&document["values"])
}
#[derive(serde::Serialize)]
#[serde(transparent)]
struct Preferences(Value);
impl Format for Preferences {
    const NAME: &'static str = "settings file";
    const MAX_BYTES: usize = 128 * 1024;
    const CONFLICT: &'static str = "Settings changed elsewhere. Reload settings before saving.";
    fn parse(bytes: &[u8]) -> Result<Self> {
        let mut document = serde_json::from_slice(bytes)?;
        normalize_document(&mut document)?;
        Ok(Self(document))
    }
    fn revision(&self) -> u64 {
        self.0["revision"].as_u64().unwrap()
    }
}

pub struct Store {
    file: Document<Preferences>,
    path: PathBuf,
    directory: PathBuf,
    /// The last valid file. A broken hand edit leaves these values in effect.
    last: Mutex<(Value, Version)>,
}
impl Store {
    pub fn load(path: PathBuf) -> Result<Self> {
        let directory = path.parent().unwrap().join("config");
        super::config::install(&directory)?;
        let initial = serde_json::to_vec_pretty(
            &json!({"version":1,"revision":0,"values":super::config::defaults(&directory)?}),
        )?;
        if !path.exists() {
            super::http::private_write(&path, &initial)?;
        }
        let file = Document::new(path.clone()).or_missing(initial);
        let (document, version) = file.read()?;
        Ok(Self {
            file,
            path,
            directory,
            last: Mutex::new((document.0, version)),
        })
    }
    fn refresh(&self) -> Result<()> {
        let (document, version) = self.file.read()?;
        *self.last.lock().map_err(|_| Error::io())? = (document.0, version);
        Ok(())
    }
    pub fn values(&self) -> Value {
        let _ = self.refresh();
        self.last.lock().unwrap().0["values"].clone()
    }
    pub fn view(&self) -> Value {
        let error = self.refresh().err();
        let (mut doc, version) = self.last.lock().unwrap().clone();
        doc["contentHash"] = json!(version.content_hash);
        doc["defaults"] = defaults();
        doc["commands"] = json!(commands());
        doc["approval"] = json!(defaults()
            .as_object()
            .unwrap()
            .keys()
            .map(|key| (key.clone(), approval(key)))
            .collect::<std::collections::BTreeMap<_, _>>());
        let builtin: Value =
            serde_json::from_str(include_str!("../../../static/config/settings-fields.json"))
                .unwrap();
        doc["fields"] = builtin["fields"].clone();
        doc["categories"] = builtin["categories"].clone();
        let metadata = (|| -> Result<()> {
            doc["defaults"] = super::config::defaults(&self.directory)?;
            doc["commands"] = json!(super::config::commands(&self.directory)?);
            let fields = super::config::fields(&self.directory)?;
            doc["fields"] = fields["fields"].clone();
            doc["categories"] = fields["categories"].clone();
            Ok(())
        })();
        if let Some(e) = error.or_else(|| metadata.err()) {
            doc["configError"] = json!(e.message);
        }
        match context_kit::read_bounded(&self.directory.join("workflows/agent-review.md"), 16000) {
            Ok(bytes) => {
                if let Ok(template) = String::from_utf8(bytes) {
                    doc["workflows"] =
                        json!({"agent-review":super::workflows::review_skill_discovery(&template)});
                }
            }
            Err(_) => doc["configError"] = json!("Cannot read workflows/agent-review.md."),
        }
        doc["reviewSkill"] = super::skills_catalog::review_skill(&self.directory);
        doc["files"] = json!({"preferences":self.path,"configuration":self.directory,"themes":self.directory.join("themes")});
        doc
    }
    pub fn config_directory(&self) -> &std::path::Path {
        &self.directory
    }
    pub fn themes(&self) -> Result<Value> {
        super::config::themes(&self.directory)
    }
    fn proposed_values(&self, doc: &Value, p: &Value, granted: Approval) -> Result<Value> {
        let patch = p["patch"]
            .as_object()
            .ok_or_else(|| Error::new("Supply a settings patch."))?;
        let mut values = doc["values"].clone();
        for (key, value) in patch {
            if granted == Approval::Routine
                && approval(key) == Approval::User
                && values[key] != *value
            {
                return Err(Error::new("This changes executable commands or workflows. Ask the user to approve it in Settings."));
            }
            values[key] = value.clone();
        }
        validate(&values)?;
        // A saved theme whose file was removed stays valid until the user picks another one.
        if values["theme"] != doc["values"]["theme"]
            && values["theme"] != "system"
            && self.themes()?["themes"]
                .get(text(&values, "theme"))
                .is_none()
        {
            return Err(Error::new(
                "Choose an installed theme, or add its JSON file to the themes folder.",
            ));
        }
        Ok(values)
    }
    /// Validate a patch that `granted` may approve, without saving it.
    pub fn preview(&self, p: &Value, granted: Approval) -> Result<Value> {
        let expected = Version::sent(p);
        let current = self.file.read_at(&expected)?.0;
        let values = self.proposed_values(&current, p, granted)?;
        Ok(
            json!({"revision":expected.revision,"contentHash":expected.content_hash,"values":values}),
        )
    }
    /// Save a patch that `granted` may approve.
    pub fn apply(&self, p: &Value, granted: Approval) -> Result<Value> {
        let expected = Version::sent(p);
        let current = self.file.read_at(&expected)?.0;
        let values = self.proposed_values(&current, p, granted)?;
        if values != current["values"] {
            let next = json!({"version":1,"revision":expected.revision,"values":values,"updatedAt":now(),"updatedBy":match granted{Approval::Routine=>"agent",Approval::User=>"user"}});
            self.file.replace(&expected, Preferences(next))?;
        }
        Ok(self.view())
    }
}
#[cfg(test)]
mod editable_file_tests;
#[cfg(test)]
mod tests;
