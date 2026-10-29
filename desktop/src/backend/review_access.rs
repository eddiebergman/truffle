use super::*;

fn revision(review: &Value) -> Result<u64> {
    review["revision"]
        .as_u64()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or(Error::new("Review revision is unavailable."))
}

fn display(repo: &Repository, value: &Value, limit: usize, flags: &mut Value) -> Result<Value> {
    let original = value
        .as_str()
        .ok_or(Error::new("Invalid saved review text."))?;
    let mut text = original.to_owned();
    commands::redact_text(repo, &mut text);
    if text != original {
        flags["contentRedacted"] = json!(true);
    }
    if text.chars().count() > limit {
        flags["contentTruncated"] = json!(true);
    }
    Ok(json!(text.chars().take(limit).collect::<String>()))
}

fn note_token(id: &str, review: &Value) -> Result<String> {
    Ok(context_kit::sha256(&serde_json::to_vec(
        &json!({"id":id,"revision":revision(review)?,"note":review["note"]}),
    )?))
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
        .ok_or(Error::new("Review run state is unavailable."))?;
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
                .ok_or(Error::new("Invalid saved review side."))?;
            summary_identity(repo, &json!(side), 20)?;
            entry["side"] = json!(side);
            for key in ["start", "end"] {
                entry[key] = json!(row[key]
                    .as_u64()
                    .filter(|value| *value > 0 && *value <= 9_007_199_254_740_991)
                    .ok_or(Error::new("Invalid saved review line."))?);
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
        state["sessions"][position(&state, id)?].clone()
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
    let token = note_token(id, &review)?;
    summary_identity(repo, &json!(token), 64)?;
    Ok(json!({"id":id,"readOnly":false,"noteToken":token,"review":public_review(repo,&review)?}))
}

pub fn history(repo: &Repository, args: &Value) -> Result<Value> {
    let id = text(args, "id");
    summary_identity(repo, &json!(id), 80)?;
    let record = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        state["sessions"][position(&state, id)?].clone()
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
        .ok_or(Error::new("Supply a review note of at most 8000 bytes."))?;
    let expected = args["revision"]
        .as_u64()
        .filter(|revision| *revision < 9_007_199_254_740_991)
        .ok_or(Error::new("Supply the current review revision."))?;
    summary_identity(repo, &json!(id), 80)?;
    repo.transaction(|state| {
        let index = position(state, id)?;
        let mut review = draft(&state["sessions"][index]);
        if revision(&review)? != expected || args["noteToken"] != note_token(id, &review)? {
            let mut error =
                Error::new("The review changed. Read it again before editing the note.");
            error.status = 409;
            return Err(error);
        }
        let changed = review["note"] != note;
        if changed {
            review["note"] = json!(note);
            review["revision"] = json!(expected + 1);
        }
        let next = revision(&review)?;
        if changed {
            state["sessions"][index]["reviewDraft"] = review;
        }
        Ok(json!({"id":id,"revision":next,"changed":changed}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, std::sync::Arc<Repository>, String) {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".env"),
            "REVIEW_ACCESS_SECRET=benign-review-secret-value\n",
        )
        .unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        let id = uuid();
        let review = json!({"revision":3,"note":"Check benign-review-secret-value carefully","privateNs":"private-review-canary",
            "comments":[{"id":"comment-a","author":"You","text":"Local feedback","path":"code.rs","root":"/missing-review-checkout","side":"additions","start":1,"end":2,"excerpt":"let x = 1;","privateNs":"private-review-canary"}],
            "assessments":[{"id":"assessment-a","author":"Reviewer","text":"Assessment","pass":"correctness","privateNs":"private-review-canary"}],
            "run":{"id":"run-a","state":"complete","inputNote":"private-review-canary","inputCommentIds":["private-review-canary"],"versions":{"private-review-canary":{}},"privateNs":"private-review-canary","passes":{"private-review-canary":"complete"}}});
        repo.transaction(|state| {
            state["sessions"] = json!([{"id":id,"cwd":"/missing-review-checkout","privateNs":"private-review-canary","reviewDraft":review,"reviewHistory":{"version":1,"privateNs":"private-review-canary","entries":[{"id":"history-a","archivedAt":1,"review":review,"privateNs":"private-review-canary"}]}}]);
            Ok(())
        }).unwrap();
        (temp, repo, id)
    }

    fn invoke(repo: &std::sync::Arc<Repository>, name: &str, args: Value) -> Result<Value> {
        commands::invoke(repo, &json!({"name":name,"args":args}), true)
    }

    #[test]
    fn named_review_reads_project_current_and_historical_data_without_writes() {
        let (_temp, repo, id) = fixture();
        let before = repo.state.lock().unwrap().clone();
        let bytes = std::fs::read(&repo.data).unwrap();
        let current = invoke(&repo, "review.read", json!({"id":id})).unwrap();
        let history = invoke(&repo, "review.history", json!({"id":id})).unwrap();
        let archived = invoke(
            &repo,
            "review.read",
            json!({"id":id,"historyId":history["reviews"][0]["id"]}),
        )
        .unwrap();
        assert_eq!(current["readOnly"], false);
        assert_eq!(current["review"]["revision"], 3);
        assert_eq!(current["review"]["note"], "Check [redacted] carefully");
        assert_eq!(current["review"]["contentRedacted"], true);
        assert_eq!(current["review"]["comments"][0]["id"], "comment-a");
        assert_eq!(current["review"]["assessments"][0]["id"], "assessment-a");
        assert_eq!(archived["readOnly"], true);
        assert!(archived.get("noteToken").is_none());
        assert_eq!(archived["review"], current["review"]);
        assert_eq!(history["reviews"][0]["commentCount"], 1);
        assert!(history["reviews"][0].get("review").is_none());
        for value in [current, history, archived] {
            let text = value.to_string();
            for private in [
                "private-review-canary",
                "benign-review-secret-value",
                "inputNote",
                "inputCommentIds",
                "versions",
                "passes",
            ] {
                assert!(
                    !text.contains(private),
                    "Private review data escaped: {private}"
                );
            }
        }
        assert!(invoke(&repo, "review.read", json!({"id":id,"historyId":"missing"})).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }

    #[test]
    fn review_note_update_uses_current_tokens_and_preserves_unrelated_state() {
        let (_temp, repo, id) = fixture();
        let current = invoke(&repo, "review.read", json!({"id":id})).unwrap();
        let before = repo.state.lock().unwrap().clone();
        let args = json!({"id":id,"revision":3,"noteToken":current["noteToken"],"note":"Replacement note"});
        let receipt = invoke(&repo, "review.note.update", args.clone()).unwrap();
        assert_eq!(receipt, json!({"id":id,"revision":4,"changed":true}));
        let mut expected = before;
        expected["sessions"][0]["reviewDraft"]["note"] = json!("Replacement note");
        expected["sessions"][0]["reviewDraft"]["revision"] = json!(4);
        assert_eq!(*repo.state.lock().unwrap(), expected);
        let bytes = std::fs::read(&repo.data).unwrap();
        assert_eq!(
            invoke(&repo, "review.note.update", args)
                .unwrap_err()
                .status,
            409
        );
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
        let latest = invoke(&repo, "review.read", json!({"id":id})).unwrap();
        assert_eq!(invoke(&repo,"review.note.update",json!({"id":id,"revision":4,"noteToken":latest["noteToken"],"note":"Replacement note"})).unwrap()["changed"],false);
        for invalid in [
            json!({"id":id,"revision":4,"noteToken":current["noteToken"],"note":"stale"}),
            json!({"id":id,"revision":4,"noteToken":latest["noteToken"],"note":"é".repeat(4001)}),
            json!({"id":id,"revision":4,"noteToken":latest["noteToken"],"note":"bad","historyId":"history-a"}),
        ] {
            assert!(invoke(&repo, "review.note.update", invalid).is_err());
        }
        assert_eq!(*repo.state.lock().unwrap(), expected);
    }

    #[test]
    fn named_review_reads_reject_unsafe_identities_and_oversized_notes() {
        let (_temp, repo, id) = fixture();
        repo.transaction(|state| {
            state["sessions"][0]["reviewDraft"]["comments"][0]["id"] =
                json!("benign-review-secret-value");
            Ok(())
        })
        .unwrap();
        assert!(invoke(&repo, "review.read", json!({"id":id})).is_err());
        repo.transaction(|state| {
            state["sessions"][0]["reviewDraft"]["comments"][0]["id"] = json!("comment-a");
            state["sessions"][0]["reviewDraft"]["note"] = json!("x".repeat(8001));
            Ok(())
        })
        .unwrap();
        assert!(invoke(&repo, "review.read", json!({"id":id})).is_err());
    }

    #[test]
    fn named_review_reads_redact_before_display_caps_and_enforce_row_bounds() {
        let (_temp, repo, id) = fixture();
        repo.transaction(|state| {
            state["sessions"][0]["reviewDraft"]["comments"][0]["excerpt"] =
                json!("x".repeat(4090) + "benign-review-secret-value" + &"y".repeat(30));
            Ok(())
        })
        .unwrap();
        let current = invoke(&repo, "review.read", json!({"id":id})).unwrap();
        assert_eq!(
            current["review"]["comments"][0]["excerpt"],
            "x".repeat(4090) + "[redac"
        );
        assert_eq!(current["review"]["contentTruncated"], true);
        assert_eq!(current["review"]["contentRedacted"], true);
        repo.transaction(|state| {
            let row = state["sessions"][0]["reviewDraft"]["comments"][0].clone();
            state["sessions"][0]["reviewDraft"]["comments"] = json!((0..101)
                .map(|index| {
                    let mut row = row.clone();
                    row["id"] = json!(format!("comment-{index}"));
                    row
                })
                .collect::<Vec<_>>());
            Ok(())
        })
        .unwrap();
        assert!(invoke(&repo, "review.read", json!({"id":id})).is_err());
    }
}
