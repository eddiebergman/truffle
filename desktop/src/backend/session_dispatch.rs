//! Conservative tracked submission over the existing, generation-unfenced PTY transport.
use super::{arr, checked, domain, json, now, sid, text, uuid, Error, Repository, Result, Value};
const JOURNAL: &str = "sessionDispatch";
const LEGACY_MAX_ENTRIES: usize = 1024;
const MAX_UNRESOLVED: usize = 128;
const RECENT_COMPLETED: usize = 128;
const MAX_BYTES: usize = 5 * 1024 * 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    ReviewRequest,
    ReviewFeedback,
    CiInvestigation,
}
impl Kind {
    const fn name(self) -> &'static str {
        match self {
            Self::ReviewRequest => "review-request",
            Self::ReviewFeedback => "review-feedback",
            Self::CiInvestigation => "ci-investigation",
        }
    }
}
fn entries(state: &Value) -> Result<Vec<Value>> {
    let journal = &state[JOURNAL];
    if journal.is_null() {
        return Ok(vec![]);
    }
    if journal["version"] != 1 && journal["version"] != 2 {
        return Err(Error::new(
            "Unsupported dispatch journal version. Existing terminals were kept.",
        ));
    }
    let limit = if journal["version"] == 1 {
        LEGACY_MAX_ENTRIES
    } else {
        LEGACY_MAX_ENTRIES + RECENT_COMPLETED
    };
    let byte_limit = if journal["version"] == 1 {
        4 * 1024 * 1024
    } else {
        MAX_BYTES
    };
    let entries = journal["entries"]
        .as_array()
        .filter(|entries| entries.len() <= limit)
        .ok_or_else(|| Error::new("Invalid dispatch journal."))?;
    if serde_json::to_vec(journal)?.len() > byte_limit {
        return Err(Error::new(
            "Dispatch journal exceeds its safe capacity. Existing terminals were kept.",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let mut tickets = std::collections::HashSet::new();
    let next = if journal["version"] == 2 {
        let next = decimal(&journal["nextTicket"])?;
        let retired = decimal(&journal["retiredThrough"])?;
        if next == 0 || retired >= next {
            return Err(Error::new("Invalid dispatch ticket metadata."));
        }
        Some(next)
    } else {
        None
    };
    for entry in entries {
        if !ids.insert(text(entry, "operationId")) {
            return Err(Error::new("Duplicate dispatch operation identity."));
        }
        if let Some(next) = next {
            let ticket = decimal(&entry["ticket"])?;
            if ticket == 0 || ticket >= next || !tickets.insert(ticket) {
                return Err(Error::new("Invalid dispatch ticket metadata."));
            }
        }
        if !["review-request", "review-feedback", "ci-investigation"].contains(&text(entry, "kind"))
            || !["prepared", "sending", "written", "uncertain"].contains(&text(entry, "status"))
            || !entry["finalized"].is_boolean()
        {
            return Err(Error::new("Invalid dispatch receipt metadata."));
        }
        for key in ["operationId", "recordId", "generation"] {
            sid(text(entry, key))?;
        }
        let hash = text(entry, "payloadHash");
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::new("Invalid dispatch payload identity."));
        }
        if entry
            .get("resolved")
            .is_some_and(|value| !value.is_boolean())
            || entry.get("resolution").is_some_and(|value| {
                !value.as_str().is_some_and(|value| {
                    ["user-confirmed-submitted", "user-abandoned"].contains(&value)
                })
            })
        {
            return Err(Error::new("Invalid dispatch resolution metadata."));
        }
        if entry["kind"] == "ci-investigation"
            && (text(entry, "itemId").is_empty() || text(entry, "itemId").len() > 200)
        {
            return Err(Error::new("Invalid dispatch work identity."));
        }
        if !closed(entry)
            && entry["payload"]
                .as_str()
                .is_none_or(|payload| payload.len() > 65536)
        {
            return Err(Error::new("Invalid frozen dispatch payload."));
        }
    }
    Ok(entries.clone())
}
fn decimal(value: &Value) -> Result<u64> {
    let text = value
        .as_str()
        .ok_or_else(|| Error::new("Invalid dispatch ticket metadata."))?;
    if text.is_empty()
        || text.len() > 20
        || !text.bytes().all(|byte| byte.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return Err(Error::new("Invalid dispatch ticket metadata."));
    }
    text.parse()
        .map_err(|_| Error::new("Invalid dispatch ticket metadata."))
}
fn closed(entry: &Value) -> bool {
    entry["finalized"] == true || entry["resolution"] == "user-abandoned"
}
fn migrate(state: &mut Value) -> Result<()> {
    let mut all = entries(state)?;
    if state[JOURNAL]["version"] == 2 {
        return Ok(());
    }
    if state[JOURNAL].is_null() {
        state[JOURNAL] = json!({});
    }
    if ["nextTicket", "retiredThrough"]
        .iter()
        .any(|key| state[JOURNAL].get(*key).is_some())
    {
        return Err(Error::new(
            "Legacy dispatch ticket extensions cannot be migrated safely. Existing data was kept.",
        ));
    }
    for (index, entry) in all.iter_mut().enumerate() {
        if entry.get("ticket").is_some() {
            return Err(Error::new("Legacy dispatch ticket extension cannot be migrated safely. Existing data was kept."));
        }
        entry["ticket"] = json!((index as u64 + 1).to_string());
    }
    state[JOURNAL]["version"] = json!(2);
    state[JOURNAL]["nextTicket"] = json!((all.len() as u64 + 1).to_string());
    state[JOURNAL]["retiredThrough"] = json!("0");
    state[JOURNAL]["entries"] = json!(all);
    entries(state)?;
    Ok(())
}
fn migrate_owned(repo: &Repository, state: &mut Value) -> Result<()> {
    if state[JOURNAL]["version"] == 1 {
        entries(state)?;
        repo.backup_dispatch_v1(&state[JOURNAL])?;
    }
    migrate(state)
}
fn ensure_migrated(repo: &Repository) -> Result<()> {
    repo.transaction_if_changed(|state| migrate_owned(repo, state))
}
fn store(state: &mut Value, all: Vec<Value>) -> Result<()> {
    store_entries(state, all, true)
}
fn store_entries(state: &mut Value, mut all: Vec<Value>, compact: bool) -> Result<()> {
    // Only receipts with a terminal user decision or completed acknowledgement may retire.
    // A retirement floor is diagnostic: unresolved tickets below it remain fully recoverable.
    let mut completed = all
        .iter()
        .filter(|entry| closed(entry))
        .map(|entry| {
            Ok((
                entry["finalizedAt"]
                    .as_f64()
                    .or_else(|| entry["resolvedAt"].as_f64())
                    .filter(|at| at.is_finite())
                    .unwrap_or(0.0),
                decimal(&entry["ticket"])?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    completed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let remove = completed.len().saturating_sub(RECENT_COMPLETED);
    if compact && remove > 0 {
        let retired_tickets = completed
            .iter()
            .take(remove)
            .map(|row| row.1)
            .collect::<std::collections::HashSet<_>>();
        let floor = retired_tickets.iter().copied().max().unwrap();
        all.retain(|entry| {
            !closed(entry) || !retired_tickets.contains(&decimal(&entry["ticket"]).unwrap())
        });
        let retired = decimal(&state[JOURNAL]["retiredThrough"])?;
        state[JOURNAL]["retiredThrough"] = json!(retired.max(floor).to_string());
    }
    state[JOURNAL]["entries"] = json!(all);
    if serde_json::to_vec(&state[JOURNAL])?.len() > MAX_BYTES {
        return Err(Error::new("Tracked dispatch storage is full of retained recovery data. Resolve an earlier request; no input was sent."));
    }
    entries(state)?;
    Ok(())
}
fn record(state: &Value, id: &str) -> Result<Value> {
    arr(&state["sessions"])
        .into_iter()
        .find(|record| record["id"] == id)
        .ok_or_else(|| Error::new("Session unavailable."))
}
fn public(entry: &Value) -> Value {
    let status = match text(entry, "status") {
        "prepared" => "prepared",
        "written" => "written",
        _ => "uncertain",
    };
    let finalized = entry["finalized"] == true;
    let message = if entry["resolution"] == "user-confirmed-submitted" {
        if finalized {
            "You marked this request as submitted; its local acknowledgement is saved. This does not confirm agent consumption or completion."
        } else {
            "You marked this request as submitted. Save its local acknowledgement without sending again."
        }
    } else if entry["resolution"] == "user-abandoned" {
        "You released this request. Any uncertain delivery remains unknown. A new submission requires a separate action and may duplicate it."
    } else {
        match (status,finalized){("prepared",_)=>"Not submitted. Retry this same operation; no terminal input was sent.",("written",true)=>"Submitted to the terminal. This does not confirm the agent has read it or completed the task.",("written",false)=>"Submitted to the terminal. Save the local acknowledgement without sending again.",_=>"Terminal delivery is uncertain. Do not send again. Check the existing terminal; refreshing delivery status will not resend."}
    };
    let mut result = json!({"type":"operation","operationId":entry["operationId"],"recordId":entry["recordId"],"kind":entry["kind"],"generation":entry["generation"],"payloadHash":entry["payloadHash"],"status":status,"finalized":finalized,"transport":"legacy-unfenced","message":message});
    if entry.get("ticket").is_some() {
        result["ticket"] = entry["ticket"].clone();
    }
    if entry["kind"] == "ci-investigation" {
        result["itemId"] = entry["itemId"].clone();
    }
    if entry.get("resolved").is_some() {
        result["resolved"] = entry["resolved"].clone();
        result["resolution"] = entry["resolution"].clone();
    }
    result
}
#[cfg(test)]
fn matching(
    entries: &[Value],
    id: &str,
    operation: &str,
    kind: Kind,
    item: &str,
) -> Result<Option<Value>> {
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry["operationId"] == operation)
    {
        if entry["recordId"] != id || entry["kind"] != kind.name() || text(entry, "itemId") != item
        {
            return Err(Error::new(
                "This dispatch operation ID belongs to a different feature or target.",
            ));
        }
        return Ok(Some(entry.clone()));
    }
    Ok(entries
        .iter()
        .rev()
        .find(|entry| {
            entry["recordId"] == id
                && entry["kind"] == kind.name()
                && text(entry, "itemId") == item
                && entry["finalized"] != true
                && entry["resolution"] != "user-abandoned"
        })
        .cloned())
}
pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    let id = sid(text(args, "recordId"))?;
    let state = repo.state.lock().map_err(|_| Error::io())?;
    record(&state, &id)?;
    let mut all = entries(&state)?
        .into_iter()
        .rev()
        .filter(|entry| entry["recordId"] == id)
        .collect::<Vec<_>>();
    all.sort_by_key(|entry| entry["finalized"] == true || entry["resolution"] == "user-abandoned");
    let operations = all.iter().take(32).map(public).collect::<Vec<_>>();
    Ok(json!({"operations":operations}))
}
fn locate(repo: &Repository, id: &str, operation: &str) -> Result<Value> {
    let state = repo.state.lock().map_err(|_| Error::io())?;
    entries(&state)?
        .into_iter()
        .find(|entry| entry["recordId"] == id && entry["operationId"] == operation)
        .ok_or_else(|| Error::new("Dispatch receipt not found."))
}
fn mutate(
    repo: &Repository,
    id: &str,
    operation: &str,
    update: impl FnOnce(&mut Value) -> Result<()>,
) -> Result<Value> {
    repo.transaction(|state| {
        migrate_owned(repo, state)?;
        let mut all = entries(state)?;
        let entry = all
            .iter_mut()
            .find(|entry| entry["recordId"] == id && entry["operationId"] == operation)
            .ok_or_else(|| Error::new("Dispatch receipt not found."))?;
        update(entry)?;
        let result = entry.clone();
        store(state, all)?;
        Ok(result)
    })
}
pub fn finalize(repo: &Repository, args: &Value) -> Result<Value> {
    let id = sid(text(args, "recordId"))?;
    let operation = sid(text(args, "operationId"))?;
    finalize_entry(repo, &id, &operation)
}
fn finalize_entry(repo: &Repository, id: &str, operation: &str) -> Result<Value> {
    ensure_migrated(repo)?;
    let entry = locate(repo, id, operation)?;
    if (entry["status"] != "written" && entry["resolution"] != "user-confirmed-submitted")
        || entry["finalized"] == true
    {
        return Ok(public(&entry));
    }
    let result = repo.transaction(|state| {
        let mut all = entries(state)?;
        let index = all
            .iter()
            .position(|entry| entry["recordId"] == id && entry["operationId"] == operation)
            .ok_or_else(|| Error::new("Dispatch receipt not found."))?;
        if (all[index]["status"] != "written"
            && all[index]["resolution"] != "user-confirmed-submitted")
            || all[index]["finalized"] == true
        {
            return Ok(all[index].clone());
        }
        if all[index]["kind"] == "review-feedback" {
            super::reviews::acknowledge_dispatch(state, id, &all[index]["ack"])?;
        }
        all[index]["finalized"] = json!(true);
        all[index]["finalizedAt"] = json!(now());
        all[index].as_object_mut().unwrap().remove("payload");
        all[index].as_object_mut().unwrap().remove("ack");
        let result = all[index].clone();
        store(state, all)?;
        Ok(result)
    });
    Ok(public(&result.unwrap_or(entry)))
}
pub fn resolve(repo: &Repository, args: &Value) -> Result<Value> {
    let id = sid(text(args, "recordId"))?;
    let operation = sid(text(args, "operationId"))?;
    let decision = text(args, "decision");
    if !["submitted", "abandon"].contains(&decision) {
        return Err(Error::new("Choose submitted or abandon."));
    }
    let entry = repo.transaction(|state| {
        migrate_owned(repo, state)?;
        let mut all = entries(state)?;
        let index = all
            .iter()
            .position(|entry| entry["recordId"] == id && entry["operationId"] == operation)
            .ok_or_else(|| Error::new("Dispatch receipt not found."))?;
        if all[index]["finalized"] == true
            || all[index]["resolved"] == true
            || all[index]["status"] == "written"
        {
            return Ok(all[index].clone());
        }
        if decision == "submitted" && all[index]["status"] == "prepared" {
            return Err(Error::new(
                "This request was not submitted. Abandon it or retry its original operation.",
            ));
        }
        all[index]["resolved"] = json!(true);
        all[index]["resolvedAt"] = json!(now());
        all[index]["resolution"] = json!(if decision == "submitted" {
            "user-confirmed-submitted"
        } else {
            "user-abandoned"
        });
        if decision == "abandon" {
            all[index].as_object_mut().unwrap().remove("payload");
            all[index].as_object_mut().unwrap().remove("ack");
            match text(&all[index], "kind") {
                "review-request" => super::reviews::cancel_dispatch(state, &id, &operation)?,
                "ci-investigation" => super::session_ci::cancel_dispatch(
                    state,
                    &id,
                    text(&all[index], "itemId"),
                    &operation,
                )?,
                _ => (),
            }
        }
        let result = all[index].clone();
        store(state, all)?;
        Ok(result)
    })?;
    if entry["resolution"] == "user-confirmed-submitted" {
        finalize_entry(repo, &id, &operation)
    } else {
        Ok(public(&entry))
    }
}
fn safe(value: &str) -> String {
    value
        .chars()
        .filter(|c| !((*c <= '\u{1f}' && *c != '\n' && *c != '\t') || *c == '\u{7f}'))
        .collect()
}
fn review_pending(draft: &Value, entry: &Value) -> bool {
    entry["reviewedAt"].is_null()
        && !(draft["run"]["state"] == "complete"
            && !draft["run"]["inputCommentIds"].is_array()
            && entry["createdAt"].as_f64().is_some_and(|at| {
                at <= draft["run"]["completedAt"]
                    .as_f64()
                    .or_else(|| draft["run"]["updatedAt"].as_f64())
                    .unwrap_or(0.0)
            }))
}
fn feedback(record: &Value, draft: &Value, checked: &Value) -> Result<(String, Value)> {
    let comments = arr(&draft["comments"])
        .into_iter()
        .filter(|entry| review_pending(draft, entry))
        .collect::<Vec<_>>();
    let assessments = arr(&draft["assessments"])
        .into_iter()
        .filter(|entry| entry["sentAt"].is_null() && review_pending(draft, entry))
        .collect::<Vec<_>>();
    let note = if draft["note"] == draft["reviewedNote"] {
        ""
    } else {
        text(draft, "note")
    };
    if comments.is_empty() && assessments.is_empty() && note.trim().is_empty() {
        return Err(Error::new("There is no pending review feedback to send."));
    }
    let mut lines=vec!["Please address this review of your current work. These comments are one batch. Check each against the referenced file version before changing code.".into(),format!("Working directory: {}",safe(text(record,"cwd")))];
    for assessment in &assessments {
        let fresh = arr(&checked["assessments"])
            .into_iter()
            .find(|entry| entry["id"] == assessment["id"])
            .unwrap_or_else(|| assessment.clone());
        if fresh["freshness"].is_string() && fresh["freshness"] != "current" {
            lines.push(format!(
                "Reviewer {} assessment freshness: {}. Recheck its findings against current code.",
                safe(text(assessment, "author")),
                safe(text(&fresh, "freshness"))
            ));
        }
    }
    for comment in &comments {
        let fresh = arr(&checked["comments"])
            .into_iter()
            .find(|entry| entry["id"] == comment["id"])
            .unwrap_or_else(|| comment.clone());
        if fresh["freshness"].is_string() && fresh["freshness"] != "current" {
            lines.push(format!(
                "Comment on {}/{} freshness: {}.",
                safe(text(comment, "root")),
                safe(text(comment, "path")),
                safe(text(&fresh, "freshness"))
            ));
        }
    }
    if !note.trim().is_empty() {
        lines.push(format!("\nOverall review:\n{}", safe(note)));
    }
    for entry in &assessments {
        let scope = if text(entry, "root").is_empty() {
            "Scope: conversation across repositories".into()
        } else {
            format!(
                "Worktree: {} · base: {}",
                safe(text(entry, "root")),
                safe(text(entry, "baseCommit"))
            )
        };
        lines.push(format!(
            "\nReviewer assessment · {} · {}\n{}\n{}",
            safe(text(entry, "author")),
            safe(text(entry, "pass")),
            scope,
            safe(text(entry, "text"))
        ));
    }
    for (n, entry) in comments.iter().enumerate() {
        lines.push(format!("\n{}. {} · {} lines {}–{}\nWorktree: {}\nBase: {} · diff fingerprint: {}\nCode excerpt:\n{}\nAuthor: {}\nComment: {}",n+1,safe(text(entry,"path")),if entry["side"]=="deletions"{"old"}else{"new"},entry["start"],entry["end"],safe(entry["root"].as_str().unwrap_or_else(|| text(record,"cwd"))),safe(text(entry,"baseCommit")),safe(text(entry,"fingerprint")),safe(text(entry,"excerpt")),safe(entry["author"].as_str().unwrap_or("You")),safe(text(entry,"text"))));
    }
    let ack = json!({"commentIds":comments.iter().map(|entry|entry["id"].clone()).collect::<Vec<_>>(),"assessmentIds":assessments.iter().map(|entry|entry["id"].clone()).collect::<Vec<_>>(),"comments":comments,"assessments":assessments,"note":note,"historyId":record["reviewHistory"]["entries"][0]["id"]});
    Ok((lines.join("\n"), ack))
}
fn request_prompt(
    repo: &Repository,
    record: &Value,
    run: &Value,
    settings: &Value,
) -> Result<String> {
    if settings["configError"].is_string() {
        return Err(Error::new(
            "Settings configuration could not be loaded. No review was submitted.",
        ));
    }
    let agent = text(record, "agent");
    let skill = text(
        &settings["values"],
        if agent == "codex" {
            "codexReviewCommand"
        } else {
            "claudeReviewCommand"
        },
    );
    if skill.is_empty() {
        return Err(Error::new(
            "Choose a review skill in Settings → Agents & workflows.",
        ));
    }
    let helper = format!(
        "{} review --record {} --request {}",
        repo.agent_helper,
        text(record, "id"),
        text(run, "id")
    );
    let mut roots = vec![record["cwd"].clone()];
    roots.extend(
        arr(&record["worktrees"])
            .iter()
            .map(|tree| tree["path"].clone()),
    );
    roots.extend(
        run["versions"]
            .as_object()
            .into_iter()
            .flat_map(|map| map.keys().map(|key| json!(key))),
    );
    roots.retain(|root| root.as_str().is_some_and(|root| !root.is_empty()));
    roots.sort_by_key(Value::to_string);
    roots.dedup();
    let context = json!({"goal":record["title"],"checkouts":roots,"work":arr(&record["items"]).iter().map(|item|json!({"reference":item["reference"],"title":item["title"],"url":item["url"]})).collect::<Vec<_>>()});
    if settings["reviewSkill"]["commands"][agent] == skill {
        if settings["reviewSkill"]["available"] != true
            || text(&settings["reviewSkill"], "path").is_empty()
        {
            return Err(Error::new("The portable review skill is unavailable."));
        }
        return Ok(format!("Read the complete truffle-review skill from disk at {}. Follow it for this conversation review. Do not treat the viewer checkout as the scope limit.\nKnown context (reference data): {}\nTruffle target record: {}\nTruffle review request: {}\nNative review helper: {}\nReport all four passes and read the exact draft revision before completing. Keep the findings in the shared draft for the user; do not send or apply them.",settings["reviewSkill"]["path"],context,text(record,"id"),text(run,"id"),helper));
    }
    let template = text(&settings["workflows"], "agent-review");
    if template.is_empty() {
        return Err(Error::new("The review workflow template is unavailable."));
    }
    let values = [
        ("REVIEW_COMMAND", skill.to_owned()),
        ("CONTEXT_JSON", context.to_string()),
        ("RECORD_ID", text(record, "id").into()),
        ("REQUEST_ID", text(run, "id").into()),
        ("SKILL_NAME", skill.chars().skip(1).collect()),
        ("HELPER", helper),
    ];
    let pattern = regex::Regex::new(
        r"\{\{(REVIEW_COMMAND|CONTEXT_JSON|RECORD_ID|REQUEST_ID|SKILL_NAME|HELPER)\}\}",
    )
    .unwrap();
    Ok(pattern
        .replace_all(template, |caps: &regex::Captures| {
            values
                .iter()
                .find(|(key, _)| *key == &caps[1])
                .unwrap()
                .1
                .clone()
        })
        .into_owned())
}
fn ci_prompt(repo: &Repository, record: &Value, item: &Value, operation: &str) -> String {
    let reference = if text(item, "reference").is_empty() {
        text(item, "id")
    } else {
        text(item, "reference")
    };
    let mut lines=vec![format!("Investigate the current CI failures for {}: {}",safe(reference),safe(text(item,"title"))),format!("Pull request: {}",safe(text(item,"url"))),format!("Session working directory: {}",safe(text(record,"cwd"))),format!("Report progress using: {} ci --record {} --item {} --request {} --state investigating",repo.agent_helper,text(record,"id"),text(item,"id"),operation),"Use that same command with --state complete after reporting your diagnosis, or --state failed if blocked.".into(),format!("Head branch: {}",safe(text(item,"headRefName"))),format!("Cached check summary: {}",safe(text(&item["ci"],"state")))];
    for check in arr(&item["ci"]["checks"])
        .iter()
        .filter(|check| check["state"] == "failed")
    {
        lines.push(format!(
            "- {} · {} · {}",
            safe(text(check, "name")),
            safe(text(check, "result")),
            safe(text(check, "url"))
        ));
    }
    lines.push("Refresh the PR checks first; this cached snapshot may be stale or incomplete. Read the failing job logs, distinguish failures caused by this PR from infrastructure failures, and report the cause and a proposed fix. Investigate only; do not change code, rerun workflows, push, or post externally.".into());
    lines.join("\n")
}
fn scope(all: &[Value], id: &str, kind: Kind, item: &str) -> Option<Value> {
    all.iter()
        .rev()
        .find(|entry| {
            entry["recordId"] == id
                && entry["kind"] == kind.name()
                && text(entry, "itemId") == item
                && !closed(entry)
        })
        .cloned()
}
pub fn prepare(repo: &Repository, kind: Kind, args: &Value) -> Result<Value> {
    prepare_with(repo, kind, args, || Ok(repo.embedded.inventory()), uuid)
}
fn prepare_with(
    repo: &Repository,
    kind: Kind,
    args: &Value,
    inventory: impl Fn() -> Result<Vec<Value>>,
    allocate: impl Fn() -> String,
) -> Result<Value> {
    let id = sid(text(args, "recordId"))?;
    let item = if kind == Kind::CiInvestigation {
        checked(args, "itemId", 200, true)?
    } else {
        String::new()
    };
    let expected_revision = if kind == Kind::CiInvestigation {
        None
    } else {
        Some(
            args["revision"]
                .as_u64()
                .ok_or_else(|| Error::new("Supply the captured review revision."))?,
        )
    };
    let previous = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        record(&state, &id)?;
        scope(&entries(&state)?, &id, kind, &item)
    };
    if let Some(entry) = previous {
        ensure_migrated(repo)?;
        return Ok(public(&locate(repo, &id, text(&entry, "operationId"))?));
    }
    let generation = inventory()?
        .into_iter()
        .find(|entry| entry["recordId"] == id && entry["live"] == true)
        .and_then(|entry| entry["generation"].as_str().map(str::to_owned))
        .ok_or_else(|| Error::new("Resume this chat in Truffle before submitting this request."))?;
    sid(&generation)?;
    let settings = if kind == Kind::ReviewRequest {
        repo.settings.view()
    } else {
        Value::Null
    };
    let baseline = if kind == Kind::ReviewRequest {
        super::reviews::versions(repo, &id)?
    } else {
        Value::Null
    };
    let checked = if kind == Kind::ReviewFeedback {
        super::reviews::action(repo, &json!({"id":id,"action":"check"}))?["review"].clone()
    } else {
        Value::Null
    };
    let entry=repo.transaction(|state|{
            let migrated=state[JOURNAL]["version"]!=2;migrate_owned(repo,state)?;let mut all=entries(state)?;if let Some(entry)=scope(&all,&id,kind,&item){return Ok(entry);}
            if all.iter().filter(|entry|!closed(entry)).count()>=MAX_UNRESOLVED{return Err(Error::new("Too many unresolved dispatch requests. Resolve an earlier request; no input was sent."));}
            let ticket=decimal(&state[JOURNAL]["nextTicket"])?;let next=ticket.checked_add(1).ok_or_else(|| Error::new("Dispatch ticket capacity is exhausted. Existing requests remain recoverable."))?;let operation=allocate();sid(&operation)?;if all.iter().any(|entry|entry["operationId"]==operation){return Err(Error::new("Dispatch identity allocation collided. No input was sent."));}
            let record=record(state,&id)?;let draft=super::reviews::draft(&record);
            if kind!=Kind::CiInvestigation&&expected_revision!=draft["revision"].as_u64(){return Err(Error::new("The review changed. Reload it before submitting."));}
            let (prompt,ack)=match kind{
                Kind::ReviewRequest=>{let review=super::reviews::reserve_request(state,&id,&operation,expected_revision.unwrap(),&baseline)?;(request_prompt(repo,&record,&review["run"],&settings)?,Value::Null)},
                Kind::ReviewFeedback=>feedback(&record,&draft,&checked)?,
                Kind::CiInvestigation=>{let work=domain::catalog(state).remove(&item).filter(|item|item["source"]=="github"&&item["kind"]=="Pull request").ok_or_else(|| Error::new("Choose a linked GitHub pull request."))?;super::session_ci::investigation_in_state(state,&json!({"id":id,"itemId":item,"requestId":operation,"state":"queued"}))?;(ci_prompt(repo,&record,&work,&operation),Value::Null)},
            };
            let payload=format!("\u{1b}[200~{prompt}\u{1b}[201~\r");if payload.len()>65536{return Err(Error::new("This request exceeds 64 KiB. Shorten it before submitting."));}
            let entry=json!({"operationId":operation,"ticket":ticket.to_string(),"recordId":id,"kind":kind.name(),"itemId":item,"generation":generation,"payloadHash":context_kit::sha256(payload.as_bytes()),"payload":payload,"ack":ack,"sourceRevision":expected_revision,"status":"prepared","finalized":false,"createdAt":now()});all.push(entry.clone());state[JOURNAL]["nextTicket"]=json!(next.to_string());store_entries(state,all,!migrated)?;Ok(entry)
        })?;
    Ok(public(&entry))
}
fn expired(id: &str, operation: &str) -> Value {
    json!({"type":"unavailable","operationId":operation,"recordId":id,"reason":"possibly-expired","transport":"legacy-unfenced","message":"This operation is unavailable or its receipt expired. Prior delivery may have occurred. No input was sent by this submission. Inspect the chat before preparing any separate new request."})
}
pub fn submit(repo: &Repository, args: &Value) -> Result<Value> {
    submit_with(
        repo,
        args,
        || Ok(repo.embedded.inventory()),
        |id, data| repo.embedded.input(id, data),
        || Ok(()),
    )
}
fn submit_with(
    repo: &Repository,
    args: &Value,
    inventory: impl Fn() -> Result<Vec<Value>>,
    input: impl Fn(&str, &str) -> Result<()>,
    after_written: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    let id = sid(text(args, "recordId"))?;
    let operation = sid(text(args, "operationId"))?;
    ensure_migrated(repo)?;
    let prepared = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        record(&state, &id)?;
        let all = entries(&state)?;
        if let Some(entry) = all.iter().find(|entry| entry["operationId"] == operation) {
            if entry["recordId"] != id {
                return Err(Error::new(
                    "This dispatch operation belongs to another chat.",
                ));
            }
            entry.clone()
        } else {
            return Ok(expired(&id, &operation));
        }
    };
    if prepared["resolution"] == "user-abandoned" {
        return Ok(public(&prepared));
    }
    if prepared["status"] == "written" || prepared["resolution"] == "user-confirmed-submitted" {
        return finalize_entry(repo, &id, &operation);
    }
    if prepared["status"] != "prepared" {
        return Ok(public(&prepared));
    }
    let payload = text(&prepared, "payload");
    if context_kit::sha256(payload.as_bytes()) != text(&prepared, "payloadHash") {
        return Err(Error::new(
            "The frozen dispatch payload failed its integrity check. No input was sent.",
        ));
    }
    // This is only a best-effort preflight; the legacy daemon does not fence the write by generation.
    if !inventory()?.iter().any(|entry| {
        entry["recordId"] == id
            && entry["live"] == true
            && entry["generation"] == prepared["generation"]
    }) {
        return Err(Error::new(
            "The terminal generation changed. The prepared request was not sent.",
        ));
    }
    let sending = mutate(repo, &id, &operation, |entry| {
        if entry["status"] != "prepared" || entry["resolved"] == true {
            return Err(Error::new("Dispatch is already reserved."));
        }
        entry["status"] = json!("sending");
        Ok(())
    });
    if sending.is_err() {
        return Ok(public(&locate(repo, &id, &operation).unwrap_or(prepared)));
    }
    if input(&id, payload).is_err() {
        let entry = mutate(repo, &id, &operation, |entry| {
            entry["status"] = json!("uncertain");
            Ok(())
        })
        .unwrap_or_else(|_| sending.unwrap());
        return Ok(public(&entry));
    }
    let written = match mutate(repo, &id, &operation, |entry| {
        if entry["resolved"] == true || entry["status"] != "sending" {
            return Err(Error::new("Dispatch resolution changed."));
        }
        entry["status"] = json!("written");
        entry["writtenAt"] = json!(now());
        Ok(())
    }) {
        Ok(entry) => entry,
        Err(_) => {
            return Ok(public(
                &locate(repo, &id, &operation).unwrap_or_else(|_| sending.unwrap()),
            ))
        }
    };
    if after_written().is_err() {
        return Ok(public(&written));
    }
    finalize_entry(repo, &id, &operation)
}

#[cfg(test)]
fn dispatch_with(
    repo: &Repository,
    kind: Kind,
    args: &Value,
    inventory: impl Fn() -> Result<Vec<Value>>,
    input: impl Fn(&str, &str) -> Result<()>,
    after_written: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    let previous = {
        let state = repo.state.lock().unwrap();
        matching(
            &entries(&state)?,
            text(args, "recordId"),
            text(args, "operationId"),
            kind,
            text(args, "itemId"),
        )?
    };
    let receipt = if let Some(entry) = previous {
        public(&entry)
    } else {
        prepare_with(repo, kind, args, &inventory, || {
            text(args, "operationId").to_owned()
        })?
    };
    submit_with(
        repo,
        &json!({"recordId":args["recordId"],"operationId":receipt["operationId"]}),
        inventory,
        input,
        after_written,
    )
}

#[cfg(test)]
mod tests;
