//! The requests the agent CLI posts to the local service.
//!
//! Older CLIs stay installed on disk and keep posting the same JSON, so field names and operation
//! strings must not change. Fields a request does not declare are ignored.
use super::agents::Agent;
use super::operations::{self, Caller};
use super::{
    arr, checkouts, commands, json, resources, reviews, session_ci, session_context, sessions,
    workflows, Error, Repository, Result, Value,
};
use super::{now, sid, text};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One agent CLI call. Registration predates operations and carries no `operation` field.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Request {
    Register(Registration),
    Operation(Operation),
}
impl Request {
    pub fn parse(payload: &Value) -> Result<Self> {
        let request = if payload.get("operation").is_none() {
            Registration::deserialize(payload).map(Self::Register)
        } else {
            Operation::deserialize(payload).map(Self::Operation)
        };
        request.map_err(|error| Error::new(error.to_string()))
    }
    pub fn to_json(&self) -> Result<Value> {
        Ok(serde_json::to_value(self)?)
    }
}

/// Link the agent's own conversation to a chat, creating the chat when none matches.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    pub agent: Agent,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub item_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum Operation {
    Command(Command),
    SettingsRead(SettingsRead),
    SettingsApply(SettingsApply),
    CiInvestigation(CiInvestigation),
    Review(ReviewUpdate),
    ContextRead(ContextRead),
    ContextReplace(ContextReplace),
    Resources(ResourceReport),
    Worktree(WorktreeReport),
    RuntimeEvent(RuntimeEvent),
    Runtime(StatusReport),
    Checkpoint(Checkpoint),
    SaveBriefing(Briefing),
    SaveArtifact(ArtifactAttachment),
}

/// Run an operation-table command. `id` is the chat that shows it.
#[derive(Debug, Serialize, Deserialize)]
pub struct Command {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default = "empty_object")]
    pub args: Value,
}
fn empty_object() -> Value {
    json!({})
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsRead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsApply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub patch: Value,
    pub revision: u64,
    pub content_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CiInvestigation {
    pub id: String,
    pub item_id: String,
    pub request_id: String,
    pub state: String,
}

/// Add review feedback, move a review run on, or with neither, read the draft.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUpdate {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_revision: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextRead {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextReplace {
    pub id: String,
    pub context: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResourceReport {
    pub id: String,
    pub report: Value,
}

/// Link a further checkout of the chat to some of its work items.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeReport {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub item_ids: Vec<String>,
    #[serde(default)]
    pub primary: bool,
}

/// A lifecycle hook of the agent's own conversation.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    pub id: String,
    pub event: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub subagent_id: String,
    #[serde(flatten)]
    pub background: Background,
}

/// The background work a turn-end hook saw. `None` means unknown, not zero.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Background {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_counts: Option<BTreeMap<String, u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_wakeups: Option<u64>,
}

/// The agent's own report of its state, for agents without hooks.
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusReport {
    pub id: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagents: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Handoff notes. The owner validates the values, which are copied from the agent's file.
#[derive(Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Briefing {
    pub id: String,
    pub body: String,
}

/// Attach a local file or an HTTPS URL to the chat.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactAttachment {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skill_output: bool,
}

