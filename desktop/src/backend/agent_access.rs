use super::*;
use std::path::{Component, PathBuf};

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024;
const OUTPUT_CHARS: usize = 2000;

fn page(args: &Value) -> (usize, usize) {
    (
        args["offset"].as_u64().unwrap_or(0).min(100000) as usize,
        args["limit"].as_u64().unwrap_or(20).clamp(1, 40) as usize,
    )
}
fn redacted(repo: &Repository, value: &str) -> String {
    let mut value = value.to_owned();
    super::commands::redact_text(repo, &mut value);
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
                result[*field] = value.clone()
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
        .ok_or(Error::new("Work item unavailable."))?;
    let mut item = super::work_search::effective(state, &item);
    super::commands::redact(repo, &mut item);
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
pub enum ChatMetadata<'a> { Rename(&'a str), Visibility(bool), Archive(bool) }
pub fn update_chat(repo: &Repository, id: &str, change: ChatMetadata<'_>) -> Result<Value> {
    let id = sid(id)?;
    let edit = match change {
        ChatMetadata::Rename(title) => json!({"action":"rename-session","id":id,"title":title}),
        ChatMetadata::Visibility(hidden) => json!({"action":"hide-session","id":id,"hidden":hidden}),
        ChatMetadata::Archive(archived) => json!({"action":"archive-session","id":id,"restore":!archived}),
    };
    repo.transaction(|state| {
        domain::edit(state, &edit)?;
        let record = arr(&state["sessions"]).into_iter().find(|record| record["id"] == id)
            .ok_or(Error::new("Chat unavailable."))?;
        let timestamp = |key: &str| record[key].as_f64().filter(|value| value.is_finite() && *value >= 0.0);
        Ok(json!({"chat":{"id":record["id"],"title":record["title"].as_str().unwrap_or(""),"hidden":record["hidden"].as_bool().unwrap_or(false),"archivedAt":timestamp("archivedAt"),"updatedAt":timestamp("updatedAt")},"scope":"local"}))
    })
}

