use super::{
    arr, checked, directory, domain, json, now, safe_url, sessions, sid, text, workflows, Error,
    Repository, Result, Value,
};

fn snapshot(s: &Value, id: &str) -> Result<Value> {
    let record = sessions::get(s, id)?;
    let mut value = json!({
        "resources": arr(&record["resources"]),
        "worktrees": arr(&record["worktrees"]),
        "itemIds": arr(&record["items"]).iter().map(|x| x["id"].clone()).collect::<Vec<_>>(),
        "artifactIds": arr(&s["artifacts"]).iter().filter(|x| arr(&x["sessionIds"]).contains(&json!(id))).map(|x| x["id"].clone()).collect::<Vec<_>>()
    });
    value["revision"] = json!(context_kit::sha256(&serde_json::to_vec(&value)?));
    Ok(value)
}
pub fn read(repo: &Repository, id: &str) -> Result<Value> {
    let s = repo.state.lock().map_err(|_| Error::io())?;
    let record = sessions::get(&s, id)?;
    let value = json!({"id":id,"items":record["items"],"context":snapshot(&s,id)?});
    drop(s);
    Ok(value)
}
/// Add relationships to the latest chat without resubmitting session metadata.
pub fn link_work(repo: &Repository, id: &str, item_ids: &Value) -> Result<Value> {
    let id = sid(id)?;
    let ids = item_ids
        .as_array()
        .filter(|ids| !ids.is_empty() && ids.len() <= 100)
        .ok_or_else(|| Error::new("Choose 1 to 100 cached work IDs."))?;
    if ids.iter().any(|id| {
        id.as_str().is_none_or(|id| {
            id.trim().is_empty() || id.chars().count() > 300 || id.chars().any(char::is_control)
        })
    }) {
        return Err(Error::new("Invalid work item ID."));
    }
    repo.transaction(|s| append_work(s, &id, ids, None))
}
fn append_work(
    s: &mut Value,
    id: &str,
    requested: &[Value],
    imported: Option<Value>,
) -> Result<Value> {
    sessions::get(s, id)?;
    let mut catalog = domain::catalog(s);
    if let Some(item) = imported {
        let key = text(&item, "id").to_owned();
        catalog.entry(key).or_insert_with(|| {
            let mut items = arr(&s["items"]);
            items.push(item.clone());
            s["items"] = json!(items);
            item
        });
    }
    let mut items = arr(&sessions::get(s, id)?["items"]);
    let mut ids = vec![];
    let mut added = vec![];
    for item_id in requested {
        if ids.contains(item_id) {
            continue;
        }
        let item = catalog
            .get(item_id.as_str().unwrap())
            .ok_or_else(|| Error::new("Register this work item before linking it."))?;
        ids.push(item_id.clone());
        if !items.iter().any(|item| item["id"] == *item_id) {
            items.push(domain::item_ref(item));
            added.push(item_id.clone());
        }
    }
    if items.len() > 100 {
        return Err(Error::new("Choose up to 100 linked work items."));
    }
    let record = sessions::get_mut(s, id)?;
    record["items"] = json!(items);
    record["updatedAt"] = json!(now());
    Ok(
        json!({"recordId":id,"itemIds":ids,"addedItemIds":added,"revision":snapshot(s,id)?["revision"],"saved":true,"scope":"local"}),
    )
}
pub fn link_work_url(repo: &Repository, id: &str, url: &str) -> Result<Value> {
    link_work_url_with(repo, id, url, |url| {
        if repo.demo {
            return Err(Error::new(
                "Sample workspace: provider work was not imported.",
            ));
        }
        repo.providers.resolve(url)
    })
}
fn link_work_url_with(
    repo: &Repository,
    id: &str,
    url: &str,
    resolve: impl FnOnce(&str) -> Result<Value>,
) -> Result<Value> {
    let id = sid(id)?;
    let url = safe_url(url)?;
    if url.len() > 4096 {
        return Err(Error::new("Work URL is too long."));
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| Error::new("Use a provider work URL."))?;
    let host = parsed.host_str().unwrap_or("");
    let parts = parsed
        .path()
        .trim_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    let supported = match host {
        "github.com" => {
            parts.len() == 4
                && ["pull", "issues"].contains(&parts[2])
                && !parts[3].is_empty()
                && parts[3].bytes().all(|c| c.is_ascii_digit())
        }
        "linear.app" => {
            parts.len() >= 3
                && parts[1] == "issue"
                && regex::Regex::new(r"^[A-Za-z][A-Za-z0-9]*-[0-9]+$")
                    .unwrap()
                    .is_match(parts[2])
        }
        _ if host == "sentry.io" || host.ends_with(".sentry.io") => {
            parts.contains(&"issues")
                && parts
                    .last()
                    .is_some_and(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()))
        }
        _ => false,
    };
    if !supported || parsed.scheme() != "https" || parsed.port().is_some_and(|port| port != 443) {
        return Err(Error::new(
            "Use an HTTPS GitHub issue/PR, Linear issue, or Sentry issue URL.",
        ));
    }
    let cached = {
        let s = repo.state.lock().map_err(|_| Error::io())?;
        sessions::get(&s, &id)?;
        domain::catalog(&s)
            .into_values()
            .find(|item| item["url"] == url)
    };
    let item = match cached {
        Some(item) => item,
        None => resolve(&url)?,
    };
    let item_id = checked(&item, "id", 300, true)?;
    // Provider reads run before acquiring the mutation transaction; merge against current links.
    repo.transaction(|s| append_work(s, &id, &[json!(item_id)], Some(item)))
}
fn list<'a>(input: &'a Value, key: &str, limit: usize) -> Result<Option<&'a Vec<Value>>> {
    input.get(key).map_or(Ok(None), |value| {
        value
            .as_array()
            .filter(|v| v.len() <= limit)
            .map(Some)
            .ok_or_else(|| {
                Error::new(format!(
                    "{key} must be an array with at most {limit} entries."
                ))
            })
    })
}
pub fn replace(repo: &Repository, id: &str, input: &Value) -> Result<Value> {
    replace_result(repo, id, input).map(|(record, _)| record)
}
/// Identity of this exact mutation, captured before releasing the state transaction.
pub fn replace_receipt(repo: &Repository, id: &str, input: &Value) -> Result<Value> {
    replace_result(repo, id, input).map(|(_, revision)| json!({"id":id,"revision":revision}))
}
type Worktree = (Value, Vec<Value>, bool);
fn worktrees(trees: &[Value]) -> Result<Vec<Worktree>> {
    let trees = trees
        .iter()
        .map(|tree| {
            let path = directory(text(tree, "path"))?;
            let metadata = super::checkouts::metadata(&path)
                .ok_or_else(|| Error::new("Choose an existing Git checkout with a commit."))?;
            if tree.get("primary").is_some_and(|v| !v.is_boolean()) {
                return Err(Error::new("Worktree primary must be true or false."));
            }
            let ids = list(tree, "itemIds", 100)?.cloned().unwrap_or_default();
            Ok((metadata, ids, tree["primary"] == true))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut paths = std::collections::HashSet::new();
    if trees.iter().any(|(m, _, _)| !paths.insert(text(m, "path")))
        || trees.iter().filter(|(_, _, p)| *p).count() > 1
    {
        return Err(Error::new(
            "Worktree paths must be unique, with at most one primary checkout.",
        ));
    }
    Ok(trees)
}
fn replace_items(s: &mut Value, id: &str, ids: &[Value]) -> Result<()> {
    let current = arr(&sessions::get(s, id)?["items"]);
    let catalog = domain::catalog(s);
    let mut selected = vec![];
    for id in ids {
        let key = id
            .as_str()
            .ok_or_else(|| Error::new("Invalid work item ID."))?;
        let item = current
            .iter()
            .find(|x| x["id"] == *id)
            .cloned()
            .or_else(|| catalog.get(key).map(domain::item_ref))
            .ok_or_else(|| Error::new("Register this work item before linking it."))?;
        if !selected.iter().any(|x: &Value| x["id"] == *id) {
            selected.push(item);
        }
    }
    let record = sessions::get_mut(s, id)?;
    record["items"] = json!(selected);
    for tree in record["worktrees"].as_array_mut().into_iter().flatten() {
        tree["itemIds"] = json!(arr(&tree["itemIds"])
            .into_iter()
            .filter(|x| ids.contains(x))
            .collect::<Vec<_>>());
    }
    Ok(())
}
fn replace_artifacts(s: &mut Value, id: &str, ids: &[Value]) -> Result<()> {
    let mut artifacts = arr(&s["artifacts"]);
    if ids
        .iter()
        .any(|id| !id.is_string() || !artifacts.iter().any(|x| x["id"] == *id))
    {
        return Err(Error::new(
            "Attach artifact files before replacing their session links.",
        ));
    }
    for artifact in &mut artifacts {
        let mut sessions = arr(&artifact["sessionIds"]);
        sessions.retain(|x| x != id);
        if ids.contains(&artifact["id"]) {
            sessions.push(json!(id));
        }
        artifact["sessionIds"] = json!(sessions);
    }
    s["artifacts"] = json!(artifacts);
    Ok(())
}
fn replace_result(repo: &Repository, id: &str, input: &Value) -> Result<(Value, Value)> {
    let object = input
        .as_object()
        .ok_or_else(|| Error::new("Context must be a JSON object."))?;
    if object.keys().any(|k| {
        ![
            "revision",
            "resources",
            "worktrees",
            "itemIds",
            "artifactIds",
        ]
        .contains(&k.as_str())
    }) {
        return Err(Error::new(
            "Context accepts resources, worktrees, itemIds, artifactIds, and revision.",
        ));
    }
    if serde_json::to_vec(input)?.len() > 96 * 1024 {
        return Err(Error::new("Context exceeds 96 KiB."));
    }
    let revision=input["revision"].as_str().filter(|value|value.len()==64&&value.bytes().all(|byte|byte.is_ascii_hexdigit())).ok_or_else(||{
        let mut error=Error::new("Read the current session context and include its captured revision before replacing it.");error.status=409;error
    })?;
    let resources = list(input, "resources", 128)?;
    let trees = list(input, "worktrees", 64)?;
    let item_ids = list(input, "itemIds", 100)?;
    let artifact_ids = list(input, "artifactIds", 100)?;
    // Git inspection stays outside the state lock. All record changes commit together below.
    let metadata = trees.map(Vec::as_slice).map(worktrees).transpose()?;
    repo.transaction(|s| {
        sessions::get(s, id)?;
        if revision != snapshot(s, id)?["revision"].as_str().unwrap() {
            let mut error =
                Error::new("Session context changed. Read it again before replacing it.");
            error.status = 409;
            return Err(error);
        }
        if let Some(ids) = item_ids {
            replace_items(s, id, ids)?;
        }
        if let Some(entries) = resources {
            super::resources::replace_entries(s, id, &json!(entries))?;
        }
        if let Some(trees) = &metadata {
            sessions::get_mut(s, id)?["worktrees"] = json!([]);
            for (meta, ids, primary) in trees {
                workflows::save_worktree(s, id, meta, ids, *primary)?;
            }
        }
        if let Some(ids) = artifact_ids {
            replace_artifacts(s, id, ids)?;
        }
        let record = sessions::get_mut(s, id)?;
        record["updatedAt"] = json!(now());
        Ok((record.clone(), snapshot(s, id)?["revision"].clone()))
    })
}

#[cfg(test)]
mod tests;
