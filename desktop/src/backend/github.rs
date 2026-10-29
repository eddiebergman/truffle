//! Explicit PR mutations and durable review-request notification cycles.
use super::*;
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
        .ok_or(Error::new("Invalid review notification list."))?;
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
    action_with(repo,p,|id,operation,login|repo.providers.github_pr_action(id,operation,login))
}
// Keep provider invocation injectable only for isolated owner tests. All validation,
// reservation, normalization and cache commits remain in this single action owner.
fn action_with(repo: &Repository, p: &Value, remote: impl FnOnce(&str,&str,&str)->Result<(Value,String)>) -> Result<Value> {
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
        .ok_or(Error::new("Choose a cached GitHub pull request."))?;
    let node_id = id
        .strip_prefix("github:")
        .filter(|s| !s.is_empty())
        .ok_or(Error::new("Invalid GitHub PR identity."))?;
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
        let mut item = old.clone();
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
        let mut item = super::providers::github_item(&node).map_err(|_|confirmed_cache_error(&id,&operation))?;
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
        return Err(if repo.demo {Error::new("GitHub returned a different PR identity.")} else {confirmed_cache_error(&id,&operation)});
    }
    item["reviewerUpdatedAt"] = json!(now());
    repo.transaction(|s| {
        let row = s["items"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|i| i["id"] == id)
            .ok_or(Error::new(
                "This PR left the cache. Refresh GitHub to see the remote change.",
            ))?;
        *row = item.clone();
        let github = arr(&s["items"])
            .into_iter()
            .filter(|i| i["source"] == "github")
            .collect::<Vec<_>>();
        super::domain::observe(s, "github", &github);
        observe(s);
        Ok(())
    }).map_err(|error|if repo.demo {error} else {confirmed_cache_error(&id,&operation)})?;
    Ok(json!({"id":id,"action":operation,"status":if repo.demo{"preview"}else{"completed"},
        "remote":if repo.demo{"not-contacted"}else{"confirmed"},"localSaved":true}))
}
fn confirmed_cache_error(id:&str,operation:&str)->Error {
    let mut error=Error::new("GitHub returned the PR, but its local cache could not be saved. Refresh the PR before retrying; any remote change may already be applied.");
    error.data=json!({"githubReceipt":{"id":id,"action":operation,"status":"remote-confirmed-local-failed","remote":"confirmed","localSaved":false}});
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pr() -> Value {
        json!({"id":"github:PR_test","source":"github","kind":"Pull request","status":"needs-review","isDraft":false,"relations":["reviews"],"title":"Test PR","url":"https://github.com/test/repo/pull/1","canUpdate":true,"githubState":"OPEN","requestedReviewers":[]})
    }
    #[test]
    fn notification_cycles_survive_seen_and_reappear_only_after_a_new_request_or_ready_transition()
    {
        let mut s = json!({"items":[pr()]});
        observe(&mut s);
        let first = s["githubReviewInbox"]["github:PR_test"]["version"].clone();
        s["githubReviewInbox"]["github:PR_test"]["seenAt"] = json!(1);
        observe(&mut s);
        assert_eq!(s["githubReviewInbox"]["github:PR_test"]["version"], first);
        s["items"][0]["isDraft"] = json!(true);
        observe(&mut s);
        assert_eq!(s["githubReviewInbox"]["github:PR_test"]["active"], false);
        s["items"][0]["isDraft"] = json!(false);
        observe(&mut s);
        assert_ne!(s["githubReviewInbox"]["github:PR_test"]["version"], first);
        assert!(s["githubReviewInbox"]["github:PR_test"]["seenAt"].is_null());
        s["items"][0]["relations"] = json!(["linked"]);
        observe(&mut s);
        assert_eq!(s["githubReviewInbox"]["github:PR_test"]["active"], false);
    }
    #[test]
    fn demo_actions_preserve_existing_reviewers_and_stale_seen_does_not_clear_a_new_cycle() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(
            temp.path().to_path_buf(),
            Some(temp.path().join("test.json")),
            true,
        )
        .unwrap();
        repo.transaction(|s| {
            s["items"] = json!([pr()]);
            observe(s);
            Ok(())
        })
        .unwrap();
        seen(
            &repo,
            &json!({"reviews":[{"id":"github:PR_test","version":uuid()}]}),
        )
        .unwrap();
        assert!(repo.snapshot(false)["githubReviewInbox"]["github:PR_test"]["seenAt"].is_null());
        action(
            &repo,
            &json!({"id":"github:PR_test","action":"request-review","login":"someone"}),
        )
        .unwrap();
        assert_eq!(
            repo.snapshot(false)["items"][0]["requestedReviewers"][0]["login"],
            "someone"
        );
        assert!(action(
            &repo,
            &json!({"id":"github:PR_test","action":"request-review","login":"bad\"input"})
        )
        .is_err());
        assert!(action(&repo, &json!({"id":"linear:wrong","action":"ready"})).is_err());
    }
    fn action_fixture(demo:bool)->(tempfile::TempDir,std::sync::Arc<Repository>){
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,demo).unwrap();
        repo.transaction(|state|{let mut item=pr();item["privateExtension"]=json!("private-item-sentinel");state["items"]=json!([item]);Ok(())}).unwrap();(temp,repo)
    }
    fn remote_pr()->Value {json!({"id":"PR_test","isDraft":false,"state":"OPEN","title":"Test PR","url":"https://github.com/test/repo/pull/1","repository":{"nameWithOwner":"test/repo"},"author":{"login":"author"},"reviewRequests":{"nodes":[{"requestedReviewer":{"__typename":"User","login":"existing"}},{"requestedReviewer":{"__typename":"User","login":"someone"}}]}})}
    #[test]
    fn action_receipts_report_preview_or_confirmed_cache_commit_without_private_items(){
        let (_temp,repo)=action_fixture(true);
        for operation in ["ready","request-review","refresh"]{
            let receipt=action_with(&repo,&json!({"id":"github:PR_test","action":operation,"login":"someone"}),|_,_,_|panic!("Demo must not contact GitHub")).unwrap();
            assert_eq!(receipt,json!({"id":"github:PR_test","action":operation,"status":"preview","remote":"not-contacted","localSaved":true}));
            assert!(!receipt.to_string().contains("private-item"));
        }
        let (_temp,repo)=action_fixture(false);let mut called=0;
        let receipt=action_with(&repo,&json!({"id":"github:PR_test","action":"request-review","login":"someone"}),|id,action,login|{called+=1;assert_eq!((id,action,login),("PR_test","request-review","someone"));Ok((remote_pr(),"viewer".into()))}).unwrap();
        assert_eq!(called,1);assert_eq!(receipt,json!({"id":"github:PR_test","action":"request-review","status":"completed","remote":"confirmed","localSaved":true}));
        let state=repo.state.lock().unwrap();assert_eq!(arr(&state["items"][0]["requestedReviewers"]).len(),2);assert!(state["items"][0]["reviewerUpdatedAt"].as_f64().unwrap()>0.0);assert!(repo.opening.lock().unwrap().is_empty());
    }
    #[test]
    fn confirmed_remote_result_with_failed_cache_save_is_not_reported_as_rolled_back(){
        let (_temp,repo)=action_fixture(false);let before=repo.state.lock().unwrap().clone();let disk=std::fs::read(&repo.data).unwrap();
        std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
        let error=action_with(&repo,&json!({"id":"github:PR_test","action":"ready"}),|_,_,_|Ok((remote_pr(),"viewer".into()))).unwrap_err();
        assert!(error.message.contains("any remote change may already be applied"));assert!(error.message.contains("Refresh the PR before retrying"));
        assert_eq!(error.data,json!({"githubReceipt":{"id":"github:PR_test","action":"ready","status":"remote-confirmed-local-failed","remote":"confirmed","localSaved":false}}));
        assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),disk);assert!(repo.opening.lock().unwrap().is_empty());
    }
    #[test]
    fn unconfirmed_remote_failure_preserves_refresh_before_retry_error_and_cache(){
        let (_temp,repo)=action_fixture(false);let before=repo.state.lock().unwrap().clone();
        let error=action_with(&repo,&json!({"id":"github:PR_test","action":"ready"}),|_,_,_|Err(Error::new("GitHub did not confirm this change. Refresh the PR before retrying."))).unwrap_err();
        assert!(error.message.contains("did not confirm"));assert_eq!(error.data,json!({}));assert_eq!(*repo.state.lock().unwrap(),before);assert!(repo.opening.lock().unwrap().is_empty());
    }

    #[test]
    fn post_dispatch_uncertainty_has_bounded_receipt_and_preflight_errors_do_not(){
        for dispatched in [false,true]{
            let (_temp,repo)=action_fixture(false);let before=repo.state.lock().unwrap().clone();
            let error=action_with(&repo,&json!({"id":"github:PR_test","action":"ready"}),|_,_,_|{
                let mut error=Error::new(if dispatched{"GitHub did not confirm this change. Refresh the PR before retrying."}else{"Your GitHub account cannot update this pull request."});
                if dispatched{error.data=json!({"githubActionStage":"dispatched","private":"private-provider-marker"});}Err(error)
            }).unwrap_err();
            assert_eq!(error.data,if dispatched{json!({"githubReceipt":{"id":"github:PR_test","action":"ready","status":"confirmation-unknown","remote":"unconfirmed","localSaved":false}})}else{json!({})});
            assert_eq!(*repo.state.lock().unwrap(),before);assert!(repo.opening.lock().unwrap().is_empty());
        }
    }

}