pub fn list_artifacts(repo: &Repository, args: &Value) -> Result<Value> {
    let state = repo.state.lock().map_err(|_| Error::io())?;
    let catalog = super::artifacts::catalog(repo, &state)?;
    let query = text(args, "query").to_lowercase();
    let rows = arr(&catalog["artifacts"])
        .into_iter()
        .filter(|row| {
            (text(args, "presetId").is_empty() || row["presetId"] == args["presetId"])
                && (text(args, "recordId").is_empty()
                    || arr(&row["sessionIds"]).contains(&args["recordId"]))
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
        .ok_or(Error::new("Artifact file unavailable."))?;
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
    let state = repo.state.lock().map_err(|_| Error::io())?;
    let entry = arr(&super::artifacts::catalog(repo, &state)?["artifacts"])
        .into_iter()
        .find(|entry| entry["id"] == id)
        .ok_or(Error::new("Artifact unavailable."))?;
    if text(&entry, "path").is_empty() {
        return Err(Error::new(
            "Only local artifact files can be read. Remote URLs are not fetched.",
        ));
    }
    let roots = super::artifacts::roots(repo, &state)?;
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
    let root = roots.into_iter().find(|root| path.strip_prefix(root).ok().is_some_and(|relative| {
        relative.components().all(|part| matches!(part, Component::Normal(name) if !name.to_string_lossy().starts_with('.')))
    })).ok_or(Error::new("Artifact is outside its registered roots."))?;
    let key = path
        .strip_prefix(&root)
        .map_err(|_| Error::new("Invalid artifact path."))?
        .to_str()
        .ok_or(Error::new("Artifact path must be UTF-8."))?
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
    let offset = args["offset"].as_u64().unwrap_or(0);
    if offset > content.len() as u64 || !content.is_char_boundary(offset as usize) {
        return Err(Error::new(
            "Artifact offset must start at a character boundary in the redacted UTF-8 stream.",
        ));
    }
    let offset = offset as usize;
    let limit = args["maxBytes"]
        .as_u64()
        .unwrap_or(16000)
        .clamp(4, MAX_TEXT_BYTES as u64) as usize;
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
mod tests {
    use super::*;
    fn invoke(repo: &std::sync::Arc<Repository>, name: &str, args: Value) -> Result<Value> {
        super::super::commands::invoke(repo, &json!({"name":name,"args":args}), true)
    }
    #[test]
    fn artifact_pages_use_the_redacted_stream_at_every_cursor() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = "benign-split-secret-0123456789";
        std::fs::write(
            temp.path().join(".env"),
            format!("ACCESS_TEST_SECRET={sentinel}"),
        )
        .unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        let record = uuid();
        repo.transaction(|state| {
            state["sessions"] = json!([{"id":record,"cwd":project,"items":[]}]);
            Ok(())
        })
        .unwrap();
        let path = project.join("secret.md");
        let raw = format!("abc{sentinel}é end");
        std::fs::write(&path, &raw).unwrap();
        repo.edit(&json!({"action":"save-artifact","path":path,"sessionIds":[record]}))
            .unwrap();
        let id = repo.state.lock().unwrap()["artifacts"][0]["id"].clone();
        let expected = "abc[redacted]é end";
        for offset in 0..=raw.len() {
            let page = invoke(
                &repo,
                "artifacts.read-text",
                json!({"id":id,"offset":offset,"maxBytes":4}),
            );
            if offset > expected.len() || !expected.is_char_boundary(offset) {
                assert!(page.is_err());
                continue;
            }
            let page = page.unwrap();
            assert_eq!(page["offsetBasis"], "redacted-utf8");
            let chunk = page["text"].as_str().unwrap();
            assert!(expected[offset..].starts_with(chunk));
            assert!(chunk.len() <= 4);
        }
        let mut combined = String::new();
        let mut offset = 0;
        loop {
            let page = invoke(
                &repo,
                "artifacts.read-text",
                json!({"id":id,"offset":offset,"maxBytes":4}),
            )
            .unwrap();
            combined.push_str(page["text"].as_str().unwrap());
            match page["nextOffset"].as_u64() {
                Some(next) => {
                    assert!(next > offset);
                    offset = next;
                }
                None => break,
            }
        }
        assert_eq!(combined, expected);
    }

    #[test]
    fn summaries_and_output_tails_redact_before_their_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = "benign-boundary-secret-0123456789";
        std::fs::write(
            temp.path().join(".env"),
            format!("ACCESS_TEST_SECRET={sentinel}"),
        )
        .unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let mut jobs = json!({});
        for suffix in [1980, 1990, 1999, 2000, 2001] {
            jobs[suffix.to_string()] = json!({"id":suffix.to_string(),"actionId":"fixture","title":"Boundary","state":"succeeded","startedAt":suffix,"output":format!("{}{sentinel}{}","p".repeat(3000),"z".repeat(suffix))});
        }
        std::fs::write(
            data.join("quick-actions.json"),
            serde_json::to_vec(&jobs).unwrap(),
        )
        .unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        for suffix in [1980, 1990, 1999, 2000, 2001] {
            let result = invoke(
                &repo,
                "quick-actions.outputs",
                json!({"id":suffix.to_string()}),
            )
            .unwrap();
            let expected = format!("{}[redacted]{}", "p".repeat(3000), "z".repeat(suffix));
            assert_eq!(
                result["jobs"][0]["output"],
                expected
                    .chars()
                    .skip(expected.chars().count() - OUTPUT_CHARS)
                    .collect::<String>()
            );
        }
        let title = format!("{}{sentinel}", "t".repeat(295));
        let message = format!("{}{sentinel}", "m".repeat(595));
        {
            let mut sharing = repo.sharing.lock().unwrap();
            sharing["entries"] = json!({"entry":{"id":"entry","title":title,"message":message,"sender":{"name":title}}});
            sharing["jobs"] = json!({"job":{"id":"job","at":1,"summary":title}});
        }
        let inbox = invoke(&repo, "sharing.inbox", json!({})).unwrap();
        let expected_title = format!("{}[reda", "t".repeat(295));
        assert_eq!(inbox["entries"][0]["title"], expected_title);
        assert_eq!(inbox["entries"][0]["sender"]["name"], expected_title);
        assert_eq!(inbox["jobs"][0]["summary"], expected_title);
        assert_eq!(
            inbox["entries"][0]["message"],
            format!("{}[reda", "m".repeat(595))
        );
        repo.transaction(|state| {
            state["items"] = json!([{"id":"linear:secret","source":"linear","title":"Fixture","status":"todo","detail":format!("{}{sentinel}","d".repeat(1995))}]);
            Ok(())
        }).unwrap();
        let result = invoke(
            &repo,
            "work.update",
            json!({"id":"linear:secret","pinned":true}),
        )
        .unwrap();
        assert_eq!(
            result["item"]["detail"],
            format!("{}[reda", "d".repeat(1995))
        );
    }

    #[test]
    fn work_projection_failure_rolls_back_the_same_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|state| {
            state["notes"] = json!([{"id":"note","source":"notes","title":"Note","status":"todo"}]);
            Ok(())
        })
        .unwrap();
        let before = repo.state.lock().unwrap().clone();
        let saved = std::fs::read(&repo.data).unwrap();
        assert!(change_work(&repo, &json!({"action":"delete-note","id":"note"})).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), saved);
    }

    #[test]
    fn validated_artifact_rejects_replacement_symlinks_and_inodes() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        let record = uuid();
        repo.transaction(|state| {
            state["sessions"] = json!([{"id":record,"cwd":project,"items":[]}]);
            Ok(())
        })
        .unwrap();
        let path = project.join("report.md");
        std::fs::write(&path, "Registered fixture").unwrap();
        repo.edit(&json!({"action":"save-artifact","path":path,"sessionIds":[record]}))
            .unwrap();
        let id = repo.state.lock().unwrap()["artifacts"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let validated = artifact_file(&repo, &id).unwrap();
        let stored = validated.root.join(&validated.key);
        let outside = temp.path().join("outside.md");
        std::fs::write(&outside, "must-not-be-read").unwrap();
        std::fs::remove_file(&stored).unwrap();
        std::os::unix::fs::symlink(&outside, &stored).unwrap();
        assert!(validated.read_text(&repo).is_err());
        std::fs::remove_file(&stored).unwrap();
        std::fs::write(&stored, "Different inode").unwrap();
        assert!(validated.read_text(&repo).is_err());
    }

    #[test]
    fn assistant_changes_only_local_work_state_and_can_restore_it() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|state| {
            state["items"] = json!([{"id":"linear:fixture","source":"linear","title":"Fixture","status":"todo"}]);
            Ok(())
        }).unwrap();
        let sources = repo.state.lock().unwrap()["sources"].clone();
        let updated = invoke(
            &repo,
            "work.update",
            json!({"id":"linear:fixture","pinned":true,"hidden":true,"status":"done"}),
        )
        .unwrap();
        assert_eq!(updated["scope"], "local");
        assert_eq!(updated["item"]["status"], "done");
        assert_eq!(updated["item"]["sourceStatusCode"], "todo");
        assert_eq!(updated["item"]["pinned"], true);
        assert_eq!(updated["item"]["hidden"], true);
        assert_eq!(repo.state.lock().unwrap()["items"][0]["status"], "todo");
        assert_eq!(invoke(&repo, "work.query", json!({})).unwrap()["total"], 0);
        let restored = invoke(
            &repo,
            "work.update",
            json!({"id":"linear:fixture","status":null,"pinned":false,"hidden":false}),
        )
        .unwrap();
        assert_eq!(restored["item"]["status"], "todo");
        let until = (now() + 3600.0) as u64;
        assert_eq!(
            invoke(
                &repo,
                "work.snooze",
                json!({"id":"linear:fixture","mode":"until","until":until})
            )
            .unwrap()["item"]["snoozed"],
            true
        );
        assert_eq!(
            invoke(
                &repo,
                "work.snooze",
                json!({"id":"linear:fixture","mode":"clear"})
            )
            .unwrap()["item"]["snoozed"],
            false
        );
        assert_eq!(
            invoke(
                &repo,
                "work.snooze",
                json!({"id":"linear:fixture","mode":"change"})
            )
            .unwrap()["item"]["snoozed"],
            true
        );
        let before = repo.state.lock().unwrap().clone();
        for (name, args) in [
            ("work.update", json!({"id":"missing","pinned":true})),
            ("work.update", json!({"id":"linear:fixture"})),
            ("work.snooze", json!({"id":"linear:fixture","mode":"until"})),
            (
                "work.snooze",
                json!({"id":"linear:fixture","mode":"clear","until":until}),
            ),
            (
                "work.snooze",
                json!({"id":"linear:fixture","mode":"until","until":1}),
            ),
        ] {
            assert!(invoke(&repo, name, args).is_err());
        }
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(repo.state.lock().unwrap()["sources"], sources);
    }

    #[test]
    fn artifact_reads_use_registered_snapshots_bounds_and_utf8_cursors() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        let record = uuid();
        repo.transaction(|state| {
            state["sessions"] = json!([{"id":record,"cwd":project,"items":[]}]);
            Ok(())
        })
        .unwrap();
        let path = project.join("report.md");
        std::fs::write(&path, format!("abcé\n{}", "x".repeat(70000))).unwrap();
        repo.edit(&json!({"action":"save-artifact","title":"Fixture report","path":path,"sessionIds":[record]})).unwrap();
        let id = repo.state.lock().unwrap()["artifacts"][0]["id"].clone();
        std::fs::write(&path, "Changed source").unwrap();
        let before = repo.state.lock().unwrap().clone();
        let listed = invoke(
            &repo,
            "artifacts.list",
            json!({"recordId":record,"limit":1}),
        )
        .unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["artifacts"][0]["id"], id);
        assert!(listed["artifacts"][0].get("path").is_none());
        assert!(listed["artifacts"][0].get("sourcePath").is_none());
        let first = invoke(&repo, "artifacts.read-text", json!({"id":id,"maxBytes":4})).unwrap();
        assert_eq!(first["text"], "abc");
        assert_eq!(first["bytes"], 3);
        assert_eq!(first["truncated"], true);
        assert_eq!(first["nextOffset"], 3);
        let next = invoke(
            &repo,
            "artifacts.read-text",
            json!({"id":id,"offset":first["nextOffset"],"maxBytes":4}),
        )
        .unwrap();
        assert_eq!(next["text"], "é\nx");
        assert_eq!(next["nextOffset"], 7);
        let bounded = invoke(
            &repo,
            "artifacts.read-text",
            json!({"id":id,"maxBytes":65536}),
        )
        .unwrap();
        assert!(bounded["bytes"].as_u64().unwrap() <= 65536);
        assert_eq!(bounded["truncated"], true);
        assert!(invoke(&repo, "artifacts.read-text", json!({"id":id,"offset":4})).is_err());
        assert!(invoke(
            &repo,
            "artifacts.read-text",
            json!({"id":"000000000000000000000000"})
        )
        .is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        let outside = temp.path().join("outside.md");
        std::fs::write(&outside, "Outside registered roots").unwrap();
        let outside_id = "111111111111111111111111";
        repo.transaction(|state| {
            state["artifacts"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":outside_id,"path":outside,"title":"Stale path","at":1}));
            Ok(())
        })
        .unwrap();
        assert!(invoke(&repo, "artifacts.read-text", json!({"id":outside_id})).is_err());
        repo.edit(&json!({"action":"save-artifact","title":"Remote link","url":"https://example.com/report"})).unwrap();
        let remote_id = repo.state.lock().unwrap()["artifacts"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["id"]
            .clone();
        assert!(invoke(&repo, "artifacts.read-text", json!({"id":remote_id})).is_err());
    }

    #[test]
    fn cached_inbox_reads_are_bounded_and_exclude_internal_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        {
            let mut sharing = repo.sharing.lock().unwrap();
            sharing["entries"] = json!({
                "one":{"id":"one","title":"Shared fixture","created_at":"2026-01-02","sender":{"id":"sender","name":"Sender","private":"not-shared"},"message":"m".repeat(1000),"bundle":{"uri":"private-locator"}},
                "two":{"id":"two","title":"Dismissed","created_at":"2026-01-01","dismissed":true}
            });
            sharing["jobs"] = json!({"job":{"id":"job","kind":"prepare","status":"ready","at":1,"payload":{"recordId":"record","private":"not-shared"},"bundlePath":"private-path","preview":{"body":"not-shared"},"summary":"Cached job"}});
        }
        let before = repo.sharing.lock().unwrap().clone();
        let result = invoke(&repo, "sharing.inbox", json!({"limit":1})).unwrap();
        assert_eq!(result["cached"], true);
        assert_eq!(result["total"], 1);
        assert_eq!(result["entries"][0]["sender"]["name"], "Sender");
        assert_eq!(result["entries"][0]["message"].as_str().unwrap().len(), 600);
        assert!(result["entries"][0].get("bundle").is_none());
        assert!(!result.to_string().contains("not-shared"));
        assert!(!result.to_string().contains("private-path"));
        assert_eq!(
            invoke(&repo, "sharing.inbox", json!({"includeDismissed":true})).unwrap()["total"],
            2
        );
        assert_eq!(*repo.sharing.lock().unwrap(), before);
        assert!(repo.sharing_busy.lock().unwrap().is_empty());
    }

    #[test]
    fn existing_action_outputs_are_redacted_without_exposing_or_running_commands() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(
            temp.path().join(".env"),
            "ACCESS_TEST_SECRET=benign-private-output-sentinel",
        )
        .unwrap();
        let forbidden = temp.path().join("must-not-exist");
        let jobs = json!({"fixture":{"id":"fixture","actionId":"action","title":"Fixture action","state":"succeeded","startedAt":1,"command":format!("touch {}",forbidden.display()),"cwd":"not-projected","output":format!("{} benign-private-output-sentinel","o".repeat(3000))}});
        std::fs::write(
            data.join("quick-actions.json"),
            serde_json::to_vec(&jobs).unwrap(),
        )
        .unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        let before = std::fs::read(data.join("quick-actions.json")).unwrap();
        let result = invoke(&repo, "quick-actions.outputs", json!({"id":"fixture"})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["jobs"][0]["state"], "succeeded");
        assert_eq!(result["jobs"][0]["outputTruncated"], true);
        assert!(result["jobs"][0]["output"]
            .as_str()
            .unwrap()
            .contains("[redacted]"));
        assert!(!result
            .to_string()
            .contains("benign-private-output-sentinel"));
        assert!(result["jobs"][0].get("command").is_none());
        assert!(result["jobs"][0].get("cwd").is_none());
        assert!(!forbidden.exists());
        assert_eq!(
            std::fs::read(data.join("quick-actions.json")).unwrap(),
            before
        );
        assert!(invoke(
            &repo,
            "quick-actions.run",
            json!({"actionId":"action","revision":0})
        )
        .is_err());
    }
}
