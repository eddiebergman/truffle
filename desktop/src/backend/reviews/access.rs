use super::super::redaction;
use super::{
    arr, check, draft, edit_note, json, note_token, sessions, sid, text, Error, Git, Repository,
    Result, Value,
};

fn summary_identity(repo: &Repository, value: &Value, max: usize) -> Result<Value> {
    let text = value
        .as_str()
        .filter(|value| {
            !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| Error::new("Review identity is unavailable."))?;
    let mut safe = text.to_owned();
    redaction::redact_text(repo, &mut safe);
    if safe != text {
        return Err(Error::new("Review identity cannot be safely projected."));
    }
    Ok(json!(text))
}
fn summary_time(value: &Value) -> Value {
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map_or(Value::Null, |value| json!(value))
}
fn summary_freshness(value: &Value) -> Result<Value> {
    match value.as_str() {
        Some("current" | "outdated" | "unverified") => Ok(value.clone()),
        _ => Err(Error::new("Review freshness is unavailable.")),
    }
}
/// Freshness is a local observation, not completion or delivery acknowledgement.
pub fn check_summary(repo: &Repository, args: &Value) -> Result<Value> {
    let object = args
        .as_object()
        .ok_or_else(|| Error::new("Review check arguments must be an object."))?;
    if object.len() != 1 || !object.contains_key("id") {
        return Err(Error::new("Review check accepts only a saved chat ID."));
    }
    let id = sid(args["id"]
        .as_str()
        .ok_or_else(|| Error::new("Supply the saved chat ID."))?)?;
    summary_identity(repo, &json!(id), 80)?;
    let review = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        draft(sessions::get(&state, &id)?)
    };
    if arr(&review["comments"]).len() > 100 || arr(&review["assessments"]).len() > 100 {
        return Err(Error::new(
            "Review freshness summaries support up to 100 comments and 100 assessments.",
        ));
    }
    let review = &check(repo, &Git, &id)?;
    let revision = review["revision"]
        .as_u64()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or_else(|| Error::new("Review revision is unavailable."))?;
    let mut comments = vec![];
    let mut assessments = vec![];
    for (field, result) in [
        ("comments", &mut comments),
        ("assessments", &mut assessments),
    ] {
        let rows = arr(&review[field]);
        if rows.len() > 100 {
            return Err(Error::new(
                "Review freshness summary exceeds its row limit.",
            ));
        }
        for row in rows {
            let mut entry = json!({"id":summary_identity(repo,&row["id"],120)?,"freshness":summary_freshness(&row["freshness"])?,"reviewedAt":summary_time(&row["reviewedAt"])});
            if field == "assessments" {
                entry["sentAt"] = summary_time(&row["sentAt"]);
            }
            result.push(entry);
        }
    }
    let run = if review["run"].is_object() {
        let raw = &review["run"];
        let state = raw["state"]
            .as_str()
            .filter(|value| {
                ["requested", "reviewing", "complete", "failed", "cancelled"].contains(value)
            })
            .ok_or_else(|| Error::new("Review run state is unavailable."))?;
        json!({"id":summary_identity(repo,&raw["id"],80)?,"state":state,"scope":raw["scope"].as_str().filter(|value|*value=="conversation"),"startedAt":summary_time(&raw["startedAt"]),"updatedAt":summary_time(&raw["updatedAt"]),"completedAt":summary_time(&raw["completedAt"])})
    } else {
        Value::Null
    };
    let result = json!({"id":id,"revision":revision,"run":run,"freshness":summary_freshness(&review["freshness"])?,"freshnessCheckedAt":summary_time(&review["freshnessCheckedAt"]),"comments":comments,"assessments":assessments});
    // Fixed status tags are actionable too; do not let generic redaction silently
    // turn a valid freshness/run state into a different result after this owner.
    for value in [
        "current",
        "outdated",
        "unverified",
        "requested",
        "reviewing",
        "complete",
        "failed",
        "cancelled",
        "conversation",
    ] {
        summary_identity(repo, &json!(value), 80)?;
    }
    Ok(result)
}
fn revision(review: &Value) -> Result<u64> {
    review["revision"]
        .as_u64()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or_else(|| Error::new("Review revision is unavailable."))
}

