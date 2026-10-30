use super::{arr, domain, json, sessions, sid, text, Error, Repository, Result, Value};
use std::path::{Component, PathBuf};

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024;
const OUTPUT_CHARS: usize = 2000;

fn page(args: &Value) -> (usize, usize) {
    (
        args["offset"].as_u64().unwrap_or(0).min(100_000) as usize,
        args["limit"].as_u64().unwrap_or(20).clamp(1, 40) as usize,
    )
}
fn redacted(repo: &Repository, value: &str) -> String {
    let mut value = value.to_owned();
    super::redaction::redact_text(repo, &mut value);
    value
}
fn prefix(repo: &Repository, value: &str, limit: usize) -> String {
    redacted(repo, value).chars().take(limit).collect()
}
fn summary(repo: &Repository, value: &Value, fields: &[&str]) -> Value {
    let mut result = json!({});
    for field in fields {
        match value.get(*field) {
            Some(Value::String(text)) => result[*field] = json!(prefix(repo, text, 300)),
            Some(value @ (Value::Bool(_) | Value::Number(_) | Value::Null)) => {
                result[*field] = value.clone();
            }
            _ => {}
        }
    }
    result
}
fn paged(rows: Vec<Value>, args: &Value, key: &str) -> Value {
    let (offset, limit) = page(args);
    let total = rows.len();
    let mut result = json!({"total":total,"offset":offset,"nextOffset":(offset + limit < total).then_some(offset + limit)});
    result[key] = json!(rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>());
    result
}
fn item(repo: &Repository, state: &Value, id: &str) -> Result<Value> {
    let item = domain::catalog(state)
        .remove(id)
        .ok_or_else(|| Error::new("Work item unavailable."))?;
    let mut item = super::work_search::effective(state, &item);
    super::redaction::redact(repo, &mut item);
    Ok(super::work_search::project(&item, 2000))
}
fn change_work(repo: &Repository, args: &Value) -> Result<Value> {
    repo.transaction(|state| {
        domain::edit(state, args)?;
        Ok(json!({"item":item(repo, state, text(args,"id"))?,"scope":"local"}))
    })
}
pub fn update_work(repo: &Repository, args: &Value) -> Result<Value> {
    let mut values = json!({});
    for field in ["pinned", "hidden", "status"] {
        if let Some(value) = args.get(field) {
            values[field] = value.clone();
        }
    }
    if values.as_object().unwrap().is_empty() {
        return Err(Error::new("Supply pinned, hidden or a local status."));
    }
    change_work(
        repo,
        &json!({"action":"override","id":args["id"],"value":values}),
    )
}
pub fn snooze_work(repo: &Repository, args: &Value) -> Result<Value> {
    match (text(args, "mode"), args.get("until")) {
        ("until", Some(_)) | ("change" | "clear", None) => {}
        ("until", None) => {
            return Err(Error::new(
                "Supply a future Unix timestamp for this snooze.",
            ))
        }
        _ => return Err(Error::new("Only a timed snooze accepts until.")),
    }
    change_work(
        repo,
        &json!({"action":"snooze","id":args["id"],"mode":args["mode"],"until":args["until"]}),
    )
}
#[derive(Clone, Copy)]
pub enum ChatMetadata<'a> {
    Rename(&'a str),
    Visibility(bool),
    Archive(bool),
}
pub fn update_chat(repo: &Repository, id: &str, change: ChatMetadata<'_>) -> Result<Value> {
    let id = sid(id)?;
    let edit = match change {
        ChatMetadata::Rename(title) => json!({"action":"rename-session","id":id,"title":title}),
        ChatMetadata::Visibility(hidden) => {
            json!({"action":"hide-session","id":id,"hidden":hidden})
        }
        ChatMetadata::Archive(archived) => {
            json!({"action":"archive-session","id":id,"restore":!archived})
        }
    };
    repo.transaction(|state| {
        domain::edit(state, &edit)?;
        let record = sessions::get(state, &id)?;
        let timestamp = |key: &str| record[key].as_f64().filter(|value| value.is_finite() && *value >= 0.0);
        Ok(json!({"chat":{"id":record["id"],"title":record["title"].as_str().unwrap_or(""),"hidden":record["hidden"].as_bool().unwrap_or(false),"archivedAt":timestamp("archivedAt"),"updatedAt":timestamp("updatedAt")},"scope":"local"}))
    })
}