/// Run one CLI request. Returns the record or result its reply is built from.
pub fn receive(repo: &Arc<Repository>, payload: &Value) -> Result<Value> {
    match Request::parse(payload)? {
        Request::Register(registration) => workflows::register(repo, &registration),
        Request::Operation(operation) => run(repo, &operation),
    }
}
fn run(repo: &Arc<Repository>, operation: &Operation) -> Result<Value> {
    match operation {
        Operation::Command(command) => {
            let request = json!({"name":command.name,"args":command.args});
            let result = commands::invoke(repo, &request, agent(command.id.as_deref()))?;
            Ok(json!({ "commandResult": result }))
        }
        Operation::SettingsRead(read) => {
            let caller = agent(read.id.as_deref());
            let settings = operations::run(repo, caller, "settings.read", &json!({}))?;
            Ok(json!({ "settings": settings }))
        }
        Operation::SettingsApply(apply) => {
            let args = json!({"revision":apply.revision,"contentHash":apply.content_hash,"patch":apply.patch});
            let caller = agent(apply.id.as_deref());
            let settings = operations::run(repo, caller, "settings.apply", &args)?;
            Ok(json!({ "settings": settings }))
        }
        Operation::CiInvestigation(update) => {
            session_ci::investigation(repo, &serde_json::to_value(update)?)
        }
        Operation::Review(update) => reviews::agent_update(repo, &serde_json::to_value(update)?),
        Operation::ContextRead(read) => session_context::read(repo, &read.id),
        Operation::ContextReplace(replace) => {
            session_context::replace(repo, &replace.id, &replace.context)
        }
        Operation::Resources(report) => resources::apply(repo, &report.id, &report.report),
        Operation::Worktree(report) => checkouts::link_reported(repo, report),
        Operation::RuntimeEvent(event) => repo.transaction(|s| apply_runtime_event(s, event)),
        Operation::Runtime(report) => register_runtime(repo, report),
        Operation::Checkpoint(checkpoint) => {
            save_output(repo, &checkpoint.id, "checkpoint", checkpoint)
        }
        Operation::SaveBriefing(briefing) => {
            save_output(repo, &briefing.id, "save-briefing", briefing)
        }
        Operation::SaveArtifact(attachment) => {
            let mut record = sessions::get(&repo.state.lock().unwrap(), &attachment.id)?.clone();
            let receipt = super::artifacts::attach_to_chat(
                repo,
                &attachment.id,
                &serde_json::to_value(attachment)?,
            )?;
            record["artifactReceipt"] = receipt;
            Ok(record)
        }
    }
}
fn agent(chat: Option<&str>) -> Caller<'_> {
    Caller::Agent {
        chat: chat.unwrap_or(""),
    }
}
/// Save a checkpoint or briefing through the chat edit `action`. Returns the chat before the edit.
fn save_output(
    repo: &Repository,
    id: &str,
    action: &str,
    output: &impl Serialize,
) -> Result<Value> {
    let record = sessions::get(&repo.state.lock().unwrap(), id)?.clone();
    let mut edit = serde_json::to_value(output)?;
    edit["action"] = json!(action);
    repo.edit(&edit)?;
    Ok(record)
}
fn apply_background_activity(
    runtime: &mut Value,
    old: &Value,
    event: &RuntimeEvent,
    has_subagents: bool,
) -> Result<()> {
    // Turn-end hooks carry an authoritative snapshot of background work.
    // Omitted snapshots mean unknown, not zero; unrelated events retain it.
    if event.event != "SessionStart" {
        for field in ["backgroundCounts", "scheduledWakeups", "activityAt"] {
            if !old[field].is_null() {
                runtime[field] = old[field].clone();
            }
        }
    }
    if let Some(counts) = &event.background.background_counts {
        if counts.len() > 100
            || counts
                .iter()
                .any(|(kind, count)| kind.is_empty() || kind.chars().count() > 40 || *count > 1000)
        {
            return Err(Error::new("Invalid background activity counts."));
        }
        runtime["backgroundCounts"] = json!(counts);
        runtime["activityAt"] = json!(now());
    }
    if let Some(count) = event.background.scheduled_wakeups {
        if count > 1000 {
            return Err(Error::new("Invalid scheduled wakeup count."));
        }
        runtime["scheduledWakeups"] = json!(count);
        runtime["activityAt"] = json!(now());
    }
    let background = runtime["backgroundCounts"]
        .as_object()
        .map_or(0, |v| v.values().filter_map(Value::as_u64).sum::<u64>());
    if runtime["state"] == "waiting"
        && (background > 0
            || has_subagents
            || runtime["scheduledWakeups"].as_u64().unwrap_or(0) > 0)
    {
        runtime["state"] = json!("background");
    } else if runtime["state"] == "background"
        && background == 0
        && !has_subagents
        && runtime["scheduledWakeups"].as_u64().unwrap_or(0) == 0
    {
        runtime["state"] = json!("waiting");
    }
    Ok(())
}
fn complete_skill_runs(s: &mut Value, session_id: &Value) {
    for run in s["skillRuns"].as_array_mut().unwrap() {
        if run["sessionId"] == *session_id
            && ["starting", "launched"].contains(&text(run, "outcome"))
        {
            run["outcome"] = json!("completed");
            run["finishedAt"] = json!(now());
            run["message"] = json!("Agent finished its turn. Registered results appear below.");
        }
    }
}
fn apply_runtime_event(s: &mut Value, event: &RuntimeEvent) -> Result<Value> {
    let row = sessions::get_mut(s, &event.id)?;
    if event.event == "SessionStart" && !event.session_id.is_empty() {
        let id = sid(&event.session_id)?;
        if text(row, "sessionId").is_empty() {
            row["sessionId"] = json!(id);
        }
    }
    let old = row["runtime"].clone();
    let mut ids = arr(&old["subagentIds"]);
    if event.event == "SessionStart" {
        ids.clear();
    }
    if event.subagent_id.chars().count() > 200 || event.subagent_id.contains('\0') {
        return Err(Error::new("Invalid subagentId."));
    }
    let sub = event.subagent_id.trim();
    let state = match event.event.as_str() {
        "SessionStart" | "Stop" => "waiting",
        "UserPromptSubmit" => "working",
        "PermissionRequest" => "needs-input",
        "SubagentStart" => {
            if !sub.is_empty() && !ids.contains(&json!(sub)) && ids.len() < 1000 {
                ids.push(json!(sub));
            }
            old["state"].as_str().unwrap_or("working")
        }
        "SubagentStop" => {
            ids.retain(|x| x != &json!(sub));
            old["state"].as_str().unwrap_or("working")
        }
        _ => return Err(Error::new("Unsupported runtime event.")),
    };
    let reason = match event.event.as_str() {
        "PermissionRequest" => "approval",
        "Stop" => "finished",
        "SessionStart" => "ready",
        "UserPromptSubmit" => "",
        _ => text(&old, "reason"),
    };
    let mut runtime =
        json!({"reason":reason,"state":state,"subagents":ids.len(),"subagentIds":ids,"at":now()});
    apply_background_activity(&mut runtime, &old, event, !ids.is_empty())?;
    row["runtime"] = runtime;
    let record = row.clone();
    if ["Stop", "SubagentStop"].contains(&event.event.as_str())
        && record["runtime"]["state"] == "waiting"
        && record["runtime"]["subagents"] == 0
    {
        complete_skill_runs(s, &record["id"]);
    }
    Ok(record)
}
fn register_runtime(repo: &Repository, report: &StatusReport) -> Result<Value> {
    let state = report.state.trim();
    if !["working", "waiting", "needs-input"].contains(&state) {
        return Err(Error::new("Choose working, waiting, or needs-input."));
    }
    if report.subagents.is_some_and(|count| count > 1000) {
        return Err(Error::new("Subagent count must be 0–1000."));
    }
    let reason = report.reason.as_deref().unwrap_or("").trim();
    if !["", "ready", "finished", "question", "approval", "failed"].contains(&reason) {
        return Err(Error::new("Invalid attention reason."));
    }
    repo.transaction(|s| {
        let row = sessions::get_mut(s, &report.id)?;
        row["runtime"] =
            json!({"state":state,"reason":reason,"subagents":report.subagents,"at":now()});
        Ok(row.clone())
    })
}
/// The reply the CLI reads for the record or result a request produced.
pub fn reply(record: &Value) -> Value {
    if !record["commandResult"].is_null() {
        return json!({"commandResult":record["commandResult"]});
    }
    if record["settings"].is_object() {
        return json!({"settings":record["settings"]});
    }
    let mut reply = json!({"id":record["id"],"linkedItems":arr(&record["items"]).len()});
    if record["artifactReceipt"].is_object() {
        reply["artifactReceipt"] = record["artifactReceipt"].clone();
    }
    if record["review"].is_object() {
        reply["review"] = record["review"].clone();
    }
    if record["context"].is_object() {
        reply["context"] = record["context"].clone();
    }
    reply
}

#[cfg(test)]
mod command_tests;
