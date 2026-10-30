//! The requests the agent CLI posts to the local service.
//!
//! Older CLIs stay installed on disk and keep posting the same JSON, so field names and operation
//! strings must not change. Fields a request does not declare are ignored.
use super::agents::Agent;
use super::operations::{self, Caller};
use super::runtime::{self, Background, Hook};
use super::{
    arr, checkouts, commands, json, resources, reviews, scheduler, session_ci, session_context,
    sessions, workflows, Error, Repository, Result, Value,
};
use super::{now, sid, text};
use serde::{Deserialize, Serialize};
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
    pub event: Hook,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub subagent_id: String,
    #[serde(flatten)]
    pub background: Background,
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
/// Only `command` and the settings operations go through the operation table, which logs agent
/// writes in the chat. The others report the chat's own status and outputs, which the chat already
/// shows, and some fire on every turn.
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
        Operation::RuntimeEvent(event) => repo.transaction(|s| apply_hook(s, event)),
        Operation::Runtime(report) => apply_report(repo, report),
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
/// Apply a hook to the chat's runtime. An idle agent at a turn end has finished its skill runs.
fn apply_hook(s: &mut Value, event: &RuntimeEvent) -> Result<Value> {
    let row = sessions::get_mut(s, &event.id)?;
    if event.event == Hook::SessionStart && !event.session_id.is_empty() {
        let id = sid(&event.session_id)?;
        if text(row, "sessionId").is_empty() {
            row["sessionId"] = json!(id);
        }
    }
    let hook = runtime::Event::Hook {
        hook: event.event,
        subagent: &event.subagent_id,
        background: &event.background,
    };
    row["runtime"] = runtime::next(&row["runtime"], &hook, now())?;
    let record = row.clone();
    if event.event.ends_turn() && runtime::is_idle(&record["runtime"]) {
        scheduler::finish_runs(s, &event.id);
    }
    Ok(record)
}
fn apply_report(repo: &Repository, report: &StatusReport) -> Result<Value> {
    let event = runtime::Event::Report {
        state: &report.state,
        reason: report.reason.as_deref().unwrap_or(""),
        subagents: report.subagents,
    };
    repo.transaction(|s| {
        let row = sessions::get_mut(s, &report.id)?;
        row["runtime"] = runtime::next(&row["runtime"], &event, now())?;
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