fn display(repo: &Repository, value: &Value, limit: usize, flags: &mut Value) -> Result<Value> {
    let original = value
        .as_str()
        .ok_or_else(|| Error::new("Invalid saved review text."))?;
    let mut text = original.to_owned();
    redaction::redact_text(repo, &mut text);
    if text != original {
        flags["contentRedacted"] = json!(true);
    }
    if text.chars().count() > limit {
        flags["contentTruncated"] = json!(true);
    }
    Ok(json!(text.chars().take(limit).collect::<String>()))
}

fn public_run(repo: &Repository, run: &Value) -> Result<Value> {
    if run.is_null() {
        return Ok(Value::Null);
    }
    let state = run["state"]
        .as_str()
        .filter(|state| {
            ["requested", "reviewing", "complete", "failed", "cancelled"].contains(state)
        })
        .ok_or_else(|| Error::new("Review run state is unavailable."))?;
    summary_identity(repo, &json!(state), 20)?;
    Ok(
        json!({"id":summary_identity(repo,&run["id"],80)?,"state":state,
        "startedAt":summary_time(&run["startedAt"]),"updatedAt":summary_time(&run["updatedAt"]),
        "completedAt":summary_time(&run["completedAt"])}),
    )
}

fn public_rows(
    repo: &Repository,
    review: &Value,
    field: &str,
    flags: &mut Value,
) -> Result<Vec<Value>> {
    let rows = match review.get(field) {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(rows)) if rows.len() <= 100 => rows.clone(),
        _ => {
            return Err(Error::new(
                "Review reads support up to 100 comments and 100 assessments.",
            ))
        }
    };
    let mut result = Vec::with_capacity(rows.len());
    let mut ids = std::collections::HashSet::new();
    for row in rows {
        let id = summary_identity(repo, &row["id"], 120)?;
        if !ids.insert(id.as_str().unwrap().to_owned()) {
            return Err(Error::new("Duplicate saved review identity."));
        }
        let mut entry = json!({"id":id,"text":display(repo,&row["text"],8000,flags)?,
            "author":display(repo,&row["author"],120,flags)?,"createdAt":summary_time(&row["createdAt"]),
            "reviewedAt":summary_time(&row["reviewedAt"])});
        for key in ["requestId", "pass"] {
            if !row[key].is_null() {
                entry[key] = summary_identity(repo, &row[key], 120)?;
            }
        }
        if field == "comments" {
            for key in ["path", "root", "excerpt"] {
                if !row[key].is_null() {
                    entry[key] = display(repo, &row[key], 4096, flags)?;
                }
            }
            let side = row["side"]
                .as_str()
                .filter(|side| ["additions", "deletions"].contains(side))
                .ok_or_else(|| Error::new("Invalid saved review side."))?;
            summary_identity(repo, &json!(side), 20)?;
            entry["side"] = json!(side);
            for key in ["start", "end"] {
                entry[key] = json!(row[key]
                    .as_u64()
                    .filter(|value| *value > 0 && *value <= 9_007_199_254_740_991)
                    .ok_or_else(|| Error::new("Invalid saved review line."))?);
            }
        } else {
            entry["sentAt"] = summary_time(&row["sentAt"]);
        }
        result.push(entry);
    }
    Ok(result)
}

