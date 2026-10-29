//! Raw skill definitions and their custom overlay. Display paths and availability
//! belong to artifacts; edit tokens and writes belong to scheduler.
use super::*;
use std::collections::BTreeSet;

const MAX_CONFIG_BYTES: u64 = 512 * 1024;
const MAX_PRESETS: usize = 30;

pub struct Source {
    document: Value,
    pub hash: String,
}
fn config_path(repo: &Repository) -> PathBuf {
    repo.data.parent().unwrap().join("config/skills.json")
}
fn rows(value: &Value) -> Result<&Vec<Value>> {
    let rows = value.as_array().filter(|rows| rows.len() <= MAX_PRESETS)
        .ok_or(Error::new("Keep up to 30 skill presets in an array."))?;
    let mut ids = BTreeSet::new();
    for row in rows {
        let id = checked(row, "id", 80, true)?;
        if row["id"].as_str() != Some(id.as_str()) || !ids.insert(id) {
            return Err(Error::new("Skill IDs must be exact and unique."));
        }
    }
    Ok(rows)
}
/// The byte hash and raw document always come from the same bounded read.
pub fn load(repo: &Repository) -> Result<Source> {
    let bytes = context_kit::read_bounded(&config_path(repo), MAX_CONFIG_BYTES)
        .map_err(|_| Error::new("Cannot read the skills configuration."))?;
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("Invalid skills configuration."))?;
    if document["version"] != 1 {
        return Err(Error::new("Unsupported skills configuration version."));
    }
    rows(&document["presets"])?;
    Ok(Source { document, hash: context_kit::sha256(&bytes) })
}
impl Source {
    /// Call again inside the canonical transaction before materializing/editing
    /// captured configured definitions. This is an admission fence, not a lock
    /// against subsequent external configuration edits.
    pub fn is_current(&self, repo: &Repository) -> Result<bool> {
        let bytes = context_kit::read_bounded(&config_path(repo), MAX_CONFIG_BYTES)
            .map_err(|_| Error::new("Cannot validate the skills configuration."))?;
        Ok(context_kit::sha256(&bytes) == self.hash)
    }
    pub fn overlay(&self, state: &Value) -> Result<Vec<Value>> {
        let mut entries = rows(&self.document["presets"])?.clone();
        let empty = json!([]);
        let custom = rows(if state["skillPresets"].is_null() { &empty } else { &state["skillPresets"] })?;
        for row in custom {
            entries.retain(|entry| entry["id"] != row["id"]);
            entries.push(row.clone());
        }
        if entries.len() > MAX_PRESETS {
            return Err(Error::new("Keep up to 30 skill presets."));
        }
        Ok(entries)
    }
    pub fn find(&self, state: &Value, id: &str) -> Result<(Value, bool)> {
        let row = self.overlay(state)?.into_iter().find(|row| row["id"] == id)
            .ok_or_else(|| { let mut error = Error::new("Skill unavailable. Read Skills again before editing it."); error.status = 404; error })?;
        let custom = state["skillPresets"].as_array().is_some_and(|rows| rows.iter().any(|row| row["id"] == id));
        Ok((row, custom))
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(entries, vec![document["presets"][0].clone(), custom.clone(), state["skillPresets"][1].clone()]);
        assert_eq!(source.find(&state, "override").unwrap(), (custom, true));
        assert_eq!(source.find(&state, "first").unwrap(), (document["presets"][0].clone(), false));
        assert_eq!(std::fs::read(config_path(&repo)).unwrap(), bytes);
    }
    #[test]
    fn raw_skill_source_validation_is_identical_for_catalog_and_editor() {
        let (_temp, repo) = fixture();
        for document in [json!({"version":2,"presets":[]}),json!({"version":1,"presets":{}}),
            json!({"version":1,"presets":[{"id":"duplicate"},{"id":"duplicate"}]}),
            json!({"version":1,"presets":[{"id":" padded "}]}),
            json!({"version":1,"presets":(0..31).map(|i|json!({"id":format!("skill-{i}")})).collect::<Vec<_>>()})] {
            configured(&repo, &document);
            let state = json!({"skillPresets":[{"id":"duplicate","cwd":"/missing","skill":"SKILL.md"}]});
            let catalog_error = super::super::artifacts::presets(&repo, &state).unwrap_err();
            let editor_error = super::super::scheduler::preset_read(&repo, &json!({"presetId":"duplicate"})).unwrap_err();
            assert_eq!(catalog_error.message, editor_error.message);
            assert!(load(&repo).is_err());
        }
        std::fs::write(config_path(&repo), vec![b' '; MAX_CONFIG_BYTES as usize + 1]).unwrap();
        assert!(load(&repo).err().unwrap().message.contains("Cannot read"));
    }
    #[test]
    fn raw_skill_overlay_rejects_ambiguous_custom_ids_and_combined_capacity() {
        let (_temp, repo) = fixture();
        configured(&repo, &json!({"version":1,"presets":[{"id":"configured"}]}));
        let source = load(&repo).unwrap();
        for custom in [json!([{"id":"same"},{"id":"same"}]),json!({"id":"not-an-array"}),
            json!((0..30).map(|i|json!({"id":format!("custom-{i}")})).collect::<Vec<_>>())] {
            let state = json!({"skillPresets":custom});
            assert!(source.overlay(&state).is_err());
            assert!(source.find(&state, "configured").is_err());
        }
        assert_eq!(source.overlay(&json!({})).unwrap(), vec![json!({"id":"configured"})]);
    }
    #[test]
    fn raw_skill_source_exact_byte_fence_and_raw_materialization_stay_separate_from_projection() {
        let (temp, repo) = fixture();
        std::fs::write(temp.path().join("SKILL.md"), "Fixture").unwrap();
        let row = json!({"id":"configured","title":"Original","cwd":temp.path(),"skill":"SKILL.md","reports":"historic/reports","command":"$fixture","agent":"codex","provenance":{"keep":true}});
        let document = json!({"version":1,"presets":[row],"privateRoot":"keep"});
        let bytes = configured(&repo, &document);
        let source = load(&repo).unwrap();
        let baseline = super::super::scheduler::preset_read(&repo, &json!({"presetId":"configured"})).unwrap();
        let state = repo.state.lock().unwrap().clone();
        let projected = super::super::artifacts::presets(&repo, &state).unwrap();
        assert_eq!(projected[0]["skill"], json!(temp.path().join("SKILL.md")));
        assert_eq!(source.find(&state, "configured").unwrap().0["skill"], "SKILL.md");
        let canonical_bytes = std::fs::read(&repo.data).unwrap();
        let mut changed_bytes = bytes.clone(); changed_bytes.push(b'\n');
        std::fs::write(config_path(&repo), &changed_bytes).unwrap();
        assert!(!source.is_current(&repo).unwrap());
        let args = json!({"presetId":"configured","patch":{"title":"Edited"},"expected":{"title":baseline["expected"]["title"]}});
        assert_eq!(super::super::scheduler::update_preset(&repo, &args).unwrap_err().status, 409);
        assert_eq!(std::fs::read(&repo.data).unwrap(), canonical_bytes);
        std::fs::write(config_path(&repo), &bytes).unwrap();
        super::super::scheduler::update_preset(&repo, &args).unwrap();
        let saved = repo.state.lock().unwrap()["skillPresets"][0].clone();
        assert_eq!(saved["skill"], "SKILL.md");assert_eq!(saved["reports"], "historic/reports");assert_eq!(saved["provenance"], row["provenance"]);
        assert!(saved.get("available").is_none());
        assert_eq!(std::fs::read(config_path(&repo)).unwrap(), bytes);
    }
}
