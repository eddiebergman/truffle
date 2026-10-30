use super::{
    arr, draft, json, note_token, redaction, sessions, summary_identity, summary_time, text, Error,
    Repository, Result, Value,
};

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
        let mut review = draft(sessions::get(state, id)?);
        if args["noteToken"] != note_token(text(&review, "note")) {
            let mut error = Error::new("The review note changed. Read it again before editing.");
            error.status = 409;
            return Err(error);
        }
        let changed = review["note"] != note;
        if changed {
            review["note"] = json!(note);
            review["revision"] = json!(revision(&review)? + 1);
        }
        let next = revision(&review)?;
        if changed {
            sessions::get_mut(state, id)?["reviewDraft"] = review;
        }
        Ok(json!({"id":id,"revision":next,"changed":changed}))
    })
}

#[cfg(test)]
mod tests;
