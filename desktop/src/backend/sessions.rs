//! Chat records in `state["sessions"]`: lookup, saving and the public projection.
//!
//! This module is the only code that indexes the stored list. Callers reach a
//! record by its ID or by a predicate, never by position. Records stay JSON
//! objects, so fields this module does not know are kept unchanged.
use super::checkouts::item_folders::{self, Strength};
use super::checkouts::Remember;
use super::{arr, checked, directory, domain, json, now, sid, text, uuid, Error, Result, Value};

/// Stored on the record but left out of the public snapshot.
const PRIVATE_FIELDS: [&str; 3] = ["reviewDraft", "reviewHistory", "preparationRequests"];

fn unavailable() -> Error {
    Error::new("Chat unavailable.")
}
fn position(s: &Value, id: &str) -> Result<usize> {
    sid(id).map_err(|_| unavailable())?;
    all(s)
        .iter()
        .position(|record| record["id"] == id)
        .ok_or_else(unavailable)
}
pub fn all(s: &Value) -> &[Value] {
    s["sessions"].as_array().map_or(&[], Vec::as_slice)
}
pub fn all_mut(s: &mut Value) -> &mut [Value] {
    s.get_mut("sessions")
        .and_then(Value::as_array_mut)
        .map_or_else(Default::default, Vec::as_mut_slice)
}
/// The chat with this ID. A malformed or unknown ID gives the same error.
pub fn get<'a>(s: &'a Value, id: &str) -> Result<&'a Value> {
    Ok(&all(s)[position(s, id)?])
}
pub fn get_mut<'a>(s: &'a mut Value, id: &str) -> Result<&'a mut Value> {
    let index = position(s, id)?;
    Ok(&mut all_mut(s)[index])
}
/// The oldest chat that matches.
pub fn find(s: &Value, matches: impl Fn(&Value) -> bool) -> Option<&Value> {
    all(s).iter().find(|record| matches(record))
}
/// The newest chat that matches.
pub fn latest(s: &Value, matches: impl Fn(&Value) -> bool) -> Option<&Value> {
    all(s).iter().rev().find(|record| matches(record))
}
/// Remove the fields that only the owner of the stored state may read.
pub fn public(record: &mut Value) {
    if let Some(fields) = record.as_object_mut() {
        for field in PRIVATE_FIELDS {
            fields.remove(field);
        }
    }
}
pub(super) fn validate_text(fields: &Value) -> Result<(String, String)> {
    let title = checked(fields, "title", 180, true)?;
    if title.chars().any(char::is_control) {
        return Err(Error::new(
            "Session names cannot contain control characters.",
        ));
    }
    Ok((title, checked(fields, "instruction", 8000, false)?))
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    Replace,
    Register,
}
/// Create a chat, or merge `p` into the chat it names by `id` or by `agent` and
/// `sessionId`. The saved record moves to the end of the list.
pub fn save(s: &mut Value, p: &Value) -> Result<Value> {
    save_with(s, p, |_| {})
}
/// Like `save`, then let `extend` set workflow fields before the record is stored.
pub fn save_with(s: &mut Value, p: &Value, extend: impl FnOnce(&mut Value)) -> Result<Value> {
    merge(s, p, Intent::Replace, extend)
}
/// Save from an agent's own registration: keep the existing title and work links.
pub fn register(s: &mut Value, p: &Value) -> Result<Value> {
    merge(s, p, Intent::Register, |_| {})
}
fn merge(
    s: &mut Value,
    p: &Value,
    intent: Intent,
    extend: impl FnOnce(&mut Value),
) -> Result<Value> {
    let existing = if text(p, "id").is_empty() {
        find(s, |x| {
            !text(p, "sessionId").is_empty()
                && x["sessionId"] == p["sessionId"]
                && x["agent"] == p["agent"]
        })
    } else {
        Some(get(s, text(p, "id"))?)
    };
    let is_new = existing.is_none();
    let old = existing.cloned().unwrap_or_else(|| json!({}));
    let mut merged = old.clone();
    if !merged.is_object() {
        merged = json!({});
    }
    for (k, v) in p
        .as_object()
        .ok_or_else(|| Error::new("Expected a session object."))?
    {
        merged[k] = v.clone();
    }
    if intent == Intent::Register && text(p, "title").trim().is_empty() {
        merged["title"] = if text(&old, "title").is_empty() {
            json!("Agent session")
        } else {
            old["title"].clone()
        };
    }
    let agent = text(&merged, "agent");
    if !["claude", "codex"].contains(&agent) {
        return Err(Error::new("Choose Claude or Codex."));
    }
    let (title, instruction) = validate_text(&merged)?;
    let cwd = directory(text(&merged, "cwd"))?;
    let destination = match merged["destination"].as_str().unwrap_or("embedded") {
        "cmux" => "embedded",
        other => other,
    };
    if !["embedded", "desktop"].contains(&destination)
        || (agent == "claude" && destination == "desktop")
    {
        return Err(Error::new("Choose the embedded terminal or Codex desktop."));
    }
    let items = link_items(s, p, &old, intent, &cwd)?;
    let (sid_value, launch) = identity(&merged, &old, agent, is_new)?;
    let id = old["id"].as_str().map_or_else(uuid, str::to_owned);
    let mut session = old.clone();
    for(k,v)in json!({"id":id,"agent":agent,"title":title,"cwd":cwd,"sessionId":sid_value,"destination":destination,"instruction":instruction,"items":items,"launchNew":launch,"hidden":old["hidden"].as_bool().unwrap_or(false),"archivedAt":old["archivedAt"],"createdAt":old["createdAt"].as_f64().unwrap_or_else(now),"updatedAt":now()}).as_object().unwrap(){session[k]=v.clone();}
    extend(&mut session);
    let mut sessions = arr(&s["sessions"]);
    sessions.retain(|x| x["id"] != id);
    sessions.push(session.clone());
    s["sessions"] = json!(sessions);
    Ok(session)
}
fn link_items(
    s: &mut Value,
    p: &Value,
    old: &Value,
    intent: Intent,
    cwd: &str,
) -> Result<Vec<Value>> {
    let mut ids = p["itemIds"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| arr(&old["items"]).iter().map(|x| x["id"].clone()).collect());
    if intent == Intent::Register || text(p, "id").is_empty() {
        ids.extend(arr(&old["items"]).iter().map(|x| x["id"].clone()));
    }
    ids.sort_by_key(Value::to_string);
    ids.dedup();
    if ids.len() > 100 || ids.iter().any(|x| !x.is_string()) {
        return Err(Error::new("Choose up to 100 linked work items."));
    }
    let catalog = domain::catalog(s);
    let existing = if p["preserveItemFolders"] == true {
        Remember::Preserve
    } else {
        Remember::Replace
    };
    let mut items = if intent == Intent::Register {
        arr(&old["items"])
    } else {
        vec![]
    };
    for id in ids {
        if intent == Intent::Register && items.iter().any(|item| item["id"] == id) {
            continue;
        }
        let item = catalog
            .get(id.as_str().unwrap())
            .ok_or_else(|| Error::new("A linked work item no longer exists."))?;
        items.push(domain::item_ref(item));
        if !text(item, "detail").is_empty() {
            s["itemArchive"][text(item, "id")] = item.clone();
        }
        item_folders::remember(s, text(item, "id"), cwd, Strength::Implicit, existing);
    }
    Ok(items)
}
fn identity(
    merged: &Value,
    old: &Value,
    agent: &str,
    is_new: bool,
) -> Result<(Option<String>, bool)> {
    let mut sid_value = merged["sessionId"]
        .as_str()
        .filter(|x| !x.is_empty())
        .map(sid)
        .transpose()?;
    if !text(old, "sessionId").is_empty()
        && (sid_value.as_deref() != Some(text(old, "sessionId")) || old["agent"] != agent)
    {
        return Err(Error::new(
            "Remove the association before changing its conversation identity.",
        ));
    }
    let mut launch = if sid_value.is_some() && text(old, "sessionId").is_empty() {
        false
    } else {
        old["launchNew"]
            .as_bool()
            .unwrap_or_else(|| sid_value.is_none())
    };
    if is_new && sid_value.is_none() {
        launch = true;
        if agent == "claude" {
            sid_value = Some(uuid());
        }
    }
    Ok((sid_value, launch))
}

/// Save a Codex chat in `cwd` for tests that only need a record to look up.
#[cfg(test)]
pub fn fixture(s: &mut Value, cwd: &std::path::Path, extend: impl FnOnce(&mut Value)) -> String {
    let record = save_with(
        s,
        &json!({"agent":"codex","title":"Fixture","cwd":cwd,"itemIds":[]}),
        extend,
    )
    .unwrap();
    text(&record, "id").to_owned()
}

#[cfg(test)]
mod tests;
