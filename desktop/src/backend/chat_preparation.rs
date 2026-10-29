//! Preparation saves context; it never opens a terminal.
//! Aliases last while their saved record is retained; absence does not prove no effect.
use super::{arr, json, sid, text, Error, Repository, Result, Value};
const MAX_ALIASES: usize = 128;
const RECEIPT: &str = "preparationReceipt";
pub(super) const KINDS: [&str; 8] = [
    "quick",
    "pr-review",
    "review",
    "skill",
    "recover",
    "new",
    "attach",
    "view",
];

pub(super) struct Request {
    id: String,
    pub kind: String,
    hash: String,
}
fn conflict(message: &str) -> Error {
    let mut error = Error::new(message);
    error.status = 409;
    error
}
fn exact(repo: &Repository, value: &str) -> Result<()> {
    let mut safe = value.to_owned();
    super::commands::redact_text(repo, &mut safe);
    if safe != value || value.chars().any(char::is_control) {
        return Err(Error::new(
            "Preparation identity cannot be safely projected.",
        ));
    }
    Ok(())
}
fn record_identity(repo: &Repository, record: &Value) -> Result<()> {
    let id = text(record, "id");
    if sid(id).ok().as_deref() != Some(id) {
        return Err(conflict("The saved preparation chat identity is invalid."));
    }
    exact(repo, id)
}
fn projection(repo: &Repository, id: &str) -> Result<()> {
    for value in [
        id,
        "prepared",
        "not-found",
        "quick",
        "pr-review",
        "review",
        "skill",
        "recover",
        "new",
        "attach",
        "view",
    ] {
        exact(repo, value)?;
    }
    Ok(())
}
fn aliases(record: &Value) -> Result<Vec<Value>> {
    let Some(document) = record.get("preparationRequests") else {
        return Ok(vec![]);
    };
    if document["version"] != 1 || serde_json::to_vec(document)?.len() > 64 * 1024 {
        return Err(conflict(
            "The saved preparation identities are unsupported or exceed their limit.",
        ));
    }
    let entries = document["entries"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_ALIASES)
        .ok_or_else(|| conflict("The saved preparation identities are invalid."))?;
    let mut ids = std::collections::BTreeSet::new();
    for entry in entries {
        let id = sid(text(entry, "requestId"))
            .map_err(|_| conflict("The saved preparation identities are invalid."))?;
        if id != text(entry, "requestId")
            || !ids.insert(id)
            || !KINDS.contains(&text(entry, "kind"))
            || !entry["hash"].as_str().is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || !entry["reused"].is_boolean()
        {
            return Err(conflict("The saved preparation identities are invalid."));
        }
    }
    Ok(entries.clone())
}
fn find(state: &Value, id: &str) -> Result<Option<(Value, Value)>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut found = None;
    for record in arr(&state["sessions"]) {
        for entry in aliases(&record)? {
            if !seen.insert(text(&entry, "requestId").to_owned()) {
                return Err(conflict(
                    "Preparation identities must belong to exactly one saved chat.",
                ));
            }
            if entry["requestId"] == id {
                found = Some((record.clone(), entry));
            }
        }
    }
    Ok(found)
}
fn receipt(record: &Value, entry: &Value) -> Value {
    json!({"requestId":entry["requestId"],"recordId":record["id"],"kind":entry["kind"],"status":"prepared","reused":entry["reused"]})
}
impl Request {
    pub fn lookup(&self, repo: &Repository, state: &Value) -> Result<Option<Value>> {
        let Some((mut record, entry)) = find(state, &self.id)? else {
            return Ok(None);
        };
        if entry["kind"] != self.kind || entry["hash"] != self.hash {
            return Err(conflict("This preparation identity already belongs to different instructions. Keep its original request or start a separate deliberate preparation."));
        }
        record_identity(repo, &record)?;
        record[RECEIPT] = receipt(&record, &entry);
        Ok(Some(record))
    }
}
pub(super) fn lookup(
    repo: &Repository,
    request: Option<&Request>,
    state: &Value,
) -> Result<Option<Value>> {
    match request {
        Some(request) => request.lookup(repo, state),
        None => Ok(None),
    }
}
pub(super) fn finish(
    repo: &Repository,
    request: Option<&Request>,
    state: &mut Value,
    record: Value,
    reused: bool,
) -> Result<Value> {
    let Some(request) = request else {
        return Ok(record);
    };
    record_identity(repo, &record)?;
    // The caller holds the canonical transaction; duplicate checks precede effects.
    if let Some(existing) = request.lookup(repo, state)? {
        return Ok(existing);
    }
    let rows = state["sessions"].as_array_mut().ok_or(Error::io())?;
    let saved = rows
        .iter_mut()
        .find(|row| row["id"] == record["id"])
        .ok_or(Error::new("Prepared chat is unavailable."))?;
    let mut entries = aliases(saved)?;
    if entries.len() >= MAX_ALIASES {
        return Err(conflict("This saved chat has reached its preparation identity limit. Existing preparations remain readable; open that saved chat instead of preparing it again."));
    }
    let entry =
        json!({"requestId":request.id,"kind":request.kind,"hash":request.hash,"reused":reused});
    entries.push(entry.clone());
    if saved.get("preparationRequests").is_none() {
        saved["preparationRequests"] = json!({"version":1});
    }
    saved["preparationRequests"]["entries"] = json!(entries);
    if serde_json::to_vec(&saved["preparationRequests"])?.len() > 64 * 1024 {
        return Err(conflict(
            "The saved preparation identities exceed their limit.",
        ));
    }
    let mut reply = saved.clone();
    reply[RECEIPT] = receipt(saved, &entry);
    Ok(reply)
}
fn input(kind: &str, args: &Value) -> Result<(String, Value)> {
    if !KINDS.contains(&kind) {
        return Err(Error::new("Unknown chat preparation kind."));
    }
    super::commands::validate_preparation_arguments(kind, args)?;
    if serde_json::to_vec(args)?.len() > 64 * 1024 {
        return Err(Error::new("Preparation input exceeds 64 KiB."));
    }
    let id = sid(text(args, "requestId"))?;
    let mut payload = args.clone();
    payload.as_object_mut().unwrap().remove("requestId");
    if payload
        .as_object()
        .unwrap()
        .values()
        .any(|value| value.as_str().is_some_and(|text| text.contains('\0')))
    {
        return Err(Error::new("Preparation text cannot contain NUL."));
    }
    if kind == "attach" {
        payload["sessionId"] = json!(sid(text(&payload, "sessionId"))?);
    }
    if kind == "recover" {
        payload["id"] = json!(sid(text(&payload, "id"))?);
    }
    Ok((id, payload))
}
pub fn prepare(repo: &Repository, kind: &str, args: &Value) -> Result<Value> {
    let (id, payload) = input(kind, args)?;
    projection(repo, &id)?;
    let hash = context_kit::sha256(&serde_json::to_vec(
        &json!({"version":1,"kind":kind,"intent":payload}),
    )?);
    let request = Request {
        id,
        kind: kind.to_owned(),
        hash,
    };
    {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        if let Some(record) = request.lookup(repo, &state)? {
            record_identity(repo, &record)?;
            return Ok(record[RECEIPT].clone());
        }
    }
    let record = match kind {
        "quick" | "pr-review" => super::workflows::quick_with_request(
            repo,
            &payload,
            kind == "pr-review",
            Some(&request),
        )?,
        "review" => super::workflows::review_with_request(repo, &payload, Some(&request))?,
        "skill" => super::workflows::newsletter_with_request(repo, &payload, Some(&request))?,
        "recover" => super::workflows::recover_with_request(repo, &payload, Some(&request))?,
        "new" | "attach" => super::workflows::prepare_with_request(repo, &payload, Some(&request))?,
        "view" => {
            let mut payload = payload;
            payload["requestId"] = json!(request.id);
            super::view_context::prepare_with_request(repo, &payload, Some(&request))?
        }
        _ => unreachable!(),
    };
    record_identity(repo, &record)?;
    Ok(record[RECEIPT].clone())
}
pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    let object = args
        .as_object()
        .filter(|args| args.len() == 1 && args.contains_key("requestId"))
        .ok_or(Error::new("Preparation lookup accepts only requestId."))?;
    let id = sid(object["requestId"]
        .as_str()
        .ok_or(Error::new("Supply the preparation UUID."))?)?;
    projection(repo, &id)?;
    let state = repo.state.lock().map_err(|_| Error::io())?;
    match find(&state, &id)? {
        Some((record, entry)) => {
            record_identity(repo, &record)?;
            Ok(receipt(&record, &entry))
        }
        None => Ok(json!({"requestId":id,"status":"not-found"})),
    }
}

#[cfg(test)]
mod tests;
