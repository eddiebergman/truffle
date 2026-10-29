use super::*;

fn index(s: &Value, id: &str) -> Result<usize> {
    sid(id)?;
    arr(&s["sessions"])
        .iter()
        .position(|r| r["id"] == id)
        .ok_or(Error::new("Session unavailable."))
}
fn snapshot(s: &Value, id: &str) -> Result<Value> {
    let record = &s["sessions"][index(s, id)?];
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
    let record = &s["sessions"][index(&s, id)?];
    Ok(json!({"id":id,"items":record["items"],"context":snapshot(&s,id)?}))
}
/// Add relationships to the latest chat without resubmitting session metadata.
pub fn link_work(repo:&Repository,id:&str,item_ids:&Value)->Result<Value>{
    let id=sid(id)?;
    let ids=item_ids.as_array().filter(|ids|!ids.is_empty()&&ids.len()<=100).ok_or(Error::new("Choose 1 to 100 cached work IDs."))?;
    if ids.iter().any(|id|id.as_str().is_none_or(|id|id.trim().is_empty()||id.chars().count()>300||id.chars().any(char::is_control))){return Err(Error::new("Invalid work item ID."));}
    repo.transaction(|s|append_work(s,&id,ids,None))
}
fn append_work(s:&mut Value,id:&str,requested:&[Value],imported:Option<Value>)->Result<Value>{
    let position=index(s,id)?;
    let mut catalog=domain::catalog(s);
    if let Some(item)=imported {let key=text(&item,"id").to_owned();if !catalog.contains_key(&key){let mut items=arr(&s["items"]);items.push(item.clone());s["items"]=json!(items);catalog.insert(key,item);}}
    let mut items=arr(&s["sessions"][position]["items"]);let mut ids=vec![];let mut added=vec![];
    for item_id in requested {
        if ids.contains(item_id){continue;}
        let item=catalog.get(item_id.as_str().unwrap()).ok_or(Error::new("Register this work item before linking it."))?;
        ids.push(item_id.clone());
        if !items.iter().any(|item|item["id"]==*item_id){items.push(domain::item_ref(item));added.push(item_id.clone());}
    }
    if items.len()>100{return Err(Error::new("Choose up to 100 linked work items."));}
    s["sessions"][position]["items"]=json!(items);s["sessions"][position]["updatedAt"]=json!(now());
    Ok(json!({"recordId":id,"itemIds":ids,"addedItemIds":added,"revision":snapshot(s,id)?["revision"],"saved":true,"scope":"local"}))
}
pub fn link_work_url(repo:&Repository,id:&str,url:&str)->Result<Value>{
    link_work_url_with(repo,id,url,|url|{if repo.demo{return Err(Error::new("Sample workspace: provider work was not imported."));}repo.providers.resolve(url)})
}
fn link_work_url_with(repo:&Repository,id:&str,url:&str,resolve:impl FnOnce(&str)->Result<Value>)->Result<Value>{
    let id=sid(id)?;let url=safe_url(url)?;
    if url.len()>4096{return Err(Error::new("Work URL is too long."));}
    let parsed=reqwest::Url::parse(&url).map_err(|_|Error::new("Use a provider work URL."))?;
    let host=parsed.host_str().unwrap_or("");let parts=parsed.path().trim_matches('/').split('/').collect::<Vec<_>>();
    let supported=match host {
        "github.com"=>parts.len()==4&&["pull","issues"].contains(&parts[2])&&!parts[3].is_empty()&&parts[3].bytes().all(|c|c.is_ascii_digit()),
        "linear.app"=>parts.len()>=3&&parts[1]=="issue"&&regex::Regex::new(r"^[A-Za-z][A-Za-z0-9]*-[0-9]+$").unwrap().is_match(parts[2]),
        _ if host=="sentry.io"||host.ends_with(".sentry.io")=>parts.contains(&"issues")&&parts.last().is_some_and(|id|!id.is_empty()&&id.bytes().all(|c|c.is_ascii_digit())),
        _=>false,
    };
    if !supported||parsed.scheme()!="https"||parsed.port().is_some_and(|port|port!=443){return Err(Error::new("Use an HTTPS GitHub issue/PR, Linear issue, or Sentry issue URL."));}
    let cached={let s=repo.state.lock().map_err(|_|Error::io())?;index(&s,&id)?;domain::catalog(&s).into_values().find(|item|item["url"]==url)};
    let item=match cached {Some(item)=>item,None=>resolve(&url)?};
    let item_id=checked(&item,"id",300,true)?;
    // Provider reads run before acquiring the mutation transaction; merge against current links.
    repo.transaction(|s|append_work(s,&id,&[json!(item_id)],Some(item)))
}
fn list<'a>(input: &'a Value, key: &str, limit: usize) -> Result<Option<&'a Vec<Value>>> {
    match input.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_array()
            .filter(|v| v.len() <= limit)
            .map(Some)
            .ok_or(Error::new(format!(
                "{key} must be an array with at most {limit} entries."
            ))),
    }
}
pub fn replace(repo: &Repository, id: &str, input: &Value) -> Result<Value> {
    replace_result(repo, id, input).map(|(record, _)| record)
}
/// Identity of this exact mutation, captured before releasing the state transaction.
pub fn replace_receipt(repo: &Repository, id: &str, input: &Value) -> Result<Value> {
    replace_result(repo, id, input).map(|(_, revision)| json!({"id":id,"revision":revision}))
}
fn replace_result(repo: &Repository, id: &str, input: &Value) -> Result<(Value, Value)> {
    let object = input
        .as_object()
        .ok_or(Error::new("Context must be a JSON object."))?;
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
    let metadata = trees
        .map(|trees| {
            trees
                .iter()
                .map(|tree| {
                    let path = directory(text(tree, "path"))?;
                    let metadata = workflows::worktree_metadata(&path)
                        .ok_or(Error::new("Choose an existing Git checkout with a commit."))?;
                    if tree.get("primary").is_some_and(|v| !v.is_boolean()) {
                        return Err(Error::new("Worktree primary must be true or false."));
                    }
                    let ids = list(tree, "itemIds", 100)?.cloned().unwrap_or_default();
                    Ok((metadata, ids, tree["primary"] == true))
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    if let Some(trees) = &metadata {
        let mut paths = std::collections::HashSet::new();
        if trees.iter().any(|(m, _, _)| !paths.insert(text(m, "path")))
            || trees.iter().filter(|(_, _, p)| *p).count() > 1
        {
            return Err(Error::new(
                "Worktree paths must be unique, with at most one primary checkout.",
            ));
        }
    }
    repo.transaction(|s| {
        let position = index(s, id)?;
        if revision != snapshot(s, id)?["revision"].as_str().unwrap() {
            let mut error=Error::new("Session context changed. Read it again before replacing it.");
            error.status=409;
            return Err(error);
        }
        if let Some(ids) = item_ids {
            let current = arr(&s["sessions"][position]["items"]);
            let catalog = domain::catalog(s);
            let mut selected = vec![];
            for id in ids {
                let key = id.as_str().ok_or(Error::new("Invalid work item ID."))?;
                let item = current
                    .iter()
                    .find(|x| x["id"] == *id)
                    .cloned()
                    .or_else(|| catalog.get(key).map(domain::item_ref))
                    .ok_or(Error::new("Register this work item before linking it."))?;
                if !selected.iter().any(|x: &Value| x["id"] == *id) {
                    selected.push(item);
                }
            }
            s["sessions"][position]["items"] = json!(selected);
            for tree in s["sessions"][position]["worktrees"]
                .as_array_mut()
                .into_iter()
                .flatten()
            {
                tree["itemIds"] = json!(arr(&tree["itemIds"])
                    .into_iter()
                    .filter(|x| ids.contains(x))
                    .collect::<Vec<_>>());
            }
        }
        if let Some(entries) = resources {
            super::resources::replace_entries(s, id, &json!(entries))?;
        }
        if let Some(trees) = &metadata {
            s["sessions"][position]["worktrees"] = json!([]);
            for (meta, ids, primary) in trees {
                workflows::save_worktree(s, id, meta, ids, *primary)?;
            }
        }
        if let Some(ids) = artifact_ids {
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
        }
        s["sessions"][position]["updatedAt"] = json!(now());
        Ok((s["sessions"][position].clone(), snapshot(s, id)?["revision"].clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn fixture() -> (
        tempfile::TempDir,
        std::sync::Arc<Repository>,
        String,
        PathBuf,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            let result = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(result.status.success());
        };
        git(&["init", "-b", "main"]);
        std::fs::write(root.join("code.txt"), "base\n").unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ]);
        let next = temp.path().join("next");
        git(&["worktree", "add", "-b", "next", next.to_str().unwrap()]);
        let repo = Repository::open(temp.path().to_path_buf(), None, true).unwrap();
        let record = repo
            .transaction(|s| {
                workflows::save_session(
                    s,
                    &json!({"agent":"claude","title":"Context fixture","cwd":root,"itemIds":[]}),
                )
            })
            .unwrap();
        let id = text(&record, "id").to_owned();
        repo.transaction(|s|{
            s["sessions"][0]["items"]=json!([{"id":"one"},{"id":"two"}]);
            s["sessions"][0]["worktrees"]=json!([{"path":root,"primary":true,"itemIds":["one","two"]}]);
            s["artifacts"]=json!([{"id":"artifact-a","sessionIds":[id,"other"]},{"id":"artifact-b","sessionIds":[id]}]);
            resources::save(s,&id,&json!({"resources":[{"provider":"local","kind":"monitor","id":"old","state":"running","purpose":"Old monitor"}]}))?;Ok(())
        }).unwrap();
        (temp, repo, id, next.canonicalize().unwrap())
    }
    #[test]
    fn replacement_changes_only_supplied_fields_and_preserves_other_sessions_artifacts() {
        let (_temp, repo, id, _) = fixture();
        replace(
            &repo,
            &id,
            &json!({"revision":read(&repo,&id).unwrap()["context"]["revision"],"resources":[],"artifactIds":["artifact-a"]}),
        )
        .unwrap();
        let state = repo.state.lock().unwrap();
        assert!(arr(&state["sessions"][0]["resources"]).is_empty());
        assert_eq!(arr(&state["sessions"][0]["worktrees"]).len(), 1);
        assert_eq!(arr(&state["sessions"][0]["items"]).len(), 2);
        assert_eq!(state["artifacts"][0]["sessionIds"], json!(["other", id]));
        assert_eq!(state["artifacts"][1]["sessionIds"], json!([]));
    }
    #[test]
    fn worktree_replacement_updates_resume_path_and_prunes_removed_work_links() {
        let (_temp, repo, id, next) = fixture();
        replace(&repo,&id,&json!({"revision":read(&repo,&id).unwrap()["context"]["revision"],"worktrees":[{"path":next,"primary":true,"itemIds":["two"]}],"itemIds":["two"]})).unwrap();
        let state = repo.state.lock().unwrap();
        let record = &state["sessions"][0];
        assert_eq!(record["cwd"], json!(next));
        assert_eq!(arr(&record["worktrees"]).len(), 1);
        assert_eq!(record["worktrees"][0]["itemIds"], json!(["two"]));
        assert_eq!(record["items"], json!([{"id":"two"}]));
        assert_eq!(state["itemSettings"]["two"]["cwd"], json!(next));
        drop(state);
        replace(&repo, &id, &json!({"revision":read(&repo,&id).unwrap()["context"]["revision"],"itemIds":[]})).unwrap();
        assert_eq!(
            repo.state.lock().unwrap()["sessions"][0]["worktrees"][0]["itemIds"],
            json!([])
        );
    }
    #[test]
    fn invalid_or_stale_replacement_rolls_back_every_change() {
        let (_temp, repo, id, next) = fixture();
        let before = repo.state.lock().unwrap().clone();
        for mut input in [
            json!({"resources":[],"worktrees":[{"path":next,"itemIds":["unknown"]}]}),
            json!({"resources":null}),
            json!({"worktrees":[{"path":next,"primary":true},{"path":next,"primary":true}]}),
            json!({"artifactIds":["missing"],"resources":[]}),
        ] {
            input["revision"]=read(&repo,&id).unwrap()["context"]["revision"].clone();
            let error=replace(&repo, &id, &input).unwrap_err();assert_ne!(error.status,409,"Malformed input must reach its own validation");
            assert_eq!(*repo.state.lock().unwrap(), before);
        }
        let old = read(&repo, &id).unwrap()["context"].clone();
        replace(&repo, &id, &json!({"revision":old["revision"],"itemIds":["one"]})).unwrap();
        assert!(replace(&repo, &id, &old)
            .unwrap_err()
            .message
            .contains("changed"));
    }
    #[test]
    fn replacement_requires_captured_revision_and_preserves_state_on_missing_invalid_or_stale_input(){
        let (_temp,repo,id,_)=fixture();let bytes=std::fs::read(&repo.data).unwrap();let state=repo.state.lock().unwrap().clone();
        for revision in [None,Some(Value::Null),Some(json!(1)),Some(json!("")),Some(json!("a".repeat(63))),Some(json!("g".repeat(64))),Some(json!("0".repeat(64)))]{
            let mut input=json!({"resources":[]});if let Some(revision)=revision{input["revision"]=revision;}
            assert_eq!(replace_receipt(&repo,&id,&input).unwrap_err().status,409);
            assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(*repo.state.lock().unwrap(),state);
        }
        assert_eq!(workflows::register(&repo,&json!({"operation":"context-replace","id":id,"context":{"resources":[]}})).unwrap_err().status,409);
        let captured=read(&repo,&id).unwrap()["context"].clone();
        let result=workflows::register(&repo,&json!({"operation":"context-replace","id":id,"context":{"revision":captured["revision"],"resources":[]}})).unwrap();
        assert_eq!(result["id"],id);assert!(arr(&result["resources"]).is_empty());
    }
    #[test]
    fn mutation_receipt_binds_undo_to_exact_commit_and_excludes_private_record_data() {
        let (_temp, repo, id, _) = fixture();
        repo.transaction(|s| {s["sessions"][0]["privateExtension"]=json!("private-receipt-marker");Ok(())}).unwrap();
        let before=read(&repo,&id).unwrap()["context"].clone();
        let receipt=replace_receipt(&repo,&id,&json!({"revision":before["revision"],"resources":[]})).unwrap();
        assert_eq!(receipt.as_object().unwrap().len(),2);
        assert_eq!(receipt["id"],id);
        assert_eq!(receipt["revision"],read(&repo,&id).unwrap()["context"]["revision"]);
        assert!(!receipt.to_string().contains("private-receipt-marker"));
        repo.transaction(|s| resources::save(s,&id,&json!({"resources":[{"provider":"local","kind":"monitor","id":"concurrent","state":"running"}]}))).unwrap();
        let bytes=std::fs::read(&repo.data).unwrap();
        let error=replace_receipt(&repo,&id,&json!({"revision":receipt["revision"],"resources":before["resources"]})).unwrap_err();
        assert_eq!(error.status,409);
        assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);
        assert_eq!(read(&repo,&id).unwrap()["context"]["resources"][0]["id"],"concurrent");
        let fresh=read(&repo,&id).unwrap()["context"].clone();
        let restored=replace_receipt(&repo,&id,&json!({"revision":fresh["revision"],"resources":before["resources"]})).unwrap();
        assert_eq!(restored["revision"],read(&repo,&id).unwrap()["context"]["revision"]);
        assert_eq!(read(&repo,&id).unwrap()["context"]["resources"],before["resources"]);
    }
    #[test]
    fn resource_replacement_drops_stale_metadata_instead_of_merging_it() {
        let (_temp, repo, id, _) = fixture();
        resources::apply(&repo,&id,&json!({"replace":true,"revision":read(&repo,&id).unwrap()["context"]["revision"],"resources":[{"provider":"local","kind":"monitor","id":"old","state":"completed"}]})).unwrap();
        let record = read(&repo, &id).unwrap();
        assert_eq!(record["context"]["resources"][0]["purpose"], "");
        assert_eq!(record["context"]["resources"][0]["state"], "completed");
        let reply = workflows::registration_reply(&record);
        assert_eq!(reply["context"], record["context"]);
    }    #[test]
    fn link_work_adds_latest_relationships_only_with_exact_receipt_and_atomic_rollback(){
        let (_temp,repo,id,_)=fixture();
        repo.transaction(|s|{s["items"]=json!([{"id":"three","source":"linear","title":"Third","detail":"private-detail"}]);s["itemSettings"]["three"]=json!({"cwd":"unchanged"});s["sessions"][0]["title"]=json!("Newer title");s["sessions"][0]["instruction"]=json!("private-instructions");Ok(())}).unwrap();
        let before=repo.state.lock().unwrap().clone();let receipt=link_work(&repo,&id,&json!(["three","three","one"])).unwrap();let after=repo.state.lock().unwrap().clone();
        let mut expected=before["sessions"][0].clone();expected["items"]=after["sessions"][0]["items"].clone();expected["updatedAt"]=after["sessions"][0]["updatedAt"].clone();assert_eq!(expected,after["sessions"][0]);assert_eq!(before["itemSettings"],after["itemSettings"]);assert_eq!(before["items"],after["items"]);assert_eq!(after["sessions"][0]["items"].as_array().unwrap().len(),3);
        assert_eq!(receipt["addedItemIds"],json!(["three"]));assert_eq!(receipt["itemIds"],json!(["three","one"]));assert_eq!(receipt["revision"],read(&repo,&id).unwrap()["context"]["revision"]);assert_eq!(receipt.as_object().unwrap().len(),6);assert!(!receipt.to_string().contains("private"));
        assert_eq!(link_work(&repo,&id,&json!(["three"])).unwrap()["addedItemIds"],json!([]));
        for ids in [json!([]),json!(["unknown"]),json!([3]),json!([" "]),json!(["one\n"]),json!(vec!["one";101])]{let before=repo.state.lock().unwrap().clone();let disk=fs::read(&repo.data).unwrap();assert!(link_work(&repo,&id,&ids).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(fs::read(&repo.data).unwrap(),disk);}
        let before=repo.state.lock().unwrap().clone();fs::create_dir(repo.data.with_extension("tmp")).unwrap();assert!(link_work(&repo,&id,&json!(["one"])).is_err());assert_eq!(*repo.state.lock().unwrap(),before);
    }
    #[test]
    fn link_work_url_reads_outside_lock_merges_latest_and_rejects_before_resolution(){
        let (_temp,repo,id,_)=fixture();let url="https://github.com/example/project/issues/17";
        let receipt=link_work_url_with(&repo,&id,url,|_|{assert!(repo.state.try_lock().is_ok());repo.transaction(|s|{s["sessions"][0]["title"]=json!("Changed during provider read");s["sessions"][0]["items"].as_array_mut().unwrap().push(json!({"id":"concurrent"}));Ok(())})?;Ok(json!({"id":"github:17","url":url,"source":"github","title":"Imported","detail":"private-provider-detail"}))}).unwrap();
        assert_eq!(receipt["addedItemIds"],json!(["github:17"]));let state=repo.state.lock().unwrap().clone();assert_eq!(state["sessions"][0]["title"],"Changed during provider read");assert_eq!(arr(&state["sessions"][0]["items"]).len(),4);assert_eq!(receipt["revision"],read(&repo,&id).unwrap()["context"]["revision"]);
        assert_eq!(link_work_url_with(&repo,&id,url,|_|panic!("cached URL must not resolve")).unwrap()["addedItemIds"],json!([]));
        for url in ["http://github.com/example/project/issues/17","https://example.invalid/17","https://user:password@github.com/example/project/issues/17","https://github.com/example/project/issues/","https://github.com:444/example/project/issues/17"]{assert!(link_work_url_with(&repo,&id,url,|_|panic!("invalid URL must not resolve")).is_err());}
        assert!(link_work_url_with(&repo,&uuid(),url,|_|panic!("unknown chat must not resolve")).is_err());
        let before=repo.state.lock().unwrap().clone();assert!(link_work_url_with(&repo,&id,"https://linear.app/team/issue/TEAM-12",|_|Err(Error::new("Fixture read failed"))).is_err());assert_eq!(*repo.state.lock().unwrap(),before);
        repo.transaction(|s|{s["sessions"][0]["items"]=json!((0..100).map(|n|json!({"id":format!("cached:{n}")})).collect::<Vec<_>>());Ok(())}).unwrap();let before=repo.state.lock().unwrap().clone();assert!(link_work_url_with(&repo,&id,"https://linear.app/team/issue/TEAM-12",|_|Ok(json!({"id":"new","url":"https://linear.app/team/issue/TEAM-12"}))).is_err());assert_eq!(*repo.state.lock().unwrap(),before);
    }

}
