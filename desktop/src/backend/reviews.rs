use super::{arr, json, now, sessions, sid, text, uuid, Error, Repository, Result, Value};
mod access;
pub use access::{check_summary, history, read, update_note};

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
/// Supplies the checkout versions a review covers, keyed by Git root.
pub(super) trait Versions {
    /// Null marks a root whose version cannot be verified.
    fn of(&self, chat: &Value) -> Value;
}
pub(super) struct Git;
impl Versions for Git {
    fn of(&self, chat: &Value) -> Value {
        super::checkouts::versions(chat)
    }
}
#[cfg(test)]
pub(super) struct Fixed(pub Value);
#[cfg(test)]
impl Versions for Fixed {
    fn of(&self, _: &Value) -> Value {
        self.0.clone()
    }
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
/// Mark each part of the draft current, outdated or unverified against the checkouts. Writes nothing.
pub(super) fn check(repo: &Repository, versions: &impl Versions, id: &str) -> Result<Value> {
    let chat = chat(repo, id)?;
    let current = versions.of(&chat);
    let mut review = draft(&chat);
    let mut assessments = arr(&review["assessments"]);
    for assessment in &mut assessments {
        assessment["freshness"] = json!(freshness(&assessment["versions"], &current));
    }
    review["assessments"] = json!(assessments);
    let mut comments = arr(&review["comments"]);
    for comment in &mut comments {
        let stamp = super::checkouts::file_version(text(comment, "root"), text(comment, "path"));
        comment["freshness"] = json!(match (comment["fileStamp"].as_str(), stamp.as_deref()) {
            (Some(a), Some(b)) if a == b => "current",
            (Some(_), Some(_)) => "outdated",
            _ => "unverified",
        });
    }
    review["comments"] = json!(comments);
    review["freshness"] = json!(freshness(&review["run"]["versions"], &current));
    review["freshnessCheckedAt"] = json!(now());
    Ok(review)
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
        let stamp = super::checkouts::file_version(text(file, "root"), text(file, "path"));
        *comment = json!({"id":if key.is_empty(){uuid()}else{key},"author":if author.is_empty(){"You"}else{author},"createdAt":now(),"fileStamp":stamp,"requestId":p["requestId"],"path":file["path"],"root":file["root"],"side":side,"start":start,"end":end,"text":text(comment,"text").trim(),"baseCommit":file["baseCommit"],"fingerprint":file["fingerprint"],"excerpt":content.split('\n').skip(first-1).take((last-first+1).min(20)).collect::<Vec<_>>().join("\n")});
    }
    Ok(())
}
fn active(review: &Value) -> bool {
    ["requested", "reviewing"].contains(&text(&review["run"], "state"))
}
/// Apply one transition to a chat's draft. A change bumps the revision once.
fn commit(
    state: &mut Value,
    id: &str,
    change: impl FnOnce(&mut Value) -> Result<()>,
) -> Result<Value> {
    let chat = sessions::get_mut(state, id)?;
    let before = draft(chat);
    let mut review = before.clone();
    change(&mut review)?;
    if serde_json::to_vec(&review)?.len() > 256 * 1024 {
        return Err(Error::new("Review draft exceeds 256 KiB."));
    }
    if review != before {
        review["revision"] = json!(before["revision"].as_u64().unwrap_or(0) + 1);
        chat["reviewDraft"] = review.clone();
    }
    Ok(review)
}
/// Start a conversation review at the captured draft revision. Repeating the request changes nothing.
pub(super) fn request(
    state: &mut Value,
    id: &str,
    request: &str,
    revision: u64,
    baseline: &Value,
) -> Result<Value> {
    let request = sid(request)?;
    commit(state, id, |review| {
        if review["revision"] != revision {
            return Err(Error::new(
                "The review changed. Reload it before requesting a review.",
            ));
        }
        if review["run"]["id"] != request {
            if active(review) {
                return Err(Error::new(
                    "A review is already active. Clear its indicator before starting another.",
                ));
            }
            review["run"] = json!({"id":request,"state":"requested","scope":"conversation","versions":baseline,"startedAt":now(),"updatedAt":now(),"passes":{},"inputCommentIds":arr(&review["comments"]).iter().map(|c|c["id"].clone()).collect::<Vec<_>>(),"inputNote":review["note"]});
        }
        Ok(())
    })
}
/// A reviewer's update to the run it was requested for.
struct Progress<'a> {
    request: &'a str,
    /// Empty for the whole run.
    pass: &'a str,
    /// Empty when the update only adds feedback.
    state: &'a str,
    reviewed_revision: Option<&'a Value>,
    /// Versions the whole run covers once it starts reviewing.
    baseline: Option<Value>,
}
fn progress<'a>(repo: &Repository, versions: &impl Versions, p: &'a Value) -> Result<Progress<'a>> {
    let (state, pass) = (text(p, "state"), text(p, "pass"));
    let baseline = if state == "reviewing" && pass.is_empty() {
        Some(versions.of(&chat(repo, text(p, "id"))?))
    } else {
        None
    };
    Ok(Progress {
        request: text(p, "requestId"),
        pass,
        state,
        reviewed_revision: p.get("reviewedRevision"),
        baseline,
    })
}
fn current(review: &Value, request: &str) -> Result<()> {
    if request.is_empty() || review["run"]["id"] != request {
        return Err(Error::new(
            "This review request is no longer current. Read its draft before updating it.",
        ));
    }
    if !active(review) {
        return Err(Error::new("This review request has already finished."));
    }
    Ok(())
}
fn progress_guidance(state: &mut Value, progress: &Progress) {
    if ["reviewing", "complete"].contains(&progress.state) {
        super::guidance::complete(state, "review.agent-scope");
        super::guidance::complete(state, "review.pending-comments");
    }
}
/// Feedback added to the draft in one step.
struct Batch<'a> {
    /// Comments already anchored to the diff.
    comments: Vec<Value>,
    overall: Option<&'a Value>,
    progress: Option<Progress<'a>>,
}
/// Anchor comments from a browser or agent batch to the chat's diff, then report them.
pub(super) fn append(repo: &Repository, versions: &impl Versions, p: &Value) -> Result<Value> {
    let id = text(p, "id");
    let mut comments = arr(&p["comments"]);
    anchor_comments(repo, id, p, &mut comments)?;
    let progress = p
        .get("requestId")
        .map(|_| progress(repo, versions, p))
        .transpose()?;
    repo.transaction(|state| {
        report(
            state,
            id,
            Batch {
                comments,
                overall: p.get("overall"),
                progress,
            },
        )
    })
}
fn report(state: &mut Value, id: &str, batch: Batch) -> Result<Value> {
    let Batch {
        comments,
        overall,
        progress,
    } = batch;
    let review = commit(state, id, |review| {
        if let Some(progress) = &progress {
            current(review, progress.request)?;
        }
        if let Some(overall) = overall {
            add_overall(review, progress.as_ref(), overall)?;
        }
        merge_comments(review, comments)?;
        progress
            .as_ref()
            .map_or(Ok(()), |progress| apply_status(review, progress))
    })?;
    if arr(&review["comments"])
        .iter()
        .any(|c| c["author"] == "You")
    {
        super::guidance::complete(state, "review.comment-selection");
    }
    if let Some(progress) = &progress {
        progress_guidance(state, progress);
    }
    Ok(review)
}
/// Move the run or one of its passes to a new state.
pub(super) fn status(repo: &Repository, versions: &impl Versions, p: &Value) -> Result<Value> {
    let progress = progress(repo, versions, p)?;
    repo.transaction(|state| set_status(state, text(p, "id"), &progress))
}
fn set_status(state: &mut Value, id: &str, progress: &Progress) -> Result<Value> {
    let review = commit(state, id, |review| {
        current(review, progress.request)?;
        apply_status(review, progress)
    })?;
    progress_guidance(state, progress);
    Ok(review)
}
fn add_overall(review: &mut Value, progress: Option<&Progress>, overall: &Value) -> Result<()> {
    let (request, pass) = progress.map_or(("", ""), |p| (p.request, p.pass));
    if !valid_pass(pass)
        || request.is_empty()
        || text(overall, "author").trim().is_empty()
        || text(overall, "author").len() > 120
        || text(overall, "text").trim().is_empty()
        || text(overall, "text").len() > 8000
    {
        return Err(Error::new("An overall assessment needs a current request, review pass, author and text of at most 8000 bytes."));
    }
    let key = format!("{request}-{pass}");
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
        assessments.push(json!({"id":key,"requestId":request,"pass":pass,"author":text(overall,"author").trim(),"text":text(overall,"text").trim(),"root":review["run"]["root"],"baseCommit":review["run"]["baseCommit"],"versions":review["run"]["versions"],"createdAt":now()}));
    }
    review["assessments"] = json!(assessments);
    Ok(())
}
fn apply_status(review: &mut Value, progress: &Progress) -> Result<()> {
    let Progress {
        state: status,
        pass,
        reviewed_revision: receipt,
        ..
    } = *progress;
    if status.is_empty()
        || (if pass.is_empty() {
            text(&review["run"], "state") == status
        } else {
            text(&review["run"]["passes"], pass) == status
        })
    {
        return Ok(());
    }
    if !["reviewing", "complete", "failed", "cancelled"].contains(&status) {
        return Err(Error::new("Invalid review state."));
    }
    if !pass.is_empty() {
        if !valid_pass(pass) {
            return Err(Error::new("Invalid review pass."));
        }
        review["run"]["passes"][pass] = json!(status);
        review["run"]["state"] = json!("reviewing");
    } else if status == "complete" {
        if receipt.is_some() && receipt != review.get("revision") {
            return Err(Error::new(
                "Review feedback changed. Read the draft again before completing this round.",
            ));
        }
        mark_reviewed(review, receipt.is_some());
        review["run"]["state"] = json!(status);
    } else {
        if let (Some(baseline), "requested") = (&progress.baseline, text(&review["run"], "state")) {
            review["run"]["versions"] = baseline.clone();
        }
        review["run"]["state"] = json!(status);
    }
    review["run"]["updatedAt"] = json!(now());
    Ok(())
}
/// A completed run consumes the feedback it read. With a revision receipt that is the whole draft.
fn mark_reviewed(review: &mut Value, whole_draft: bool) {
    let run = review["run"].clone();
    let inputs = arr(&run["inputCommentIds"]);
    let mut comments = arr(&review["comments"]);
    for comment in &mut comments {
        if whole_draft || inputs.contains(&comment["id"]) || comment["requestId"] == run["id"] {
            comment["reviewedAt"] = json!(now());
        }
    }
    review["comments"] = json!(comments);
    let mut assessments = arr(&review["assessments"]);
    for assessment in &mut assessments {
        if whole_draft || assessment["requestId"] == run["id"] {
            assessment["reviewedAt"] = json!(now());
        }
    }
    review["assessments"] = json!(assessments);
    if whole_draft || review["note"] == run["inputNote"] {
        review["reviewedNote"] = review["note"].clone();
    }
    review["run"]["completedAt"] = json!(now());
}
fn merge_comments(review: &mut Value, comments: Vec<Value>) -> Result<()> {
    let mut saved = arr(&review["comments"]);
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
    review["comments"] = json!(saved);
    Ok(())
}
/// Replace the note when `token` matches the current one.
fn edit_note(state: &mut Value, id: &str, token: &str, note: &str) -> Result<Value> {
    if note.len() > 8000 {
        return Err(Error::new("Review note exceeds 8000 bytes."));
    }
    commit(state, id, |review| {
        if note_token(text(review, "note")) != token {
            let mut error = Error::new("The review note changed. Read it again before editing.");
            error.status = 409;
            return Err(error);
        }
        review["note"] = json!(note);
        Ok(())
    })
}
/// Identifies the note a caller based its edit on. The browser computes the same SHA-256.
fn note_token(note: &str) -> String {
    context_kit::sha256(note.as_bytes())
}
/// Delete comments, and assessments when `assessments` is given, by ID.
fn remove(
    state: &mut Value,
    id: &str,
    comments: &[Value],
    assessments: Option<&[Value]>,
) -> Result<Value> {
    commit(state, id, |review| {
        let mut saved = arr(&review["comments"]);
        saved.retain(|c| !comments.contains(&c["id"]));
        review["comments"] = json!(saved);
        if let Some(ids) = assessments {
            let mut saved = arr(&review["assessments"]);
            saved.retain(|a| !ids.contains(&a["id"]));
            review["assessments"] = json!(saved);
        }
        Ok(())
    })
}
/// Consume the feedback a written dispatch delivered. Feedback that changed since capture stays pending.
pub(super) fn acknowledge(state: &mut Value, id: &str, ack: &Value) -> Result<()> {
    if ack["historyId"] != sessions::get(state, id)?["reviewHistory"]["entries"][0]["id"] {
        return Ok(());
    }
    let delivered = |review: &Value, field: &str| {
        arr(&ack[field])
            .into_iter()
            .filter(|captured| arr(&review[field]).contains(captured))
            .map(|captured| captured["id"].clone())
            .collect::<Vec<_>>()
    };
    commit(state, id, |review| {
        let comments = delivered(review, "comments");
        let mut saved = arr(&review["comments"]);
        saved.retain(|c| !comments.contains(&c["id"]));
        review["comments"] = json!(saved);
        let sent = delivered(review, "assessments");
        let mut assessments = arr(&review["assessments"]);
        for assessment in &mut assessments {
            if sent.contains(&assessment["id"]) && assessment.get("sentAt").is_none() {
                assessment["sentAt"] = json!(now());
            }
        }
        if !assessments.is_empty() {
            review["assessments"] = json!(assessments);
        }
        if ack["note"] == review["note"] {
            review["note"] = json!("");
        }
        Ok(())
    })?;
    super::guidance::complete(state, "review.pending-comments");
    Ok(())
}
/// Cancel the run started by `request` if it is still active.
pub(super) fn cancel(state: &mut Value, id: &str, request: &str) -> Result<()> {
    commit(state, id, |review| {
        if review["run"]["id"] == request && active(review) {
            review["run"]["state"] = json!("cancelled");
            review["run"]["updatedAt"] = json!(now());
        }
        Ok(())
    })?;
    Ok(())
}
/// Archive the finished draft at `revision` and start an empty one.
fn start_new(state: &mut Value, id: &str, revision: u64) -> Result<Value> {
    let chat = sessions::get_mut(state, id)?;
    let review = draft(chat);
    if review["revision"] != revision {
        return Err(Error::new(
            "The review changed. Reload it before starting a new review.",
        ));
    }
    if active(&review) {
        return Err(Error::new(
            "Finish or cancel the active review before starting a new review.",
        ));
    }
    let history = &chat["reviewHistory"];
    if !history.is_null() && history["version"] != 1 {
        return Err(Error::new("Unsupported review history version."));
    }
    let mut entries = arr(&history["entries"]);
    if !arr(&review["comments"]).is_empty()
        || !text(&review, "note").is_empty()
        || !arr(&review["assessments"]).is_empty()
        || review["run"].is_object()
    {
        entries.insert(0, json!({"id":uuid(),"archivedAt":now(),"review":review}));
    }
    entries.truncate(20);
    while serde_json::to_vec(&entries)?.len() > 1024 * 1024 {
        entries.pop();
    }
    chat["reviewHistory"] = json!({"version":1,"entries":entries});
    commit(state, id, |review| {
        *review = json!({"comments":[],"note":"","revision":review["revision"]});
        Ok(())
    })
}
fn chat(repo: &Repository, id: &str) -> Result<Value> {
    let state = repo.state.lock().map_err(|_| Error::io())?;
    Ok(sessions::get(&state, id)?.clone())
}
/// Serve the browser's review drawer.
pub(super) fn action(repo: &Repository, versions: &impl Versions, p: &Value) -> Result<Value> {
    let id = text(p, "id");
    let review = match text(p, "action") {
        "history" => {
            let history = chat(repo, id)?["reviewHistory"].clone();
            if !history.is_null() && history["version"] != 1 {
                return Err(Error::new("Unsupported review history version."));
            }
            return Ok(json!({"id":id,"reviews":arr(&history["entries"]),"limit":20}));
        }
        "read" => draft(&chat(repo, id)?),
        "check" => check(repo, versions, id)?,
        "append" => append(repo, versions, p)?,
        "status" => status(repo, versions, p)?,
        "remove" => repo.transaction(|state| {
            remove(
                state,
                id,
                &arr(&p["commentIds"]),
                p["assessmentIds"].as_array().map(Vec::as_slice),
            )
        })?,
        "note" => {
            let note = p["note"]
                .as_str()
                .ok_or_else(|| Error::new("Expected review note."))?;
            repo.transaction(|state| edit_note(state, id, text(p, "noteToken"), note))?
        }
        "new" => {
            let revision = p["revision"].as_u64().ok_or_else(|| {
                Error::new("The review changed. Reload it before starting a new review.")
            })?;
            repo.transaction(|state| start_new(state, id, revision))?
        }
        _ => return Err(Error::new("Unknown review action.")),
    };
    Ok(json!({"id":id,"review":review}))
}

#[cfg(test)]
mod tests;
