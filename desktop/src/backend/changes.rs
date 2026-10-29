//! Cached observed changes and exact-ID read markers. No provider or filesystem reads.
use super::*;
use std::collections::BTreeSet;
const MAX_CHANGES: usize = 500;
fn keys(args: &Value, allowed: &[&str]) -> Result<()> {
    let args = args
        .as_object()
        .ok_or(Error::new("Change arguments must be an object."))?;
    if args.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::new("Unknown change argument."));
    }
    Ok(())
}
fn id(value: &Value) -> Result<&str> {
    value
        .as_str()
        .filter(|id| {
            !id.trim().is_empty() && id.chars().count() <= 160 && !id.chars().any(char::is_control)
        })
        .ok_or(Error::new("Supply a bounded observed change or work ID."))
}
fn ids(value: &Value) -> Result<Vec<String>> {
    let values = value
        .as_array()
        .filter(|values| values.len() <= MAX_CHANGES)
        .ok_or(Error::new("Supply at most 500 captured change IDs."))?;
    let mut seen = BTreeSet::new();
    let mut result = vec![];
    for value in values {
        let value = id(value)?.to_owned();
        if !seen.insert(value.clone()) {
            return Err(Error::new("Captured change IDs must be unique."));
        }
        result.push(value);
    }
    Ok(result)
}
fn rows(state: &Value) -> Result<&Vec<Value>> {
    state["changes"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_CHANGES)
        .ok_or(Error::new("Cached changes exceed their supported limit."))
}
fn prefix(repo: &Repository, value: &Value, bound: usize) -> String {
    let mut value = value.as_str().unwrap_or("").to_owned();
    commands::redact_text(repo, &mut value);
    value.chars().take(bound).collect()
}
fn projected_identity(repo: &Repository, value: &Value, optional: bool) -> Result<String> {
    let identity = if optional && (value.is_null() || value == "") {
        ""
    } else {
        id(value)?
    };
    let mut redacted = identity.to_owned();
    commands::redact_text(repo, &mut redacted);
    if redacted != identity {
        return Err(Error::new(
            "A cached change identity cannot be safely projected.",
        ));
    }
    Ok(identity.to_owned())
}
fn summary(repo: &Repository, row: &Value) -> Result<Value> {
    let source = text(row, "source");
    if !["github", "linear", "sentry"].contains(&source) {
        return Err(Error::new(
            "A cached change has an unsupported source identity.",
        ));
    }
    let mut value = json!({"id":projected_identity(repo,&row["id"],false)?,"itemId":projected_identity(repo,&row["itemId"],true)?,"source":projected_identity(repo,&row["source"],false)?});
    for (field, bound) in [("title", 300), ("reference", 160), ("message", 2000)] {
        value[field] = json!(prefix(repo, &row[field], bound));
    }
    value["url"] = json!(safe_url(&prefix(repo, &row["url"], 4096)).unwrap_or_default());
    value["read"] = json!(row["read"] == true);
    value["at"] = row["at"]
        .as_f64()
        .filter(|time| time.is_finite() && *time >= 0.0)
        .map_or(Value::Null, |time| json!(time));
    Ok(value)
}
pub fn list(repo: &Repository, args: &Value) -> Result<Value> {
    keys(args, &["limit", "afterId", "source", "unread", "itemId"])?;
    let limit = match args.get("limit") {
        Some(value) => value
            .as_u64()
            .filter(|value| (1..=100).contains(value))
            .ok_or(Error::new("Choose a change limit from 1 to 100."))?
            as usize,
        None => 40,
    };
    let after = args.get("afterId").map(id).transpose()?;
    let item = args.get("itemId").map(id).transpose()?;
    let source = match args.get("source") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|value| ["github", "linear", "sentry"].contains(value))
                .ok_or(Error::new("Choose a supported change source."))?,
        ),
        None => None,
    };
    let unread = args
        .get("unread")
        .map(|value| {
            value
                .as_bool()
                .ok_or(Error::new("Unread must be a boolean."))
        })
        .transpose()?;
    let snapshot = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        rows(&state)?.clone()
    };
    let start = match after {
        Some(after) => snapshot
            .iter()
            .position(|row| row["id"] == after)
            .map(|index| index + 1)
            .ok_or_else(|| {
                let mut error = Error::new(
                    "The change cursor was pruned. Restart changes.list without afterId.",
                );
                error.status = 409;
                error
            })?,
        None => 0,
    };
    let matches = |row: &&Value| {
        source.is_none_or(|source| row["source"] == source)
            && item.is_none_or(|item| row["itemId"] == item)
            && unread.is_none_or(|unread| (row["read"] != true) == unread)
    };
    let total = snapshot.iter().filter(matches).count();
    let matching = snapshot
        .iter()
        .skip(start)
        .filter(matches)
        .collect::<Vec<_>>();
    let has_more = matching.len() > limit;
    let page = matching
        .into_iter()
        .take(limit)
        .map(|row| summary(repo, row))
        .collect::<Result<Vec<_>>>()?;
    let next = if has_more {
        page.last()
            .map(|row| row["id"].clone())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Ok(json!({"changes":page,"total":total,"nextAfterId":next,"scope":"cached"}))
}
pub(super) fn mark_in_state(state: &mut Value, args: &Value) -> Result<Value> {
    keys(args, &["ids"])?;
    let requested = ids(&args["ids"])?;
    rows(state)?;
    let mut marked = vec![];
    let mut already = vec![];
    let mut absent = vec![];
    for id in &requested {
        match state["changes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["id"] == *id)
        {
            Some(row) => {
                if row["read"] == true {
                    already.push(id.clone());
                } else {
                    row["read"] = json!(true);
                    marked.push(id.clone());
                }
            }
            None => absent.push(id.clone()),
        }
    }
    Ok(
        json!({"requestedIds":requested,"markedCount":marked.len(),"markedIds":marked,"alreadyReadIds":already,"absentIds":absent,"scope":"local"}),
    )
}
pub fn mark_read(repo: &Repository, args: &Value) -> Result<Value> {
    keys(args, &["ids"])?;
    for value in ids(&args["ids"])? {
        projected_identity(repo, &json!(value), false)?;
    }
    repo.transaction_if_changed(|state| mark_in_state(state, args))
}
pub(super) fn legacy_mark(state: &mut Value, args: &Value) -> Result<Value> {
    if args.get("all").is_some() {
        return Err(Error::new(
            "Global mark-all is retired. Supply the captured change IDs instead.",
        ));
    }
    keys(args, &["action", "id", "ids"])?;
    if args.get("id").is_some() && args.get("ids").is_some() {
        return Err(Error::new("Supply captured IDs or one ID, not both."));
    }
    let captured = if let Some(value) = args.get("id") {
        json!({"ids":[id(value)?]})
    } else {
        json!({"ids":args["ids"]})
    };
    mark_in_state(state, &captured)
}
#[cfg(test)]
mod tests;
