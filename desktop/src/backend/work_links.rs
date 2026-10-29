//! Exact local manual work edges. Inferred/transitive bundles remain domain-owned.
use super::{domain, json, Error, Repository, Result, Value};
use std::collections::BTreeMap;

const MAX_LINKS: usize = 2000;
type Pair = [String; 2];

#[derive(Clone, Copy)]
pub enum Change {
    Add,
    Remove,
}

fn keys(args: &Value, allowed: &[&str]) -> Result<()> {
    if args
        .as_object()
        .is_none_or(|args| args.keys().any(|key| !allowed.contains(&key.as_str())))
    {
        return Err(Error::new("Unknown manual work-link argument."));
    }
    Ok(())
}
fn identity(value: &Value) -> Result<String> {
    value
        .as_str()
        .filter(|id| {
            !id.trim().is_empty() && id.chars().count() <= 300 && !id.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or(Error::new("Supply an exact bounded work item ID."))
}
fn pair(value: &Value) -> Result<Pair> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 2)
        .ok_or(Error::new("Choose two different work items."))?;
    let mut ids = [identity(&values[0])?, identity(&values[1])?];
    if ids[0] == ids[1] {
        return Err(Error::new("Choose two different work items."));
    }
    // Preserve the canonical legacy Value::to_string ordering, including escapes.
    ids.sort_by_key(|id| json!(id).to_string());
    Ok(ids)
}
fn stored(state: &Value) -> Result<Vec<Pair>> {
    let rows = state["workLinks"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_LINKS)
        .ok_or(Error::new(
            "Stored manual work links exceed their supported bounds.",
        ))?;
    rows.iter().map(pair).collect()
}
fn exact_projection(repo: &Repository, value: Value) -> Result<Value> {
    let mut projected = value.clone();
    super::commands::redact(repo, &mut projected);
    if projected != value {
        return Err(Error::new(
            "Manual work-link identities cannot be safely projected.",
        ));
    }
    Ok(value)
}
fn change_in_state(
    state: &mut Value,
    catalog: &BTreeMap<String, Value>,
    ids: &Pair,
    change: Change,
) -> Result<Value> {
    if ids.iter().any(|id| !catalog.contains_key(id)) {
        return Err(Error::new("Choose two known work items."));
    }
    let links = stored(state)?;
    let existed = links.iter().any(|link| link == ids);
    let linked = matches!(change, Change::Add);
    let changed = existed != linked;
    if changed {
        let rows = state["workLinks"].as_array_mut().unwrap();
        if linked {
            if rows.len() >= MAX_LINKS {
                return Err(Error::new("Too many manual work links."));
            }
            rows.push(json!(ids));
        } else {
            rows.retain(|row| {
                !((row[0] == ids[0] && row[1] == ids[1]) || (row[0] == ids[1] && row[1] == ids[0]))
            });
        }
    }
    Ok(json!({"scope":"local","manualOnly":true,"itemIds":ids,"linked":linked,"changed":changed}))
}
pub fn change(repo: &Repository, args: &Value, change: Change) -> Result<Value> {
    keys(args, &["itemIds"])?;
    let ids = pair(&args["itemIds"])?;
    exact_projection(repo, json!(ids))?;
    repo.transaction_if_changed(|state| {
        let catalog = domain::catalog(state);
        let receipt = change_in_state(state, &catalog, &ids, change)?;
        // Reject an unsafe receipt before committing, including fixed routing fields.
        exact_projection(repo, receipt)
    })
}
pub(super) fn legacy_change(
    state: &mut Value,
    args: &Value,
    catalog: &BTreeMap<String, Value>,
    change: Change,
) -> Result<()> {
    keys(args, &["action", "itemIds"])?;
    let ids = pair(&args["itemIds"])?;
    change_in_state(state, catalog, &ids, change)?;
    Ok(())
}
pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    keys(args, &["itemId", "offset", "snapshot"])?;
    let item = args.get("itemId").map(identity).transpose()?;
    let offset = match args.get("offset") {
        Some(value) => value
            .as_u64()
            .filter(|offset| *offset <= MAX_LINKS as u64)
            .ok_or(Error::new(
                "Choose a manual work-link offset from 0 to 2000.",
            ))? as usize,
        None => 0,
    };
    let supplied = match args.get("snapshot") {
        Some(Value::String(value))
            if value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            Some(value.as_str())
        }
        None => None,
        _ => return Err(Error::new("Supply the exact manual work-link snapshot.")),
    };
    let mut links = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        stored(&state)?
    };
    links.sort_by_key(|link| json!(link).to_string());
    links.dedup();
    // Validate the complete retained set before filters/pages can hide identities.
    exact_projection(repo, json!({"itemId":item,"links":links}))?;
    let snapshot = context_kit::sha256(&serde_json::to_vec(&json!({"itemId":item,"links":links}))?);
    if offset > 0 && supplied.is_none() || supplied.is_some_and(|value| value != snapshot) {
        let mut error = Error::new(
            "Manual work links changed. Restart work.links.read without offset or snapshot.",
        );
        error.status = 409;
        return Err(error);
    }
    let links = links
        .into_iter()
        .filter(|link| item.as_ref().is_none_or(|item| link.contains(item)))
        .collect::<Vec<_>>();
    let total = links.len();
    exact_projection(
        repo,
        json!({"scope":"local","manualOnly":true,"itemId":item,"links":links.into_iter().skip(offset).take(100).map(|item_ids|json!({"itemIds":item_ids})).collect::<Vec<_>>(),
        "total":total,"offset":offset,"nextOffset":(offset + 100 < total).then_some(offset + 100),"snapshot":snapshot}),
    )
}

#[cfg(test)]
mod tests;
