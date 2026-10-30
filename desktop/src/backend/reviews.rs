use super::{
    arr, json, now, redaction, sessions, sid, text, uuid, Error, Repository, Result, Value,
};
mod access;
pub use access::{history, read, update_note};

pub(super) fn draft(record: &Value) -> Value {
    if record["reviewDraft"].is_object() {
        record["reviewDraft"].clone()
    } else {
        json!({"comments":[],"note":"","revision":0})
    }
}
fn valid_pass(pass: &str) -> bool {
    !pass.is_empty()
        && pass.len() <= 80
        && pass
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}
pub(super) fn versions(repo: &Repository, id: &str) -> Result<Value> {
    let record = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        sessions::get(&state, id)?.clone()
    };
    let roots = std::iter::once(text(&record, "cwd").to_owned())
        .chain(
            arr(&record["worktrees"])
                .iter()
                .map(|t| text(t, "path").to_owned()),
        )
        .filter(|r| !r.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    let mut result = json!({});
    for root in roots {
        match super::git_diff::review_version(repo, id, &root) {
            Ok(v) => result[text(&v, "root")] = v["version"].clone(),
            Err(_) => result[&root] = Value::Null,
        }
    }
    Ok(result)
}
fn freshness(before: &Value, current: &Value) -> &'static str {
    let Some(before) = before.as_object().filter(|m| !m.is_empty()) else {
        return "unverified";
    };
    if before.iter().any(|(root, value)| {
        !value.is_null() && !current[root].is_null() && value != &current[root]
    }) || current
        .as_object()
        .unwrap()
        .keys()
        .any(|root| !before.contains_key(root))
    {
        return "outdated";
    }
    if before
        .iter()
        .any(|(root, value)| value.is_null() || current[root].is_null())
    {
        return "unverified";
    }
    "current"
}
fn check(repo: &Repository, id: &str, mut review: Value) -> Result<Value> {
    let current = versions(repo, id)?;
    let mut assessments = arr(&review["assessments"]);
    for assessment in &mut assessments {
        assessment["freshness"] = json!(freshness(&assessment["versions"], &current));
    }
    review["assessments"] = json!(assessments);
    let mut comments = arr(&review["comments"]);
    for comment in &mut comments {
        let stamp =
            super::git_diff::review_file_stamp(text(comment, "root"), text(comment, "path"));
        comment["freshness"] = json!(match (comment["fileStamp"].as_str(), stamp.as_deref()) {
            (Some(a), Some(b)) if a == b => "current",
            (Some(_), Some(_)) => "outdated",
            _ => "unverified",
        });
    }
    review["comments"] = json!(comments);
    review["freshness"] = json!(freshness(&review["run"]["versions"], &current));
    review["freshnessCheckedAt"] = json!(now());
    Ok(json!({"id":id,"review":review}))
}
fn summary_identity(repo: &Repository, value: &Value, max: usize) -> Result<Value> {
    let text = value
        .as_str()
        .filter(|value| {
            !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| Error::new("Review identity is unavailable."))?;
    let mut safe = text.to_owned();
    super::redaction::redact_text(repo, &mut safe);
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
    let checked = action(repo, &json!({"id":id,"action":"check"}))?;
    let review = &checked["review"];
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
fn set_request(review: &mut Value, request: &str, baseline: &Value) -> Result<()> {
    let request = sid(request)?;
    if review["run"]["id"] != request {
        if ["requested", "reviewing"].contains(&text(&review["run"], "state")) {
            return Err(Error::new(
                "A review is already active. Clear its indicator before starting another.",
            ));
        }
        review["run"] = json!({"id":request,"state":"requested","scope":"conversation","versions":baseline,"startedAt":now(),"updatedAt":now(),"passes":{},"inputCommentIds":arr(&review["comments"]).iter().map(|c|c["id"].clone()).collect::<Vec<_>>(),"inputNote":review["note"]});
    }
    Ok(())
}
fn remove_or_ack(review: &mut Value, saved: &mut Vec<Value>, p: &Value, operation: &str) {
    let ids = arr(&p["commentIds"]);
    saved.retain(|c| !ids.contains(&c["id"]));
    if operation == "remove" && p["assessmentIds"].is_array() {
        let ids = arr(&p["assessmentIds"]);
        let mut assessments = arr(&review["assessments"]);
        assessments.retain(|a| !ids.contains(&a["id"]));
        review["assessments"] = json!(assessments);
    }
    if operation == "ack" {
        let ids = arr(&p["assessmentIds"]);
        let mut assessments = arr(&review["assessments"]);
        for assessment in &mut assessments {
            if ids.contains(&assessment["id"]) && assessment.get("sentAt").is_none() {
                assessment["sentAt"] = json!(now());
            }
        }
        if !assessments.is_empty() {
            review["assessments"] = json!(assessments);
        }
    }
    if operation == "ack" && p["note"] == review["note"] {
        review["note"] = json!("");
    }
}
pub(super) fn reserve_request(
    state: &mut Value,
    id: &str,
    request: &str,
    revision: u64,
    baseline: &Value,
) -> Result<Value> {
    let mut review = draft(sessions::get(state, id)?);
    if review["revision"] != revision {
        return Err(Error::new(
            "The review changed. Reload it before requesting a review.",
        ));
    }
    let before = review.clone();
    set_request(&mut review, request, baseline)?;
    if review != before {
        review["revision"] = json!(revision + 1);
    }
    sessions::get_mut(state, id)?["reviewDraft"] = review.clone();
    Ok(review)
}
pub(super) fn acknowledge_dispatch(state: &mut Value, id: &str, ack: &Value) -> Result<()> {
    let record = sessions::get(state, id)?;
    if ack["historyId"] != record["reviewHistory"]["entries"][0]["id"] {
        return Ok(());
    }
    let mut review = draft(record);
    let before = review.clone();
    let mut saved = arr(&review["comments"]);
    let mut exact = ack.clone();
    exact["commentIds"] = json!(arr(&ack["comments"])
        .iter()
        .filter(|captured| saved.iter().any(|current| current == *captured))
        .map(|captured| captured["id"].clone())
        .collect::<Vec<_>>());
    exact["assessmentIds"] = json!(arr(&ack["assessments"])
        .iter()
        .filter(|captured| arr(&review["assessments"])
            .iter()
            .any(|current| current == *captured))
        .map(|captured| captured["id"].clone())
        .collect::<Vec<_>>());
    remove_or_ack(&mut review, &mut saved, &exact, "ack");
    review["comments"] = json!(saved);
    if review != before {
        review["revision"] = json!(before["revision"].as_u64().unwrap_or(0) + 1);
    }
    sessions::get_mut(state, id)?["reviewDraft"] = review;
    super::guidance::complete(state, "review.pending-comments");
    Ok(())
}
pub(super) fn cancel_dispatch(state: &mut Value, id: &str, operation: &str) -> Result<()> {
    let mut review = draft(sessions::get(state, id)?);
    if review["run"]["id"] == operation
        && ["requested", "reviewing"].contains(&text(&review["run"], "state"))
    {
        review["run"]["state"] = json!("cancelled");
        review["run"]["updatedAt"] = json!(now());
        review["revision"] = json!(review["revision"].as_u64().unwrap_or(0) + 1);
        sessions::get_mut(state, id)?["reviewDraft"] = review;
    }
    Ok(())
}
fn anchor_comments(repo: &Repository, id: &str, p: &Value, comments: &mut [Value]) -> Result<()> {
    if (comments.is_empty() && !p["overall"].is_object())
        || comments.len() > 100
        || serde_json::to_vec(p)?.len() > 96 * 1024
    {
        return Err(Error::new(
            "Supply comments or an overall assessment within 96 KiB.",
        ));
    }
    let mut files = std::collections::HashMap::new();
    for comment in comments.iter_mut() {
        let key = (
            text(comment, "root").to_owned(),
            text(comment, "path").to_owned(),
        );
        let file = if let Some(file) = files.get(&key) {
            file
        } else {
            let file = super::git_diff::file_for_session(repo, id, &key.1, &key.0)?;
            files.entry(key).or_insert(file)
        };
        let side = text(comment, "side");
        let content = match side {
            "additions" => text(file, "newContent"),
            "deletions" => text(file, "oldContent"),
            _ => return Err(Error::new("Comment side must be additions or deletions.")),
        };
        let start = comment["start"].as_u64().unwrap_or(0);
        let end = comment["end"].as_u64().unwrap_or(start);
        let lines = usize::try_from(start)
            .ok()
            .zip(usize::try_from(end).ok())
            .filter(|&(first, last)| {
                first != 0 && first <= last && last <= content.lines().count()
            });
        let Some((first, last)) = lines.filter(|_| {
            !text(comment, "text").trim().is_empty() && text(comment, "text").len() <= 4000
        }) else {
            return Err(Error::new(
                "Select valid lines and supply a comment of at most 4000 bytes.",
            ));
        };
        if !text(comment, "fingerprint").is_empty() && comment["fingerprint"] != file["fingerprint"]
        {
            return Err(Error::new(
                "The file changed. Reload the diff before adding this comment.",
            ));
        }
        let key = text(comment, "id").to_owned();
        if key.len() > 120
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        {
            return Err(Error::new("Invalid comment ID."));
        }
        let author = text(comment, "author").trim();
        if author.len() > 120 {
            return Err(Error::new("Comment author exceeds 120 bytes."));
        }
        let stamp = super::git_diff::review_file_stamp(text(file, "root"), text(file, "path"));
        *comment = json!({"id":if key.is_empty(){uuid()}else{key},"author":if author.is_empty(){"You"}else{author},"createdAt":now(),"fileStamp":stamp,"requestId":p["requestId"],"path":file["path"],"root":file["root"],"side":side,"start":start,"end":end,"text":text(comment,"text").trim(),"baseCommit":file["baseCommit"],"fingerprint":file["fingerprint"],"excerpt":content.split('\n').skip(first-1).take((last-first+1).min(20)).collect::<Vec<_>>().join("\n")});
    }
    Ok(())
}
fn add_overall(review: &mut Value, p: &Value) -> Result<()> {
    let pass = text(p, "pass");
    let overall = &p["overall"];
    if !valid_pass(pass)
        || text(p, "requestId").is_empty()
        || text(overall, "author").trim().is_empty()
        || text(overall, "author").len() > 120
        || text(overall, "text").trim().is_empty()
        || text(overall, "text").len() > 8000
    {
        return Err(Error::new("An overall assessment needs a current request, review pass, author and text of at most 8000 bytes."));
    }
    let key = format!("{}-{pass}", text(p, "requestId"));
    let mut assessments = arr(&review["assessments"]);
    if let Some(existing) = assessments.iter().find(|a| a["id"] == key) {
        if text(existing, "author") != text(overall, "author").trim()
            || text(existing, "text") != text(overall, "text").trim()
        {
            return Err(Error::new(
                "This reviewer already submitted a different assessment.",
            ));
        }
    } else {
        assessments.push(json!({"id":key,"requestId":p["requestId"],"pass":pass,"author":text(overall,"author").trim(),"text":text(overall,"text").trim(),"root":review["run"]["root"],"baseCommit":review["run"]["baseCommit"],"versions":review["run"]["versions"],"createdAt":now()}));
    }
    review["assessments"] = json!(assessments);
    Ok(())
}
fn apply_status(review: &mut Value, saved: &mut [Value], p: &Value) -> Result<()> {
    let status = text(p, "state");
    let pass = text(p, "pass");
    if !status.is_empty()
        && (if pass.is_empty() {
            text(&review["run"], "state") != status
        } else {
            text(&review["run"]["passes"], pass) != status
        })
    {
        if !["reviewing", "complete", "failed", "cancelled"].contains(&status) {
            return Err(Error::new("Invalid review state."));
        }
        if pass.is_empty() {
            if status == "complete" {
                let receipt = p.get("reviewedRevision");
                if receipt.is_some() && receipt != review.get("revision") {
                    return Err(Error::new("Review feedback changed. Read the draft again before completing this round."));
                }
                let inputs = arr(&review["run"]["inputCommentIds"]);
                for comment in saved.iter_mut() {
                    if receipt.is_some()
                        || inputs.contains(&comment["id"])
                        || comment["requestId"] == review["run"]["id"]
                    {
                        comment["reviewedAt"] = json!(now());
                    }
                }
                let mut assessments = arr(&review["assessments"]);
                for assessment in &mut assessments {
                    if receipt.is_some() || assessment["requestId"] == review["run"]["id"] {
                        assessment["reviewedAt"] = json!(now());
                    }
                }
                review["assessments"] = json!(assessments);
                if receipt.is_some() || review["note"] == review["run"]["inputNote"] {
                    review["reviewedNote"] = review["note"].clone();
                }
                review["run"]["completedAt"] = json!(now());
            }
            review["run"]["state"] = json!(status);
        } else {
            if !valid_pass(pass) {
                return Err(Error::new("Invalid review pass."));
            }
            review["run"]["passes"][pass] = json!(status);
            review["run"]["state"] = json!("reviewing");
        }
        review["run"]["updatedAt"] = json!(now());
    }
    Ok(())
}
fn archive_review(session: &mut Value, review: &Value, saved: &[Value], p: &Value) -> Result<()> {
    if p["revision"].as_u64().is_none() || p["revision"] != review["revision"] {
        return Err(Error::new(
            "The review changed. Reload it before starting a new review.",
        ));
    }
    if ["requested", "reviewing"].contains(&text(&review["run"], "state")) {
        return Err(Error::new(
            "Finish or cancel the active review before starting a new review.",
        ));
    }
    let history = &session["reviewHistory"];
    if !history.is_null() && history["version"] != 1 {
        return Err(Error::new("Unsupported review history version."));
    }
    let mut entries = arr(&history["entries"]);
    if !saved.is_empty()
        || !text(review, "note").is_empty()
        || !arr(&review["assessments"]).is_empty()
        || review["run"].is_object()
    {
        entries.insert(0, json!({"id":uuid(),"archivedAt":now(),"review":review}));
    }
    entries.truncate(20);
    while serde_json::to_vec(&entries)?.len() > 1024 * 1024 {
        entries.pop();
    }
    session["reviewHistory"] = json!({"version":1,"entries":entries});
    Ok(())
}
fn merge_comments(saved: &mut Vec<Value>, comments: Vec<Value>) -> Result<()> {
    for comment in comments {
        if let Some(existing) = saved.iter().find(|c| c["id"] == comment["id"]) {
            for field in [
                "author",
                "path",
                "root",
                "side",
                "start",
                "end",
                "text",
                "fingerprint",
            ] {
                if existing[field] != comment[field] {
                    return Err(Error::new(
                        "Comment ID already exists with different content.",
                    ));
                }
            }
        } else {
            saved.push(comment);
        }
    }
    if saved.len() > 100 {
        return Err(Error::new(
            "Send or remove comments before adding more. A draft holds 100 comments.",
        ));
    }
    Ok(())
}
fn set_note(review: &mut Value, p: &Value) -> Result<()> {
    let note = p["note"]
        .as_str()
        .ok_or_else(|| Error::new("Expected review note."))?;
    if note.len() > 8000 {
        return Err(Error::new("Review note exceeds 8000 bytes."));
    }
    if p["previousNote"] != review["note"] {
        return Err(Error::new(
            "The review note changed. Reload it before editing.",
        ));
    }
    review["note"] = json!(note);
    Ok(())
}
fn complete_guidance(state: &mut Value, operation: &str, p: &Value, review: &Value) {
    if operation == "append"
        && arr(&review["comments"])
            .iter()
            .any(|c| c["author"] == "You")
    {
        super::guidance::complete(state, "review.comment-selection");
    }
    if operation == "ack" {
        super::guidance::complete(state, "review.pending-comments");
    }
    if operation == "status" && ["reviewing", "complete"].contains(&text(p, "state")) {
        super::guidance::complete(state, "review.agent-scope");
        super::guidance::complete(state, "review.pending-comments");
    }
}
pub fn action(repo: &Repository, p: &Value) -> Result<Value> {
    let id = text(p, "id");
    let operation = text(p, "action");
    if operation == "history" {
        let history = {
            let state = repo.state.lock().map_err(|_| Error::io())?;
            sessions::get(&state, id)?["reviewHistory"].clone()
        };
        if !history.is_null() && history["version"] != 1 {
            return Err(Error::new("Unsupported review history version."));
        }
        return Ok(json!({"id":id,"reviews":arr(&history["entries"]),"limit":20}));
    }
    if operation == "read" || operation == "check" {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        let review = draft(sessions::get(&state, id)?);
        drop(state);
        return if operation == "check" {
            check(repo, id, review)
        } else {
            Ok(json!({"id":id,"review":review}))
        };
    }
    let baseline = if operation == "request"
        || operation == "status" && p["state"] == "reviewing" && text(p, "pass").is_empty()
    {
        Some(versions(repo, id)?)
    } else {
        None
    };
    let mut comments = arr(&p["comments"]);
    if operation == "append" {
        anchor_comments(repo, id, p, &mut comments)?;
    }
    repo.transaction(|state| {
        let mut review = draft(sessions::get(state, id)?);
        let before = review.clone();
        let mut saved = arr(&review["comments"]);
        if operation == "status" || p.get("requestId").is_some() && operation == "append" {
            if text(p, "requestId").is_empty() || review["run"]["id"] != p["requestId"] {
                return Err(Error::new(
                    "This review request is no longer current. Read its draft before updating it.",
                ));
            }
            if !["requested", "reviewing"].contains(&text(&review["run"], "state")) {
                return Err(Error::new("This review request has already finished."));
            }
        }
        if operation == "append" && p.get("overall").is_some() {
            add_overall(&mut review, p)?;
        }
        match operation {
            "new" => {
                archive_review(sessions::get_mut(state, id)?, &review, &saved, p)?;
                review = json!({"comments":[],"note":"","revision":before["revision"]});
                saved.clear();
            }
            "request" => {
                set_request(
                    &mut review,
                    text(p, "requestId"),
                    baseline.as_ref().unwrap(),
                )?;
            }
            "status" => {
                if review["run"]["state"] == "requested" {
                    if let Some(versions) = &baseline {
                        review["run"]["versions"] = versions.clone();
                    }
                }
            }

            "append" => merge_comments(&mut saved, comments)?,
            "remove" | "ack" => {
                remove_or_ack(&mut review, &mut saved, p, operation);
            }
            "note" => set_note(&mut review, p)?,
            _ => return Err(Error::new("Unknown review action.")),
        }
        if operation == "status" || operation == "append" && p.get("requestId").is_some() {
            apply_status(&mut review, &mut saved, p)?;
        }
        review["comments"] = json!(saved);
        if serde_json::to_vec(&review)?.len() > 256 * 1024 {
            return Err(Error::new("Review draft exceeds 256 KiB."));
        }
        if review != before {
            review["revision"] = json!(before["revision"].as_u64().unwrap_or(0) + 1);
        }
        sessions::get_mut(state, id)?["reviewDraft"] = review.clone();
        complete_guidance(state, operation, p, &review);
        Ok(json!({"id":id,"review":review}))
    })
}

#[cfg(test)]
mod tests;
