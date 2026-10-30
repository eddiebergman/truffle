//! Caller policy for the operation table. Every entry point names its caller explicitly.
use super::settings::Approval;
use super::{arr, json, now, redaction, sessions, sid, uuid, Error, Repository, Result, Value};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller<'a> {
    /// The user, in the browser or the desktop app.
    Browser,
    /// The built-in App chat assistant.
    Assistant,
    /// An agent CLI call. `chat` is the saved chat it reports from, unchecked until it is needed.
    Agent { chat: &'a str },
}
impl Caller<'_> {
    /// Only the user approves new or changed executable command text.
    pub const fn settings_approval(self) -> Approval {
        match self {
            Caller::Browser => Approval::User,
            Caller::Assistant | Caller::Agent { .. } => Approval::Routine,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Anyone,
    /// App chat cannot run it. Agents can, and their chat records it.
    UserAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Redacted,
    /// Exact for the browser, which edits the result and saves it back; redacted for others.
    Editable,
}

pub type Handler = fn(&Arc<Repository>, &Value, Caller<'_>) -> Result<Value>;

pub struct Operation {
    pub name: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    pub access: Access,
    pub output: Output,
    pub args: Value,
    pub run: Handler,
}
impl Operation {
    /// Whether an agent running this operation must leave a notice in its chat.
    fn shown_to_user(&self) -> bool {
        self.effect == Effect::Write || self.access == Access::UserAction
    }
    fn execute(&self, repo: &Arc<Repository>, args: &Value, caller: Caller<'_>) -> Result<Value> {
        if caller == Caller::Assistant && self.access == Access::UserAction {
            return Err(Error::new(
                "This command requires a user action. It cannot run from App chat.",
            ));
        }
        super::schema::validate(&self.args, args, "args")?;
        let mut result = (self.run)(repo, args, caller)?;
        match (self.output, caller) {
            (Output::Editable, Caller::Browser) => {}
            _ => redaction::redact(repo, &mut result),
        }
        Ok(result)
    }
}

pub const AGENT_COMMAND_LOG: usize = 20;

/// Run the named operation for `caller`.
pub fn run(repo: &Arc<Repository>, caller: Caller<'_>, name: &str, args: &Value) -> Result<Value> {
    let operation = super::commands::table()
        .into_iter()
        .find(|entry| entry.name == name)
        .ok_or_else(|| Error::new("Unknown command. Read commands.list."))?;
    match caller {
        Caller::Agent { chat } if operation.shown_to_user() => {
            recorded(repo, chat, name, || operation.execute(repo, args, caller))
        }
        _ => operation.execute(repo, args, caller),
    }
}

/// Log the operation on the agent's chat before it runs, so the chat shows every one that ran.
fn recorded(
    repo: &Repository,
    chat: &str,
    name: &str,
    execute: impl FnOnce() -> Result<Value>,
) -> Result<Value> {
    let id = sid(chat)?;
    let entry = json!({"id": uuid(), "name": name, "at": now()});
    repo.transaction(|s| {
        let record = sessions::get_mut(s, &id).map_err(|_| {
            Error::new("Pass this chat's --record ID so the chat can show the command.")
        })?;
        let mut log = arr(&record["agentCommands"]);
        log.push(entry.clone());
        let excess = log.len().saturating_sub(AGENT_COMMAND_LOG);
        log.drain(..excess);
        record["agentCommands"] = Value::Array(log);
        Ok(())
    })?;
    let result = execute();
    let ok = result.is_ok();
    // The operation already ran; failing to store its outcome must not report it as failed.
    let _ = repo.transaction(|s| {
        if let Some(logged) = sessions::get_mut(s, &id)
            .ok()
            .and_then(|record| record["agentCommands"].as_array_mut())
            .and_then(|log| log.iter_mut().find(|row| row["id"] == entry["id"]))
        {
            logged["ok"] = json!(ok);
        }
        Ok(())
    });
    result
}
#[cfg(test)]
mod tests;
