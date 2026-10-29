use super::*;
use std::{collections::BTreeSet, io::Write, os::unix::fs::OpenOptionsExt, sync::Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum Approval { Routine, User }

// This policy is code-owned. Editable labels and field metadata cannot grant access.
fn approval(key: &str) -> Approval {
    match key {
        "theme" | "uiScale" | "density" | "terminalFontSize" | "terminalScrollback"
        | "diffFontSize" | "diffLayout" | "fileDiffLayout" | "previewHeight"
        | "enterToComment" | "defaultAgent" | "keybindings" => Approval::Routine,
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
        .ok_or(Error::new("Settings must be an object."))?;
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
            .ok_or(Error::new("Review commands must be text."))?;
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
    let bindings = values["keybindings"]
        .as_object()
        .ok_or(Error::new("Keybindings must be an object."))?;
    if bindings.len() != commands().len()
        || bindings
            .keys()
            .any(|k| !commands().iter().any(|c| text(c, "id") == k))
    {
        return Err(Error::new("Unknown or missing keyboard command."));
    }
    let mut seen = BTreeSet::new();
    for chord in bindings.values() {
        let chord = chord.as_str().ok_or(Error::new("Shortcut must be text."))?;
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
        .iter().any(|reserved| reserved.eq_ignore_ascii_case(chord))
        {
            return Err(Error::new(
                "This shortcut is reserved for editing or quitting.",
            ));
        }
    }
    let actions = values["quickActions"]
        .as_array()
        .ok_or(Error::new("Quick actions must be an array."))?;
    if actions.len() > 20 {
        return Err(Error::new("Save at most 20 quick actions."));
    }
    let mut ids = BTreeSet::new();
    for action in actions {
        let a = action
            .as_object()
            .ok_or(Error::new("Quick action must be an object."))?;
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
    let values = document["values"].as_object_mut()
        .ok_or(Error::new("Settings values must be an object."))?;
    let bindings = values.get_mut("keybindings").and_then(Value::as_object_mut)
        .ok_or(Error::new("Keybindings must be an object."))?;
    for command in commands() {
        let id = text(&command, "id");
        if !bindings.contains_key(id) {
            let binding = if bindings.values().any(|value| value == &command["defaultBinding"]) {
                json!("")
            } else {
                command["defaultBinding"].clone()
            };
            bindings.insert(id.to_owned(), binding);
        }
    }
    validate(&document["values"])
}
fn document_hash(document: &Value) -> Result<String> {
    Ok(context_kit::sha256(&serde_json::to_vec(document)?))
}

pub struct Store {
    path: PathBuf,
    directory: PathBuf,
    document: Mutex<Value>,
}
impl Store {
    pub fn load(path: PathBuf) -> Result<Self> {
        let directory = path.parent().unwrap().join("config");
        super::config::install(&directory)?;
        let initial = super::config::defaults(&directory)?;
        let doc = if path.exists() {
            let bytes = std::fs::read(&path)?;
            if bytes.len() > 128 * 1024 {
                return Err(Error::new("Settings file exceeds 128 KiB."));
            }
            let mut v: Value = serde_json::from_slice(&bytes)?;
            normalize_document(&mut v)?;
            v
        } else {
            json!({"version":1,"revision":0,"values":initial})
        };
        if !path.exists() {
            super::http::private_write(&path, &serde_json::to_vec_pretty(&doc)?)?;
        }
        Ok(Self {
            path,
            directory,
            document: Mutex::new(doc),
        })
    }
    fn refresh(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut external = super::config::read(&self.path)?;
        normalize_document(&mut external)?;
        let mut doc = self.document.lock().map_err(|_| Error::io())?;
        let revision=doc["revision"].as_u64().unwrap();
        let values_changed=external["values"] != doc["values"];
        external["revision"]=json!(external["revision"].as_u64().unwrap().max(if values_changed {
            revision.checked_add(1).ok_or(Error::new("Settings revision is exhausted."))?
        } else {revision}));
        if external != *doc { *doc=external; }
        Ok(())
    }
    pub fn values(&self) -> Value {
        let _ = self.refresh();
        self.document.lock().unwrap()["values"].clone()
    }
    pub fn view(&self) -> Value {
        let error = self.refresh().err();
        let mut doc = self.document.lock().unwrap().clone();
        match document_hash(&doc) {Ok(hash)=>doc["contentHash"]=json!(hash),Err(_)=>doc["configError"]=json!("Settings configuration could not load.")}
        doc["defaults"] = defaults();
        doc["commands"] = json!(commands());
        doc["approval"] = json!(defaults().as_object().unwrap().keys()
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
        if let Some(e) = error.or(metadata.err()) {
            doc["configError"] = json!(e.message);
        }
        match context_kit::read_bounded(&self.directory.join("workflows/agent-review.md"), 16000) {
            Ok(bytes) => {
                if let Ok(template) = String::from_utf8(bytes) {
                    doc["workflows"] = json!({"agent-review":super::workflows::review_skill_discovery(&template)});
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
    fn proposed_values(&self, doc: &Value, p: &Value, agent: bool) -> Result<Value> {
        if p["revision"].as_u64().is_none() || p["revision"] != doc["revision"]
            || p["contentHash"].as_str() != Some(document_hash(doc)?.as_str()) {
            let mut error=Error::new("Settings changed elsewhere. Reload settings before saving.");error.status=409;return Err(error);
        }
        let patch = p["patch"]
            .as_object()
            .ok_or(Error::new("Supply a settings patch."))?;
        let mut values = doc["values"].clone();
        for (key, value) in patch {
            if agent
                && approval(key) == Approval::User
                && values[key] != *value
            {
                return Err(Error::new("This changes executable commands or workflows. Ask the user to approve it in Settings."));
            }
            values[key] = value.clone();
        }
        validate(&values)?;
        if values["theme"] != "system"
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
    pub fn preview(&self, p: &Value, agent: bool) -> Result<Value> {
        self.refresh()?;
        let doc = self.document.lock().map_err(|_| Error::io())?;
        let values = self.proposed_values(&doc, p, agent)?;
        Ok(json!({"revision":doc["revision"],"contentHash":document_hash(&doc)?,"values":values}))
    }
    pub fn apply(&self, p: &Value, agent: bool) -> Result<Value> {
        self.refresh()?;
        let mut doc = self.document.lock().map_err(|_| Error::io())?;
        let values = self.proposed_values(&doc, p, agent)?;
        if values == doc["values"] {
            drop(doc);
            return Ok(self.view());
        }
        let next = json!({"version":1,"revision":doc["revision"].as_u64().unwrap()+1,"values":values,"updatedAt":now(),"updatedBy":if agent{"agent"}else{"user"}});
        let temp = self.path.with_extension(format!("{}.tmp", uuid()));
        let result = (|| -> Result<()> {
            let mut f = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temp)?;
            f.write_all(&serde_json::to_vec_pretty(&next)?)?;
            f.sync_all()?;
            std::fs::rename(&temp, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result?;
        *doc = next;
        drop(doc);
        Ok(self.view())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_settings_containers_fail_safely_on_load_and_refresh_without_writes() {
        for (level, malformed) in [
            ("document", Value::Null), ("document", json!([])), ("document", json!("bad")),
            ("values", Value::Null), ("values", json!([])), ("values", json!("bad")),
            ("keybindings", Value::Null), ("keybindings", json!([])), ("keybindings", json!("bad")),
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
            assert!(Store::load(path.clone()).is_err(), "startup must reject malformed {level}");
            let current = store.view();
            assert!(current["configError"].is_string(), "refresh must report malformed {level}");
            assert_eq!(current["values"], baseline["values"]);
            let patch = json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"patch":{"uiScale":120}});
            assert!(store.preview(&patch, false).is_err());
            assert!(store.apply(&patch, false).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn document_content_hash_rejects_stale_and_missing_baselines_without_persisting_metadata() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("settings.json");let store=Store::load(path.clone()).unwrap();
        let baseline=store.view();assert_eq!(baseline["contentHash"],store.view()["contentHash"]);
        let mut external:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();external["updatedBy"]=json!("manual-edit-same-revision");
        std::fs::write(&path,serde_json::to_vec(&external).unwrap()).unwrap();let bytes=std::fs::read(&path).unwrap();
        let fresh=store.view();assert_eq!(fresh["revision"],baseline["revision"]);assert_ne!(fresh["contentHash"],baseline["contentHash"]);assert_eq!(fresh["contentHash"],store.view()["contentHash"]);
        for request in [json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"patch":{"uiScale":120}}),json!({"revision":fresh["revision"],"patch":{}}),json!({"contentHash":fresh["contentHash"],"patch":{}})] {
            assert_eq!(store.preview(&request,false).unwrap_err().status,409);assert_eq!(store.apply(&request,false).unwrap_err().status,409);assert_eq!(std::fs::read(&path).unwrap(),bytes);
        }
        std::fs::write(store.config_directory().join("workflows/agent-review.md"),"metadata-only-workflow-change").unwrap();assert_eq!(store.view()["contentHash"],fresh["contentHash"]);
        let request=json!({"revision":fresh["revision"],"contentHash":fresh["contentHash"],"patch":{"uiScale":120}});
        let preview=store.preview(&request,false).unwrap();assert_eq!(preview["contentHash"],fresh["contentHash"]);assert_eq!(std::fs::read(&path).unwrap(),bytes);
        let applied=store.apply(&request,false).unwrap();assert_ne!(applied["contentHash"],fresh["contentHash"]);
        let persisted:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();assert!(persisted.get("contentHash").is_none());
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
        let user = ["quickActions", "editor", "claudeReviewCommand", "codexReviewCommand"];
        let routine = ["theme", "uiScale", "density", "terminalFontSize", "terminalScrollback", "diffFontSize", "diffLayout", "fileDiffLayout", "previewHeight", "enterToComment", "defaultAgent", "keybindings"];
        let known = defaults();
        assert_eq!(known.as_object().unwrap().len(), user.len() + routine.len());
        for key in user { assert!(known.get(key).is_some()); assert_eq!(view["approval"][key], "user"); }
        for key in routine { assert!(known.get(key).is_some()); assert_eq!(view["approval"][key], "routine"); }
        assert_eq!(approval("future-setting"), Approval::User);
        assert_eq!(approval(""), Approval::User);
        let mut metadata = super::super::config::read(&store.config_directory().join("settings-fields.json")).unwrap();
        metadata["approval"] = json!({"quickActions":"routine","theme":"user"});
        std::fs::write(store.config_directory().join("settings-fields.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert_eq!(store.view()["approval"], view["approval"]);
    }

    #[test]
    fn agent_preview_and_apply_share_approval_policy() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let store = Store::load(path.clone()).unwrap();
        let before = std::fs::read(&path).unwrap();
        for patch in [json!({"quickActions":[{"id":"test","title":"Test","command":"printf test"}]}), json!({"editor":"code"}), json!({"claudeReviewCommand":"/review"}), json!({"codexReviewCommand":"$review"}), json!({"future-setting":true})] {
            let request = json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":patch});
            assert!(store.preview(&request, true).is_err());
            assert!(store.apply(&request, true).is_err());
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let request = json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"defaultAgent":"codex","uiScale":120,"keybindings":store.values()["keybindings"]}});
        let preview = store.preview(&request, true).unwrap();
        assert_eq!(store.apply(&request, true).unwrap()["values"], preview["values"]);
        let request = json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"editor":"code"}});
        assert!(store.preview(&request, false).is_ok());
        assert_eq!(store.apply(&request, false).unwrap()["values"]["editor"], "code");
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
        assert_eq!(skill, store.config_directory().join("skills/truffle-review/SKILL.md"));
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
        for patch in [json!({"uiScale":-1}), json!({"uiScale":null}), json!({"density":"bad"}), json!({"quickActions":[{"id":"","title":"","command":""}]}), json!({"theme":"unknown-theme-fixture"})] {
            let request = json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":patch});
            assert!(store.preview(&request, false).is_err());
            assert!(store.apply(&request, false).is_err());
        }
        let request = json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"uiScale":120}});
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
    fn settings_persist_reject_conflicts_and_gate_agent_commands() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let store = Store::load(path.clone()).unwrap();
        let changed = store
            .apply(
                &json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"theme":"light","diffFontSize":11}}),
                true,
            )
            .unwrap();
        assert_eq!(changed["revision"], 1);
        assert_eq!(Store::load(path).unwrap().values()["theme"], "light");
        assert!(store
            .apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"theme":"midnight"}}), false)
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
}
#[cfg(test)]
mod editable_file_tests {
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
            .apply(&json!({"revision":0,"contentHash":store.view()["contentHash"],"patch":{"diffFontSize":11}}), false)
            .is_err());
        std::fs::write(&path, b"broken").unwrap();
        assert_eq!(store.view()["values"]["theme"], "catppuccin-latte");
        assert!(store.view()["configError"].is_string());
        assert!(store
            .apply(&json!({"revision":1,"contentHash":store.view()["contentHash"],"patch":{"diffFontSize":11}}), false)
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
}
