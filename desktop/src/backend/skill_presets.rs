//! Raw skill definitions and their custom overlay. Display paths and availability
//! belong to artifacts; edit tokens and writes belong to scheduler.
use super::{checked, json, Error, PathBuf, Repository, Result, Value};
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
    let rows = value
        .as_array()
        .filter(|rows| rows.len() <= MAX_PRESETS)
        .ok_or_else(|| Error::new("Keep up to 30 skill presets in an array."))?;
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
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|_| Error::new("Invalid skills configuration."))?;
    if document["version"] != 1 {
        return Err(Error::new("Unsupported skills configuration version."));
    }
    rows(&document["presets"])?;
    Ok(Source {
        document,
        hash: context_kit::sha256(&bytes),
    })
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
        let custom = rows(if state["skillPresets"].is_null() {
            &empty
        } else {
            &state["skillPresets"]
        })?;
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
        let row = self
            .overlay(state)?
            .into_iter()
            .find(|row| row["id"] == id)
            .ok_or_else(|| {
                let mut error =
                    Error::new("Skill unavailable. Read Skills again before editing it.");
                error.status = 404;
                error
            })?;
        let custom = state["skillPresets"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["id"] == id));
        Ok((row, custom))
    }
}

#[cfg(test)]
mod tests;
