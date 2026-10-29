use super::*;

fn linked_prs(state: &Value, id: &str) -> Result<Vec<Value>> {
    sid(id)?;
    let session=arr(&state["sessions"]).into_iter().find(|s| s["id"]==id)
        .ok_or(Error::new("Session unavailable."))?;
    let ids=arr(&session["items"]).iter().map(|i| text(i,"id").to_owned()).collect::<std::collections::BTreeSet<_>>();
    Ok(domain::catalog(state).into_values().filter(|i| ids.contains(text(i,"id")) && i["source"]=="github" && i["kind"]=="Pull request").collect())
}
const CI_FIELDS:[&str;8]=["ci","status","sourceStatus","githubState","mergeState","mergeable","isDraft","headRefOid"];
fn updates(state: &Value, fresh: &[Value]) -> Vec<Value> {
    let catalog=domain::catalog(state);
    fresh.iter().filter_map(|new| {
        let old=catalog.get(text(new,"id"))?;
        let mut next=old.clone();
        for field in CI_FIELDS {
            if let Some(value)=new.get(field){next[field]=value.clone();}
        }
        (next!=*old).then_some(next)
    }).collect()
}
fn guarded_updates(state:&Value,baseline:&[Value],fresh:&[Value])->(Vec<Value>,Vec<String>,Vec<String>){
    let catalog=domain::catalog(state);let mut accepted=vec![];let mut ids=vec![];let mut skipped=vec![];
    for item in fresh {
        let id=text(item,"id");
        let unchanged=baseline.iter().find(|old|old["id"]==id).zip(catalog.get(id)).is_some_and(|(old,current)|CI_FIELDS.iter().all(|field|old.get(field)==current.get(field)));
        if unchanged {accepted.push(item.clone());ids.push(id.to_owned());}else{skipped.push(id.to_owned());}
    }
    (updates(state,&accepted),ids,skipped)
}
fn apply(state: &mut Value, changed: &[Value]) {
    for new in changed {
        let id=text(new,"id");
        if let Some(row)=state["items"].as_array_mut().unwrap().iter_mut().find(|i| i["id"]==id) { *row=new.clone(); }
        state["itemArchive"][id]=new.clone();
        for session in state["sessions"].as_array_mut().unwrap() {
            for item in session["items"].as_array_mut().unwrap() {
                if item["id"]==id { *item=domain::item_ref(new); }
            }
        }
    }
    domain::observe(state,"github",changed);
    super::github::observe(state);
}
#[derive(serde::Serialize)]
#[serde(rename_all="kebab-case")]
enum SkipReason {Disabled,Backoff,AlreadyRunning,CiBackoff,Cooldown,NoLinkedPrs,Demo,StateChanged}
#[derive(serde::Serialize)]
#[serde(tag="status",rename_all="kebab-case")]
enum RefreshOutcome {
    Checked {#[serde(rename="checkedAt")]checked_at:f64},
    Skipped {reason:SkipReason,#[serde(rename="checkedAt")]checked_at:(),#[serde(rename="retryAt",skip_serializing_if="Option::is_none")]retry_at:Option<f64>},
}
#[derive(serde::Serialize)]
#[serde(rename_all="camelCase")]
struct RefreshReceipt {record_id:String,item_ids:Vec<String>,#[serde(skip_serializing_if="Vec::is_empty")]skipped_item_ids:Vec<String>,#[serde(flatten)]outcome:RefreshOutcome}
fn receipt(id:&str,ids:Vec<String>,outcome:RefreshOutcome)->Result<Value>{
    Ok(serde_json::to_value(RefreshReceipt{record_id:id.into(),item_ids:ids,skipped_item_ids:vec![],outcome})?)
}
fn skipped(id:&str,ids:Vec<String>,reason:SkipReason,retry_at:Option<f64>)->Result<Value>{
    receipt(id,ids,RefreshOutcome::Skipped{reason,checked_at:(),retry_at:retry_at.filter(|time|time.is_finite()&&*time>=0.0)})
}
fn pr_ids(refs:&[Value])->Result<Vec<String>>{
    if refs.len()>100{return Err(Error::new("Refresh CI for at most 100 linked PRs."));}
    refs.iter().map(|item|{let id=text(item,"id");if id.is_empty()||id.len()>200||id.chars().any(char::is_control){return Err(Error::new("Invalid linked PR identity."));}Ok(id.to_owned())}).collect()
}
pub fn refresh(repo:&Repository,id:&str)->Result<Value>{
    refresh_with(repo,id,|refs|repo.providers.refresh("github",refs))
}
fn refresh_with(repo:&Repository,id:&str,remote:impl FnOnce(&[Value])->Result<Vec<Value>>)->Result<Value>{
    let id=sid(id)?;let key=format!("session-ci:{id}");
    let (refs,ids)={
        // Admission is the read boundary. A later policy change does not cancel an admitted read.
        // Follow the repository state -> running -> cooldown lock order; no lock spans provider I/O.
        let state=repo.state.lock().map_err(|_|Error::io())?;
        let refs=linked_prs(&state,&id)?;let ids=pr_ids(&refs)?;
        if refs.is_empty(){return skipped(&id,ids,SkipReason::NoLinkedPrs,None);}
        if repo.demo{return skipped(&id,ids,SkipReason::Demo,None);}
        let running=repo.running.lock().map_err(|_|Error::io())?;let time=now();
        if let Some((reason,retry_at))=super::repository::source_skip(&state["sources"]["github"],running.contains("github"),true,time){
            let reason=match reason {"disabled"=>SkipReason::Disabled,"backoff"=>SkipReason::Backoff,"already-running"=>SkipReason::AlreadyRunning,_=>return Err(Error::new("Unsupported CI source admission result."))};
            return skipped(&id,ids,reason,retry_at);
        }
        let mut cooldown=repo.cooldown.lock().map_err(|_|Error::io())?;
        if let Some(retry)=cooldown.get("session-ci-backoff").copied().filter(|retry|*retry>time){return skipped(&id,ids,SkipReason::CiBackoff,Some(retry));}
        let mut opening=repo.opening.lock().map_err(|_|Error::io())?;
        if opening.contains(&key){return skipped(&id,ids,SkipReason::AlreadyRunning,None);}
        if let Some(retry)=cooldown.get(&key).copied().filter(|retry|*retry>time){return skipped(&id,ids,SkipReason::Cooldown,Some(retry));}
        opening.insert(key.clone());cooldown.insert(key.clone(),time+25.0);
        (refs,ids)
    };
    struct Guard<'a>(&'a Repository,String);
    impl Drop for Guard<'_>{fn drop(&mut self){self.0.opening.lock().unwrap().remove(&self.1);}}
    let _guard=Guard(repo,key);
    let fresh=match remote(&refs){
        Ok(items)=>items,
        Err(error)=>{repo.cooldown.lock().unwrap().insert("session-ci-backoff".into(),now()+error.retry.max(60.0));return Err(error);}
    };
    let fresh_ids=pr_ids(&fresh)?;
    if fresh_ids.iter().any(|id|!ids.contains(id)){return Err(Error::new("GitHub refresh returned an unexpected PR identity."));}
    let (accepted,discarded)=repo.transaction_if_changed(|state|{
        let (changed,accepted,discarded)=guarded_updates(state,&refs,&fresh);
        if !changed.is_empty(){apply(state,&changed);}
        Ok((accepted,discarded))
    })?;
    if accepted.is_empty()&&!discarded.is_empty(){return skipped(&id,ids,SkipReason::StateChanged,None);}
    Ok(serde_json::to_value(RefreshReceipt{record_id:id,item_ids:accepted,skipped_item_ids:discarded,outcome:RefreshOutcome::Checked{checked_at:now()}})?)
}

pub fn investigation(repo:&Repository,p:&Value)->Result<Value>{repo.transaction(|state|investigation_in_state(state,p))}
pub(super) fn investigation_in_state(state:&mut Value,p:&Value)->Result<Value>{
    let id=text(p,"id");let item=text(p,"itemId");let request=sid(text(p,"requestId"))?;

        if !linked_prs(state,id)?.iter().any(|pr|pr["id"]==item){return Err(Error::new("This PR is not linked to the chat."));}
        let session=state["sessions"].as_array_mut().unwrap().iter_mut().find(|s|s["id"]==id).unwrap();
        if !session["ciInvestigations"].is_object(){session["ciInvestigations"]=json!({});}
        let previous=&session["ciInvestigations"][item];
        let status=text(p,"state");
        if status=="queued" {
            if ["queued","investigating"].contains(&text(previous,"state")) && previous["id"]!=request {return Err(Error::new("An investigation is already active. Clear it before retrying."));}
            if previous["id"]!=request {session["ciInvestigations"][item]=json!({"id":request,"state":status,"queuedAt":now(),"updatedAt":now()});}
        } else {
            if previous["id"]!=request || !["queued","investigating"].contains(&text(previous,"state")){return Err(Error::new("This investigation is no longer active."));}
            if !["investigating","complete","failed","cancelled"].contains(&status){return Err(Error::new("Invalid investigation state."));}
            session["ciInvestigations"][item]["state"]=json!(status);
            session["ciInvestigations"][item]["updatedAt"]=json!(now());
        }
        Ok(json!({"id":id,"investigation":session["ciInvestigations"][item]}))
}

pub(super) fn cancel_dispatch(state:&mut Value,id:&str,item:&str,operation:&str)->Result<()> {
    let index=state["sessions"].as_array().unwrap().iter().position(|record|record["id"]==id).ok_or(Error::new("Session unavailable."))?;
    let progress=&mut state["sessions"][index]["ciInvestigations"][item];
    if progress["id"]==operation&&["queued","investigating"].contains(&text(progress,"state")){progress["state"]=json!("cancelled");progress["updatedAt"]=json!(now());}Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn investigation_progress_is_linked_and_request_guarded() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let id=uuid();let request=uuid();
        repo.transaction(|s|{s["items"]=json!([{"id":"github:a","source":"github","kind":"Pull request"}]);s["sessions"]=json!([{"id":id,"items":[{"id":"github:a"}]}]);Ok(())}).unwrap();
        let report=|request:&str,status:&str|investigation(&repo,&json!({"id":id,"itemId":"github:a","requestId":request,"state":status}));
        assert_eq!(report(&request,"queued").unwrap()["investigation"]["state"],"queued");
        assert!(report(&uuid(),"queued").is_err());assert!(report(&uuid(),"complete").is_err());
        assert_eq!(report(&request,"investigating").unwrap()["investigation"]["state"],"investigating");
        assert!(investigation(&repo,&json!({"id":id,"itemId":"github:unlinked","requestId":request,"state":"complete"})).is_err());
        assert_eq!(report(&request,"complete").unwrap()["investigation"]["state"],"complete");
        assert!(report(&request,"investigating").is_err());assert!(report(&uuid(),"queued").is_ok());
    }
    #[test]
    fn scope_and_updates_preserve_other_prs_relations_and_source_health() {
        let id=uuid();
        let pr=|key:&str|json!({"id":key,"source":"github","kind":"Pull request","title":"PR","status":"needs-review","relations":["authored"],"ci":{"state":"pending"}});
        let mut state=json!({"version":1,"sessions":[{"id":id,"items":[{"id":"github:a"},{"id":"linear:b"}]}],"items":[pr("github:a"),pr("github:other"),{"id":"linear:b","source":"linear","kind":"Issue"}],"sources":{"github":{"error":"Full sync failed"}}});
        domain::initialize(&mut state).unwrap();
        let linked=linked_prs(&state,&id).unwrap();assert_eq!(linked.len(),1);assert_eq!(linked[0]["id"],"github:a");
        let health=state["sources"].clone();
        let mut fresh=linked[0].clone();fresh["ci"]=json!({"state":"failed","checks":[{"name":"unit","state":"failed"}]});fresh["relations"]=json!(["linked"]);
        let changed=updates(&state,&[fresh.clone()]);assert_eq!(changed.len(),1);apply(&mut state,&changed);
        assert_eq!(state["items"][0]["ci"]["state"],"failed");assert_eq!(state["items"][0]["relations"],json!(["authored"]));assert_eq!(state["items"][1]["ci"]["state"],"pending");assert_eq!(state["sources"],health);
        assert!(updates(&state,&[fresh]).is_empty());
        assert!(linked_prs(&state,&uuid()).is_err());
    }
    #[test]
    fn demo_refresh_never_claims_a_provider_check_or_consumes_admission() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let id=uuid();
        repo.transaction(|s|{s["items"]=json!([{"id":"github:a","source":"github","kind":"Pull request","ci":{"state":"pending"}}]);s["sessions"]=json!([{"id":id,"items":[{"id":"github:a"}]}]);Ok(())}).unwrap();
        let before=repo.snapshot(false)["revision"].clone();
        assert_eq!(refresh(&repo,&id).unwrap()["reason"],"demo");
        assert!(refresh(&repo,&id).unwrap()["checkedAt"].is_null());
        assert!(repo.cooldown.lock().unwrap().is_empty());
        assert_eq!(repo.snapshot(false)["revision"],before);
    }    fn ci_fixture(demo:bool)->(tempfile::TempDir,std::sync::Arc<Repository>,String){
        let temp=tempfile::tempdir().unwrap();let mut repo=Repository::open(temp.path().into(),None,true).unwrap();std::sync::Arc::get_mut(&mut repo).unwrap().demo=demo;let id=uuid();
        repo.transaction(|s|{s["items"]=json!([{"id":"github:a","source":"github","kind":"Pull request","title":"PR","ci":{"state":"pending"},"privateExtension":"private-fixture-marker"}]);s["sessions"]=json!([{"id":id,"items":[{"id":"github:a"}]}]);s["sources"]["github"]=json!({"failures":0,"nextAttempt":now()+300.0,"error":null});Ok(())}).unwrap();(temp,repo,id)
    }
    #[test]
    fn ci_admission_skips_preserve_cache_and_all_reservations_without_provider_reads(){
        for reason in ["disabled","backoff","already-running","ci-backoff","cooldown","opening"]{
            let (_temp,repo,id)=ci_fixture(false);let retry=now()+3600.0;let key=format!("session-ci:{id}");
            match reason {"disabled"=>{repo.transaction(|s|{s["sources"]["github"]["disabled"]=json!(true);Ok(())}).unwrap();},"backoff"=>{repo.transaction(|s|{s["sources"]["github"]["failures"]=json!(2);s["sources"]["github"]["nextAttempt"]=json!(retry);Ok(())}).unwrap();},"already-running"=>{repo.running.lock().unwrap().insert("github".into());},"ci-backoff"=>{repo.cooldown.lock().unwrap().insert("session-ci-backoff".into(),retry);},"cooldown"=>{repo.cooldown.lock().unwrap().insert(key.clone(),retry);},"opening"=>{repo.opening.lock().unwrap().insert(key.clone());},_=>unreachable!()}
            let before=repo.state.lock().unwrap().clone();let disk=std::fs::read(&repo.data).unwrap();let cooldown=repo.cooldown.lock().unwrap().clone();let opening=repo.opening.lock().unwrap().clone();let running=repo.running.lock().unwrap().clone();
            let result=refresh_with(&repo,&id,|_|panic!("blocked admission must not read GitHub")).unwrap();assert_eq!(result["status"],"skipped");assert_eq!(result["checkedAt"],Value::Null);assert_eq!(result["reason"],if reason=="opening"{"already-running"}else{reason});assert_eq!(result["itemIds"],json!(["github:a"]));assert_eq!(result["recordId"],id);assert!(!result.to_string().contains("private"));
            if ["backoff","ci-backoff","cooldown"].contains(&reason){assert_eq!(result["retryAt"],retry);}else{assert!(result.get("retryAt").is_none());}
            assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),disk);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);assert_eq!(*repo.opening.lock().unwrap(),opening);assert_eq!(*repo.running.lock().unwrap(),running);
        }
    }
    #[test]
    fn ci_admission_bypasses_healthy_interval_releases_locks_and_returns_real_check_only(){
        let (_temp,repo,id)=ci_fixture(false);let health=repo.state.lock().unwrap()["sources"].clone();let before=repo.snapshot(false)["revision"].clone();
        let receipt=refresh_with(&repo,&id,|refs|{
            assert!(repo.state.try_lock().is_ok());assert!(repo.running.try_lock().is_ok());assert!(repo.cooldown.try_lock().is_ok());assert!(repo.opening.try_lock().is_ok());
            let times=repo.cooldown.lock().unwrap().clone();let duplicate=refresh_with(&repo,&id,|_|panic!("inflight request must not duplicate")).unwrap();assert_eq!(duplicate["reason"],"already-running");assert_eq!(*repo.cooldown.lock().unwrap(),times);
            Ok(refs.to_vec())
        }).unwrap();assert_eq!(receipt["status"],"checked");assert!(receipt["checkedAt"].is_number());assert_eq!(receipt.as_object().unwrap().len(),4);assert_eq!(receipt["itemIds"],json!(["github:a"]));assert!(repo.opening.lock().unwrap().is_empty());assert_eq!(repo.state.lock().unwrap()["sources"],health);assert_eq!(repo.snapshot(false)["revision"],before);
        assert_eq!(refresh_with(&repo,&id,|_|panic!("cooldown request must not duplicate")).unwrap()["reason"],"cooldown");
    }
    #[test]
    fn ci_admission_observes_policy_changed_while_waiting_and_failure_releases_reservation(){
        let (_temp,repo,id)=ci_fixture(false);let mut state=repo.state.lock().unwrap();let (ready,started)=std::sync::mpsc::channel();let worker_repo=repo.clone();let worker_id=id.clone();let worker=std::thread::spawn(move||{ready.send(()).unwrap();refresh_with(&worker_repo,&worker_id,|_|panic!("disabled before admission must not read"))});started.recv().unwrap();state["sources"]["github"]["disabled"]=json!(true);drop(state);assert_eq!(worker.join().unwrap().unwrap()["reason"],"disabled");assert!(repo.cooldown.lock().unwrap().is_empty());
        repo.transaction(|s|{s["sources"]["github"]["disabled"]=json!(false);Ok(())}).unwrap();let before=repo.state.lock().unwrap().clone();let disk=std::fs::read(&repo.data).unwrap();assert!(refresh_with(&repo,&id,|_|{let mut error=Error::new("Fixture GitHub read failed");error.retry=120.0;Err(error)}).is_err());assert!(repo.opening.lock().unwrap().is_empty());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),disk);
        let cooldown=repo.cooldown.lock().unwrap().clone();assert_eq!(refresh_with(&repo,&id,|_|panic!("CI backoff must prevent retry")).unwrap()["reason"],"ci-backoff");assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);
    }
    #[test]
    fn ci_receipts_bound_scope_and_demo_never_claims_a_check(){
        let (_temp,repo,id)=ci_fixture(true);let result=refresh(&repo,&id).unwrap();assert_eq!(result["reason"],"demo");assert_eq!(result["checkedAt"],Value::Null);assert!(repo.cooldown.lock().unwrap().is_empty());
        repo.transaction(|s|{s["sessions"][0]["items"]=json!([]);Ok(())}).unwrap();assert_eq!(refresh(&repo,&id).unwrap()["reason"],"no-linked-prs");
        repo.transaction(|s|{let items=(0..101).map(|n|json!({"id":format!("github:{n}"),"source":"github","kind":"Pull request"})).collect::<Vec<_>>();s["items"]=json!(items);s["sessions"][0]["items"]=s["items"].clone();Ok(())}).unwrap();assert!(refresh_with(&repo,&id,|_|panic!("oversized scope must not read")).is_err());assert!(repo.cooldown.lock().unwrap().is_empty());
    }
    #[test]
    fn ci_stale_provider_result_never_overwrites_newer_protected_fields_or_claims_freshness(){
        let (_temp,repo,id)=ci_fixture(false);let captured=std::sync::Mutex::new(None);
        let result=refresh_with(&repo,&id,|refs|{repo.transaction(|s|{s["items"][0]["ci"]=json!({"state":"passed"});s["items"][0]["status"]=json!("ready-to-merge");Ok(())})?;let state=repo.state.lock().unwrap().clone();let disk=std::fs::read(&repo.data).unwrap();let revision=repo.snapshot(false)["revision"].clone();*captured.lock().unwrap()=Some((state,disk,revision));let mut older=refs.to_vec();older[0]["ci"]=json!({"state":"failed"});Ok(older)}).unwrap();
        assert_eq!(result["status"],"skipped");assert_eq!(result["reason"],"state-changed");assert_eq!(result["checkedAt"],Value::Null);let expected=captured.into_inner().unwrap().unwrap();assert_eq!(*repo.state.lock().unwrap(),expected.0);assert_eq!(std::fs::read(&repo.data).unwrap(),expected.1);assert_eq!(repo.snapshot(false)["revision"],expected.2);
    }
    #[test]
    fn ci_partial_conflict_receipt_names_only_accepted_fresh_rows_and_preserves_unrelated_metadata(){
        let (_temp,repo,id)=ci_fixture(false);repo.transaction(|s|{let mut other=s["items"][0].clone();other["id"]=json!("github:b");s["items"].as_array_mut().unwrap().push(other);s["sessions"][0]["items"].as_array_mut().unwrap().push(json!({"id":"github:b"}));Ok(())}).unwrap();
        let result=refresh_with(&repo,&id,|refs|{repo.transaction(|s|{s["items"][0]["ci"]=json!({"state":"passed"});s["items"][1]["title"]=json!("Newer title");s["items"][1]["relations"]=json!(["authored"]);Ok(())})?;let mut fresh=refs.to_vec();for row in &mut fresh{row["ci"]=json!({"state":"failed"});}Ok(fresh)}).unwrap();
        assert_eq!(result["status"],"checked");assert!(result["checkedAt"].is_number());assert_eq!(result["itemIds"],json!(["github:b"]));assert_eq!(result["skippedItemIds"],json!(["github:a"]));let state=repo.state.lock().unwrap();assert_eq!(state["items"][0]["ci"]["state"],"passed");assert_eq!(state["items"][1]["ci"]["state"],"failed");assert_eq!(state["items"][1]["title"],"Newer title");assert_eq!(state["items"][1]["relations"],json!(["authored"]));
    }
    #[test]
    fn ci_head_identity_change_alone_discards_older_checks(){
        let (_temp,repo,id)=ci_fixture(false);repo.transaction(|s|{s["items"][0]["headRefOid"]=json!("head-before");Ok(())}).unwrap();
        let result=refresh_with(&repo,&id,|refs|{repo.transaction(|s|{s["items"][0]["headRefOid"]=json!("head-after");Ok(())})?;let mut older=refs.to_vec();older[0]["ci"]=json!({"state":"failed"});Ok(older)}).unwrap();assert_eq!(result["reason"],"state-changed");assert_eq!(result["checkedAt"],Value::Null);let state=repo.state.lock().unwrap();assert_eq!(state["items"][0]["headRefOid"],"head-after");assert_eq!(state["items"][0]["ci"]["state"],"pending");
    }

}