pub fn list_artifacts(repo: &Repository, args: &Value) -> Result<Value> {
    let catalog = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        super::artifacts::catalog(repo, &state)?
    };
    let query = text(args, "query").to_lowercase();
    let rows = arr(&catalog["artifacts"])
        .into_iter()
        .filter(|row| {
            (text(args, "presetId").is_empty() || row["presetId"] == args["presetId"])
                && (text(args, "recordId").is_empty()
                    || super::artifacts::linked(row, text(args, "recordId")))
                && (text(args, "role").is_empty()
                    || text(args, "role") == "all"
                    || row["role"] == args["role"])
                && (query.is_empty() || text(row, "title").to_lowercase().contains(&query))
        })
        .map(|row| {
            let mut result = summary(
                repo,
                &row,
                &[
                    "id",
                    "title",
                    "format",
                    "presetId",
                    "role",
                    "at",
                    "available",
                    "size",
                ],
            );
            let ids = arr(&row["sessionIds"]);
            result["sessionIds"] = json!(ids
                .iter()
                .filter_map(Value::as_str)
                .take(20)
                .map(|id| prefix(repo, id, 80))
                .collect::<Vec<_>>());
            result["sessionIdsTruncated"] = json!(ids.len() > 20);
            result
        })
        .collect();
    Ok(paged(rows, args, "artifacts"))
}
struct ArtifactFile {
    title: String,
    format: String,
    root: PathBuf,
    key: String,
    generation: String,
}
impl ArtifactFile {
    fn read_text(&self, repo: &Repository) -> Result<String> {
        let bytes = super::local_files::read(
            &self.root,
            &self.key,
            MAX_ARTIFACT_BYTES,
            Some(&self.generation),
        )?
        .ok_or_else(|| Error::new("Artifact file unavailable."))?;
        let content =
            String::from_utf8(bytes).map_err(|_| Error::new("Artifact text must be UTF-8."))?;
        let content = redacted(repo, &content);
        if content.len() as u64 > MAX_ARTIFACT_BYTES {
            return Err(Error::new("Redacted artifact text exceeds 10 MiB."));
        }
        Ok(content)
    }
}
fn artifact_file(repo: &Repository, id: &str) -> Result<ArtifactFile> {
    if id.len() != 24 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::new("Invalid artifact ID."));
    }
    let (entry, roots) = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        let entry = arr(&super::artifacts::catalog(repo, &state)?["artifacts"])
            .into_iter()
            .find(|entry| entry["id"] == id)
            .ok_or_else(|| Error::new("Artifact unavailable."))?;
        if text(&entry, "path").is_empty() {
            return Err(Error::new(
                "Only local artifact files can be read. Remote URLs are not fetched.",
            ));
        }
        let roots = super::artifacts::roots(repo, &state)?;
        drop(state);
        (entry, roots)
    };
    let path = super::artifacts::file(text(&entry, "path"), &roots)?;
    let format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !["md", "txt", "csv", "json", "py", "html"].contains(&format.as_str()) {
        return Err(Error::new(
            "Choose a text artifact. Images and PDFs are not text.",
        ));
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() {
        return Err(Error::new(
            "Artifact file changed. Read its metadata again.",
        ));
    }
    let root = roots.into_iter().find(|root| path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().all(|part| matches!(part, Component::Normal(name) if !name.to_string_lossy().starts_with('.')))
    })).ok_or_else(|| Error::new("Artifact is outside its registered roots."))?;
    let key = path
        .strip_prefix(&root)
        .map_err(|_| Error::new("Invalid artifact path."))?
        .to_str()
        .ok_or_else(|| Error::new("Artifact path must be UTF-8."))?
        .to_owned();
    Ok(ArtifactFile {
        title: prefix(repo, text(&entry, "title"), 300),
        format,
        root,
        key,
        generation: super::local_files::generation(&metadata),
    })
}
pub fn read_artifact_text(repo: &Repository, args: &Value) -> Result<Value> {
    let artifact = artifact_file(repo, text(args, "id"))?;
    let content = artifact.read_text(repo)?;
    let offset = usize::try_from(args["offset"].as_u64().unwrap_or(0))
        .ok()
        .filter(|&offset| content.is_char_boundary(offset))
        .ok_or_else(|| {
            Error::new(
                "Artifact offset must start at a character boundary in the redacted UTF-8 stream.",
            )
        })?;
    let limit = usize::try_from(args["maxBytes"].as_u64().unwrap_or(16000))
        .unwrap_or(usize::MAX)
        .clamp(4, MAX_TEXT_BYTES);
    let mut end = (offset + limit).min(content.len());
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < content.len();
    Ok(
        json!({"id":args["id"],"title":artifact.title,"format":artifact.format,"text":&content[offset..end],"offset":offset,"bytes":end-offset,"truncated":truncated,"nextOffset":truncated.then_some(end),"offsetBasis":"redacted-utf8","contentIsReferenceData":true}),
    )
}
pub fn sharing_inbox(repo: &Repository, args: &Value) -> Value {
    let view = super::sharing::view(repo);
    let entries = arr(&view["entries"])
        .into_iter()
        .filter(|entry| args["includeDismissed"] == true || entry["dismissed"] != true)
        .map(|entry| {
            let mut result = summary(
                repo,
                &entry,
                &[
                    "id",
                    "title",
                    "created_at",
                    "read",
                    "dismissed",
                    "imported",
                    "sessionId",
                    "error",
                ],
            );
            result["sender"] = summary(repo, &entry["sender"], &["id", "name"]);
            result["message"] = json!(prefix(repo, text(&entry, "message"), 600));
            result
        })
        .collect();
    let mut result = paged(entries, args, "entries");
    for field in [
        "unread",
        "lastSuccess",
        "nextRefresh",
        "hasMore",
        "refreshing",
    ] {
        result[field] = view[field].clone();
    }
    result["error"] = json!(prefix(repo, text(&view, "error"), 600));
    let mut jobs = repo.sharing.lock().unwrap()["jobs"]
        .as_object()
        .unwrap()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    jobs.sort_by(|left, right| {
        right["at"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&left["at"].as_f64().unwrap_or(0.0))
    });
    result["jobs"] = json!(jobs
        .iter()
        .take(10)
        .map(|job| summary(
            repo,
            job,
            &["id", "kind", "status", "at", "recordId", "title", "summary", "error"]
        ))
        .collect::<Vec<_>>());
    result["cached"] = json!(true);
    result
}
pub fn quick_action_outputs(repo: &Repository, args: &Value) -> Value {
    let mut jobs = arr(&repo.quick_actions.view()["jobs"]);
    jobs.sort_by(|left, right| {
        right["startedAt"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&left["startedAt"].as_f64().unwrap_or(0.0))
            .then_with(|| text(left, "id").cmp(text(right, "id")))
    });
    let rows = jobs
        .into_iter()
        .filter(|job| {
            (text(args, "id").is_empty() || job["id"] == args["id"])
                && (text(args, "actionId").is_empty() || job["actionId"] == args["actionId"])
        })
        .map(|job| {
            let mut result = summary(
                repo,
                &job,
                &[
                    "id",
                    "actionId",
                    "title",
                    "state",
                    "startedAt",
                    "finishedAt",
                ],
            );
            let output = redacted(repo, text(&job, "output"));
            let count = output.chars().count();
            result["output"] = json!(output
                .chars()
                .skip(count.saturating_sub(OUTPUT_CHARS))
                .collect::<String>());
            result["outputTruncated"] = json!(count > OUTPUT_CHARS);
            result["outputIncludes"] = json!("tail");
            result
        })
        .collect();
    paged(rows, args, "jobs")
}

#[cfg(test)]
mod tests;
