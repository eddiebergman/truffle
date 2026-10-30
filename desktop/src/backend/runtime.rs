//! What a chat's agent is doing, from its lifecycle hooks or its own reports.
use super::{arr, json, text, Error, Result, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The lifecycle hooks truffle installs for an agent's conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hook {
    SessionStart,
    UserPromptSubmit,
    Stop,
    PermissionRequest,
    SubagentStart,
    SubagentStop,
}
impl Hook {
    /// A turn of the conversation or of one of its subagents ended.
    pub const fn ends_turn(self) -> bool {
        matches!(self, Self::Stop | Self::SubagentStop)
    }
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

pub enum Event<'a> {
    Hook {
        hook: Hook,
        /// Empty unless the hook is about one subagent.
        subagent: &'a str,
        background: &'a Background,
    },
    /// The agent's own report, for agents without hooks. It is a full snapshot: hook-only fields
    /// such as subagent IDs and background counts are dropped, so stale hook data cannot hold a
    /// waiting agent in the background state.
    Report {
        state: &'a str,
        reason: &'a str,
        subagents: Option<u64>,
    },
}

/// The runtime after `event`, given the stored runtime `old`, at time `at`.
pub fn next(old: &Value, event: &Event, at: f64) -> Result<Value> {
    match *event {
        Event::Hook {
            hook,
            subagent,
            background,
        } => after_hook(old, hook, subagent, background, at),
        Event::Report {
            state,
            reason,
            subagents,
        } => report(state, reason, subagents, at),
    }
}
/// The agent handed control back and runs no subagents.
pub fn is_idle(runtime: &Value) -> bool {
    runtime["state"] == "waiting" && runtime["subagents"].as_u64().unwrap_or(0) == 0
}
fn report(state: &str, reason: &str, subagents: Option<u64>, at: f64) -> Result<Value> {
    let state = state.trim();
    if !["working", "waiting", "needs-input"].contains(&state) {
        return Err(Error::new("Choose working, waiting, or needs-input."));
    }
    if subagents.is_some_and(|count| count > 1000) {
        return Err(Error::new("Subagent count must be 0–1000."));
    }
    let reason = reason.trim();
    if !["", "ready", "finished", "question", "approval", "failed"].contains(&reason) {
        return Err(Error::new("Invalid attention reason."));
    }
    Ok(json!({"state":state,"reason":reason,"subagents":subagents,"at":at}))
}
fn after_hook(
    old: &Value,
    hook: Hook,
    subagent: &str,
    background: &Background,
    at: f64,
) -> Result<Value> {
    if subagent.chars().count() > 200 || subagent.contains('\0') {
        return Err(Error::new("Invalid subagentId."));
    }
    let subagent = json!(subagent.trim());
    let mut ids = arr(&old["subagentIds"]);
    let unchanged = old["state"].as_str().unwrap_or("working");
    let (state, reason) = match hook {
        Hook::SessionStart => {
            ids.clear();
            ("waiting", "ready")
        }
        Hook::Stop => ("waiting", "finished"),
        Hook::UserPromptSubmit => ("working", ""),
        Hook::PermissionRequest => ("needs-input", "approval"),
        Hook::SubagentStart => {
            if subagent != "" && !ids.contains(&subagent) && ids.len() < 1000 {
                ids.push(subagent);
            }
            (unchanged, text(old, "reason"))
        }
        Hook::SubagentStop => {
            ids.retain(|id| *id != subagent);
            (unchanged, text(old, "reason"))
        }
    };
    let mut runtime =
        json!({"reason":reason,"state":state,"subagents":ids.len(),"subagentIds":ids,"at":at});
    // Turn-end hooks carry an authoritative snapshot of background work.
    // Omitted snapshots mean unknown, not zero; unrelated events retain it.
    if hook != Hook::SessionStart {
        for field in ["backgroundCounts", "scheduledWakeups", "activityAt"] {
            if !old[field].is_null() {
                runtime[field] = old[field].clone();
            }
        }
    }
    record_background(&mut runtime, background, at)?;
    settle_background(&mut runtime, !ids.is_empty());
    Ok(runtime)
}
fn record_background(runtime: &mut Value, background: &Background, at: f64) -> Result<()> {
    if let Some(counts) = &background.background_counts {
        if counts.len() > 100
            || counts
                .iter()
                .any(|(kind, count)| kind.is_empty() || kind.chars().count() > 40 || *count > 1000)
        {
            return Err(Error::new("Invalid background activity counts."));
        }
        runtime["backgroundCounts"] = json!(counts);
        runtime["activityAt"] = json!(at);
    }
    if let Some(count) = background.scheduled_wakeups {
        if count > 1000 {
            return Err(Error::new("Invalid scheduled wakeup count."));
        }
        runtime["scheduledWakeups"] = json!(count);
        runtime["activityAt"] = json!(at);
    }
    Ok(())
}
/// A waiting agent with background work, subagents or wakeups is in the background state.
fn settle_background(runtime: &mut Value, has_subagents: bool) {
    let background = runtime["backgroundCounts"]
        .as_object()
        .map_or(0, |v| v.values().filter_map(Value::as_u64).sum::<u64>());
    let busy =
        background > 0 || has_subagents || runtime["scheduledWakeups"].as_u64().unwrap_or(0) > 0;
    if runtime["state"] == "waiting" && busy {
        runtime["state"] = json!("background");
    } else if runtime["state"] == "background" && !busy {
        runtime["state"] = json!("waiting");
    }
}

#[cfg(test)]
mod tests;
