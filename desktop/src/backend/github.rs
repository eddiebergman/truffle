//! Explicit PR mutations and durable review-request notification cycles.
use super::{arr, checked, json, now, sid, text, uuid, Error, Repository, Result, Value};
pub fn review_ready(item: &Value) -> bool {
    item["source"] == "github"
        && item["kind"] == "Pull request"
        && item["status"] != "done"
        && item["isDraft"] != true
        && item["status"] != "draft"
        && arr(&item["relations"]).contains(&json!("reviews"))
}
pub fn observe(state: &mut Value) {
    if !state["githubReviewInbox"].is_object() {
        state["githubReviewInbox"] = json!({});
    }
    let ready = arr(&state["items"])
        .into_iter()
        .filter(review_ready)
        .map(|i| text(&i, "id").to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    for (id, event) in state["githubReviewInbox"].as_object_mut().unwrap() {
        if !ready.contains(id) {
            event["active"] = json!(false);
        }
    }
    for id in ready {
        if state["githubReviewInbox"][&id]["active"] != true {
            state["githubReviewInbox"][&id] =
                json!({"active":true,"version":uuid(),"at":now(),"seenAt":null});
        }
    }
    // Keep inactive cycles only while the PR remains cached, bounding long-lived state.
    let known = arr(&state["items"])
        .iter()
        .map(|i| text(i, "id").to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    state["githubReviewInbox"]
        .as_object_mut()
        .unwrap()
        .retain(|id, _| known.contains(id));
}
pub fn seen(repo: &Repository, p: &Value) -> Result<()> {
    let ids = p["reviews"]
        .as_array()
        .filter(|v| v.len() <= 4096)
        .ok_or_else(|| Error::new("Invalid review notification list."))?;
    if ids.iter().any(|v| {
        text(v, "id").len() > 200
            || !text(v, "id").starts_with("github:")
            || sid(text(v, "version")).is_err()
    }) {
        return Err(Error::new("Invalid review notification identity."));
    }
    repo.transaction(|s| {
        for entry in ids {
            if let Some(event) = s["githubReviewInbox"]
                .as_object_mut()
                .and_then(|v| v.get_mut(text(entry, "id")))
            {
                if event["version"] == entry["version"] && event["active"] == true {
                    event["seenAt"] = json!(now());
                }
            }
        }
        Ok(())
    })
}
pub fn action(repo: &Repository, p: &Value) -> Result<Value> {
    action_with(repo, p, |id, operation, login| {
        repo.providers.github_pr_action(id, operation, login)
    })
}
// Keep provider invocation injectable only for isolated owner tests. All validation,
// reservation, normalization and cache commits remain in this single action owner.
fn action_with(
    repo: &Repository,
    p: &Value,
    remote: impl FnOnce(&str, &str, &str) -> Result<(Value, String)>,
) -> Result<Value> {
    let id = checked(p, "id", 200, true)?;
    let operation = checked(p, "action", 30, true)?;
    if !["ready", "request-review", "refresh"].contains(&operation.as_str()) {
        return Err(Error::new("Unsupported GitHub action."));
    }
    let login = checked(p, "login", 39, operation == "request-review")?;
    if !login.is_empty()
        && !regex::Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$")
            .unwrap()
            .is_match(&login)
    {
        return Err(Error::new("Enter a GitHub username, without @."));
    }
    let old = arr(&repo.state.lock().unwrap()["items"])
        .into_iter()
        .find(|i| i["id"] == id && i["source"] == "github" && i["kind"] == "Pull request")
        .ok_or_else(|| Error::new("Choose a cached GitHub pull request."))?;
    let node_id = id
        .strip_prefix("github:")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new("Invalid GitHub PR identity."))?;
    let key = format!("github-action:{id}");
    if !repo.opening.lock().unwrap().insert(key.clone()) {
        return Err(Error::new("This PR action is already running."));
    }
    struct Guard<'a>(&'a Repository, String);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.opening.lock().unwrap().remove(&self.1);
        }
    }
    let _guard = Guard(repo, key.clone());
    repo.cooldown(&format!("{key}:{operation}"), 3.0)?;
    let mut item = if repo.demo {
        let mut item = old;
        match operation.as_str() {
            "ready" => {
                item["isDraft"] = json!(false);
                item["status"] = json!("needs-review");
                item["sourceStatus"] = json!("Awaiting review");
            }
            "request-review" => {
                if item["isDraft"] == true {
                    return Err(Error::new(
                        "Mark this draft ready before requesting a review.",
                    ));
                }
                let mut reviewers = arr(&item["requestedReviewers"]);
                if !reviewers
                    .iter()
                    .any(|r| text(r, "login").eq_ignore_ascii_case(&login))
                {
                    reviewers.push(json!({"login":login,"kind":"user"}));
                }
                item["requestedReviewers"] = json!(reviewers);
                item["reviewerCount"] = json!(reviewers.len());
            }
            _ => (),
        }
        item
    } else {
        let (node, viewer) = remote(node_id, &operation, &login).map_err(|mut error|{
            if error.data["githubActionStage"]=="dispatched" {
                error.data=json!({"githubReceipt":{"id":id,"action":operation,"status":"confirmation-unknown","remote":"unconfirmed","localSaved":false}});
            }
            error
        })?;
        let mut item = super::providers::github_item(&node)
            .map_err(|_| confirmed_cache_error(&id, &operation))?;
        let mut roles = arr(&old["relations"]);
        roles.retain(|r| r != "reviews" && r != "authored");
        if text(&item, "author").eq_ignore_ascii_case(&viewer) {
            roles.push(json!("authored"));
        }
        if arr(&item["requestedReviewers"])
            .iter()
            .any(|r| text(r, "login").eq_ignore_ascii_case(&viewer))
            || (arr(&old["relations"]).contains(&json!("reviews"))
                && arr(&item["requestedReviewers"])
                    .iter()
                    .any(|r| r["kind"] == "team"))
        {
            roles.push(json!("reviews"));
        }
        item["relations"] = json!(roles);
        item
    };
    if item["id"] != id {
        return Err(if repo.demo {
            Error::new("GitHub returned a different PR identity.")
        } else {
            confirmed_cache_error(&id, &operation)
        });
    }
    item["reviewerUpdatedAt"] = json!(now());
    repo.transaction(|s| {
        let row = s["items"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|i| i["id"] == id)
            .ok_or_else(|| {
                Error::new("This PR left the cache. Refresh GitHub to see the remote change.")
            })?;
        *row = item.clone();
        let github = arr(&s["items"])
            .into_iter()
            .filter(|i| i["source"] == "github")
            .collect::<Vec<_>>();
        super::domain::observe(s, "github", &github);
        observe(s);
        Ok(())
    })
    .map_err(|error| {
        if repo.demo {
            error
        } else {
            confirmed_cache_error(&id, &operation)
        }
    })?;
    Ok(
        json!({"id":id,"action":operation,"status":if repo.demo{"preview"}else{"completed"},
        "remote":if repo.demo{"not-contacted"}else{"confirmed"},"localSaved":true}),
    )
}
fn confirmed_cache_error(id: &str, operation: &str) -> Error {
    let mut error=Error::new("GitHub returned the PR, but its local cache could not be saved. Refresh the PR before retrying; any remote change may already be applied.");
    error.data = json!({"githubReceipt":{"id":id,"action":operation,"status":"remote-confirmed-local-failed","remote":"confirmed","localSaved":false}});
    error
}

#[cfg(test)]
mod tests;
