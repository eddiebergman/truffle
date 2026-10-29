//! Agent-reported resource metadata. Reporting never provisions, polls or stops cloud resources.
use super::{arr, checked, json, repository, sid, text, Error, Repository, Result, Value};
fn short(v: &Value, key: &str, max: usize) -> String {
    text(v, key).chars().take(max).collect()
}
/// Public reporting owner: replacement is always checked against a captured context revision.
pub fn apply(repo: &Repository, id: &str, report: &Value) -> Result<Value> {
    if report
        .get("replace")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(Error::new("replace must be true or false."));
    }
    if serde_json::to_vec(report)?.len() > 64 * 1024 {
        return Err(Error::new("Resource report exceeds 64 KiB."));
    }
    if report["replace"] == true {
        super::session_context::replace(
            repo,
            id,
            &json!({"revision":report["revision"],"resources":report["resources"]}),
        )
    } else {
        repo.transaction(|s| save(s, id, report))
    }
}
#[derive(Clone, Copy)]
enum SaveMode {
    Merge,
    Replace,
}
/// Transaction-local additive reporting. A destructive flag cannot bypass apply's CAS.
pub fn save(s: &mut Value, id: &str, report: &Value) -> Result<Value> {
    if report["replace"] == true {
        return Err(Error::new(
            "Resource replacement requires a captured context revision through the report owner.",
        ));
    }
    save_mode(s, id, report, SaveMode::Merge)
}
/// Called only by the context owner after validating its revision in the same transaction.
pub(super) fn replace_entries(s: &mut Value, id: &str, entries: &Value) -> Result<Value> {
    save_mode(s, id, &json!({"resources":entries}), SaveMode::Replace)
}
fn save_mode(s: &mut Value, id: &str, report: &Value, mode: SaveMode) -> Result<Value> {
    sid(id)?;
    if serde_json::to_vec(report)?.len() > 64 * 1024 {
        return Err(Error::new("Resource report exceeds 64 KiB."));
    }
    let limit = match mode {
        SaveMode::Merge => 64,
        SaveMode::Replace => 128,
    };
    let entries = report["resources"]
        .as_array()
        .filter(|v| v.len() <= limit)
        .ok_or_else(|| {
            Error::new(format!(
                "Report up to {limit} resources as a resources array."
            ))
        })?;
    let index = arr(&s["sessions"])
        .iter()
        .position(|r| r["id"] == id)
        .ok_or_else(|| Error::new("Session unavailable."))?;
    let mut resources = match mode {
        SaveMode::Replace => vec![],
        SaveMode::Merge => arr(&s["sessions"][index]["resources"]),
    };
    for entry in entries {
        if !entry.is_object() {
            return Err(Error::new("Each resource must be an object."));
        }
        let external_id = if text(entry, "id").is_empty() {
            checked(entry, "identifier", 500, true)?
        } else {
            checked(entry, "id", 500, true)?
        };
        let provider = checked(entry, "provider", 80, true)?;
        let kind = checked(entry, "kind", 80, true)?;
        let identity =
            serde_json::to_vec(&json!([provider, external_id, short(entry, "scope", 500)]))?;
        let key = context_kit::sha256(&identity);
        let mut row = entry.clone(); // Unknown metadata is retained, not forced into provider-specific columns.
        row["id"] = json!(external_id);
        row["provider"] = json!(provider);
        row["kind"] = json!(kind);
        row["resourceKey"] = json!(key);
        row["name"] = json!(if text(entry, "name").is_empty() {
            external_id.clone()
        } else {
            short(entry, "name", 500)
        });
        row["state"] = json!(if text(entry, "state").is_empty() {
            "unknown".into()
        } else {
            short(entry, "state", 1000)
        });
        // Observation metadata belongs to this reported state, not a prior state update.
        row["verification"] = if entry["verification"].is_null() {
            json!(if entry["state_source"] == "checked-now" {
                "checked"
            } else {
                "reported"
            })
        } else {
            entry["verification"].clone()
        };
        for key in ["observedAt", "checked_at", "state_source"] {
            row[key] = entry[key].clone();
        }
        row["purpose"] = json!(short(entry, "purpose", 2000));
        // Complete replacement restores captured metadata, including its report
        // time. A new additive report always receives the host's current time.
        let captured_time = match mode {
            SaveMode::Replace => entry["reportedAt"].as_str().filter(|value| {
                value.len() <= 100 && chrono::DateTime::parse_from_rfc3339(value).is_ok()
            }),
            SaveMode::Merge => None,
        };
        row["reportedAt"] = json!(captured_time.map_or_else(repository::iso_time, str::to_owned));
        if let Some(previous) = resources.iter().position(|r| r["resourceKey"] == key) {
            // Reporting a state update preserves an earlier purpose or locator omitted by the agent.
            let mut merged = resources[previous].clone();
            for (k, v) in row.as_object().unwrap() {
                if k == "name" && text(entry, "name").is_empty() {
                    continue;
                }
                if !["purpose", "name"].contains(&k.as_str())
                    || !v.as_str().unwrap_or("").is_empty()
                {
                    merged[k] = v.clone();
                }
            }
            resources[previous] = merged;
        } else {
            resources.push(row);
        }
    }
    if resources.len() > 128 {
        return Err(Error::new(
            "This conversation already tracks 128 resources.",
        ));
    }
    s["sessions"][index]["resources"] = json!(resources);
    Ok(s["sessions"][index].clone())
}
/// Seed reports collected from existing agents without editing a live service's state file.
pub fn import_pending(repo: &Repository) {
    let dir = repo.data.parent().unwrap().join("resource-reports-pending");
    let files = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|s| s == "json"))
        .take(64);
    for entry in files {
        if !entry.file_type().is_ok_and(|t| t.is_file())
            || entry.path().extension().is_none_or(|s| s != "json")
        {
            continue;
        }
        let result = (|| -> Result<()> {
            let bytes = context_kit::read_bounded(&entry.path(), 96 * 1024).map_err(Error::new)?;
            let input: Value = serde_json::from_slice(&bytes)?;
            apply(repo, text(&input, "recordId"), &input["report"])?;
            std::fs::rename(entry.path(), entry.path().with_extension("imported"))?;
            Ok(())
        })();
        if let Err(error) = result {
            let _=repo.log(&json!({"source":"resources","operation":"import","outcome":"failure","message":error.message,"time":repository::iso_time()}));
        }
    }
}
#[cfg(test)]
mod tests;
