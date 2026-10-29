use super::{arr, domain, json, now, sessions, sid, text, Error, Repository, Result, Value};

fn linked_prs(state: &Value, id: &str) -> Result<Vec<Value>> {
    let session = sessions::get(state, id)?;
    let ids = arr(&session["items"])
        .iter()
        .map(|i| text(i, "id").to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    Ok(domain::catalog(state)
        .into_values()
        .filter(|i| {
            ids.contains(text(i, "id")) && i["source"] == "github" && i["kind"] == "Pull request"
        })
        .collect())
}
const CI_FIELDS: [&str; 8] = [
    "ci",
    "status",
    "sourceStatus",
    "githubState",
    "mergeState",
    "mergeable",
    "isDraft",
    "headRefOid",
];
fn updates(state: &Value, fresh: &[Value]) -> Vec<Value> {
    let catalog = domain::catalog(state);
    fresh
        .iter()
        .filter_map(|new| {
            let old = catalog.get(text(new, "id"))?;
            let mut next = old.clone();
            for field in CI_FIELDS {
                if let Some(value) = new.get(field) {
                    next[field] = value.clone();
                }
            }
            (next != *old).then_some(next)
        })
        .collect()
}
fn guarded_updates(
    state: &Value,
    baseline: &[Value],
    fresh: &[Value],
) -> (Vec<Value>, Vec<String>, Vec<String>) {
    let catalog = domain::catalog(state);
    let mut accepted = vec![];
    let mut ids = vec![];
    let mut skipped = vec![];
    for item in fresh {
        let id = text(item, "id");
        let unchanged = baseline
            .iter()
            .find(|old| old["id"] == id)
            .zip(catalog.get(id))
            .is_some_and(|(old, current)| {
                CI_FIELDS
                    .iter()
                    .all(|field| old.get(field) == current.get(field))
            });
        if unchanged {
            accepted.push(item.clone());
            ids.push(id.to_owned());
        } else {
            skipped.push(id.to_owned());
        }
    }
    (updates(state, &accepted), ids, skipped)
}
fn apply(state: &mut Value, changed: &[Value]) {
    for new in changed {
        let id = text(new, "id");
        if let Some(row) = state["items"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|i| i["id"] == id)
        {
            *row = new.clone();
        }
        state["itemArchive"][id] = new.clone();
        for session in sessions::all_mut(state) {
            for item in session["items"].as_array_mut().unwrap() {
                if item["id"] == id {
                    *item = domain::item_ref(new);
                }
            }
        }
    }
    domain::observe(state, "github", changed);
    super::github::observe(state);
}
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum SkipReason {
    Disabled,
    Backoff,
    AlreadyRunning,
    CiBackoff,
    Cooldown,
    NoLinkedPrs,
    Demo,
    StateChanged,
}
#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
enum RefreshOutcome {
    Checked {
        #[serde(rename = "checkedAt")]
        checked_at: f64,
    },
    Skipped {
        reason: SkipReason,
        #[serde(rename = "checkedAt")]
        checked_at: (),
        #[serde(rename = "retryAt", skip_serializing_if = "Option::is_none")]
        retry_at: Option<f64>,
    },
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshReceipt {
    record_id: String,
    item_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skipped_item_ids: Vec<String>,
    #[serde(flatten)]
    outcome: RefreshOutcome,
}
fn receipt(id: &str, ids: Vec<String>, outcome: RefreshOutcome) -> Result<Value> {
    Ok(serde_json::to_value(RefreshReceipt {
        record_id: id.into(),
        item_ids: ids,
        skipped_item_ids: vec![],
        outcome,
    })?)
}
fn skipped(id: &str, ids: Vec<String>, reason: SkipReason, retry_at: Option<f64>) -> Result<Value> {
    receipt(
        id,
        ids,
        RefreshOutcome::Skipped {
            reason,
            checked_at: (),
            retry_at: retry_at.filter(|time| time.is_finite() && *time >= 0.0),
        },
    )
}
fn pr_ids(refs: &[Value]) -> Result<Vec<String>> {
    if refs.len() > 100 {
        return Err(Error::new("Refresh CI for at most 100 linked PRs."));
    }
    refs.iter()
        .map(|item| {
            let id = text(item, "id");
            if id.is_empty() || id.len() > 200 || id.chars().any(char::is_control) {
                return Err(Error::new("Invalid linked PR identity."));
            }
            Ok(id.to_owned())
        })
        .collect()
}
pub fn refresh(repo: &Repository, id: &str) -> Result<Value> {
    refresh_with(repo, id, |refs| repo.providers.refresh("github", refs))
}
struct Guard<'a>(&'a Repository, String);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.opening.lock().unwrap().remove(&self.1);
    }
}
fn refresh_with(
    repo: &Repository,
    id: &str,
    remote: impl FnOnce(&[Value]) -> Result<Vec<Value>>,
) -> Result<Value> {
    let id = sid(id)?;
    let key = format!("session-ci:{id}");
    let (refs, ids) = {
        // Admission is the read boundary. A later policy change does not cancel an admitted read.
        // Follow the repository state -> running -> cooldown lock order; no lock spans provider I/O.
        let state = repo.state.lock().map_err(|_| Error::io())?;
        let refs = linked_prs(&state, &id)?;
        let ids = pr_ids(&refs)?;
        if refs.is_empty() {
            return skipped(&id, ids, SkipReason::NoLinkedPrs, None);
        }
        if repo.demo {
            return skipped(&id, ids, SkipReason::Demo, None);
        }
        let running = repo.running.lock().map_err(|_| Error::io())?;
        let time = now();
        if let Some((reason, retry_at)) = super::repository::source_skip(
            &state["sources"]["github"],
            running.contains("github"),
            true,
            time,
        ) {
            let reason = match reason {
                "disabled" => SkipReason::Disabled,
                "backoff" => SkipReason::Backoff,
                "already-running" => SkipReason::AlreadyRunning,
                _ => return Err(Error::new("Unsupported CI source admission result.")),
            };
            return skipped(&id, ids, reason, retry_at);
        }
        let mut cooldown = repo.cooldown.lock().map_err(|_| Error::io())?;
        if let Some(retry) = cooldown
            .get("session-ci-backoff")
            .copied()
            .filter(|retry| *retry > time)
        {
            return skipped(&id, ids, SkipReason::CiBackoff, Some(retry));
        }
        let mut opening = repo.opening.lock().map_err(|_| Error::io())?;
        if opening.contains(&key) {
            return skipped(&id, ids, SkipReason::AlreadyRunning, None);
        }
        if let Some(retry) = cooldown.get(&key).copied().filter(|retry| *retry > time) {
            return skipped(&id, ids, SkipReason::Cooldown, Some(retry));
        }
        opening.insert(key.clone());
        cooldown.insert(key.clone(), time + 25.0);
        drop(opening);
        drop(cooldown);
        drop(running);
        drop(state);
        (refs, ids)
    };
    let _guard = Guard(repo, key);
    let fresh = match remote(&refs) {
        Ok(items) => items,
        Err(error) => {
            repo.cooldown
                .lock()
                .unwrap()
                .insert("session-ci-backoff".into(), now() + error.retry.max(60.0));
            return Err(error);
        }
    };
    let fresh_ids = pr_ids(&fresh)?;
    if fresh_ids.iter().any(|id| !ids.contains(id)) {
        return Err(Error::new(
            "GitHub refresh returned an unexpected PR identity.",
        ));
    }
    let (accepted, discarded) = repo.transaction_if_changed(|state| {
        let (changed, accepted, discarded) = guarded_updates(state, &refs, &fresh);
        if !changed.is_empty() {
            apply(state, &changed);
        }
        Ok((accepted, discarded))
    })?;
    if accepted.is_empty() && !discarded.is_empty() {
        return skipped(&id, ids, SkipReason::StateChanged, None);
    }
    Ok(serde_json::to_value(RefreshReceipt {
        record_id: id,
        item_ids: accepted,
        skipped_item_ids: discarded,
        outcome: RefreshOutcome::Checked { checked_at: now() },
    })?)
}