fn public_review(repo: &Repository, review: &Value) -> Result<Value> {
    let mut flags = json!({"contentRedacted":false,"contentTruncated":false});
    if review["note"].as_str().is_none_or(|note| note.len() > 8000) {
        return Err(Error::new("Saved review note exceeds its editable limit."));
    }
    let result = json!({"revision":revision(review)?,"note":display(repo,&review["note"],8000,&mut flags)?,
        "comments":public_rows(repo,review,"comments",&mut flags)?,
        "assessments":public_rows(repo,review,"assessments",&mut flags)?,"run":public_run(repo,&review["run"])?});
    let mut result = result;
    for key in ["contentRedacted", "contentTruncated"] {
        result[key] = flags[key].clone();
    }
    if serde_json::to_vec(&result)?.len() > 4 * 1024 * 1024 {
        return Err(Error::new("Review projection exceeds 4 MiB."));
    }
    Ok(result)
}

fn history_entries(record: &Value) -> Result<Vec<Value>> {
    let history = &record["reviewHistory"];
    if history.is_null() {
        return Ok(vec![]);
    }
    if history["version"] != 1 {
        return Err(Error::new("Unsupported review history version."));
    }
    match history["entries"].as_array() {
        Some(entries) if entries.len() <= 20 => Ok(entries.clone()),
        _ => Err(Error::new("Review history exceeds 20 rounds.")),
    }
}

pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    let id = text(args, "id");
    summary_identity(repo, &json!(id), 80)?;
    let record = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        sessions::get(&state, id)?.clone()
    };
    if let Some(history_id) = args.get("historyId") {
        summary_identity(repo, history_id, 80)?;
        let entries = history_entries(&record)?;
        let matches = entries
            .iter()
            .filter(|entry| entry["id"] == *history_id)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(Error::new("Saved review round unavailable."));
        }
        return Ok(
            json!({"id":id,"historyId":history_id,"readOnly":true,"archivedAt":summary_time(&matches[0]["archivedAt"]),"review":public_review(repo,&matches[0]["review"])?}),
        );
    }
    let review = draft(&record);
    let token = note_token(text(&review, "note"));
    summary_identity(repo, &json!(token), 64)?;
    Ok(json!({"id":id,"readOnly":false,"noteToken":token,"review":public_review(repo,&review)?}))
}

pub fn history(repo: &Repository, args: &Value) -> Result<Value> {
    let id = text(args, "id");
    summary_identity(repo, &json!(id), 80)?;
    let record = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        sessions::get(&state, id)?.clone()
    };
    let mut rounds = vec![];
    let mut ids = std::collections::HashSet::new();
    for entry in history_entries(&record)? {
        let history_id = summary_identity(repo, &entry["id"], 80)?;
        if !ids.insert(history_id.as_str().unwrap().to_owned()) {
            return Err(Error::new("Duplicate saved review round."));
        }
        let review = &entry["review"];
        let mut flags = json!({"contentRedacted":false,"contentTruncated":false});
        let preview = display(repo, &review["note"], 240, &mut flags)?;
        rounds.push(json!({"id":history_id,"archivedAt":summary_time(&entry["archivedAt"]),"revision":revision(review)?,
            "run":public_run(repo,&review["run"])?,"commentCount":arr(&review["comments"]).len(),
            "assessmentCount":arr(&review["assessments"]).len(),"notePreview":preview,
            "contentRedacted":flags["contentRedacted"],"contentTruncated":flags["contentTruncated"],"readOnly":true}));
    }
    Ok(json!({"id":id,"reviews":rounds,"limit":20}))
}

pub fn update_note(repo: &Repository, args: &Value) -> Result<Value> {
    let id = text(args, "id");
    let note = args["note"]
        .as_str()
        .filter(|note| note.len() <= 8000)
        .ok_or_else(|| Error::new("Supply a review note of at most 8000 bytes."))?;
    summary_identity(repo, &json!(id), 80)?;
    repo.transaction(|state| {
        let before = draft(sessions::get(state, id)?);
        let review = edit_note(state, id, text(args, "noteToken"), note)?;
        Ok(json!({"id":id,"revision":revision(&review)?,"changed":review != before}))
    })
}

#[cfg(test)]
mod tests;
