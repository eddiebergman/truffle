//! Bounded navigation requests. Queuing does not confirm that a UI showed them.
use super::{
    arr, chat_search, checked, commands, domain, json, sid, text, views, work_search, Error,
    Repository, Result, Value,
};
use std::collections::BTreeSet;
const MAX_PENDING: usize = 8;
const MAX_ACKNOWLEDGED: usize = 32;
const MAX_BYTES: usize = 512 * 1024;
fn conflict(message: &str) -> Error {
    let mut error = Error::new(message);
    error.status = 409;
    error
}
fn exact(repo: &Repository, value: &str) -> Result<()> {
    let mut projected = value.to_owned();
    commands::redact_text(repo, &mut projected);
    if projected != value {
        return Err(Error::new(
            "Presentation identity is unavailable for safe projection.",
        ));
    }
    Ok(())
}
fn identity(repo: &Repository, value: &Value, max: usize) -> Result<String> {
    let value = value
        .as_str()
        .filter(|value| {
            !value.is_empty()
                && value.chars().count() <= max
                && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| Error::new("Supply an exact bounded presentation identity."))?;
    exact(repo, value)?;
    Ok(value.into())
}
fn request(repo: &Repository, value: &Value) -> Result<String> {
    let value = identity(repo, value, 36)?;
    if sid(&value)? != value {
        return Err(Error::new("Supply a canonical presentation request UUID."));
    }
    Ok(value)
}
fn vocabulary(repo: &Repository) -> Result<()> {
    for value in [
        "queued",
        "shown",
        "dismissed",
        "saved-view",
        "display-items",
        "work",
        "chat",
    ] {
        exact(repo, value)?;
    }
    Ok(())
}
fn clipped(repo: &Repository, value: &Value, max: usize) -> Value {
    let mut value = value.as_str().unwrap_or("").to_owned();
    commands::redact_text(repo, &mut value);
    json!(value.chars().take(max).collect::<String>())
}
fn work(repo: &Repository, item: &Value) -> Result<Value> {
    let projected = work_search::project(item, usize::MAX);
    let item = &projected;
    let mut result = json!({"id":identity(repo,&item["id"],160)?});
    for field in [
        "title",
        "reference",
        "url",
        "project",
        "sourceStatus",
        "detail",
    ] {
        if item[field].is_string() {
            result[field] = clipped(repo, &item[field], if field == "url" { 4096 } else { 300 });
        }
    }
    for field in ["source", "kind", "status", "sourceStatusCode"] {
        if item[field].is_string() {
            result[field] = json!(identity(repo, &item[field], 80)?);
        }
    }
    if item["projectId"].is_string() {
        result["projectId"] = json!(identity(repo, &item["projectId"], 160)?);
    }
    for field in [
        "pinned",
        "hidden",
        "snoozed",
        "overridden",
        "sourceUnavailable",
    ] {
        if let Some(value) = item[field].as_bool() {
            result[field] = json!(value);
        }
    }
    for field in ["createdAt", "updatedAt", "archivedAt"] {
        if let Some(value) = item[field]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            result[field] = json!(value);
        } else if item[field].is_string() {
            result[field] = clipped(repo, &item[field], 80);
        }
    }
    if let Some(reasons) = item["actionReasons"].as_array() {
        result["actionReasons"] = json!(reasons
            .iter()
            .take(20)
            .map(|value| clipped(repo, value, 300))
            .collect::<Vec<_>>());
    }
    if item["parent"].is_object() {
        result["parent"] = parent(repo, &item["parent"], 0)?;
    }
    if let Some(relations) = item["relations"].as_array() {
        result["relations"] = json!(relations
            .iter()
            .take(100)
            .filter_map(Value::as_str)
            .map(|value| {
                let mut value = value.to_owned();
                commands::redact_text(repo, &mut value);
                value.chars().take(160).collect::<String>()
            })
            .collect::<Vec<_>>());
    }
    if item["ci"].is_object() {
        result["ci"] = ci(repo, &item["ci"])?;
    }
    Ok(result)
}
fn ci(repo: &Repository, source: &Value) -> Result<Value> {
    let mut ci = json!({});
    if source["state"].is_string() {
        ci["state"] = json!(identity(repo, &source["state"], 40)?);
    }
    if source["summary"].is_string() {
        ci["summary"] = clipped(repo, &source["summary"], 300);
    }
    for field in ["total", "passed", "failed", "pending", "skipped"] {
        if let Some(count) = source[field].as_u64() {
            ci[field] = json!(count.min(1_000_000));
        }
    }
    if let Some(checks) = source["checks"].as_array() {
        ci["checks"] = json!(checks
            .iter()
            .take(100)
            .map(|check| {
                let mut result = json!({});
                for field in ["name", "state", "url", "workflow"] {
                    if check[field].is_string() {
                        result[field] =
                            clipped(repo, &check[field], if field == "url" { 4096 } else { 300 });
                    }
                }
                result
            })
            .collect::<Vec<_>>());
    }
    Ok(ci)
}
fn parent(repo: &Repository, item: &Value, depth: usize) -> Result<Value> {
    let mut result = json!({"id":identity(repo,&item["id"],160)?});
    for field in ["title", "reference", "url"] {
        if item[field].is_string() {
            result[field] = clipped(repo, &item[field], if field == "url" { 4096 } else { 300 });
        }
    }
    if depth < 4 && item["parent"].is_object() {
        result["parent"] = parent(repo, &item["parent"], depth + 1)?;
    }
    Ok(result)
}
fn chat(repo: &Repository, item: &Value) -> Result<Value> {
    let mut result = json!({"id":identity(repo,&item["id"],160)?,"title":clipped(repo,&item["title"],300),"agent":identity(repo,&item["agent"],30)?,"cwd":clipped(repo,&item["cwd"],4096),"archived":item["archived"]==true,"live":item["live"]==true});
    result["items"] = json!(arr(&item["items"])
        .iter()
        .take(100)
        .map(|item| work(repo, item))
        .collect::<Result<Vec<_>>>()?);
    Ok(result)
}
/// Shared immediate App chat and queued external presentation builders.
pub(super) fn saved_view(repo: &Repository, args: &Value) -> Result<Value> {
    let id = identity(repo, &args["id"], 80)?;
    let doc = views::load(repo)?;
    let view = doc
        .views
        .iter()
        .find(|view| view.id == id)
        .ok_or_else(|| Error::new("Saved view unavailable. Read views.read first."))?;
    Ok(json!({"type":"saved-view","id":id,"name":clipped(repo,&json!(view.name),120)}))
}
pub(super) fn display_items(repo: &Repository, args: &Value) -> Result<Value> {
    let items = args["items"]
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= 40)
        .ok_or_else(|| Error::new("Supply 1 to 40 exact item targets."))?;
    let catalog = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        domain::catalog(&state)
            .into_iter()
            .map(|(id, item)| (id, work_search::effective(&state, &item)))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let mut rows = vec![];
    let mut keys = BTreeSet::new();
    for target in items {
        let id = identity(repo, &target["id"], 160)?;
        let kind = text(target, "type");
        let item = match kind {
            "work" => work(
                repo,
                catalog
                    .get(&id)
                    .ok_or_else(|| Error::new("A work item is no longer cached. Search again."))?,
            )?,
            "chat" => chat(repo, &chat_search::target(repo, &id)?)?,
            _ => return Err(Error::new("Choose a work or chat target.")),
        };
        if keys.insert(format!("{kind}:{id}")) {
            rows.push(json!({"type":kind,"id":id,"item":item}));
        }
    }
    let recommended = match args.get("recommended").filter(|value| !value.is_null()) {
        Some(value) => {
            let id = identity(repo, &value["id"], 160)?;
            let kind = text(value, "type");
            if !rows
                .iter()
                .any(|row| row["type"] == kind && row["id"] == id)
            {
                return Err(Error::new(
                    "The recommendation must be one of the displayed targets.",
                ));
            }
            json!({"type":kind,"id":id})
        }
        None => Value::Null,
    };
    Ok(
        json!({"type":"display-items","title":clipped(repo,&json!(checked(args,"title",120,false)?),120),"items":rows,"recommended":recommended,"reason":clipped(repo,&json!(checked(args,"reason",300,false)?),300)}),
    )
}
fn canonical(repo: &Repository, args: &Value) -> Result<Value> {
    match text(args, "type") {
        "saved-view" => saved_view(repo, args),
        "display-items" => display_items(repo, args),
        _ => Err(Error::new("Unknown presentation type.")),
    }
}
fn empty() -> Value {
    json!({"version":1,"entries":[]})
}
fn entries(document: &Value) -> Result<Vec<Value>> {
    if document.is_null() {
        return Ok(vec![]);
    }
    if document["version"] != 1 {
        return Err(Error::new("Unsupported presentation state version."));
    }
    if serde_json::to_vec(document)?.len() > MAX_BYTES {
        return Err(Error::new(
            "Presentation state exceeds its supported limit.",
        ));
    }
    let rows = document["entries"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_PENDING + MAX_ACKNOWLEDGED)
        .ok_or_else(|| Error::new("Invalid presentation state."))?;
    let mut seen = BTreeSet::new();
    let mut pending = 0;
    for row in rows {
        let id = text(row, "requestId");
        if sid(id).is_err()
            || sid(id)? != id
            || !seen.insert(id)
            || !row["hash"].as_str().is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || !matches!(text(row, "status"), "queued" | "shown" | "dismissed")
        {
            return Err(Error::new("Invalid presentation state."));
        }
        if row["status"] == "queued" {
            pending += 1;
            if !row["presentation"].is_object() {
                return Err(Error::new("Invalid queued presentation."));
            }
        }
    }
    if pending > MAX_PENDING || rows.len() - pending > MAX_ACKNOWLEDGED {
        return Err(Error::new(
            "Presentation state exceeds its supported limit.",
        ));
    }
    Ok(rows.clone())
}
fn receipt(repo: &Repository, row: &Value) -> Result<Value> {
    let id = request(repo, &row["requestId"])?;
    exact(repo, text(row, "status"))?;
    Ok(json!({"requestId":id,"status":row["status"]}))
}
fn stored_descriptor(repo: &Repository, value: &Value) -> Result<Value> {
    match text(value, "type") {
        "saved-view" => Ok(
            json!({"type":"saved-view","id":identity(repo,&value["id"],80)?,"name":clipped(repo,&value["name"],120)}),
        ),
        "display-items" => {
            let rows = value["items"]
                .as_array()
                .filter(|rows| !rows.is_empty() && rows.len() <= 40)
                .ok_or_else(|| Error::new("Invalid queued presentation."))?;
            let mut items = vec![];
            for row in rows {
                let id = identity(repo, &row["id"], 160)?;
                if row["item"]["id"] != id {
                    return Err(Error::new("Invalid queued presentation identity."));
                }
                let kind = text(row, "type");
                let item = match kind {
                    "work" => work(repo, &row["item"])?,
                    "chat" => chat(repo, &row["item"])?,
                    _ => return Err(Error::new("Invalid queued presentation type.")),
                };
                items.push(json!({"type":kind,"id":id,"item":item}));
            }
            let recommended = &value["recommended"];
            if !recommended.is_null()
                && !items
                    .iter()
                    .any(|row| row["type"] == recommended["type"] && row["id"] == recommended["id"])
            {
                return Err(Error::new("Invalid queued recommendation."));
            }
            Ok(
                json!({"type":"display-items","title":clipped(repo,&value["title"],120),"items":items,"recommended":if recommended.is_null(){Value::Null}else{json!({"type":recommended["type"],"id":recommended["id"]})},"reason":clipped(repo,&value["reason"],300)}),
            )
        }
        _ => Err(Error::new("Invalid queued presentation type.")),
    }
}
pub(super) fn present(repo: &Repository, args: &Value) -> Result<Value> {
    commands::validate_presentation_arguments("present", args)?;
    vocabulary(repo)?;
    let id = request(repo, &args["requestId"])?;
    let hash = context_kit::sha256(&serde_json::to_vec(&args["presentation"])?);
    let rows = entries(&repo.state.lock().map_err(|_| Error::io())?["uiPresentations"])?;
    if let Some(existing) = existing_receipt(repo, &rows, &id, &hash)? {
        return Ok(existing);
    }
    let descriptor = canonical(repo, &args["presentation"])?;
    repo.transaction_if_changed(|state| {
        let mut rows = entries(&state["uiPresentations"])?;
        if let Some(existing) = existing_receipt(repo, &rows, &id, &hash)? {
            return Ok(existing);
        }
        let row = json!({"requestId":id,"hash":hash,"status":"queued","presentation":descriptor});
        let result = receipt(repo, &row)?;
        rows.push(row);
        if state["uiPresentations"].is_null() {
            state["uiPresentations"] = empty();
        }
        state["uiPresentations"]["entries"] = json!(rows);
        entries(&state["uiPresentations"])?;
        Ok(result)
    })
}
fn existing_receipt(
    repo: &Repository,
    rows: &[Value],
    id: &str,
    hash: &str,
) -> Result<Option<Value>> {
    if let Some(row) = rows.iter().find(|row| row["requestId"] == id) {
        if row["hash"] != hash {
            return Err(conflict(
                "This presentation request ID already has different content.",
            ));
        }
        return receipt(repo, row).map(Some);
    }
    if rows.iter().filter(|row| row["status"] == "queued").count() >= MAX_PENDING {
        return Err(conflict("Eight presentation requests are awaiting review. Show or dismiss one before queuing another."));
    }
    Ok(None)
}
pub(super) fn read(repo: &Repository) -> Result<Value> {
    vocabulary(repo)?;
    let rows = entries(&repo.state.lock().map_err(|_| Error::io())?["uiPresentations"])?;
    let mut pending = vec![];
    let mut receipts = vec![];
    for row in rows {
        receipts.push(receipt(repo, &row)?);
        if row["status"] == "queued" {
            pending.push(json!({"requestId":request(repo,&row["requestId"])?,"presentation":stored_descriptor(repo,&row["presentation"])?}));
        }
    }
    Ok(json!({"pending":pending,"receipts":receipts}))
}
pub(super) fn acknowledge(repo: &Repository, args: &Value) -> Result<Value> {
    commands::validate_presentation_arguments("ack", args)?;
    vocabulary(repo)?;
    let id = request(repo, &args["requestId"])?;
    let outcome = text(args, "outcome");
    repo.transaction_if_changed(|state| {
        let mut rows = entries(&state["uiPresentations"])?;
        let row = rows
            .iter_mut()
            .find(|row| row["requestId"] == id)
            .ok_or_else(|| {
                conflict("This presentation request is unavailable or its receipt has expired.")
            })?;
        if row["status"] != "queued" {
            if row["status"] != outcome {
                return Err(conflict(
                    "This presentation request already has a different acknowledgment.",
                ));
            }
            return receipt(repo, row);
        }
        row["status"] = json!(outcome);
        row.as_object_mut().unwrap().remove("presentation");
        let result = receipt(repo, row)?;
        while rows.iter().filter(|row| row["status"] != "queued").count() > MAX_ACKNOWLEDGED {
            let index = rows
                .iter()
                .position(|row| row["status"] != "queued")
                .unwrap();
            rows.remove(index);
        }
        state["uiPresentations"]["entries"] = json!(rows);
        Ok(result)
    })
}

#[cfg(test)]
mod tests;