pub fn investigation(repo: &Repository, p: &Value) -> Result<Value> {
    repo.transaction(|state| investigation_in_state(state, p))
}
pub(super) fn investigation_in_state(state: &mut Value, p: &Value) -> Result<Value> {
    let id = text(p, "id");
    let item = text(p, "itemId");
    let request = sid(text(p, "requestId"))?;

    if !linked_prs(state, id)?.iter().any(|pr| pr["id"] == item) {
        return Err(Error::new("This PR is not linked to the chat."));
    }
    let session = sessions::get_mut(state, id)?;
    if !session["ciInvestigations"].is_object() {
        session["ciInvestigations"] = json!({});
    }
    let previous = &session["ciInvestigations"][item];
    let status = text(p, "state");
    if status == "queued" {
        if ["queued", "investigating"].contains(&text(previous, "state"))
            && previous["id"] != request
        {
            return Err(Error::new(
                "An investigation is already active. Clear it before retrying.",
            ));
        }
        if previous["id"] != request {
            session["ciInvestigations"][item] =
                json!({"id":request,"state":status,"queuedAt":now(),"updatedAt":now()});
        }
    } else {
        if previous["id"] != request
            || !["queued", "investigating"].contains(&text(previous, "state"))
        {
            return Err(Error::new("This investigation is no longer active."));
        }
        if !["investigating", "complete", "failed", "cancelled"].contains(&status) {
            return Err(Error::new("Invalid investigation state."));
        }
        session["ciInvestigations"][item]["state"] = json!(status);
        session["ciInvestigations"][item]["updatedAt"] = json!(now());
    }
    Ok(json!({"id":id,"investigation":session["ciInvestigations"][item]}))
}

pub(super) fn cancel_dispatch(
    state: &mut Value,
    id: &str,
    item: &str,
    operation: &str,
) -> Result<()> {
    let progress = &mut sessions::get_mut(state, id)?["ciInvestigations"][item];
    if progress["id"] == operation && ["queued", "investigating"].contains(&text(progress, "state"))
    {
        progress["state"] = json!("cancelled");
        progress["updatedAt"] = json!(now());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
